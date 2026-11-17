//! Non-blocking, incremental, cancellable indexing of a trusted project's
//! files, and the encrypted local store the index lives in.
//!
//! # `.xfalcon/` is out of scope entirely
//!
//! `<trusted-root>/.xfalcon/` is a distinct, separately-trust-gated
//! namespace owned by `crate::skills::discovery` (see that module's doc
//! comment), not "just another project directory". [`run_index_pass`]
//! excludes any entry named exactly `.xfalcon` at any depth via
//! `ignore::WalkBuilder::filter_entry`, so it is never walked, never
//! indexed, and never searchable through `super::search`.
//!
//! # `ignore` proposes; `TrustedRoot` decides
//!
//! The `ignore` crate (BurntSushi/ripgrep) drives the actual directory walk
//! -- `.gitignore`/`.ignore` matching, hidden-file policy, and a bounded
//! worker-thread pool are exactly what it is for. But it only ever
//! *proposes* candidate relative-path strings, derived from
//! [`super::domain::TrustedRoot::canonical_path`] (a value already recorded
//! at trust-establishment time, never attacker input -- see
//! [`ignore::WalkBuilder::new`]'s argument below). Every candidate is
//! independently re-validated through [`super::path_policy::RelativeProjectPath::new`]
//! and [`super::domain::TrustedRoot::resolve_existing`] before any content,
//! size, or hash is ever recorded (see [`process_entry`]) -- mirroring
//! `crate::skills::discovery`'s "propose via a lexical boundary, then
//! re-validate through the real containment API" split, except here the
//! real API actually exists and is used for the decision, not just as
//! defense in depth on top of a lexical check. This also means a symlink
//! `ignore` declines to follow (its own default) but still *surfaces* as an
//! entry is independently caught a second time by `TrustedRoot`'s own
//! link/reparse rejection -- never trust the walker alone.
//!
//! # Incremental: mark, sweep, and skip-if-unchanged
//!
//! Each pass loads every existing row's plaintext `(size_bytes,
//! modified_at_ms)` fingerprint up front ([`IndexStore::fingerprints`] --
//! cheap, no decryption). A candidate whose current stat matches its stored
//! fingerprint is left untouched (no re-open, no re-hash, no DB write) --
//! this is what makes a repeat pass over a mostly-unchanged 100,000-file
//! tree fast. Every path seen during a *fully completed* (non-cancelled,
//! non-truncated) pass is tracked; anything previously indexed but not seen
//! this pass is deleted at the end ("mark and sweep"). A **cancelled** pass
//! never prunes: rows for files the walk simply did not reach yet before
//! cancellation must not look deleted (plan: search "must behave sanely
//! against a partially-populated index"). A pass that stops because
//! [`super::domain::TrustedRoot`]'s identity changed partway through
//! (`IndexPassOutcome::root_identity_changed`), or because
//! [`MAX_INDEXED_FILES_PER_PASS`] truncated it before the whole tree was
//! walked (`IndexPassOutcome::truncated`), is treated identically for this
//! purpose: in both cases the walk stopped before reaching every file, so
//! `seen_path_keys` never gains an entry for the unreached remainder, and
//! the sweep below would otherwise misread "not yet reached" as "deleted
//! from disk" -- silently wiping an otherwise-healthy index instead of just
//! stopping. A **per-candidate** failure that is not a genuine
//! [`super::domain::ContainmentError::NotFound`] (a permission-denied/
//! sharing-violation/other transient I/O error, for instance) is likewise
//! never treated as deletion -- see [`process_entry`]'s doc comment and
//! [`EntryOutcome::TransientError`].
//!
//! # Cancellation and non-blocking execution
//!
//! [`run_index_pass`] takes a [`super::trust::CancellationSignal`] by
//! constructor injection (never constructs its own) and checks it both in
//! the parallel walk's visitor (so enumeration itself stops promptly) and
//! in the single-threaded consumer loop that performs containment checks
//! and batched DB writes. [`ProjectIndexer::start`] is the actual
//! non-blocking entry point, mirroring
//! `crate::security::account_cleanup::AccountCleanup::start`'s exact shape
//! (named background thread, atomic busy-guard, `catch_unwind`, a
//! `tokio::sync::oneshot` bridge back to an awaitable `async fn`) -- see
//! that type's doc comment for the full rationale and the "no caller yet"
//! disclosure this indexer shares with it (nothing in this task registers
//! a Tauri command that calls `ProjectIndexer::start`; that is Task 7's
//! agent-loop/dispatch wiring, same as the four tool descriptors
//! `crate::tools::registry` exposes for this subsystem).

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use crate::domain::error::{AppError, AppResult};
use crate::persistence::{
    decrypt_project_index, encrypt_project_index, ProjectIndexAuthenticatedMetadata, SecretStore,
    PROJECT_INDEX_MIGRATION, PROJECT_INDEX_SCHEMA_VERSION,
};
use crate::security::account_cleanup::ValidatedAccountId;

use super::domain::{ContainmentError, ProjectId, TrustedRoot};
use super::path_policy::RelativeProjectPath;
use super::platform::RootIdentity;
use super::read::{hash_and_classify_streaming, ContentKind, MAX_HASHED_FILE_BYTES};
use super::sensitive::SecretClassifier;
use super::trust::CancellationSignal;

/// Directory name excluded from indexing/search at any depth -- see the
/// module doc comment. Broader than this one name: every path component that
/// starts with `.` is skipped by [`is_dot_entry`].
const XFALCON_NAMESPACE_DIR: &str = ".xfalcon";

