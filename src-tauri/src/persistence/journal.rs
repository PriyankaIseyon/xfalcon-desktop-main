use std::{
    fmt,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::{
    crypto::{self, AuthenticatedMetadata},
    SecretStore,
};
use crate::domain::error::{AppError, AppResult};

const COMMAND_QUEUE_CAPACITY: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const MAX_IDENTIFIER_BYTES: usize = 160;
const MAX_EVENT_TYPE_BYTES: usize = 96;
const MAX_STATE_BYTES: usize = 64;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const REPLAY_PAGE_SIZE: usize = 512;
const MIGRATION: &str = include_str!("migrations/0001_journal.sql");
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendOutcome {
    Inserted,
    Duplicate,
    CapacityReached,
}

pub struct NewJournalEvent {
    pub account_id: String,
    pub event_id: String,
    pub run_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub state: String,
    pub payload: Vec<u8>,
    pub created_at_ms: i64,
}

pub struct JournalEvent {
    pub account_id: String,
    pub event_id: String,
    pub run_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub state: String,
    pub payload: Vec<u8>,
    pub created_at_ms: i64,
}

impl fmt::Debug for JournalEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalEvent")
            .field("account_id", &self.account_id)
            .field("event_id", &self.event_id)
            .field("run_id", &self.run_id)
            .field("sequence", &self.sequence)
            .field("event_type", &self.event_type)
            .field("state", &self.state)
            .field("payload", &"[REDACTED]")
            .field("created_at_ms", &self.created_at_ms)
            .finish()
    }
}

#[derive(Debug)]
pub struct ReplayPage {
    events: Vec<JournalEvent>,
    next_sequence: Option<u64>,
}

impl ReplayPage {
    #[must_use]
    pub fn into_events(self) -> Vec<JournalEvent> {
        self.events
    }

    #[must_use]
    pub const fn next_sequence(&self) -> Option<u64> {
        self.next_sequence
    }

    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.next_sequence.is_some()
    }
}

impl Deref for ReplayPage {
    type Target = [JournalEvent];

    fn deref(&self) -> &Self::Target {
        &self.events
    }
}

pub struct Journal {
    commands: mpsc::SyncSender<Command>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Journal {
    pub fn open(path: impl AsRef<Path>, secrets: Arc<dyn SecretStore>) -> AppResult<Self> {
        let path = path.as_ref().to_path_buf();
        let (commands, receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("xfalcon-persistence".to_owned())
            .spawn(move || worker_main(path, secrets, receiver, ready_tx))
            .map_err(AppError::PersistenceStartup)?;

        match ready_rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                commands,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                drop(worker);
                Err(AppError::PersistenceUnavailable)
            }
        }
    }

    pub fn append(&self, event: NewJournalEvent) -> AppResult<AppendOutcome> {
        self.append_inner(event, None)
    }

    pub fn append_bounded(
        &self,
        event: NewJournalEvent,
        maximum_events: usize,
    ) -> AppResult<AppendOutcome> {
        if maximum_events == 0 {
            return Err(AppError::PersistenceInvalid(
                "maximum_events must be greater than zero",
            ));
        }
        self.append_inner(event, Some(maximum_events))
    }

