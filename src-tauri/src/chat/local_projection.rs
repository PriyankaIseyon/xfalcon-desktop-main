use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use xfalcon_contracts::{ConversationId, RunId, RunState};

use crate::domain::error::{AppError, AppResult};
use crate::persistence::{
    decrypt_projection, encrypt_projection, ProjectionAuthenticatedMetadata, SecretStore,
    CHAT_PROJECTION_MIGRATION, CHAT_PROJECTION_SCHEMA_VERSION,
};
use crate::security::account_cleanup::ValidatedAccountId;

use super::domain::{
    AssistantDraft, ChatError, ChatProjectionEvent, ChatSnapshot, Conversation, Message,
    MessageContent, MessageId, MessageRole, RunRecord, SyncCursor, ToolCallRef,
};
use super::repository::LocalChatProjection;
use super::title::TitleSource;

pub const LOCAL_PROJECTIONS_DIR: &str = "local-projections";
pub const CHAT_PROJECTION_DB_FILE: &str = "chat_projection.sqlite3";

pub struct EncryptedChatProjection {
    connection: Mutex<Connection>,
    secrets: Arc<dyn SecretStore>,
    account: ValidatedAccountId,
    #[allow(dead_code)]
    path: PathBuf,
}

impl EncryptedChatProjection {
    pub fn open(
        path: impl AsRef<Path>,
        secrets: Arc<dyn SecretStore>,
        account: ValidatedAccountId,
    ) -> Result<Self, ChatError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| ChatError::Unavailable)?;
        }
        let connection = open_connection(&path).map_err(map_persist)?;
        Ok(Self {
            connection: Mutex::new(connection),
            secrets,
            account,
            path,
        })
    }

    #[must_use]
    pub fn account(&self) -> &ValidatedAccountId {
        &self.account
    }

    pub(crate) fn title_source(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<TitleSource, ChatError> {
        let connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        load_title_authority(
            &connection,
            self.secrets.as_ref(),
            self.account.as_str(),
            conversation_id,
        )
        .map(|authority| {
            authority
                .map(|authority| authority.source)
                .unwrap_or(TitleSource::User)
        })
        .map_err(map_persist)
    }

    pub(crate) fn record_new_title_source(
        &self,
        conversation_id: &ConversationId,
        source: TitleSource,
    ) -> Result<bool, ChatError> {
        let mut connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sql)?;
        let changed = write_title_authority(
            &transaction,
            self.secrets.as_ref(),
            self.account.as_str(),
            conversation_id,
            TitleAuthorityPayload {
                source,
                upgrade_started: source != TitleSource::Derived,
            },
            true,
        )
        .map_err(map_persist)?;
        transaction.commit().map_err(map_sql)?;
        Ok(changed)
    }

    pub(crate) fn advance_title_source(
        &self,
        conversation_id: &ConversationId,
        source: TitleSource,
    ) -> Result<bool, ChatError> {
        let mut connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        let transaction = connection.transaction().map_err(map_sql)?;
        let current = load_title_authority(
            &transaction,
            self.secrets.as_ref(),
            self.account.as_str(),
            conversation_id,
        )
        .map_err(map_persist)?
        .unwrap_or(TitleAuthorityPayload {
            source: TitleSource::User,
            upgrade_started: true,
        });
        if source < current.source {
            return Ok(false);
        }
        write_title_authority(
            &transaction,
            self.secrets.as_ref(),
            self.account.as_str(),
            conversation_id,
            TitleAuthorityPayload {
                source,
                upgrade_started: current.upgrade_started || source != TitleSource::Derived,
            },
            false,
        )
        .map_err(map_persist)?;
        transaction.commit().map_err(map_sql)?;
        Ok(true)
    }

    pub(crate) fn begin_title_upgrade(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<bool, ChatError> {
        let mut connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        let transaction = connection.transaction().map_err(map_sql)?;
        let Some(mut authority) = load_title_authority(
            &transaction,
            self.secrets.as_ref(),
            self.account.as_str(),
            conversation_id,
        )
        .map_err(map_persist)?
        else {
            return Ok(false);
        };
        if authority.source != TitleSource::Derived || authority.upgrade_started {
            return Ok(false);
        }
        authority.upgrade_started = true;
        write_title_authority(
            &transaction,
            self.secrets.as_ref(),
            self.account.as_str(),
            conversation_id,
            authority,
            false,
        )
        .map_err(map_persist)?;
        transaction.commit().map_err(map_sql)?;
        Ok(true)
    }

    /// Decrypts sealed message bodies in memory, matches, and forgets them.
    /// Nothing is written: a plaintext index beside the sealed store would
    /// undo the encryption this file exists to keep.
    pub fn search_message_bodies(&self, query: &str) -> Result<Vec<ConversationId>, ChatError> {
        let needle = normalize_search_query(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        let conversation_ids =
            list_conversation_ids(&connection, self.account.as_str()).map_err(map_persist)?;
        let mut hits = Vec::new();
        for conversation_id in conversation_ids {
            let messages = match load_messages(
                &connection,
                self.secrets.as_ref(),
                self.account.as_str(),
                &conversation_id,
            ) {
                Ok(messages) => messages,
                Err(_) => continue,
            };
            if messages
                .iter()
                .any(|message| message_matches_query(message, &needle))
            {
                hits.push(conversation_id);
                if hits.len() >= 32 {
                    break;
                }
            }
        }
        Ok(hits)
    }
}

#[async_trait]
impl LocalChatProjection for EncryptedChatProjection {
    fn account_id(&self) -> &ValidatedAccountId {
        &self.account
    }

    async fn load_snapshot(&self, id: ConversationId) -> Result<Option<ChatSnapshot>, ChatError> {
        let connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        if is_deleted(&connection, self.account.as_str(), &id).map_err(map_sql)? {
            return Ok(None);
        }
        match load_snapshot_inner(
            &connection,
            self.secrets.as_ref(),
            self.account.as_str(),
            &id,
        ) {
            Ok(snapshot) => Ok(snapshot),
            // An unreadable cache row is a cache miss, not a broken
            // conversation: rows sealed by an older build (AAD schema drift) or
            // under a key this device no longer holds would otherwise brick
            // every open of a conversation whose truth still lives in DA. The
            // reconcile that asked for this snapshot overwrites the row with a
            // freshly sealed one.
            Err(AppError::PersistenceDecryptionFailed | AppError::PersistenceKeyUnavailable) => {
                Ok(None)
            }
            Err(error) => Err(map_persist(error)),
        }
    }

    async fn load_runs(&self, id: ConversationId) -> Result<Option<Vec<RunRecord>>, ChatError> {
        let connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        if is_deleted(&connection, self.account.as_str(), &id).map_err(map_sql)? {
            return Ok(None);
        }
        match load_runs_inner(
            &connection,
            self.secrets.as_ref(),
            self.account.as_str(),
            &id,
        ) {
            Ok(runs) => Ok(runs),
            // Same cache-miss mapping as load_snapshot.
            Err(AppError::PersistenceDecryptionFailed | AppError::PersistenceKeyUnavailable) => {
                Ok(None)
            }
            Err(error) => Err(map_persist(error)),
        }
    }

    async fn deleted_conversations(
        &self,
    ) -> Result<std::collections::HashSet<ConversationId>, ChatError> {
        let connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        let mut statement = connection
            .prepare("SELECT conversation_id FROM chat_deleted_conversations WHERE account_id = ?1")
            .map_err(map_sql)?;
        let rows = statement
            .query_map(params![self.account.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(map_sql)?;
        rows.map(|row| row.map(ConversationId::from).map_err(map_sql))
            .collect()
    }

    async fn apply(&self, event: ChatProjectionEvent) -> Result<(), ChatError> {
        let mut connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        apply_event(
            &mut connection,
            self.secrets.as_ref(),
            self.account.as_str(),
            event,
        )
    }

    async fn purge_account(&self, account: ValidatedAccountId) -> Result<(), ChatError> {
        if account.as_str() != self.account.as_str() {
            return Err(ChatError::Validation("account mismatch"));
        }
        let connection = self.connection.lock().map_err(|_| ChatError::Unavailable)?;
        purge_account_rows(&connection, account.as_str()).map_err(map_persist)
    }
}

fn open_connection(path: &Path) -> AppResult<Connection> {
    crate::persistence::open_local_sqlite_store(
        path,
        crate::persistence::LocalSqliteSynchronous::Normal,
        migrate,
    )
}

fn migrate(connection: &mut Connection) -> AppResult<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > CHAT_PROJECTION_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == CHAT_PROJECTION_SCHEMA_VERSION {
        return Ok(());
    }
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > CHAT_PROJECTION_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == CHAT_PROJECTION_SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    transaction.execute_batch(CHAT_PROJECTION_MIGRATION)?;
    transaction.pragma_update(None, "user_version", CHAT_PROJECTION_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn map_sql(error: rusqlite::Error) -> ChatError {
    map_persist(error.into())
}

fn map_persist(error: AppError) -> ChatError {
    match error {
        AppError::PersistenceInvalid(message) => ChatError::Validation(message),
        AppError::PersistenceKeyUnavailable
        | AppError::PersistenceDecryptionFailed
        | AppError::PersistenceEncryptionFailed => ChatError::Unavailable,
        AppError::PersistenceSchemaUnsupported => ChatError::Unavailable,
        _ => ChatError::Unavailable,
    }
}

fn is_deleted(
    connection: &Connection,
    account: &str,
    id: &ConversationId,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_deleted_conversations WHERE account_id = ?1 AND conversation_id = ?2)",
        params![account, id.as_str()], |row| row.get(0),
    )
}

fn apply_event(
    connection: &mut Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    event: ChatProjectionEvent,
) -> Result<(), ChatError> {
    // Serialize the deletion fence and every mutation across independently
    // opened projection connections, including late cloud snapshot responses.
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_sql)?;
    let id = match &event {
        ChatProjectionEvent::UpsertConversation(value) => Some(value.id()),
        ChatProjectionEvent::AppendMessage(value) => Some(value.conversation_id()),
        ChatProjectionEvent::UpsertDraft(value) => Some(value.conversation_id()),
        ChatProjectionEvent::ClearDraft { conversation_id } => Some(conversation_id),
        ChatProjectionEvent::UpsertRun(value) => Some(value.conversation_id()),
        ChatProjectionEvent::ReplaceSnapshot(value) => Some(value.conversation().id()),
        ChatProjectionEvent::AdvanceCursor(_) | ChatProjectionEvent::RemoveConversation { .. } => {
            None
        }
    };
    if let Some(id) = id {
        if is_deleted(&transaction, account_id, id).map_err(map_sql)? {
            return Err(ChatError::NotFound);
        }
    }
    apply_event_inner(&transaction, secrets, account_id, event)?;
    transaction.commit().map_err(map_sql)?;
    Ok(())
}

fn apply_event_inner(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    event: ChatProjectionEvent,
) -> Result<(), ChatError> {
    match event {
        ChatProjectionEvent::UpsertConversation(conversation) => {
            let head = load_head(connection, account_id, conversation.id()).map_err(map_persist)?;
            upsert_conversation(
                connection,
                secrets,
                account_id,
                &conversation,
                head.unwrap_or(0),
            )
            .map_err(map_persist)?;
            Ok(())
        }
        ChatProjectionEvent::AppendMessage(message) => {
            append_message(connection, secrets, account_id, &message)?;
            Ok(())
        }
        ChatProjectionEvent::UpsertDraft(draft) => {
            upsert_draft(connection, secrets, account_id, &draft).map_err(map_persist)?;
            Ok(())
        }
        ChatProjectionEvent::ClearDraft { conversation_id } => {
            connection
                .execute(
                    "DELETE FROM chat_drafts WHERE account_id = ?1 AND conversation_id = ?2",
                    params![account_id, conversation_id.as_str()],
                )
                .map_err(|error| map_persist(error.into()))?;
            Ok(())
        }
        ChatProjectionEvent::UpsertRun(run) => {
            upsert_run(connection, secrets, account_id, &run).map_err(map_persist)?;
            Ok(())
        }
        ChatProjectionEvent::ReplaceSnapshot(snapshot) => {
            replace_snapshot(connection, secrets, account_id, &snapshot)?;
            Ok(())
        }
        ChatProjectionEvent::AdvanceCursor(cursor) => {
            // Cursor advances require an existing conversation row; no-op otherwise.
            let _ = cursor;
            Ok(())
        }
        ChatProjectionEvent::RemoveConversation { conversation_id } => {
            let conversation_id = conversation_id.as_str();
            connection.execute(
                "INSERT INTO chat_deleted_conversations(account_id, conversation_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                params![account_id, conversation_id],
            ).map_err(map_sql)?;
            connection
                .execute(
                    "DELETE FROM chat_title_sources WHERE account_id = ?1 AND conversation_id = ?2",
                    params![account_id, conversation_id],
                )
                .map_err(|error| map_persist(error.into()))?;
            connection
                .execute(
                    "DELETE FROM chat_messages WHERE account_id = ?1 AND conversation_id = ?2",
                    params![account_id, conversation_id],
                )
                .map_err(|error| map_persist(error.into()))?;
            connection
                .execute(
                    "DELETE FROM chat_runs WHERE account_id = ?1 AND conversation_id = ?2",
                    params![account_id, conversation_id],
                )
                .map_err(|error| map_persist(error.into()))?;
            connection
                .execute(
                    "DELETE FROM chat_drafts WHERE account_id = ?1 AND conversation_id = ?2",
                    params![account_id, conversation_id],
                )
                .map_err(|error| map_persist(error.into()))?;
            connection
                .execute(
                    "DELETE FROM chat_conversations WHERE account_id = ?1 AND conversation_id = ?2",
                    params![account_id, conversation_id],
                )
                .map_err(|error| map_persist(error.into()))?;
            Ok(())
        }
    }
}

fn replace_snapshot(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    snapshot: &ChatSnapshot,
) -> Result<(), ChatError> {
    let conversation_id = snapshot.conversation().id().as_str();
    connection
        .execute(
            "DELETE FROM chat_messages WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id],
        )
        .map_err(|error| map_persist(error.into()))?;
    connection
        .execute(
            "DELETE FROM chat_runs WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id],
        )
        .map_err(|error| map_persist(error.into()))?;
    connection
        .execute(
            "DELETE FROM chat_drafts WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id],
        )
        .map_err(|error| map_persist(error.into()))?;

    upsert_conversation(
        connection,
        secrets,
        account_id,
        snapshot.conversation(),
        snapshot.cursor().head_sequence(),
    )
    .map_err(map_persist)?;

    for message in snapshot.messages() {
        insert_message(connection, secrets, account_id, message, true).map_err(map_persist)?;
    }
    for run in snapshot.runs() {
        upsert_run(connection, secrets, account_id, run).map_err(map_persist)?;
    }
    if let Some(draft) = snapshot.draft() {
        upsert_draft(connection, secrets, account_id, draft).map_err(map_persist)?;
    }
    Ok(())
}

fn append_message(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    message: &Message,
) -> Result<(), ChatError> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT message_id FROM chat_messages
             WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3",
            params![
                account_id,
                message.conversation_id().as_str(),
                message.id().as_uuid().to_string()
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| map_persist(error.into()))?;
    if existing.is_some() {
        return Ok(());
    }

    let head = load_head(connection, account_id, message.conversation_id()).map_err(map_persist)?;
    let head = head.unwrap_or(0);
    if message.sequence() != head.saturating_add(1) {
        return Err(ChatError::SequenceNotMonotonic);
    }

    insert_message(connection, secrets, account_id, message, false).map_err(map_persist)?;
    bump_head(
        connection,
        account_id,
        message.conversation_id().as_str(),
        message.sequence(),
    )
    .map_err(map_persist)?;
    Ok(())
}