/// Whether this walk entry is a dotfile or a directory whose name starts with
/// `.`. The walk root itself (depth 0) is never skipped — a trusted folder
/// named `.something` is still a root; only its children are filtered.
fn is_dot_entry(entry: &ignore::DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.starts_with('.'))
}

/// Bounded worker-thread count for the `ignore` parallel walk (plan: "Use
/// `ignore` walker with bounded worker count"). Not a product-tunable
/// setting -- a small, fixed, named bound like every other `MAX_*`/fixed
/// concurrency constant in this module tree.
pub const MAX_INDEX_WORKER_THREADS: usize = 4;

/// How many changed/new records accumulate before one batched
/// `IndexStore::upsert_batch` transaction is committed (plan: "Batch DB
/// writes... do not open a transaction per file for 100,000 files").
pub const INDEX_BATCH_SIZE: usize = 500;

/// Hard ceiling on how many files a single pass will index (and on how many
/// rows `IndexStore::list_all` will return to `super::search`) -- defense
/// against an unbounded/hostile tree exhausting memory or DB size before
/// anything else notices, mirroring `crate::skills::discovery::MAX_SKILLS_PER_SOURCE`'s
/// identical rationale at a scale appropriate to a project index rather
/// than a skill catalog.
pub const MAX_INDEXED_FILES_PER_PASS: usize = 250_000;

/// Bound on the channel connecting the parallel walk's worker threads to
/// the single-threaded consumer -- never unbounded, per this codebase's
/// "bounded everything" convention.
const CHANNEL_BOUND: usize = 2 * INDEX_BATCH_SIZE;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(transparent)]
    Containment(#[from] super::domain::ContainmentError),
    #[error(transparent)]
    Sensitive(#[from] super::sensitive::SensitiveError),
    #[error("io error while indexing a trusted-project file: {0}")]
    Io(#[source] std::io::Error),
    #[error("the local project index store is unavailable")]
    Storage,
    #[error("the indexing worker panicked")]
    Panicked,
}

impl From<AppError> for IndexError {
    fn from(_: AppError) -> Self {
        Self::Storage
    }
}

impl From<rusqlite::Error> for IndexError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

/// The walk configuration every project walk shares.
///
/// Extracted so `super::search`'s live walk sees exactly the files an index
/// pass would: a search that surfaced a file indexing deliberately skips (or
/// skipped one indexing would have found) would make "found in the index" and
/// "found on disk" two different answers to the same question.
///
/// `ignore::IgnoreBuilder::new`'s own default for `require_git` is already
/// `true` (confirmed by reading `ignore-0.4.31/src/dir.rs`), so pinning it is
/// belt-and-suspenders, not a behavior change: it makes this walk's
/// ".gitignore is only honored inside an actual git repository boundary, never
/// read from an arbitrary ancestor directory above one" posture self-evident
/// here rather than resting on an easily-overlooked upstream default (matching
/// `source-examples/codex/codex-rs/file-search/src/lib.rs`'s own explicit
/// `require_git(true)` call and its doc comment's identical rationale).
pub(super) fn project_walk_builder(root_path: &Path, _policy: &IndexPolicy) -> ignore::WalkBuilder {
    let mut builder = ignore::WalkBuilder::new(root_path);
    builder
        // Hidden names are never indexed. `.gitignore` is still read —
        // `hidden(true)` skips walking those entries, it does not skip
        // gitignore matching.
        .hidden(true)
        .parents(false)
        .git_global(false)
        .require_git(true)
        .same_file_system(true)
        .follow_links(false)
        .filter_entry(|entry| {
            if is_dot_entry(entry) {
                return false;
            }
            entry.file_name() != OsStr::new(XFALCON_NAMESPACE_DIR)
        });
    builder
}

/// Walk options for one index pass. Dot names are never included. Gitignore
/// inside a real repository is always honored. The type remains so
/// [`IndexPassRequest`] stays stable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexPolicy {}

/// One indexed file's metadata, as persisted (and as returned by
/// [`IndexStore::list_all`] to `super::search`). Never carries file
/// content.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedFileRecord {
    pub relative_path: RelativeProjectPath,
    pub size_bytes: u64,
    pub modified_at_ms: i64,
    pub indexed_at_ms: i64,
    pub content_kind: ContentKind,
    pub content_hash: Option<String>,
    pub is_sensitive: bool,
}

#[derive(Serialize, Deserialize)]
struct IndexPayload {
    relative_path: String,
    content_kind: String,
    content_hash: Option<String>,
    is_sensitive: bool,
}

fn encode_kind(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => "text",
        ContentKind::Binary => "binary",
        ContentKind::NonUtf8Text => "non_utf8_text",
        ContentKind::Oversized => "oversized",
    }
}

fn decode_kind(value: &str) -> Option<ContentKind> {
    match value {
        "text" => Some(ContentKind::Text),
        "binary" => Some(ContentKind::Binary),
        "non_utf8_text" => Some(ContentKind::NonUtf8Text),
        "oversized" => Some(ContentKind::Oversized),
        _ => None,
    }
}

struct IndexRow {
    path_key: Vec<u8>,
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    size_bytes: i64,
    modified_at_ms: i64,
    indexed_at_ms: i64,
}

/// Encrypted local storage for one project's indexed-file rows. Opens a
/// fresh `rusqlite::Connection` per call rather than holding one across
/// calls, matching `super::trust::TrustStore::open`'s documented idiom
/// (itself matching `crate::chat::service::ChatService::sync_service`).
pub struct IndexStore {
    connection: Mutex<Connection>,
    secrets: Arc<dyn SecretStore>,
}