    fn append_inner(
        &self,
        event: NewJournalEvent,
        maximum_events: Option<usize>,
    ) -> AppResult<AppendOutcome> {
        validate_event(&event)?;
        let (reply, response) = mpsc::sync_channel(1);
        send_command(
            &self.commands,
            Command::Append {
                event,
                maximum_events,
                reply,
            },
        )?;
        response
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| AppError::PersistenceUnavailable)?
    }

    pub fn events_after(
        &self,
        account_id: &str,
        run_id: &str,
        sequence: u64,
    ) -> AppResult<ReplayPage> {
        validate_token(account_id, MAX_IDENTIFIER_BYTES, "account_id is invalid")?;
        validate_token(run_id, MAX_IDENTIFIER_BYTES, "run_id is invalid")?;
        let (reply, response) = mpsc::sync_channel(1);
        send_command(
            &self.commands,
            Command::Replay {
                account_id: account_id.to_owned(),
                run_id: run_id.to_owned(),
                sequence,
                reply,
            },
        )?;
        response
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| AppError::PersistenceUnavailable)?
    }

    pub fn purge_account(&self, account_id: &str) -> AppResult<usize> {
        validate_token(account_id, MAX_IDENTIFIER_BYTES, "account_id is invalid")?;
        let (reply, response) = mpsc::sync_channel(1);
        send_command(
            &self.commands,
            Command::Purge {
                account_id: account_id.to_owned(),
                reply,
            },
        )?;
        response
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| AppError::PersistenceUnavailable)?
    }

    pub fn delete_event(&self, account_id: &str, run_id: &str, event_id: &str) -> AppResult<usize> {
        validate_token(account_id, MAX_IDENTIFIER_BYTES, "account_id is invalid")?;
        validate_token(run_id, MAX_IDENTIFIER_BYTES, "run_id is invalid")?;
        validate_token(event_id, MAX_IDENTIFIER_BYTES, "event_id is invalid")?;
        let (reply, response) = mpsc::sync_channel(1);
        send_command(
            &self.commands,
            Command::DeleteEvent {
                account_id: account_id.to_owned(),
                run_id: run_id.to_owned(),
                event_id: event_id.to_owned(),
                reply,
            },
        )?;
        response
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| AppError::PersistenceUnavailable)?
    }

    pub fn purge_run(&self, account_id: &str, run_id: &str) -> AppResult<usize> {
        validate_token(account_id, MAX_IDENTIFIER_BYTES, "account_id is invalid")?;
        validate_token(run_id, MAX_IDENTIFIER_BYTES, "run_id is invalid")?;
        let (reply, response) = mpsc::sync_channel(1);
        send_command(
            &self.commands,
            Command::PurgeRun {
                account_id: account_id.to_owned(),
                run_id: run_id.to_owned(),
                reply,
            },
        )?;
        response
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| AppError::PersistenceUnavailable)?
    }

    pub fn close(mut self) -> AppResult<()> {
        let (reply, response) = mpsc::sync_channel(1);
        send_command(&self.commands, Command::Shutdown { reply })?;
        response
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| AppError::PersistenceUnavailable)?;
        let worker = self.worker.take().ok_or(AppError::PersistenceUnavailable)?;
        worker.join().map_err(|_| AppError::PersistenceUnavailable)
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        let (reply, _response) = mpsc::sync_channel(1);
        let shutdown_requested = self.commands.try_send(Command::Shutdown { reply }).is_ok();
        if let Some(worker) = self.worker.take() {
            if shutdown_requested {
                let deadline = Instant::now() + SHUTDOWN_GRACE;
                while !worker.is_finished() && Instant::now() < deadline {
                    thread::yield_now();
                }
                if worker.is_finished() {
                    let _ = worker.join();
                }
            }
        }
    }
}

fn send_command(sender: &mpsc::SyncSender<Command>, command: Command) -> AppResult<()> {
    sender.try_send(command).map_err(|error| match error {
        mpsc::TrySendError::Full(_) => AppError::PersistenceBusy,
        mpsc::TrySendError::Disconnected(_) => AppError::PersistenceUnavailable,
    })
}

enum Command {
    Append {
        event: NewJournalEvent,
        maximum_events: Option<usize>,
        reply: mpsc::SyncSender<AppResult<AppendOutcome>>,
    },
    Replay {
        account_id: String,
        run_id: String,
        sequence: u64,
        reply: mpsc::SyncSender<AppResult<ReplayPage>>,
    },
    Purge {
        account_id: String,
        reply: mpsc::SyncSender<AppResult<usize>>,
    },
    DeleteEvent {
        account_id: String,
        run_id: String,
        event_id: String,
        reply: mpsc::SyncSender<AppResult<usize>>,
    },
    PurgeRun {
        account_id: String,
        run_id: String,
        reply: mpsc::SyncSender<AppResult<usize>>,
    },
    Shutdown {
        reply: mpsc::SyncSender<()>,
    },
}