fn insert_message(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    message: &Message,
    replace: bool,
) -> AppResult<()> {
    let payload = MessagePayload::from_message(message);
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|_| AppError::PersistenceInvalid("message payload encoding failed"))?;
    let content_hash = hash_bytes(&plaintext);
    let message_id = message.id().as_uuid().to_string();
    let encrypted = seal(
        secrets,
        account_id,
        message.conversation_id().as_str(),
        "message",
        &message_id,
        message.sequence(),
        &plaintext,
    )?;
    let sequence = i64::try_from(message.sequence())
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    let sql = if replace {
        "INSERT INTO chat_messages (
            account_id, conversation_id, message_id, sequence, content_hash,
            aad_schema_version, key_version, nonce, ciphertext
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(account_id, conversation_id, message_id) DO UPDATE SET
            sequence = excluded.sequence,
            content_hash = excluded.content_hash,
            aad_schema_version = excluded.aad_schema_version,
            key_version = excluded.key_version,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext"
    } else {
        "INSERT INTO chat_messages (
            account_id, conversation_id, message_id, sequence, content_hash,
            aad_schema_version, key_version, nonce, ciphertext
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    };
    connection.execute(
        sql,
        params![
            account_id,
            message.conversation_id().as_str(),
            message_id,
            sequence,
            content_hash,
            i64::from(encrypted.aad_schema_version),
            i64::from(encrypted.key_version),
            encrypted.nonce,
            encrypted.ciphertext,
        ],
    )?;
    Ok(())
}