impl IndexStore {
    pub fn open(path: impl AsRef<Path>, secrets: Arc<dyn SecretStore>) -> Result<Self, IndexError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| IndexError::Storage)?;
        }
        let connection = open_connection(path).map_err(|_| IndexError::Storage)?;
        Ok(Self {
            connection: Mutex::new(connection),
            secrets,
        })
    }

    /// Cheap, plaintext-only `(size_bytes, modified_at_ms)` fingerprints for
    /// every row currently stored for `project_id`, keyed by `path_key`. No
    /// decryption occurs; used only to decide which candidates an
    /// incremental pass can skip re-reading.
    pub fn fingerprints(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<HashMap<Vec<u8>, (u64, i64)>, IndexError> {
        let connection = self.connection.lock().map_err(|_| IndexError::Storage)?;
        let mut statement = connection.prepare(
            "SELECT path_key, size_bytes, modified_at_ms FROM project_index_files
             WHERE account_id = ?1 AND project_id = ?2",
        )?;
        let rows = statement.query_map(
            params![account.as_str(), project_id.to_string()],
            |row| -> rusqlite::Result<(Vec<u8>, i64, i64)> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            },
        )?;
        let mut map = HashMap::new();
        for row in rows {
            let (key, size, modified) = row?;
            map.insert(key, (size.max(0) as u64, modified));
        }
        Ok(map)
    }

    /// Inserts or updates `records` in one transaction (plan: "Batch DB
    /// writes"). A no-op for an empty slice.
    pub fn upsert_batch(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        records: &[IndexedFileRecord],
    ) -> Result<(), IndexError> {
        if records.is_empty() {
            return Ok(());
        }
        let project_id_string = project_id.to_string();
        let mut connection = self.connection.lock().map_err(|_| IndexError::Storage)?;
        let transaction = connection.transaction()?;
        for record in records {
            let key = path_key(&record.relative_path);
            let payload = IndexPayload {
                relative_path: record.relative_path.as_str().to_owned(),
                content_kind: encode_kind(record.content_kind).to_owned(),
                content_hash: record.content_hash.clone(),
                is_sensitive: record.is_sensitive,
            };
            let plaintext = serde_json::to_vec(&payload).map_err(|_| IndexError::Storage)?;
            let sealed = seal(
                self.secrets.as_ref(),
                account.as_str(),
                &project_id_string,
                &key,
                &plaintext,
            )?;
            transaction.execute(
                "INSERT INTO project_index_files (
                    account_id, project_id, path_key, aad_schema_version, key_version,
                    nonce, ciphertext, size_bytes, modified_at_ms, indexed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (account_id, project_id, path_key) DO UPDATE SET
                    aad_schema_version = excluded.aad_schema_version,
                    key_version = excluded.key_version,
                    nonce = excluded.nonce,
                    ciphertext = excluded.ciphertext,
                    size_bytes = excluded.size_bytes,
                    modified_at_ms = excluded.modified_at_ms,
                    indexed_at_ms = excluded.indexed_at_ms",
                params![
                    account.as_str(),
                    project_id_string,
                    key,
                    i64::from(sealed.aad_schema_version),
                    i64::from(sealed.key_version),
                    sealed.nonce,
                    sealed.ciphertext,
                    i64::try_from(record.size_bytes).unwrap_or(i64::MAX),
                    record.modified_at_ms,
                    record.indexed_at_ms,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Deletes rows by `path_key` in one transaction. Returns the number of
    /// rows actually removed.
    pub fn delete_by_path_keys(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        path_keys: &[Vec<u8>],
    ) -> Result<usize, IndexError> {
        if path_keys.is_empty() {
            return Ok(0);
        }
        let project_id_string = project_id.to_string();
        let mut connection = self.connection.lock().map_err(|_| IndexError::Storage)?;
        let transaction = connection.transaction()?;
        let mut removed = 0usize;
        for key in path_keys {
            removed += transaction.execute(
                "DELETE FROM project_index_files WHERE account_id = ?1 AND project_id = ?2 AND path_key = ?3",
                params![account.as_str(), project_id_string, key],
            )?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    /// Deletes every row for `project_id` -- used when a caller wants a full
    /// rebuild rather than an incremental pass (e.g. after a root identity
    /// change is resolved by re-trusting the project as a fresh
    /// [`ProjectId`]; no in-place "same project id, different disk
    /// contents" rebuild trigger exists in this task's scope).
    pub fn clear_project(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<usize, IndexError> {
        let connection = self.connection.lock().map_err(|_| IndexError::Storage)?;
        let removed = connection.execute(
            "DELETE FROM project_index_files WHERE account_id = ?1 AND project_id = ?2",
            params![account.as_str(), project_id.to_string()],
        )?;
        Ok(removed)
    }

    /// Loads every indexed record for `project_id`, bounded by
    /// [`MAX_INDEXED_FILES_PER_PASS`]. The returned `bool` is whether the
    /// bound truncated the result -- `super::search` propagates this so a
    /// caller knows the index itself (not just one search) may hold more
    /// than was returned.
    pub fn list_all(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<(Vec<IndexedFileRecord>, bool), IndexError> {
        let project_id_string = project_id.to_string();
        let connection = self.connection.lock().map_err(|_| IndexError::Storage)?;
        let mut statement = connection.prepare(
            "SELECT path_key, aad_schema_version, key_version, nonce, ciphertext,
                    size_bytes, modified_at_ms, indexed_at_ms
             FROM project_index_files WHERE account_id = ?1 AND project_id = ?2
             ORDER BY path_key ASC
             LIMIT ?3",
        )?;
        let limit = i64::try_from(MAX_INDEXED_FILES_PER_PASS + 1).unwrap_or(i64::MAX);
        let rows = statement.query_map(
            params![account.as_str(), project_id_string, limit],
            |row| -> rusqlite::Result<IndexRow> {
                Ok(IndexRow {
                    path_key: row.get(0)?,
                    aad_schema_version: row.get(1)?,
                    key_version: row.get(2)?,
                    nonce: row.get(3)?,
                    ciphertext: row.get(4)?,
                    size_bytes: row.get(5)?,
                    modified_at_ms: row.get(6)?,
                    indexed_at_ms: row.get(7)?,
                })
            },
        )?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row?);
        }
        let truncated = collected.len() > MAX_INDEXED_FILES_PER_PASS;
        collected.truncate(MAX_INDEXED_FILES_PER_PASS);

        let mut records = Vec::with_capacity(collected.len());
        for row in collected {
            records.push(self.record_from_row(account, &project_id_string, row)?);
        }
        Ok((records, truncated))
    }

    fn record_from_row(
        &self,
        account: &ValidatedAccountId,
        project_id: &str,
        row: IndexRow,
    ) -> Result<IndexedFileRecord, IndexError> {
        let plaintext = open(self.secrets.as_ref(), account.as_str(), project_id, &row)?;
        let payload: IndexPayload =
            serde_json::from_slice(&plaintext).map_err(|_| IndexError::Storage)?;
        let relative_path =
            RelativeProjectPath::new(&payload.relative_path).map_err(|_| IndexError::Storage)?;
        let content_kind = decode_kind(&payload.content_kind).ok_or(IndexError::Storage)?;
        Ok(IndexedFileRecord {
            relative_path,
            size_bytes: row.size_bytes.max(0) as u64,
            modified_at_ms: row.modified_at_ms,
            indexed_at_ms: row.indexed_at_ms,
            content_kind,
            content_hash: payload.content_hash,
            is_sensitive: payload.is_sensitive,
        })
    }
}

struct Sealed {
    aad_schema_version: u32,
    key_version: u32,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn seal(
    secrets: &dyn SecretStore,
    account_id: &str,
    project_id: &str,
    path_key: &[u8],
    plaintext: &[u8],
) -> AppResult<Sealed> {
    // Mint-on-first-use — see `chat::local_projection::seal`: a seal that only
    // reads the active key fails on a device that has never sealed before.
    let (key_version, key) = secrets.load_or_create_active_key(account_id)?;
    let metadata = ProjectIndexAuthenticatedMetadata {
        account_id,
        project_id,
        path_key,
    };
    let encrypted = encrypt_project_index(&key, key_version, &metadata, plaintext)?;
    Ok(Sealed {
        aad_schema_version: encrypted.aad_schema_version,
        key_version,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
}

fn open(
    secrets: &dyn SecretStore,
    account_id: &str,
    project_id: &str,
    row: &IndexRow,
) -> AppResult<Vec<u8>> {
    let aad_schema_version =
        u32::try_from(row.aad_schema_version).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key_version =
        u32::try_from(row.key_version).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key = secrets.account_key(account_id, key_version)?;
    let metadata = ProjectIndexAuthenticatedMetadata {
        account_id,
        project_id,
        path_key: &row.path_key,
    };
    decrypt_project_index(
        &key,
        aad_schema_version,
        key_version,
        &row.nonce,
        &metadata,
        &row.ciphertext,
    )
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
    if version > PROJECT_INDEX_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_INDEX_SCHEMA_VERSION {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > PROJECT_INDEX_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_INDEX_SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    transaction.execute_batch(PROJECT_INDEX_MIGRATION)?;
    transaction.pragma_update(None, "user_version", PROJECT_INDEX_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// SHA-256 of the relative path string alone (the `WHERE account_id = ? AND
/// project_id = ?` scoping on every query already disambiguates accounts
/// and projects, so the key itself only needs to be unique *within* one
/// project). Unsalted and unkeyed, matching `crate::skills::discovery`'s and
/// `crate::chat::local_projection::hash_bytes`'s existing unkeyed-SHA-256
/// identity/dedup idiom -- confidentiality of the actual path comes from the
/// AEAD ciphertext column (`xfalcon-project-index-v1` AAD domain), not from
/// this hash; this is a lookup key, not a security boundary.
fn path_key(relative: &RelativeProjectPath) -> Vec<u8> {
    Sha256::digest(relative.as_str().as_bytes()).to_vec()
}

fn modified_millis(metadata: &cap_std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.into_std().duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

/// Outcome of one call to [`run_index_pass`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexPassOutcome {
    /// New or changed files written this pass.
    pub files_indexed: usize,
    /// Files whose stored fingerprint already matched -- skipped without a
    /// re-open or re-hash.
    pub files_unchanged: usize,
    /// Previously indexed files removed because a *fully completed* pass
    /// did not see them again (never set on a cancelled pass).
    pub files_removed: usize,
    /// Candidates that were not indexable for a structural reason (not a
    /// regular file, an unparsable relative path) or that genuinely no
    /// longer exist on disk ([`super::domain::ContainmentError::NotFound`])
    /// -- non-fatal, the pass continues. Excluded from `seen_path_keys`, so
    /// a previously-indexed row for a `NotFound` candidate is correctly
    /// pruned by mark-and-sweep.
    pub files_skipped: usize,
    /// Candidates that failed with a transient or unexpected error *other*
    /// than [`super::domain::ContainmentError::NotFound`] (permission
    /// denied, a sharing violation, another I/O error, a symlink/reparse
    /// point encountered mid-walk, or a mismatched-root/invalid-component
    /// containment rejection) -- not proof the file was deleted, so its
    /// `path_key` (if previously indexed) is still marked seen this pass
    /// and its row survives mark-and-sweep rather than being silently
    /// removed over a one-off failure. See [`EntryOutcome::TransientError`].
    pub files_errored: usize,
    /// Whether [`super::trust::CancellationSignal`]'s generation changed
    /// during this pass, stopping it early.
    pub cancelled: bool,
    /// Whether [`MAX_INDEXED_FILES_PER_PASS`] stopped this pass before the
    /// whole tree was walked.
    pub truncated: bool,
    /// Whether `trusted_root`'s stable identity stopped matching the
    /// identity it was constructed/reopened with partway through this pass
    /// (the root was deleted and recreated, or a different volume mounted at
    /// the same path, while this pass was already running). Handled exactly
    /// like `cancelled` for the mark-and-sweep decision below (a pass that
    /// stopped early for this reason must never prune "unseen" rows -- see
    /// [`process_entry`]'s doc comment for why a single occurrence of this
    /// error necessarily recurs on every remaining candidate, and why
    /// treating that the same as "every remaining file was deleted from
    /// disk" would silently wipe an otherwise-healthy index).
    pub root_identity_changed: bool,
}

enum EntryOutcome {
    /// Not indexable for a structural reason, and no `path_key` was ever
    /// derived (the candidate never resolved into a valid relative path, or
    /// is not a regular file) -- nothing to mark seen or unseen.
    Skip,
    /// The candidate genuinely no longer exists on disk
    /// ([`ContainmentError::NotFound`], surfaced between `ignore` proposing
    /// it and this pass's own containment re-check). Correctly excluded
    /// from `seen_path_keys` so mark-and-sweep prunes any existing row.
    NotFound,
    /// A transient or unexpected failure occurred *after* `relative_path`
    /// was already known valid -- a permission-denied/sharing-violation/
    /// other I/O error, a symlink/reparse point, or a mismatched-root/
    /// invalid-component containment rejection. This is NOT proof the file
    /// was deleted, so `path_key` is still marked seen (see
    /// [`process_entry`]'s doc comment / [`containment_outcome`]).
    TransientError {
        path_key: Vec<u8>,
    },
    Unchanged {
        path_key: Vec<u8>,
    },
    Changed {
        path_key: Vec<u8>,
        record: IndexedFileRecord,
    },
}

/// Runs one incremental index pass over `trusted_root`, persisting results
/// to `store`. See the module doc comment for the walk/containment,
/// incremental, and cancellation design. Synchronous and directly
/// unit-testable; [`ProjectIndexer::start`] is the non-blocking wrapper a
/// real caller uses.
pub fn run_index_pass(
    trusted_root: &TrustedRoot,
    store: &IndexStore,
    account: &ValidatedAccountId,
    cancellation: &CancellationSignal,
    classifier: &SecretClassifier,
    policy: &IndexPolicy,
) -> Result<IndexPassOutcome, IndexError> {
    let project_id = *trusted_root.project_id();
    let generation_at_start = cancellation.generation();
    let root_path = trusted_root.canonical_path().to_path_buf();
    let fingerprints = store.fingerprints(account, project_id)?;

    let mut outcome = IndexPassOutcome::default();
    if generation_at_start != cancellation.generation() {
        outcome.cancelled = true;
        return Ok(outcome);
    }

    let mut walk_builder = project_walk_builder(&root_path, policy);
    walk_builder.threads(MAX_INDEX_WORKER_THREADS);
    let walker = walk_builder.build_parallel();

    let (tx, rx) = mpsc::sync_channel::<ignore::DirEntry>(CHANNEL_BOUND);
    let cancellation_for_walk = cancellation.clone();
    // Shared with the walker's visitor closure below (not the same thing as
    // `cancellation`: a root-identity change or a `MAX_INDEXED_FILES_PER_PASS`
    // truncation are facts about *this pass*, not "the account/project was
    // revoked", so neither should bump the caller's own `CancellationSignal`
    // -- doing so would falsely signal revocation to any other code sharing
    // that same signal). Set by the consumer loop below the moment it
    // decides to stop for ANY reason (cancellation, truncation, or a root
    // identity change) so a producer thread's visitor closure quits on its
    // very next invocation, the same way an observed generation change
    // already does -- see the consumer loop's own doc comment for why it
    // must still keep draining (never abandon `rx` outright) once this is
    // set: a producer thread already blocked in `tx.send` on a full channel
    // at that moment only unblocks once the consumer reads at least one more
    // entry.
    let stop_producer = Arc::new(AtomicBool::new(false));
    let stop_producer_for_walk = stop_producer.clone();

    let mut seen_path_keys: HashSet<Vec<u8>> = HashSet::with_capacity(fingerprints.len());
    let mut batch: Vec<IndexedFileRecord> = Vec::with_capacity(INDEX_BATCH_SIZE);

    let pass_result: Result<(), IndexError> = std::thread::scope(|scope| {
        scope.spawn(move || {
            walker.run(move || {
                let tx = tx.clone();
                let cancellation = cancellation_for_walk.clone();
                let stop_producer = stop_producer_for_walk.clone();
                Box::new(move |result: Result<ignore::DirEntry, ignore::Error>| {
                    if cancellation.generation() != generation_at_start
                        || stop_producer.load(Ordering::Acquire)
                    {
                        return ignore::WalkState::Quit;
                    }
                    match result {
                        Ok(entry) => {
                            if tx.send(entry).is_err() {
                                ignore::WalkState::Quit
                            } else {
                                ignore::WalkState::Continue
                            }
                        }
                        Err(_) => ignore::WalkState::Continue,
                    }
                })
            });
        });

        for entry in rx.iter() {
            // Deterministic seam for the cancellation tests: fires exactly
            // once, on this (the calling) thread, after the walk has produced
            // its first entry and before this iteration's generation check —
            // i.e. unambiguously *during* an in-flight pass. Replaces a
            // sleep-and-hope race that biased toward, but could not
            // guarantee, the same interleaving.
            #[cfg(test)]
            test_hooks::run_first_entry_hook();

            // A prior iteration already decided to stop this pass
            // (cancellation, `MAX_INDEXED_FILES_PER_PASS` truncation, or a
            // root identity change): `stop_producer` is already set so no
            // *new* work will be scheduled, but a worker thread already
            // blocked in `tx.send` on a full channel at the moment that flag
            // was set only unblocks once the consumer reads at least one
            // more entry -- so every one of these three reasons must keep
            // draining (discarding) here rather than abandoning `rx`
            // outright, or that worker thread (and the `std::thread::scope`
            // join waiting on it) would hang forever.
            if outcome.cancelled || outcome.truncated || outcome.root_identity_changed {
                continue;
            }
            if cancellation.generation() != generation_at_start {
                outcome.cancelled = true;
                stop_producer.store(true, Ordering::Release);
                continue;
            }
            if seen_path_keys.len() + batch.len() >= MAX_INDEXED_FILES_PER_PASS {
                outcome.truncated = true;
                stop_producer.store(true, Ordering::Release);
                continue;
            }
            match process_entry(&root_path, trusted_root, classifier, &fingerprints, &entry) {
                Ok(EntryOutcome::Skip) => outcome.files_skipped += 1,
                Ok(EntryOutcome::NotFound) => outcome.files_skipped += 1,
                Ok(EntryOutcome::TransientError { path_key }) => {
                    // Not proof of deletion -- see `EntryOutcome::TransientError`'s
                    // doc comment. Marking it seen keeps an already-indexed
                    // row alive through this pass's mark-and-sweep step so a
                    // single permission-denied/sharing-violation/other
                    // transient failure never deletes a still-existing
                    // file's index entry.
                    seen_path_keys.insert(path_key);
                    outcome.files_errored += 1;
                }
                Ok(EntryOutcome::Unchanged { path_key }) => {
                    seen_path_keys.insert(path_key);
                    outcome.files_unchanged += 1;
                }
                Ok(EntryOutcome::Changed { path_key, record }) => {
                    seen_path_keys.insert(path_key);
                    batch.push(record);
                    if batch.len() >= INDEX_BATCH_SIZE {
                        store.upsert_batch(account, project_id, &batch)?;
                        outcome.files_indexed += batch.len();
                        batch.clear();
                    }
                }
                Err(IndexError::Containment(ContainmentError::RootIdentityChanged)) => {
                    // Fatal for the whole pass, not a per-file skip: this
                    // recurs identically for every remaining candidate (see
                    // the doc comment above), so continuing to count it as
                    // "skipped" would let mark-and-sweep below conclude every
                    // not-yet-reached file was deleted and wipe an otherwise
                    // healthy index. Stop the producer too and report it
                    // distinctly from `cancelled` so a caller can tell
                    // "operator cancelled" from "this project needs
                    // re-trusting" apart.
                    outcome.root_identity_changed = true;
                    stop_producer.store(true, Ordering::Release);
                }
                Err(_) => {
                    // `process_entry` only returns `Err` for
                    // `RootIdentityChanged` (matched above) -- every other
                    // containment/IO failure it encounters is translated
                    // into `Ok(EntryOutcome::TransientError { .. })` or
                    // `Ok(EntryOutcome::NotFound)` before reaching here (see
                    // `containment_outcome`). This arm exists only so a
                    // future, currently-unreachable `IndexError` variant
                    // still fails safe (counted, not silently dropped or
                    // panicking) rather than assuming the match above is
                    // exhaustive of everything `process_entry` could ever
                    // return.
                    outcome.files_skipped += 1;
                }
            }
        }
        if !batch.is_empty() {
            store.upsert_batch(account, project_id, &batch)?;
            outcome.files_indexed += batch.len();
        }
        Ok(())
    });
    pass_result?;

    if !outcome.cancelled && !outcome.truncated && !outcome.root_identity_changed {
        let stale: Vec<Vec<u8>> = fingerprints
            .keys()
            .filter(|key| !seen_path_keys.contains(*key))
            .cloned()
            .collect();
        if !stale.is_empty() {
            outcome.files_removed = store.delete_by_path_keys(account, project_id, &stale)?;
        }
    }

    Ok(outcome)
}

/// Validates and classifies one `ignore`-proposed candidate. See the module
/// doc comment's "`ignore` proposes; `TrustedRoot` decides" section.
///
/// A [`ContainmentError::RootIdentityChanged`] surfaced here is
/// deterministic and total, not a one-off: [`TrustedRoot::resolve_existing`]
/// revalidates `trusted_root`'s identity against the OS on *every* call, so
/// once the on-disk root's identity stops matching what `trusted_root` was
/// constructed/reopened with, every remaining candidate in this pass will
/// fail the exact same way. [`run_index_pass`] treats this as fatal for the
/// whole pass (`IndexPassOutcome::root_identity_changed`) rather than
/// per-file `files_skipped` for exactly this reason -- see that function's
/// dedicated match arm. Every *other* containment or I/O failure
/// encountered here is deliberately NOT propagated as an `Err`: see
/// [`containment_outcome`] and [`EntryOutcome::TransientError`] for why only
/// a genuine [`ContainmentError::NotFound`] is treated as "this file no
/// longer exists."
fn process_entry(
    root_path: &Path,
    trusted_root: &TrustedRoot,
    classifier: &SecretClassifier,
    fingerprints: &HashMap<Vec<u8>, (u64, i64)>,
    entry: &ignore::DirEntry,
) -> Result<EntryOutcome, IndexError> {
    let Some(file_type) = entry.file_type() else {
        return Ok(EntryOutcome::Skip);
    };
    if !file_type.is_file() {
        // Directories are traversal, not searchable leaves; symlinks and
        // other special types are not covered by this indexer (a symlink
        // `ignore` still surfaced despite `follow_links(false)` is rejected
        // a second time below anyway, once its relative path resolves
        // through `TrustedRoot`).
        return Ok(EntryOutcome::Skip);
    }
    let Ok(relative) = entry.path().strip_prefix(root_path) else {
        return Ok(EntryOutcome::Skip);
    };
    // `from_os_relative`, not `new`: the walk yields `dir\file` on Windows and
    // `dir/file` on Unix, and indexing the host's own form would make stored
    // paths — and every search result and model-visible path derived from
    // them — differ across platforms for one identical project tree.
    let Ok(relative_path) = RelativeProjectPath::from_os_relative(relative) else {
        return Ok(EntryOutcome::Skip);
    };
    let key = path_key(&relative_path);

    // Defense in depth: independently re-validate containment through
    // `TrustedRoot` for every candidate, never trusting the `ignore` walk
    // alone. A failure here (e.g. a symlink that slipped through as a
    // surfaced-but-not-followed entry, or a permission/sharing-violation
    // error) is translated by `containment_outcome` below, not treated as
    // fatal for the whole pass.
    let resolved = match trusted_root.resolve_existing(relative_path.clone()) {
        Ok(resolved) => resolved,
        Err(error) => return containment_outcome(error, key),
    };
    let metadata = match trusted_root.file_metadata(&resolved) {
        Ok(metadata) => metadata,
        Err(error) => return containment_outcome(error, key),
    };
    if !metadata.is_file() {
        return Ok(EntryOutcome::Skip);
    }

    let size_bytes = metadata.len();
    let modified_at_ms = modified_millis(&metadata);

    if let Some((old_size, old_modified)) = fingerprints.get(&key) {
        if *old_size == size_bytes && *old_modified == modified_at_ms {
            return Ok(EntryOutcome::Unchanged { path_key: key });
        }
    }

    let is_sensitive = classifier.classify(&relative_path).is_some();
    let (content_kind, content_hash) = if size_bytes > MAX_HASHED_FILE_BYTES {
        (ContentKind::Oversized, None)
    } else {
        let file = match trusted_root.open_file(&resolved) {
            Ok(file) => file,
            Err(error) => return containment_outcome(error, key),
        };
        match hash_and_classify_streaming(file, MAX_HASHED_FILE_BYTES) {
            Ok((kind, hash)) => (kind, Some(hash)),
            Err(_io_error) => {
                // Reading a file this call already confirmed exists and is a
                // regular file failed -- a transient I/O condition
                // (permission change, sharing violation, disk hiccup), not
                // proof the file was deleted. See
                // `EntryOutcome::TransientError`.
                return Ok(EntryOutcome::TransientError { path_key: key });
            }
        }
    };

    Ok(EntryOutcome::Changed {
        path_key: key,
        record: IndexedFileRecord {
            relative_path,
            size_bytes,
            modified_at_ms,
            indexed_at_ms: chrono::Utc::now().timestamp_millis(),
            content_kind,
            content_hash,
            is_sensitive,
        },
    })
}

/// Converts a [`ContainmentError`] encountered mid-[`process_entry`] into the
/// [`EntryOutcome`] `run_index_pass`'s mark-and-sweep step must see to behave
/// correctly:
///
/// - [`ContainmentError::RootIdentityChanged`] is fatal for the whole pass,
///   not a per-candidate outcome -- propagated as `Err` so
///   [`run_index_pass`]'s dedicated match arm handles it (see that
///   function).
/// - [`ContainmentError::NotFound`] means the candidate genuinely no longer
///   exists (deleted between `ignore` surfacing it and this containment
///   re-check) -- [`EntryOutcome::NotFound`] is correctly excluded from
///   `seen_path_keys` so mark-and-sweep prunes its row, exactly as before
///   this function existed.
/// - Every other variant (`Io`, `LinkOrReparse`, `Platform`, `NotADirectory`,
///   `InvalidComponent`, `MismatchedRoot`) is a transient or unexpected
///   failure that is NOT proof of deletion -- a permission-denied/
///   sharing-violation/other transient I/O error is the common real-world
///   case (e.g. a Windows indexer/AV file lock). Returning
///   [`EntryOutcome::TransientError`] keeps `path_key` marked "seen" so an
///   already-indexed, still-existing file's row survives this pass instead
///   of being silently deleted by mark-and-sweep over a one-off failure.
fn containment_outcome(
    error: ContainmentError,
    path_key: Vec<u8>,
) -> Result<EntryOutcome, IndexError> {
    match error {
        ContainmentError::RootIdentityChanged => Err(IndexError::Containment(
            ContainmentError::RootIdentityChanged,
        )),
        ContainmentError::NotFound => Ok(EntryOutcome::NotFound),
        _ => Ok(EntryOutcome::TransientError { path_key }),
    }
}

/// Everything [`ProjectIndexer::start`] needs to run one pass on a
/// background thread without borrowing anything from its caller.
pub struct IndexPassRequest {
    pub project_id: ProjectId,
    pub canonical_path: PathBuf,
    pub expected_identity: RootIdentity,
    pub account: ValidatedAccountId,
    pub secrets: Arc<dyn SecretStore>,
    pub index_db_path: PathBuf,
    pub cancellation: CancellationSignal,
    pub classifier: Arc<SecretClassifier>,
    pub policy: IndexPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IndexStartError {
    #[error("an index pass is already running on this indexer")]
    Busy,
    #[error("failed to start the background indexing thread")]
    WorkerUnavailable,
}

/// Runs one project's index pass on a dedicated, named OS thread outside
/// the Tauri/UI async core -- mirrors
/// `crate::security::account_cleanup::AccountCleanup::start`'s exact shape:
/// a named `thread::Builder` spawn, an `AtomicBool` busy-guard reset via a
/// `Drop` guard, a `catch_unwind` boundary so a panic inside the walk
/// cannot poison anything, and a `tokio::sync::oneshot` channel bridging
/// the background thread's result back to an `async fn` a Tauri command
/// layer can await. Guards one concurrent pass **per instance**; a caller
/// managing several trusted projects holds one `ProjectIndexer` per
/// project, the same per-scope-instance discipline
/// `super::trust::ProjectsService.active`'s per-account map already uses.
/// **No production caller exists yet** -- wiring a real trigger (on
/// project activation, on a schedule, on file-system change notification)
/// into the Tauri command layer is Task 7's job, the same disclosed-seam
/// pattern already used for `crate::skills::discovery::discover_project`'s
/// `TrustState` adapter and `super::trust::CancellationSignal` itself.
#[derive(Clone)]
pub struct ProjectIndexer {
    busy: Arc<AtomicBool>,
}

impl Default for ProjectIndexer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectIndexer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&self, request: IndexPassRequest) -> Result<IndexWaiter, IndexStartError> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| IndexStartError::Busy)?;
        let busy = self.busy.clone();
        let (result_tx, result_rx) = oneshot::channel();
        if thread::Builder::new()
            .name("xfalcon-project-index".to_owned())
            .spawn(move || {
                let _reset = BusyReset(busy);
                let outcome = catch_unwind(AssertUnwindSafe(|| run_request(request)))
                    .unwrap_or(Err(IndexError::Panicked));
                let _ = result_tx.send(outcome);
            })
            .is_err()
        {
            self.busy.store(false, Ordering::Release);
            return Err(IndexStartError::WorkerUnavailable);
        }
        Ok(IndexWaiter { result: result_rx })
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
}

struct BusyReset(Arc<AtomicBool>);

impl Drop for BusyReset {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
pub struct IndexWaiter {
    result: oneshot::Receiver<Result<IndexPassOutcome, IndexError>>,
}

impl IndexWaiter {
    pub async fn wait(self) -> Result<IndexPassOutcome, IndexError> {
        self.result.await.unwrap_or(Err(IndexError::Panicked))
    }
}

fn run_request(request: IndexPassRequest) -> Result<IndexPassOutcome, IndexError> {
    let trusted_root = TrustedRoot::reopen(
        request.project_id,
        request.canonical_path,
        request.expected_identity,
    )?;
    let store = IndexStore::open(&request.index_db_path, request.secrets)?;
    run_index_pass(
        &trusted_root,
        &store,
        &request.account,
        &request.cancellation,
        &request.classifier,
        &request.policy,
    )
}

#[cfg(test)]
pub(crate) mod test_hooks {
    use std::cell::RefCell;

    thread_local! {
        static FIRST_ENTRY_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
            const { RefCell::new(None) };
    }

    /// Runs (and consumes) the hook installed by [`OnFirstIndexedEntry`], if
    /// any. Called from `run_index_pass`'s consumer loop, which executes on
    /// the caller's own thread — so a thread-local is the right storage even
    /// though the walk itself is parallel.
    pub(crate) fn run_first_entry_hook() {
        let hook = FIRST_ENTRY_HOOK.with(|cell| cell.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Installs `hook` to fire once, mid-pass, the moment the walk yields its
    /// first entry.
    ///
    /// This exists so the cancellation tests can prove the behaviour they
    /// actually care about — "a generation bump that lands *during* an
    /// in-flight pass marks it cancelled and prunes nothing" — without a
    /// two-thread race. The previous fixtures wrote 20,000 filler files and
    /// slept 2 ms in a cancelling thread to *bias* the interleaving, which
    /// their own comment conceded could not guarantee it; on a fast enough
    /// machine the bump landed outside the window and the assertion failed
    /// for reasons unrelated to the code under test.
    ///
    /// Resets on drop, so a leaked guard cannot affect a later test even when
    /// the harness reuses the thread.
    pub(crate) struct OnFirstIndexedEntry;

    impl OnFirstIndexedEntry {
        pub(crate) fn install(hook: impl FnOnce() + 'static) -> Self {
            FIRST_ENTRY_HOOK.with(|cell| *cell.borrow_mut() = Some(Box::new(hook)));
            Self
        }
    }

    impl Drop for OnFirstIndexedEntry {
        fn drop(&mut self) {
            FIRST_ENTRY_HOOK.with(|cell| *cell.borrow_mut() = None);
        }
    }
}