fn worker_main(
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    commands: mpsc::Receiver<Command>,
    ready: mpsc::SyncSender<AppResult<()>>,
) {
    let connection = match open_connection(&path) {
        Ok(connection) => connection,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    while let Ok(command) = commands.recv() {
        match command {
            Command::Append {
                event,
                maximum_events,
                reply,
            } => {
                let _ = reply.send(append_event(
                    &connection,
                    secrets.as_ref(),
                    event,
                    maximum_events,
                ));
            }
            Command::Replay {
                account_id,
                run_id,
                sequence,
                reply,
            } => {
                let _ = reply.send(replay_events(
                    &connection,
                    secrets.as_ref(),
                    &account_id,
                    &run_id,
                    sequence,
                ));
            }
            Command::Purge { account_id, reply } => {
                let _ = reply.send(purge_account(&connection, &account_id));
            }
            Command::DeleteEvent {
                account_id,
                run_id,
                event_id,
                reply,
            } => {
                let _ = reply.send(delete_event(&connection, &account_id, &run_id, &event_id));
            }
            Command::PurgeRun {
                account_id,
                run_id,
                reply,
            } => {
                let _ = reply.send(purge_run(&connection, &account_id, &run_id));
            }
            Command::Shutdown { reply } => {
                let _ = reply.send(());
                break;
            }
        }
    }
}

fn open_connection(path: &Path) -> AppResult<Connection> {
    super::open_local_sqlite_store(path, super::LocalSqliteSynchronous::Normal, migrate)
}

fn migrate(connection: &mut Connection) -> AppResult<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == SCHEMA_VERSION {
        return Ok(());
    }

    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    transaction.execute_batch(MIGRATION)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn append_event(
    connection: &Connection,
    secrets: &dyn SecretStore,
    event: NewJournalEvent,
    maximum_events: Option<usize>,
) -> AppResult<AppendOutcome> {
    let existing = connection
        .query_row(
            "SELECT 1 FROM journal_events WHERE account_id = ?1 AND event_id = ?2",
            params![event.account_id, event.event_id],
            |_| Ok(()),
        )
        .optional()?;
    if existing.is_some() {
        return Ok(AppendOutcome::Duplicate);
    }
    if let Some(maximum_events) = maximum_events {
        let event_count: i64 = connection.query_row(
            "SELECT count(*) FROM journal_events WHERE account_id = ?1 AND run_id = ?2",
            params![event.account_id, event.run_id],
            |row| row.get(0),
        )?;
        let maximum_events = i64::try_from(maximum_events)
            .map_err(|_| AppError::PersistenceInvalid("maximum_events exceeds SQLite range"))?;
        if event_count >= maximum_events {
            return Ok(AppendOutcome::CapacityReached);
        }
    }

    // Mint-on-first-use — see `chat::local_projection::seal`: a seal that only
    // reads the active key fails on a device that has never sealed before.
    let (key_version, key) = secrets.load_or_create_active_key(&event.account_id)?;
    let metadata = AuthenticatedMetadata {
        account_id: &event.account_id,
        event_id: &event.event_id,
        run_id: &event.run_id,
        sequence: event.sequence,
        event_type: &event.event_type,
        state: &event.state,
        created_at_ms: event.created_at_ms,
    };
    let encrypted = crypto::encrypt(&key, key_version, &metadata, &event.payload)?;
    let sequence = i64::try_from(event.sequence)
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    let aad_schema_version = i64::from(encrypted.aad_schema_version);
    let key_version = i64::from(key_version);
    let inserted = connection.execute(
        "INSERT INTO journal_events (
            account_id, event_id, run_id, sequence, event_type, state,
            aad_schema_version, key_version, nonce, ciphertext, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(account_id, event_id) DO NOTHING",
        params![
            event.account_id,
            event.event_id,
            event.run_id,
            sequence,
            event.event_type,
            event.state,
            aad_schema_version,
            key_version,
            encrypted.nonce,
            encrypted.ciphertext,
            event.created_at_ms,
        ],
    )?;

    Ok(if inserted == 1 {
        AppendOutcome::Inserted
    } else {
        AppendOutcome::Duplicate
    })
}

fn replay_events(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    run_id: &str,
    after_sequence: u64,
) -> AppResult<ReplayPage> {
    let after_sequence = i64::try_from(after_sequence)
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    let mut statement = connection.prepare(
        "SELECT event_id, sequence, event_type, state, aad_schema_version,
                key_version, nonce, ciphertext, created_at_ms
         FROM journal_events
         WHERE account_id = ?1 AND run_id = ?2 AND sequence > ?3
         ORDER BY sequence ASC, event_id ASC
         LIMIT ?4",
    )?;
    let rows = statement.query_map(
        params![
            account_id,
            run_id,
            after_sequence,
            i64::try_from(REPLAY_PAGE_SIZE + 1).map_err(|_| AppError::PersistenceInvalid(
                "replay page size exceeds SQLite range"
            ))?
        ],
        |row| {
            Ok(EncryptedRow {
                event_id: row.get(0)?,
                sequence: row.get(1)?,
                event_type: row.get(2)?,
                state: row.get(3)?,
                aad_schema_version: row.get(4)?,
                key_version: row.get(5)?,
                nonce: row.get(6)?,
                ciphertext: row.get(7)?,
                created_at_ms: row.get(8)?,
            })
        },
    )?;
    let mut events = Vec::new();
    for row in rows {
        let row = row?;
        let sequence =
            u64::try_from(row.sequence).map_err(|_| AppError::PersistenceDecryptionFailed)?;
        let key_version =
            u32::try_from(row.key_version).map_err(|_| AppError::PersistenceDecryptionFailed)?;
        let aad_schema_version = u32::try_from(row.aad_schema_version)
            .map_err(|_| AppError::PersistenceDecryptionFailed)?;
        let key = secrets.account_key(account_id, key_version)?;
        let metadata = AuthenticatedMetadata {
            account_id,
            event_id: &row.event_id,
            run_id,
            sequence,
            event_type: &row.event_type,
            state: &row.state,
            created_at_ms: row.created_at_ms,
        };
        let payload = crypto::decrypt(
            &key,
            aad_schema_version,
            key_version,
            &row.nonce,
            &metadata,
            &row.ciphertext,
        )?;
        events.push(JournalEvent {
            account_id: account_id.to_owned(),
            event_id: row.event_id,
            run_id: run_id.to_owned(),
            sequence,
            event_type: row.event_type,
            state: row.state,
            payload,
            created_at_ms: row.created_at_ms,
        });
    }
    let next_sequence = if events.len() > REPLAY_PAGE_SIZE {
        events.pop();
        events.last().map(|event| event.sequence)
    } else {
        None
    };
    Ok(ReplayPage {
        events,
        next_sequence,
    })
}

fn purge_account(connection: &Connection, account_id: &str) -> AppResult<usize> {
    connection
        .execute(
            "DELETE FROM journal_events WHERE account_id = ?1",
            [account_id],
        )
        .map_err(Into::into)
}

fn delete_event(
    connection: &Connection,
    account_id: &str,
    run_id: &str,
    event_id: &str,
) -> AppResult<usize> {
    connection
        .execute(
            "DELETE FROM journal_events
             WHERE account_id = ?1 AND run_id = ?2 AND event_id = ?3",
            params![account_id, run_id, event_id],
        )
        .map_err(Into::into)
}

fn purge_run(connection: &Connection, account_id: &str, run_id: &str) -> AppResult<usize> {
    connection
        .execute(
            "DELETE FROM journal_events WHERE account_id = ?1 AND run_id = ?2",
            params![account_id, run_id],
        )
        .map_err(Into::into)
}

struct EncryptedRow {
    event_id: String,
    sequence: i64,
    event_type: String,
    state: String,
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    created_at_ms: i64,
}

fn validate_event(event: &NewJournalEvent) -> AppResult<()> {
    validate_token(
        &event.account_id,
        MAX_IDENTIFIER_BYTES,
        "account_id is invalid",
    )?;
    validate_token(&event.event_id, MAX_IDENTIFIER_BYTES, "event_id is invalid")?;
    validate_token(&event.run_id, MAX_IDENTIFIER_BYTES, "run_id is invalid")?;
    validate_token(
        &event.event_type,
        MAX_EVENT_TYPE_BYTES,
        "event_type is invalid",
    )?;
    validate_token(&event.state, MAX_STATE_BYTES, "state is invalid")?;
    if event.payload.len() > MAX_PAYLOAD_BYTES {
        return Err(AppError::PersistenceInvalid(
            "payload exceeds the encrypted journal limit",
        ));
    }
    if event.sequence == 0 {
        return Err(AppError::PersistenceInvalid(
            "sequence must be greater than zero",
        ));
    }
    i64::try_from(event.sequence)
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    Ok(())
}

fn validate_token(value: &str, maximum_bytes: usize, message: &'static str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > maximum_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::PersistenceInvalid(message));
    }
    Ok(())
}