fn upsert_conversation(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation: &Conversation,
    head_sequence: u64,
) -> AppResult<()> {
    let payload = ConversationPayload {
        title: conversation.title().to_owned(),
    };
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|_| AppError::PersistenceInvalid("conversation payload encoding failed"))?;
    let content_hash = hash_bytes(&plaintext);
    let encrypted = seal(
        secrets,
        account_id,
        conversation.id().as_str(),
        "conversation",
        conversation.id().as_str(),
        0,
        &plaintext,
    )?;
    let head = i64::try_from(head_sequence)
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    connection.execute(
        "INSERT INTO chat_conversations (
            account_id, conversation_id, head_sequence, content_hash,
            aad_schema_version, key_version, nonce, ciphertext
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(account_id, conversation_id) DO UPDATE SET
            head_sequence = excluded.head_sequence,
            content_hash = excluded.content_hash,
            aad_schema_version = excluded.aad_schema_version,
            key_version = excluded.key_version,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext",
        params![
            account_id,
            conversation.id().as_str(),
            head,
            content_hash,
            i64::from(encrypted.aad_schema_version),
            i64::from(encrypted.key_version),
            encrypted.nonce,
            encrypted.ciphertext,
        ],
    )?;
    Ok(())
}

fn upsert_draft(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    draft: &AssistantDraft,
) -> AppResult<()> {
    let payload = DraftPayload {
        text: draft.text().to_owned(),
    };
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|_| AppError::PersistenceInvalid("draft payload encoding failed"))?;
    let content_hash = hash_bytes(&plaintext);
    let encrypted = seal(
        secrets,
        account_id,
        draft.conversation_id().as_str(),
        "draft",
        draft.conversation_id().as_str(),
        draft.sequence(),
        &plaintext,
    )?;
    let sequence = i64::try_from(draft.sequence())
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    connection.execute(
        "INSERT INTO chat_drafts (
            account_id, conversation_id, sequence, content_hash,
            aad_schema_version, key_version, nonce, ciphertext
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(account_id, conversation_id) DO UPDATE SET
            sequence = excluded.sequence,
            content_hash = excluded.content_hash,
            aad_schema_version = excluded.aad_schema_version,
            key_version = excluded.key_version,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext",
        params![
            account_id,
            draft.conversation_id().as_str(),
            sequence,
            content_hash,
            i64::from(encrypted.aad_schema_version),
            i64::from(encrypted.key_version),
            encrypted.nonce,
            encrypted.ciphertext,
        ],
    )?;
    Ok(())
}

fn upsert_run(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    run: &RunRecord,
) -> AppResult<()> {
    let payload = RunPayload {
        state: run.state().clone(),
    };
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|_| AppError::PersistenceInvalid("run payload encoding failed"))?;
    let content_hash = hash_bytes(&plaintext);
    let run_id = run.run_id().as_str().to_owned();
    let encrypted = seal(
        secrets,
        account_id,
        run.conversation_id().as_str(),
        "run",
        &run_id,
        run.sequence(),
        &plaintext,
    )?;
    let sequence = i64::try_from(run.sequence())
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    // Legacy `state` column is NOT NULL; keep an empty placeholder and read
    // authoritative state only from the sealed ciphertext payload.
    connection.execute(
        "INSERT INTO chat_runs (
            account_id, conversation_id, run_id, sequence, state, content_hash,
            aad_schema_version, key_version, nonce, ciphertext
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(account_id, conversation_id, run_id) DO UPDATE SET
            sequence = excluded.sequence,
            state = excluded.state,
            content_hash = excluded.content_hash,
            aad_schema_version = excluded.aad_schema_version,
            key_version = excluded.key_version,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext",
        params![
            account_id,
            run.conversation_id().as_str(),
            run_id,
            sequence,
            "",
            content_hash,
            i64::from(encrypted.aad_schema_version),
            i64::from(encrypted.key_version),
            encrypted.nonce,
            encrypted.ciphertext,
        ],
    )?;
    Ok(())
}

fn bump_head(
    connection: &Connection,
    account_id: &str,
    conversation_id: &str,
    head_sequence: u64,
) -> AppResult<()> {
    let head = i64::try_from(head_sequence)
        .map_err(|_| AppError::PersistenceInvalid("sequence exceeds SQLite range"))?;
    let updated = connection.execute(
        "UPDATE chat_conversations SET head_sequence = ?3
         WHERE account_id = ?1 AND conversation_id = ?2",
        params![account_id, conversation_id, head],
    )?;
    if updated == 0 {
        return Err(AppError::PersistenceInvalid("conversation is missing"));
    }
    Ok(())
}

fn load_head(
    connection: &Connection,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Option<u64>> {
    let head: Option<i64> = connection
        .query_row(
            "SELECT head_sequence FROM chat_conversations
             WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    head.map(|value| u64::try_from(value).map_err(|_| AppError::PersistenceDecryptionFailed))
        .transpose()
}

fn load_snapshot_inner(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Option<ChatSnapshot>> {
    let Some(conversation_row) = load_conversation_row(connection, account_id, conversation_id)?
    else {
        return Ok(None);
    };
    let title = open_conversation_payload(
        secrets,
        account_id,
        conversation_id.as_str(),
        &conversation_row,
    )?;
    let conversation = Conversation::new(conversation_id.clone(), title);
    let messages = load_messages(connection, secrets, account_id, conversation_id)?;
    let runs = load_runs(connection, secrets, account_id, conversation_id)?;
    let draft = load_draft(connection, secrets, account_id, conversation_id)?;
    let head = u64::try_from(conversation_row.head_sequence)
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
    Ok(Some(ChatSnapshot::new(
        conversation,
        messages,
        draft,
        runs,
        SyncCursor::new(head),
    )))
}

/// Mirrors the existence and header checks of `load_snapshot_inner`, then
/// decrypts only the run rows. Messages and the draft are never opened.
fn load_runs_inner(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Option<Vec<RunRecord>>> {
    let Some(conversation_row) = load_conversation_row(connection, account_id, conversation_id)?
    else {
        return Ok(None);
    };
    open_conversation_payload(
        secrets,
        account_id,
        conversation_id.as_str(),
        &conversation_row,
    )?;
    u64::try_from(conversation_row.head_sequence)
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
    load_runs(connection, secrets, account_id, conversation_id).map(Some)
}

struct ConversationRow {
    head_sequence: i64,
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn load_conversation_row(
    connection: &Connection,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Option<ConversationRow>> {
    connection
        .query_row(
            "SELECT head_sequence, aad_schema_version, key_version, nonce, ciphertext
             FROM chat_conversations
             WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id.as_str()],
            |row| {
                Ok(ConversationRow {
                    head_sequence: row.get(0)?,
                    aad_schema_version: row.get(1)?,
                    key_version: row.get(2)?,
                    nonce: row.get(3)?,
                    ciphertext: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn open_conversation_payload(
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &str,
    row: &ConversationRow,
) -> AppResult<String> {
    let plaintext = open_sealed(
        secrets,
        account_id,
        conversation_id,
        "conversation",
        conversation_id,
        0,
        row,
    )?;
    let payload: ConversationPayload =
        serde_json::from_slice(&plaintext).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    Ok(payload.title)
}

fn load_messages(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Vec<Message>> {
    let mut statement = connection.prepare(
        "SELECT message_id, sequence, aad_schema_version, key_version, nonce, ciphertext
         FROM chat_messages
         WHERE account_id = ?1 AND conversation_id = ?2
         ORDER BY sequence ASC, message_id ASC",
    )?;
    let rows = statement.query_map(params![account_id, conversation_id.as_str()], |row| {
        Ok(EntityRow {
            entity_id: row.get(0)?,
            sequence: row.get(1)?,
            aad_schema_version: row.get(2)?,
            key_version: row.get(3)?,
            nonce: row.get(4)?,
            ciphertext: row.get(5)?,
        })
    })?;
    let mut messages = Vec::new();
    for row in rows {
        let row = row?;
        let plaintext = open_sealed(
            secrets,
            account_id,
            conversation_id.as_str(),
            "message",
            &row.entity_id,
            u64::try_from(row.sequence).map_err(|_| AppError::PersistenceDecryptionFailed)?,
            &row,
        )?;
        let payload: MessagePayload = serde_json::from_slice(&plaintext)
            .map_err(|_| AppError::PersistenceDecryptionFailed)?;
        messages.push(payload.into_message()?);
    }
    Ok(messages)
}

fn list_conversation_ids(
    connection: &Connection,
    account_id: &str,
) -> AppResult<Vec<ConversationId>> {
    let mut statement = connection
        .prepare("SELECT conversation_id FROM chat_conversations WHERE account_id = ?1")?;
    let rows = statement.query_map(params![account_id], |row| row.get::<_, String>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(ConversationId::from(row?));
    }
    Ok(ids)
}

fn normalize_search_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn message_matches_query(message: &Message, needle: &str) -> bool {
    for part in message.content() {
        let haystack = match part {
            MessageContent::Text { text } => text.as_str(),
            MessageContent::AttachmentRef { file_name, .. } => file_name.as_str(),
            MessageContent::ArtifactRef { title, .. } => title.as_str(),
            MessageContent::ToolResultRef { .. } => continue,
        };
        if haystack.to_lowercase().contains(needle) {
            return true;
        }
    }
    for call in message.tool_calls() {
        if call.name.to_lowercase().contains(needle) {
            return true;
        }
        if call
            .arguments
            .as_deref()
            .is_some_and(|text| text.to_lowercase().contains(needle))
        {
            return true;
        }
        if call
            .result
            .as_deref()
            .is_some_and(|text| text.to_lowercase().contains(needle))
        {
            return true;
        }
    }
    false
}

fn load_runs(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Vec<RunRecord>> {
    let mut statement = connection.prepare(
        "SELECT run_id, sequence, aad_schema_version, key_version, nonce, ciphertext
         FROM chat_runs
         WHERE account_id = ?1 AND conversation_id = ?2
         ORDER BY sequence ASC, run_id ASC",
    )?;
    let rows = statement.query_map(params![account_id, conversation_id.as_str()], |row| {
        Ok(EntityRow {
            entity_id: row.get(0)?,
            sequence: row.get(1)?,
            aad_schema_version: row.get(2)?,
            key_version: row.get(3)?,
            nonce: row.get(4)?,
            ciphertext: row.get(5)?,
        })
    })?;
    let mut runs = Vec::new();
    for row in rows {
        let row = row?;
        let plaintext = open_sealed(
            secrets,
            account_id,
            conversation_id.as_str(),
            "run",
            &row.entity_id,
            u64::try_from(row.sequence).map_err(|_| AppError::PersistenceDecryptionFailed)?,
            &row,
        )?;
        let payload: RunPayload = serde_json::from_slice(&plaintext)
            .map_err(|_| AppError::PersistenceDecryptionFailed)?;
        runs.push(
            RunRecord::try_new(
                RunId::from(row.entity_id),
                conversation_id.clone(),
                payload.state,
                u64::try_from(row.sequence).map_err(|_| AppError::PersistenceDecryptionFailed)?,
            )
            .map_err(|_| AppError::PersistenceDecryptionFailed)?,
        );
    }
    Ok(runs)
}

fn load_draft(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Option<AssistantDraft>> {
    let row = connection
        .query_row(
            "SELECT sequence, aad_schema_version, key_version, nonce, ciphertext
             FROM chat_drafts
             WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id.as_str()],
            |row| {
                Ok(EntityRow {
                    entity_id: conversation_id.as_str().to_owned(),
                    sequence: row.get(0)?,
                    aad_schema_version: row.get(1)?,
                    key_version: row.get(2)?,
                    nonce: row.get(3)?,
                    ciphertext: row.get(4)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let plaintext = open_sealed(
        secrets,
        account_id,
        conversation_id.as_str(),
        "draft",
        conversation_id.as_str(),
        u64::try_from(row.sequence).map_err(|_| AppError::PersistenceDecryptionFailed)?,
        &row,
    )?;
    let payload: DraftPayload =
        serde_json::from_slice(&plaintext).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    AssistantDraft::try_restore(
        conversation_id.clone(),
        u64::try_from(row.sequence).map_err(|_| AppError::PersistenceDecryptionFailed)?,
        payload.text,
    )
    .map(Some)
    .map_err(|_| AppError::PersistenceDecryptionFailed)
}

fn purge_account_rows(connection: &Connection, account_id: &str) -> AppResult<()> {
    for table in [
        "chat_messages",
        "chat_drafts",
        "chat_runs",
        "chat_title_sources",
        "chat_conversations",
        "chat_reconcile_temp",
        "chat_deleted_conversations",
    ] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE account_id = ?1"),
            params![account_id],
        )?;
    }
    Ok(())
}

struct Sealed {
    aad_schema_version: u32,
    key_version: u32,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

trait SealedRow {
    fn aad_schema_version(&self) -> i64;
    fn key_version(&self) -> i64;
    fn nonce(&self) -> &[u8];
    fn ciphertext(&self) -> &[u8];
}

impl SealedRow for ConversationRow {
    fn aad_schema_version(&self) -> i64 {
        self.aad_schema_version
    }
    fn key_version(&self) -> i64 {
        self.key_version
    }
    fn nonce(&self) -> &[u8] {
        &self.nonce
    }
    fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }
}

struct EntityRow {
    entity_id: String,
    sequence: i64,
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

impl SealedRow for EntityRow {
    fn aad_schema_version(&self) -> i64 {
        self.aad_schema_version
    }
    fn key_version(&self) -> i64 {
        self.key_version
    }
    fn nonce(&self) -> &[u8] {
        &self.nonce
    }
    fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }
}

fn seal(
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &str,
    entity_kind: &str,
    entity_id: &str,
    sequence: u64,
    plaintext: &[u8],
) -> AppResult<Sealed> {
    // Mint-on-first-use: this device may never have sealed anything for this
    // account before, and a writer that only *reads* the active key bricks
    // every write on a fresh device (the original "chat is unavailable").
    let (key_version, key) = secrets.load_or_create_active_key(account_id)?;
    let metadata = ProjectionAuthenticatedMetadata {
        account_id,
        conversation_id,
        entity_kind,
        entity_id,
        sequence,
    };
    let encrypted = encrypt_projection(&key, key_version, &metadata, plaintext)?;
    Ok(Sealed {
        aad_schema_version: encrypted.aad_schema_version,
        key_version,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
}

fn open_sealed(
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &str,
    entity_kind: &str,
    entity_id: &str,
    sequence: u64,
    row: &impl SealedRow,
) -> AppResult<Vec<u8>> {
    let aad_schema_version = u32::try_from(row.aad_schema_version())
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key_version =
        u32::try_from(row.key_version()).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key = secrets.account_key(account_id, key_version)?;
    let metadata = ProjectionAuthenticatedMetadata {
        account_id,
        conversation_id,
        entity_kind,
        entity_id,
        sequence,
    };
    decrypt_projection(
        &key,
        aad_schema_version,
        key_version,
        row.nonce(),
        &metadata,
        row.ciphertext(),
    )
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

#[derive(Serialize, Deserialize)]
struct ConversationPayload {
    title: String,
}

#[derive(Serialize, Deserialize)]
struct DraftPayload {
    text: String,
}

#[derive(Serialize, Deserialize)]
struct RunPayload {
    state: RunState,
}

#[derive(Serialize, Deserialize)]
struct MessagePayload {
    id: MessageId,
    conversation_id: ConversationId,
    sequence: u64,
    role: MessageRole,
    content: Vec<MessageContent>,
    tool_calls: Vec<ToolCallRef>,
    completed: bool,
}

#[derive(Serialize, Deserialize)]
struct TitleAuthorityPayload {
    source: TitleSource,
    upgrade_started: bool,
}

fn load_title_authority(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
) -> AppResult<Option<TitleAuthorityPayload>> {
    let row = connection
        .query_row(
            "SELECT aad_schema_version, key_version, nonce, ciphertext
             FROM chat_title_sources
             WHERE account_id = ?1 AND conversation_id = ?2",
            params![account_id, conversation_id.as_str()],
            |row| {
                Ok(EntityRow {
                    entity_id: conversation_id.as_str().to_owned(),
                    sequence: 0,
                    aad_schema_version: row.get(0)?,
                    key_version: row.get(1)?,
                    nonce: row.get(2)?,
                    ciphertext: row.get(3)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let plaintext = open_sealed(
        secrets,
        account_id,
        conversation_id.as_str(),
        "title_source",
        conversation_id.as_str(),
        0,
        &row,
    )?;
    serde_json::from_slice(&plaintext)
        .map(Some)
        .map_err(|_| AppError::PersistenceDecryptionFailed)
}

fn write_title_authority(
    connection: &Connection,
    secrets: &dyn SecretStore,
    account_id: &str,
    conversation_id: &ConversationId,
    authority: TitleAuthorityPayload,
    insert_only: bool,
) -> AppResult<bool> {
    if is_deleted(connection, account_id, conversation_id)? {
        return Ok(false);
    }
    let plaintext = serde_json::to_vec(&authority)
        .map_err(|_| AppError::PersistenceInvalid("title source encoding failed"))?;
    let encrypted = seal(
        secrets,
        account_id,
        conversation_id.as_str(),
        "title_source",
        conversation_id.as_str(),
        0,
        &plaintext,
    )?;
    let conflict = if insert_only {
        "DO NOTHING"
    } else {
        "DO UPDATE SET
            aad_schema_version = excluded.aad_schema_version,
            key_version = excluded.key_version,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext"
    };
    let changed = connection.execute(
        &format!(
            "INSERT INTO chat_title_sources (
                account_id, conversation_id, aad_schema_version, key_version, nonce, ciphertext
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id, conversation_id) {conflict}"
        ),
        params![
            account_id,
            conversation_id.as_str(),
            i64::from(encrypted.aad_schema_version),
            i64::from(encrypted.key_version),
            encrypted.nonce,
            encrypted.ciphertext,
        ],
    )?;
    Ok(changed > 0)
}

impl MessagePayload {
    fn from_message(message: &Message) -> Self {
        Self {
            id: message.id(),
            conversation_id: message.conversation_id().clone(),
            sequence: message.sequence(),
            role: message.role(),
            content: message.content().to_vec(),
            tool_calls: message.tool_calls().to_vec(),
            completed: message.is_completed(),
        }
    }

    fn into_message(self) -> AppResult<Message> {
        let mut message = Message::try_new(
            self.id,
            self.conversation_id,
            self.sequence,
            self.role,
            self.content,
            self.tool_calls,
        )
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
        if self.completed {
            message.complete();
        }
        Ok(message)
    }
}
