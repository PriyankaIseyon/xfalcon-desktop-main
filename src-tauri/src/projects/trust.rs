//! Trust persistence and the Tauri-facing project runtime.
//!
//! [`TrustStore`] seals/opens `trusted_projects` rows exactly the way
//! `crate::chat::local_projection::EncryptedChatProjection` seals chat rows
//! (AES-256-GCM via `crate::persistence::crypto`, a fresh `Connection` opened
//! per call rather than held across calls -- see
//! `crate::chat::service::ChatService::sync_service`, which does the same).
//! [`ProjectsService`] is the runtime layer `src-tauri/src/app/commands.rs`
//! calls into; it resolves the current account, revalidates a project's
//! [`super::domain::TrustedRoot`] fresh on every list/activate (never trusts
//! a cached "still valid" flag), and tracks the active project per account
//! (mirroring `crate::chat::service::ChatService`'s
//! `active_conversations: Mutex<HashMap<String, ConversationId>>` idiom).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use xfalcon_contracts::projects::{ProjectListResult, ProjectTrustStatus, ProjectView};
use xfalcon_contracts::ConversationId;

use crate::domain::error::{AppError, AppResult};
use crate::persistence::{
    decrypt_project_trust, encrypt_project_trust, ProjectTrustAuthenticatedMetadata, SecretStore,
    PROJECT_TRUST_MIGRATION, PROJECT_TRUST_SCHEMA_VERSION,
};
use crate::security::account_cleanup::{
    account_directory_name, ProjectCancellation, ValidatedAccountId,
};

use super::domain::{ContainmentError, ProjectId};
use super::index::{IndexPassOutcome, IndexPassRequest, IndexPolicy, IndexStore, ProjectIndexer};
use super::platform::RootIdentity;
use super::TrustedRoot;

/// Directory (under an account's root) holding the trust store database and
/// the future project index / patch-temporary files (Task 5/6). Matches the
/// literal string `crate::security::account_cleanup`'s `ProjectDataHandler`
/// removes wholesale on cleanup -- kept as one named constant here since
/// this module is the one that builds paths under it; `account_cleanup.rs`
/// hard-codes the same literal directly, matching how it already hard-codes
/// `"local-projections"`/`"export"` rather than importing `chat`'s
/// equivalent constants.
pub const PROJECT_DATA_DIR: &str = "project-data";
const PROJECT_TRUST_DB_FILE: &str = "projects.sqlite3";
const PROJECT_INDEX_DB_FILE: &str = "project-index.sqlite3";

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error(transparent)]
    Containment(#[from] ContainmentError),
    #[error("the requested project was not found")]
    NotFound,
    #[error("the conversation is already bound to a different project")]
    ConversationAlreadyBound,
    #[error("the local project store is unavailable")]
    Storage,
}

impl From<AppError> for TrustError {
    fn from(_: AppError) -> Self {
        Self::Storage
    }
}

impl From<rusqlite::Error> for TrustError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

/// One trusted project's durable record: canonical path, stable platform
/// identity, display label, and trust timestamp. Never crosses the Tauri IPC
/// boundary directly -- `ProjectsService` projects it into
/// `xfalcon_contracts::projects::ProjectView`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRecord {
    pub project_id: ProjectId,
    pub canonical_path: PathBuf,
    pub display_name: String,
    pub identity: RootIdentity,
    pub trusted_at_ms: i64,
    deletion_evidence: Option<DeletionEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeletionEvidence {
    UnlinkedDirectoryV1,
}

#[derive(Serialize, Deserialize)]
struct ProjectPayload {
    canonical_path: String,
    display_name: String,
    identity_primary: u64,
    identity_secondary: u64,
    #[serde(default)]
    // The former deletion_confirmed boolean included reversible Windows
    // pending-delete states. Ignore that field; only versioned unlink proof
    // can authorize durable cleanup after restart.
    deletion_evidence: Option<DeletionEvidence>,
}

impl ProjectPayload {
    fn from_record(record: &ProjectRecord) -> Self {
        let (primary, secondary) = record.identity.into_parts();
        Self {
            canonical_path: record.canonical_path.to_string_lossy().into_owned(),
            display_name: record.display_name.clone(),
            identity_primary: primary,
            identity_secondary: secondary,
            deletion_evidence: record.deletion_evidence,
        }
    }
}

struct ProjectRow {
    project_id: String,
    created_at_ms: i64,
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

/// Encrypted local storage for one account's trusted-project records.
pub struct TrustStore {
    connection: Mutex<Connection>,
    secrets: Arc<dyn SecretStore>,
}

impl TrustStore {
    pub fn open(path: impl AsRef<Path>, secrets: Arc<dyn SecretStore>) -> Result<Self, TrustError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| TrustError::Storage)?;
        }
        let connection = open_connection(path).map_err(|_| TrustError::Storage)?;
        Ok(Self {
            connection: Mutex::new(connection),
            secrets,
        })
    }

    /// Canonicalizes and opens `root` through `cap-std`, records its stable
    /// identity, and seals a new trust row. Fails closed (no row written) if
    /// the root cannot be established as a valid, plain, link-free
    /// directory.
    pub fn create(
        &self,
        account: &ValidatedAccountId,
        root: &Path,
        display_name: Option<String>,
    ) -> Result<ProjectRecord, TrustError> {
        let project_id = ProjectId::new();
        let trusted_root = TrustedRoot::establish(project_id, root)?;
        let display_name = display_name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| default_display_name(trusted_root.canonical_path()));
        let record = ProjectRecord {
            project_id,
            canonical_path: trusted_root.canonical_path().to_path_buf(),
            display_name,
            identity: trusted_root.identity(),
            trusted_at_ms: chrono::Utc::now().timestamp_millis(),
            deletion_evidence: None,
        };
        self.insert(account, &record)?;
        Ok(record)
    }

    pub fn list(&self, account: &ValidatedAccountId) -> Result<Vec<ProjectRecord>, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let mut statement = connection.prepare(
            "SELECT project_id, created_at_ms, aad_schema_version, key_version, nonce, ciphertext
             FROM trusted_projects WHERE account_id = ?1
             ORDER BY created_at_ms ASC, project_id ASC",
        )?;
        let rows = statement.query_map(params![account.as_str()], row_from_sql)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(self.record_from_row(account, row?)?);
        }
        Ok(records)
    }

    pub fn get(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<Option<ProjectRecord>, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let row = connection
            .query_row(
                "SELECT project_id, created_at_ms, aad_schema_version, key_version, nonce, ciphertext
                 FROM trusted_projects WHERE account_id = ?1 AND project_id = ?2",
                params![account.as_str(), project_id.to_string()],
                row_from_sql,
            )
            .optional()?;
        row.map(|row| self.record_from_row(account, row))
            .transpose()
    }

    /// Deletes one project's trust row. Returns whether a row was removed.
    pub fn revoke(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<bool, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let removed = connection.execute(
            "DELETE FROM trusted_projects WHERE account_id = ?1 AND project_id = ?2",
            params![account.as_str(), project_id.to_string()],
        )?;
        Ok(removed > 0)
    }

    /// Durably binds a DA conversation to one device-local trusted project.
    /// Repeating the same binding is idempotent; changing an existing binding
    /// fails closed so opening or sending a chat can never silently retarget
    /// its filesystem authority.
    pub fn bind_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
        project_id: ProjectId,
    ) -> Result<(), TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let inserted = connection.execute(
            "INSERT INTO conversation_project_bindings (
                account_id, conversation_id, project_id, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, conversation_id) DO NOTHING",
            params![
                account.as_str(),
                conversation_id.as_str(),
                project_id.to_string(),
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
        if inserted > 0 {
            return Ok(());
        }

        let existing: String = connection.query_row(
            "SELECT project_id FROM conversation_project_bindings
             WHERE account_id = ?1 AND conversation_id = ?2",
            params![account.as_str(), conversation_id.as_str()],
            |row| row.get(0),
        )?;
        if existing == project_id.to_string() {
            Ok(())
        } else {
            Err(TrustError::ConversationAlreadyBound)
        }
    }

    pub fn project_for_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
    ) -> Result<Option<ProjectId>, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let project_id: Option<String> = connection
            .query_row(
                "SELECT project_id FROM conversation_project_bindings
                 WHERE account_id = ?1 AND conversation_id = ?2",
                params![account.as_str(), conversation_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        project_id
            .map(|id| ProjectId::from_str(&id).map_err(|_| TrustError::Storage))
            .transpose()
    }

    pub fn conversation_bindings(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<HashMap<String, ProjectId>, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let mut statement = connection.prepare(
            "SELECT conversation_id, project_id FROM conversation_project_bindings
             WHERE account_id = ?1",
        )?;
        let rows = statement.query_map(params![account.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut bindings = HashMap::new();
        for row in rows {
            let (conversation_id, project_id) = row?;
            let project_id = ProjectId::from_str(&project_id).map_err(|_| TrustError::Storage)?;
            bindings.insert(conversation_id, project_id);
        }
        Ok(bindings)
    }

    pub fn unbind_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
    ) -> Result<(), TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        connection.execute(
            "DELETE FROM conversation_project_bindings
             WHERE account_id = ?1 AND conversation_id = ?2",
            params![account.as_str(), conversation_id.as_str()],
        )?;
        Ok(())
    }

    /// Remembers which project a saved report (a DA kit) was created in.
    ///
    /// The first origin wins: a repeat, or a different project later, changes
    /// nothing and answers `false`. A report is linked to where it began, not to
    /// wherever it was last edited. Fails (`Storage`) for a project the account
    /// does not hold, through the foreign key.
    pub fn record_report_origin(
        &self,
        account: &ValidatedAccountId,
        kit_id: &str,
        project_id: ProjectId,
        conversation_id: &ConversationId,
    ) -> Result<bool, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let inserted = connection.execute(
            "INSERT INTO report_kit_projects (
                account_id, kit_id, project_id, conversation_id, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account_id, kit_id) DO NOTHING",
            params![
                account.as_str(),
                kit_id,
                project_id.to_string(),
                conversation_id.as_str(),
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Every linked report of the account, by DA kit id.
    pub fn report_origins(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<HashMap<String, ProjectId>, TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        let mut statement = connection
            .prepare("SELECT kit_id, project_id FROM report_kit_projects WHERE account_id = ?1")?;
        let rows = statement.query_map(params![account.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut origins = HashMap::new();
        for row in rows {
            let (kit_id, project_id) = row?;
            let project_id = ProjectId::from_str(&project_id).map_err(|_| TrustError::Storage)?;
            origins.insert(kit_id, project_id);
        }
        Ok(origins)
    }

    /// Drops a report's link, when the report itself is deleted.
    pub fn forget_report_origin(
        &self,
        account: &ValidatedAccountId,
        kit_id: &str,
    ) -> Result<(), TrustError> {
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        connection.execute(
            "DELETE FROM report_kit_projects WHERE account_id = ?1 AND kit_id = ?2",
            params![account.as_str(), kit_id],
        )?;
        Ok(())
    }

    fn insert(
        &self,
        account: &ValidatedAccountId,
        record: &ProjectRecord,
    ) -> Result<(), TrustError> {
        let payload = ProjectPayload::from_record(record);
        let plaintext = serde_json::to_vec(&payload).map_err(|_| TrustError::Storage)?;
        let project_id = record.project_id.to_string();
        let sealed = seal(
            self.secrets.as_ref(),
            account.as_str(),
            &project_id,
            &plaintext,
        )?;
        let connection = self.connection.lock().map_err(|_| TrustError::Storage)?;
        // No stored content hash: the AEAD tag on `ciphertext` (AES-256-GCM,
        // see `seal` above) already gives this row cryptographic
        // tamper-evidence under the trust store's own key -- a separate hash
        // would authenticate the same ciphertext against the same tampering
        // threat a second time, not add defense-in-depth. Unlike `chat`'s
        // `content_hash` (consumed by an `ON CONFLICT DO UPDATE SET` upsert
        // path), this table does not use content hashes for updates, so a
        // hash column here would have zero consumers.
        connection.execute(
            "INSERT INTO trusted_projects (
                account_id, project_id, aad_schema_version,
                key_version, nonce, ciphertext, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                account.as_str(),
                project_id,
                i64::from(sealed.aad_schema_version),
                i64::from(sealed.key_version),
                sealed.nonce,
                sealed.ciphertext,
                record.trusted_at_ms,
            ],
        )?;
        Ok(())
    }

    #[cfg(unix)]
    fn confirm_deletion(
        &self,
        account: &ValidatedAccountId,
        record: &ProjectRecord,
    ) -> Result<(), TrustError> {
        let mut updated = record.clone();
        updated.deletion_evidence = Some(DeletionEvidence::UnlinkedDirectoryV1);
        let plaintext = serde_json::to_vec(&ProjectPayload::from_record(&updated))
            .map_err(|_| TrustError::Storage)?;
        let sealed = seal(
            self.secrets.as_ref(),
            account.as_str(),
            &record.project_id.to_string(),
            &plaintext,
        )?;
        self.connection.lock().map_err(|_| TrustError::Storage)?.execute(
            "UPDATE trusted_projects SET aad_schema_version = ?3, key_version = ?4, nonce = ?5, ciphertext = ?6
             WHERE account_id = ?1 AND project_id = ?2",
            params![account.as_str(), record.project_id.to_string(), sealed.aad_schema_version,
                sealed.key_version, sealed.nonce, sealed.ciphertext],
        )?;
        Ok(())
    }

    fn record_from_row(
        &self,
        account: &ValidatedAccountId,
        row: ProjectRow,
    ) -> Result<ProjectRecord, TrustError> {
        let plaintext = open(self.secrets.as_ref(), account.as_str(), &row)?;
        let payload: ProjectPayload =
            serde_json::from_slice(&plaintext).map_err(|_| TrustError::Storage)?;
        let project_id = ProjectId::from_str(&row.project_id).map_err(|_| TrustError::Storage)?;
        Ok(ProjectRecord {
            project_id,
            canonical_path: PathBuf::from(payload.canonical_path),
            display_name: payload.display_name,
            identity: RootIdentity::from_parts(
                payload.identity_primary,
                payload.identity_secondary,
            ),
            trusted_at_ms: row.created_at_ms,
            deletion_evidence: payload.deletion_evidence,
        })
    }
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        project_id: row.get(0)?,
        created_at_ms: row.get(1)?,
        aad_schema_version: row.get(2)?,
        key_version: row.get(3)?,
        nonce: row.get(4)?,
        ciphertext: row.get(5)?,
    })
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
    plaintext: &[u8],
) -> AppResult<Sealed> {
    // Mint-on-first-use — see `chat::local_projection::seal`: a seal that only
    // reads the active key fails on a device that has never sealed before.
    let (key_version, key) = secrets.load_or_create_active_key(account_id)?;
    let metadata = ProjectTrustAuthenticatedMetadata {
        account_id,
        project_id,
    };
    let encrypted = encrypt_project_trust(&key, key_version, &metadata, plaintext)?;
    Ok(Sealed {
        aad_schema_version: encrypted.aad_schema_version,
        key_version,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
}

fn open(secrets: &dyn SecretStore, account_id: &str, row: &ProjectRow) -> AppResult<Vec<u8>> {
    let aad_schema_version =
        u32::try_from(row.aad_schema_version).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key_version =
        u32::try_from(row.key_version).map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key = secrets.account_key(account_id, key_version)?;
    let metadata = ProjectTrustAuthenticatedMetadata {
        account_id,
        project_id: &row.project_id,
    };
    decrypt_project_trust(
        &key,
        aad_schema_version,
        key_version,
        &row.nonce,
        &metadata,
        &row.ciphertext,
    )
}

fn default_display_name(canonical_path: &Path) -> String {
    canonical_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| canonical_path.to_string_lossy().into_owned())
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
    if version > PROJECT_TRUST_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_TRUST_SCHEMA_VERSION {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > PROJECT_TRUST_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_TRUST_SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    transaction.execute_batch(PROJECT_TRUST_MIGRATION)?;
    transaction.pragma_update(None, "user_version", PROJECT_TRUST_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn project_trust_db_path(app_root: &Path, account: &ValidatedAccountId) -> PathBuf {
    project_data_dir(app_root, account).join(PROJECT_TRUST_DB_FILE)
}

fn project_index_db_path(app_root: &Path, account: &ValidatedAccountId) -> PathBuf {
    project_data_dir(app_root, account).join(PROJECT_INDEX_DB_FILE)
}

fn project_data_dir(app_root: &Path, account: &ValidatedAccountId) -> PathBuf {
    app_root
        .join("accounts")
        .join(account_directory_name(account.as_str()))
        .join(PROJECT_DATA_DIR)
}

/// A cheap, cloneable handle onto one shared cancellation generation
/// counter. `crate::security::account_cleanup::ProjectDataHandler` bumps it
/// on account cleanup; `ProjectsService::revoke` bumps it on an explicit
/// per-project revoke. Task 5's indexing/read/write loops (not implemented
/// yet) are expected to capture [`Self::generation`] at operation start and
/// periodically compare -- a changed value means "stop now, the project (or
/// account) that authorized this work is no longer trusted."
#[derive(Clone, Default)]
pub struct CancellationSignal(Arc<AtomicU64>);

impl CancellationSignal {
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    /// Test-only escape hatch onto the same private [`Self::bump`] production
    /// callers (`ProjectsService::revoke`/`ProjectCancellation::cancel_all_projects`)
    /// use, so `super::index`/`super::read` tests can simulate "the account or
    /// project that authorized this work is no longer trusted" without
    /// needing a full `ProjectsService`/`TrustStore` harness. Mirrors
    /// `crate::tools::registry::ToolDescriptor::for_test_with_raw_schema`'s
    /// identical "test-only escape hatch" naming and `#[cfg(test)]` gating.
    #[cfg(test)]
    pub(crate) fn force_bump_for_test(&self) {
        self.bump();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectsServiceError {
    #[error("no account is currently active")]
    NoActiveAccount,
    #[error("the requested project was not found")]
    NotFound,
    #[error(transparent)]
    Containment(#[from] ContainmentError),
    #[error(transparent)]
    Trust(#[from] TrustError),
    #[error("the local project store is unavailable")]
    Unavailable,
}

/// The Tauri-facing project runtime: resolves the current account, opens a
/// fresh [`TrustStore`] per call (matching
/// `crate::chat::service::ChatService::sync_service`'s per-call
/// `EncryptedChatProjection::open`), and tracks the active project per
/// account in memory only (never persisted -- restarting the app or
/// switching accounts starts with no active project, matching
/// `ChatService::active_conversations`'s equivalent in-memory-only scope).
pub struct ProjectsService {
    app_root: PathBuf,
    accounts: Arc<crate::security::account_cleanup::CurrentAccountScope>,
    secrets: Arc<dyn SecretStore>,
    active: Mutex<HashMap<String, ProjectId>>,
    index_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    cancellation: CancellationSignal,
    observed_roots: Mutex<HashMap<(String, ProjectId), super::platform::RootObserver>>,
    /// Best-effort `project.trust`/`project.revoke` audit publication — see
    /// [`Self::with_audit`]. `None` (the default) means this service simply
    /// never publishes, exactly like `crate::agent::tool_loop::ToolLoop`
    /// with no `ToolLoopAudit` configured.
    audit: Option<crate::audit::AuditPublisher>,
}

impl ProjectsService {
    #[must_use]
    pub fn new(
        app_root: PathBuf,
        accounts: Arc<crate::security::account_cleanup::CurrentAccountScope>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            app_root,
            accounts,
            secrets,
            active: Mutex::new(HashMap::new()),
            index_locks: Mutex::new(HashMap::new()),
            cancellation: CancellationSignal::default(),
            observed_roots: Mutex::new(HashMap::new()),
            audit: None,
        }
    }

    /// Wires a real `crate::audit::AuditSink`/`crate::audit::AuditQueue`
    /// pair for `project.trust`/`project.revoke` events (Task 7 Step 3's
    /// "publish content-free DA audit events ... for project
    /// trust/revocation" — previously wired nowhere in this crate at all,
    /// a review finding caught this re-opened a Task 6 SOC2 finding rather
    /// than closing it). No production caller supplies one yet — the real
    /// actor/tenant/device identity to construct
    /// `crate::audit::AuditPublisher` with is only available once a Tauri
    /// composition root threads it through (`lib.rs`/`app/commands.rs`,
    /// same disclosed-seam shape as `crate::agent::tool_loop::ToolLoop::with_audit`
    /// and `crate::approvals::service::ApprovalsService::with_da_sync`).
    #[must_use]
    pub fn with_audit(mut self, audit: crate::audit::AuditPublisher) -> Self {
        self.audit = Some(audit);
        self
    }

    #[must_use]
    pub fn cancellation(&self) -> CancellationSignal {
        self.cancellation.clone()
    }

    pub async fn create(
        &self,
        root_path: &str,
        display_name: Option<String>,
    ) -> Result<ProjectView, ProjectsServiceError> {
        let account = self.current_account()?;
        let store = self.store_for(&account)?;
        let record = store.create(&account, Path::new(root_path), display_name)?;
        self.observe_root(&account, &record);
        self.set_active(&account, record.project_id);
        self.publish_audit(
            crate::audit::AuditAction::ProjectTrust,
            crate::audit::AuditOutcome::Success,
            &record.project_id.to_string(),
        )
        .await;
        Ok(view(&record, true, ProjectTrustStatus::Trusted))
    }

    pub fn list(&self) -> Result<ProjectListResult, ProjectsServiceError> {
        let account = self.current_account()?;
        let store = self.store_for(&account)?;
        let active = self.active_for(&account);
        let records = store.list(&account)?;
        let projects = records
            .iter()
            .map(|record| {
                let is_active = active == Some(record.project_id);
                let status = revalidate_status(record);
                if status == ProjectTrustStatus::Trusted {
                    self.observe_root(&account, record);
                }
                view(record, is_active, status)
            })
            .collect();
        Ok(ProjectListResult::new(projects))
    }

    /// Finds existing projects for a picked directory without granting trust or
    /// changing the active project. Compare platform identities, not path text:
    /// case aliases and canonical paths must follow the actual filesystem.
    pub fn find_by_folder(
        &self,
        root_path: &str,
    ) -> Result<ProjectListResult, ProjectsServiceError> {
        let account = self.current_account()?;
        let selected = TrustedRoot::establish(ProjectId::new(), Path::new(root_path))?;
        let store = self.store_for(&account)?;
        let active = self.active_for(&account);
        let projects = store
            .list(&account)?
            .into_iter()
            .filter(|record| {
                record.deletion_evidence.is_none()
                    && record.identity == selected.identity()
                    && revalidate_status(record) == ProjectTrustStatus::Trusted
            })
            .map(|record| {
                view(
                    &record,
                    active == Some(record.project_id),
                    ProjectTrustStatus::Trusted,
                )
            })
            .collect();
        Ok(ProjectListResult::new(projects))
    }

    fn observe_root(&self, account: &ValidatedAccountId, record: &ProjectRecord) {
        let mut roots = self
            .observed_roots
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let key = (account.as_str().to_owned(), record.project_id);
        if roots.contains_key(&key) || record.deletion_evidence.is_some() {
            return;
        }
        let Ok(root) = super::platform::open_observer_root(&record.canonical_path) else {
            return;
        };
        if super::platform::root_identity(&root.root).ok() == Some(record.identity) {
            roots.insert(key, root);
        }
    }

    /// Returns confirmed unlink evidence, never merely missing paths or Windows
    /// pending deletion. Persist proof before chat cleanup for interrupted retries.
    pub fn deleted_projects(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<Vec<ProjectId>, ProjectsServiceError> {
        let store = self.store_for(account)?;
        let mut deleted = Vec::new();
        for record in store.list(account)? {
            if record.deletion_evidence.is_some() {
                deleted.push(record.project_id);
                continue;
            }
            let mut roots = self
                .observed_roots
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let key = (account.as_str().to_owned(), record.project_id);
            let state = roots
                .get_mut(&key)
                .and_then(|root| super::platform::root_deletion_state(root).ok());
            match state {
                #[cfg(unix)]
                Some(super::platform::RootDeletionState::Confirmed) => {
                    store.confirm_deletion(account, &record)?;
                    roots.remove(&key);
                    deleted.push(record.project_id);
                }
                #[cfg(windows)]
                Some(super::platform::RootDeletionState::Pending) => {
                    // DeletePending can be cancelled, even with NumberOfLinks == 0.
                    // Release our observer so it cannot postpone OS deletion, but
                    // retain trust and chats: missing paths cannot distinguish a
                    // completed deletion from a cancelled deletion followed by rename.
                    roots.remove(&key);
                }
                _ => {}
            }
        }
        Ok(deleted)
    }

    pub fn activate(
        &self,
        project_id: &xfalcon_contracts::projects::ProjectId,
    ) -> Result<ProjectView, ProjectsServiceError> {
        let account = self.current_account()?;
        let store = self.store_for(&account)?;
        let domain_id = parse_project_id(project_id)?;
        let record = store
            .get(&account, domain_id)?
            .ok_or(ProjectsServiceError::NotFound)?;
        // Activation must prove the root is valid *right now*, not merely
        // that it was valid when trusted -- fail closed rather than silently
        // activating a root whose identity has since changed.
        TrustedRoot::reopen(
            record.project_id,
            record.canonical_path.clone(),
            record.identity,
        )?;
        self.observe_root(&account, &record);
        self.set_active(&account, domain_id);
        Ok(view(&record, true, ProjectTrustStatus::Trusted))
    }

    /// Clears the ambient project selection without revoking trust or changing
    /// any conversation-to-project binding.
    pub fn deactivate(&self) -> Result<(), ProjectsServiceError> {
        let account = self.current_account()?;
        self.clear_active_project(&account);
        Ok(())
    }

    pub async fn revoke(
        &self,
        project_id: &xfalcon_contracts::projects::ProjectId,
    ) -> Result<(), ProjectsServiceError> {
        let account = self.current_account()?;
        self.revoke_for_account(&account, parse_project_id(project_id)?)
            .await
    }

    pub async fn revoke_for_account(
        &self,
        account: &ValidatedAccountId,
        domain_id: ProjectId,
    ) -> Result<(), ProjectsServiceError> {
        let store = self.store_for(account)?;
        let removed = store.revoke(account, domain_id)?;
        if !removed {
            return Err(ProjectsServiceError::NotFound);
        }
        self.observed_roots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(account.as_str().to_owned(), domain_id));
        self.clear_active_if(account, domain_id);
        self.cancellation.bump();
        self.publish_audit(
            crate::audit::AuditAction::ProjectRevoke,
            crate::audit::AuditOutcome::Success,
            &domain_id.to_string(),
        )
        .await;
        Ok(())
    }

    /// Best-effort, content-free audit publication — never blocks or
    /// reverses a trust/revoke decision this method already durably
    /// recorded (mirrors `crate::approvals::service::ApprovalsService::sync_da`'s
    /// established precedent). A no-op when [`Self::with_audit`] was never
    /// called.
    async fn publish_audit(
        &self,
        action: crate::audit::AuditAction,
        outcome: crate::audit::AuditOutcome,
        project_id: &str,
    ) {
        let Some(audit) = self.audit.as_ref() else {
            return;
        };
        audit
            .publish(
                action,
                crate::audit::AuditResourceType::Project,
                project_id,
                outcome,
                "projects.trust.v1",
                uuid::Uuid::new_v4().to_string().as_str(),
            )
            .await;
    }

    /// Reopens `project_id`'s trusted root, proving its identity is still
    /// what it was when the project was trusted.
    ///
    /// This is the accessor an agent tool dispatcher needs and the reason it
    /// is public: without it, every caller outside this module has to
    /// re-derive the trust-database path itself, which
    /// `crate::approvals::service` already does and already flags as a
    /// duplication smell in its own comments.
    pub fn trusted_root(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<TrustedRoot, ProjectsServiceError> {
        let record = self.project_record(account, project_id)?;
        TrustedRoot::reopen(project_id, record.canonical_path, record.identity)
            .map_err(ProjectsServiceError::from)
    }

    pub fn project_record(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<ProjectRecord, ProjectsServiceError> {
        self.store_for(account)?
            .get(account, project_id)?
            .ok_or(ProjectsServiceError::NotFound)
    }

    /// Completes one bounded incremental index pass away from the async/UI
    /// thread before search tools are advertised for a run. Per-project async
    /// locks serialize simultaneous runs; the second pass is incremental and
    /// normally skips every unchanged file.
    pub async fn refresh_index(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<IndexPassOutcome, ProjectsServiceError> {
        let key = format!("{}:{project_id}", account.as_str());
        let lock = self
            .index_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(key)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        let record = self.project_record(account, project_id)?;
        let waiter = ProjectIndexer::new()
            .start(IndexPassRequest {
                project_id,
                canonical_path: record.canonical_path,
                expected_identity: record.identity,
                account: account.clone(),
                secrets: self.secrets.clone(),
                index_db_path: self.index_db_path(account),
                cancellation: self.cancellation(),
                classifier: Arc::new(super::SecretClassifier::built_in()),
                policy: IndexPolicy::default(),
            })
            .map_err(|_| ProjectsServiceError::Unavailable)?;
        waiter
            .wait()
            .await
            .map_err(|_| ProjectsServiceError::Unavailable)
    }

    pub fn project_display_name(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<String, ProjectsServiceError> {
        Ok(self.project_record(account, project_id)?.display_name)
    }

    /// [`Self::trusted_root`] for a caller that is not already scoped to one
    /// account.
    ///
    /// An agent tool call is pinned to the account and project the run was
    /// authorized under, and passes both explicitly. A user-driven Tauri
    /// command (`super::commands`) has neither in hand and must ask who is
    /// signed in *now* — so it asks here rather than resolving the account and
    /// then re-deriving the trust-database path itself, which is exactly the
    /// duplication [`Self::trusted_root`] exists to prevent. Both routes end
    /// in the same store lookup and the same `TrustedRoot::reopen` identity
    /// revalidation.
    pub fn trusted_root_for_current_account(
        &self,
        project_id: ProjectId,
    ) -> Result<TrustedRoot, ProjectsServiceError> {
        let account = self.current_account()?;
        self.trusted_root(&account, project_id)
    }

    /// Opens the account's project index database — the store
    /// `crate::projects::search` reads candidates from.
    pub fn index_store(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<IndexStore, ProjectsServiceError> {
        IndexStore::open(self.index_db_path(account), self.secrets.clone())
            .map_err(|_| ProjectsServiceError::Unavailable)
    }

    /// Path of the account's project index database. Lives beside the trust
    /// database under the same per-account `PROJECT_DATA_DIR`, so
    /// `crate::security::account_cleanup`'s existing project-data handler
    /// removes it with everything else on account cleanup.
    #[must_use]
    pub fn index_db_path(&self, account: &ValidatedAccountId) -> PathBuf {
        project_index_db_path(&self.app_root, account)
    }

    /// The project this account has activated, if any.
    #[must_use]
    pub fn active_project(&self, account: &ValidatedAccountId) -> Option<ProjectId> {
        self.active_for(account)
    }

    pub fn clear_active_project(&self, account: &ValidatedAccountId) {
        self.clear_active(account);
    }

    /// Binds a newly created conversation to the project selected when it was
    /// created. An unbound conversation is valid and remains an ordinary chat.
    pub fn bind_active_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
    ) -> Result<Option<ProjectId>, ProjectsServiceError> {
        let Some(project_id) = self.active_for(account) else {
            return Ok(None);
        };
        self.bind_conversation(account, conversation_id, project_id)?;
        Ok(Some(project_id))
    }

    pub fn bind_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
        project_id: ProjectId,
    ) -> Result<(), ProjectsServiceError> {
        self.store_for(account)?
            .bind_conversation(account, conversation_id, project_id)?;
        Ok(())
    }

    pub fn project_for_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
    ) -> Result<Option<ProjectId>, ProjectsServiceError> {
        self.store_for(account)?
            .project_for_conversation(account, conversation_id)
            .map_err(ProjectsServiceError::from)
    }

    pub fn conversation_bindings(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<HashMap<String, ProjectId>, ProjectsServiceError> {
        self.store_for(account)?
            .conversation_bindings(account)
            .map_err(ProjectsServiceError::from)
    }

    /// Restores sidebar selection from the durable conversation binding.
    /// Ordinary unbound chats deliberately clear the account's project
    /// selection instead of inheriting unrelated ambient state.
    pub fn sync_active_to_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
    ) -> Result<Option<ProjectId>, ProjectsServiceError> {
        let project_id = self.project_for_conversation(account, conversation_id)?;
        match project_id {
            Some(project_id) => self.set_active(account, project_id),
            None => self.clear_active(account),
        }
        Ok(project_id)
    }

    pub fn unbind_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
    ) -> Result<(), ProjectsServiceError> {
        self.store_for(account)?
            .unbind_conversation(account, conversation_id)
            .map_err(ProjectsServiceError::from)
    }

    /// See [`TrustStore::record_report_origin`].
    pub fn record_report_origin(
        &self,
        account: &ValidatedAccountId,
        kit_id: &str,
        project_id: ProjectId,
        conversation_id: &ConversationId,
    ) -> Result<bool, ProjectsServiceError> {
        self.store_for(account)?
            .record_report_origin(account, kit_id, project_id, conversation_id)
            .map_err(ProjectsServiceError::from)
    }

    /// Each linked report of the account with the project's current name, for
    /// the Reports library. A link to a project whose record is gone cannot exist
    /// (the key cascades), so every entry names a project the account still has
    /// a row for. A project marked as deleted on disk keeps its row until the
    /// deletion is confirmed, so its reports keep their project meanwhile.
    pub fn report_origins(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<Vec<(String, ProjectId, String)>, ProjectsServiceError> {
        let store = self.store_for(account)?;
        let origins = store.report_origins(account)?;
        let names: HashMap<ProjectId, String> = store
            .list(account)?
            .into_iter()
            .map(|record| (record.project_id, record.display_name))
            .collect();
        let mut linked: Vec<_> = origins
            .into_iter()
            .filter_map(|(kit_id, project_id)| {
                names
                    .get(&project_id)
                    .map(|name| (kit_id, project_id, name.clone()))
            })
            .collect();
        linked.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(linked)
    }

    /// See [`TrustStore::forget_report_origin`].
    pub fn forget_report_origin(
        &self,
        account: &ValidatedAccountId,
        kit_id: &str,
    ) -> Result<(), ProjectsServiceError> {
        self.store_for(account)?
            .forget_report_origin(account, kit_id)
            .map_err(ProjectsServiceError::from)
    }

    fn store_for(&self, account: &ValidatedAccountId) -> Result<TrustStore, ProjectsServiceError> {
        let path = project_trust_db_path(&self.app_root, account);
        TrustStore::open(path, self.secrets.clone()).map_err(ProjectsServiceError::from)
    }

    fn current_account(&self) -> Result<ValidatedAccountId, ProjectsServiceError> {
        self.accounts
            .current()
            .map_err(|_| ProjectsServiceError::Unavailable)?
            .ok_or(ProjectsServiceError::NoActiveAccount)
    }

    fn active_for(&self, account: &ValidatedAccountId) -> Option<ProjectId> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(account.as_str())
            .copied()
    }

    fn set_active(&self, account: &ValidatedAccountId, project_id: ProjectId) {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(account.as_str().to_owned(), project_id);
    }

    fn clear_active_if(&self, account: &ValidatedAccountId, project_id: ProjectId) {
        let mut guard = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.get(account.as_str()) == Some(&project_id) {
            guard.remove(account.as_str());
        }
    }

    fn clear_active(&self, account: &ValidatedAccountId) {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(account.as_str());
    }
}

impl ProjectCancellation for ProjectsService {
    fn cancel_all_projects(&self, account: &ValidatedAccountId) {
        self.clear_active(account);
        self.cancellation.bump();
        self.observed_roots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(owner, _), _| owner != account.as_str());
    }
}

fn revalidate_status(record: &ProjectRecord) -> ProjectTrustStatus {
    match TrustedRoot::reopen(
        record.project_id,
        record.canonical_path.clone(),
        record.identity,
    ) {
        Ok(_) => ProjectTrustStatus::Trusted,
        Err(_) => ProjectTrustStatus::Unavailable,
    }
}

fn view(record: &ProjectRecord, active: bool, status: ProjectTrustStatus) -> ProjectView {
    ProjectView::new(
        record.project_id.to_string(),
        record.display_name.clone(),
        record.canonical_path.to_string_lossy().into_owned(),
        status,
        active,
        record.trusted_at_ms,
    )
}

fn parse_project_id(
    id: &xfalcon_contracts::projects::ProjectId,
) -> Result<ProjectId, ProjectsServiceError> {
    ProjectId::from_str(id.as_str()).map_err(|_| ProjectsServiceError::NotFound)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::persistence::InMemorySecretStore;
    use crate::security::account_cleanup::CurrentAccountScope;

    use super::*;

    const ACCOUNT_ID: &str = "account-1";

    #[test]
    fn legacy_pending_deletion_boolean_is_not_unlink_evidence() {
        let payload: ProjectPayload = serde_json::from_value(serde_json::json!({
            "canonical_path": "/project", "display_name": "Project",
            "identity_primary": 1, "identity_secondary": 2,
            "deletion_confirmed": true
        }))
        .expect("legacy payload remains readable");
        assert_eq!(payload.deletion_evidence, None);
    }

    #[cfg(windows)]
    fn set_pending_deletion(file: &std::fs::File, pending: bool) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
        };
        let information = FILE_DISPOSITION_INFO {
            DeleteFile: pending,
        };
        // SAFETY: borrowed live file handle and exactly sized disposition struct.
        assert_ne!(
            unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    FileDispositionInfo,
                    std::ptr::from_ref(&information).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_pending_delete_cancelled_then_renamed_preserves_chats() {
        use std::os::windows::fs::OpenOptionsExt;
        let app_root = tempdir().expect("app");
        let parent = tempdir().expect("parent");
        let root = parent.path().join("project");
        fs::create_dir(&root).expect("root");
        let (runtime, _) = service(app_root.path());
        runtime
            .create(&root.to_string_lossy(), None)
            .await
            .expect("trust");
        let conversation = ConversationId::from("project-chat");
        runtime
            .bind_active_conversation(&account(), &conversation)
            .expect("bind");
        let file = std::fs::OpenOptions::new()
            .access_mode(0x10080)
            .share_mode(7)
            .custom_flags(0x02000000)
            .open(&root)
            .expect("deletion-capable handle");
        set_pending_deletion(&file, true);
        assert!(runtime
            .deleted_projects(&account())
            .expect("pending is not proof")
            .is_empty());
        assert!(
            runtime.observed_roots.lock().unwrap().is_empty(),
            "observer must not block OS deletion"
        );
        set_pending_deletion(&file, false);
        drop(file);
        fs::rename(&root, parent.path().join("renamed"))
            .expect("cancelled deletion can be renamed");
        assert!(runtime
            .deleted_projects(&account())
            .expect("rename is not deletion")
            .is_empty());
        assert!(runtime
            .project_for_conversation(&account(), &conversation)
            .expect("binding")
            .is_some());
        assert_eq!(runtime.list().expect("retained").projects.len(), 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_permanent_delete_releases_observer_but_retains_unproven_project() {
        let app_root = tempdir().expect("app");
        let parent = tempdir().expect("parent");
        let root = parent.path().join("project");
        fs::create_dir(&root).expect("root");
        let (runtime, _) = service(app_root.path());
        runtime
            .create(&root.to_string_lossy(), None)
            .await
            .expect("trust");
        fs::remove_dir(&root).expect("request deletion");
        assert!(runtime
            .deleted_projects(&account())
            .expect("pending")
            .is_empty());
        assert!(
            !root.exists(),
            "observer release permits deletion to complete"
        );
        // Without a handle, a missing pathname cannot distinguish this actual
        // deletion from cancelled deletion followed by rename. Retain safely.
        assert!(runtime
            .deleted_projects(&account())
            .expect("missing alone")
            .is_empty());
        assert_eq!(runtime.list().expect("retained").projects.len(), 1);
    }

    fn account() -> ValidatedAccountId {
        ValidatedAccountId::parse(ACCOUNT_ID).expect("valid account id")
    }

    fn secrets() -> Arc<InMemorySecretStore> {
        let store = Arc::new(InMemorySecretStore::default());
        store.put_key(ACCOUNT_ID, [0x11; 32]).expect("account key");
        store
    }

    fn store(app_root: &Path) -> TrustStore {
        TrustStore::open(
            project_trust_db_path(app_root, &account()),
            secrets() as Arc<dyn SecretStore>,
        )
        .expect("open trust store")
    }

    #[test]
    fn create_seals_the_row_and_the_canonical_path_never_appears_in_ciphertext() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let store = store(app_root.path());

        let record = store
            .create(
                &account(),
                project_root.path(),
                Some("My Project".to_owned()),
            )
            .expect("create trusted project");
        assert_eq!(record.display_name, "My Project");

        let database_path = project_trust_db_path(app_root.path(), &account());
        let ciphertext: Vec<u8> = Connection::open(&database_path)
            .expect("open raw database")
            .query_row(
                "SELECT ciphertext FROM trusted_projects WHERE project_id = ?1",
                [record.project_id.to_string()],
                |row| row.get(0),
            )
            .expect("stored ciphertext");
        let canonical_path_bytes = record.canonical_path.to_string_lossy().into_owned();
        assert!(!ciphertext
            .windows(canonical_path_bytes.len().max(1))
            .any(|window| window == canonical_path_bytes.as_bytes()));
    }

    #[test]
    fn list_round_trips_the_display_name_and_canonical_path() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let store = store(app_root.path());
        let created = store
            .create(&account(), project_root.path(), None)
            .expect("create");

        let listed = store.list(&account()).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].project_id, created.project_id);
        assert_eq!(listed[0].canonical_path, created.canonical_path);
        assert_eq!(listed[0].display_name, created.display_name);
    }

    #[test]
    fn a_different_accounts_projects_are_never_returned_even_from_the_same_database_file() {
        // `project_trust_db_path` already segregates accounts into distinct
        // hashed directories (never a literally shared file in normal use --
        // mirrors `chat::local_projection::projection_db_path`'s identical
        // per-account layout), so this test opens one `TrustStore` directly
        // against a single fixed path shared by two account ids, proving the
        // SQL-level `account_id` scoping is real defense in depth, not
        // merely incidental to that directory layout.
        let database_path = tempdir()
            .expect("database directory")
            .path()
            .join("shared.sqlite3");
        let secrets = secrets();
        let store_for_account_1 =
            TrustStore::open(&database_path, secrets.clone() as Arc<dyn SecretStore>)
                .expect("open trust store");
        let project_root = tempdir().expect("project root");
        store_for_account_1
            .create(&account(), project_root.path(), None)
            .expect("create for account-1");

        let other_account = ValidatedAccountId::parse("account-2").expect("other account");
        secrets
            .put_key("account-2", [0x22; 32])
            .expect("other account key");
        let store_for_account_2 = TrustStore::open(&database_path, secrets as Arc<dyn SecretStore>)
            .expect("reopen the same database file");
        assert!(store_for_account_2
            .list(&other_account)
            .expect("list for the other account")
            .is_empty());
        assert_eq!(store_for_account_1.list(&account()).expect("list").len(), 1);
    }

    #[test]
    fn revoke_removes_the_row_and_a_second_revoke_reports_nothing_removed() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let store = store(app_root.path());
        let created = store
            .create(&account(), project_root.path(), None)
            .expect("create");

        assert!(store
            .revoke(&account(), created.project_id)
            .expect("first revoke"));
        assert!(!store
            .revoke(&account(), created.project_id)
            .expect("second revoke is a no-op"));
        assert!(store.list(&account()).expect("list").is_empty());
    }

    #[test]
    fn conversation_binding_survives_reopen_and_revoke_cascades_it() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let database_path = project_trust_db_path(app_root.path(), &account());
        let conversation_id = xfalcon_contracts::ConversationId::from("conversation-1");

        let store = store(app_root.path());
        let created = store
            .create(&account(), project_root.path(), None)
            .expect("create");
        store
            .bind_conversation(&account(), &conversation_id, created.project_id)
            .expect("bind conversation");
        drop(store);

        let reopened = TrustStore::open(&database_path, secrets() as Arc<dyn SecretStore>)
            .expect("reopen trust store");
        assert_eq!(
            reopened
                .project_for_conversation(&account(), &conversation_id)
                .expect("read binding"),
            Some(created.project_id)
        );

        reopened
            .revoke(&account(), created.project_id)
            .expect("revoke project");
        assert_eq!(
            reopened
                .project_for_conversation(&account(), &conversation_id)
                .expect("binding cascaded"),
            None
        );
    }

    #[test]
    fn conversation_cannot_be_silently_rebound_to_another_project() {
        let app_root = tempdir().expect("app root");
        let first_root = tempdir().expect("first project root");
        let second_root = tempdir().expect("second project root");
        let store = store(app_root.path());
        let first = store
            .create(&account(), first_root.path(), None)
            .expect("create first project");
        let second = store
            .create(&account(), second_root.path(), None)
            .expect("create second project");
        let conversation_id = xfalcon_contracts::ConversationId::from("conversation-1");

        store
            .bind_conversation(&account(), &conversation_id, first.project_id)
            .expect("initial binding");
        let error = store
            .bind_conversation(&account(), &conversation_id, second.project_id)
            .expect_err("rebind must fail closed");

        assert!(matches!(error, TrustError::ConversationAlreadyBound));
        assert_eq!(
            store
                .project_for_conversation(&account(), &conversation_id)
                .expect("read original binding"),
            Some(first.project_id)
        );
    }

    #[test]
    fn a_report_origin_is_recorded_once_and_the_first_project_wins() {
        let app_root = tempdir().expect("app root");
        let first_root = tempdir().expect("first project root");
        let second_root = tempdir().expect("second project root");
        let store = store(app_root.path());
        let first = store
            .create(&account(), first_root.path(), None)
            .expect("first project");
        let second = store
            .create(&account(), second_root.path(), None)
            .expect("second project");
        let conversation = ConversationId::from("conversation-1");

        assert!(store
            .record_report_origin(&account(), "kit-1", first.project_id, &conversation)
            .expect("first record"));
        // The same kit again, from another project, changes nothing.
        assert!(!store
            .record_report_origin(&account(), "kit-1", second.project_id, &conversation)
            .expect("second record"));

        let origins = store.report_origins(&account()).expect("origins");
        assert_eq!(origins.len(), 1);
        assert_eq!(origins.get("kit-1"), Some(&first.project_id));
    }

    #[test]
    fn report_origins_belong_to_one_account_and_survive_reopen() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let database_path = project_trust_db_path(app_root.path(), &account());
        let store = store(app_root.path());
        let project = store
            .create(&account(), project_root.path(), None)
            .expect("project");
        store
            .record_report_origin(
                &account(),
                "kit-1",
                project.project_id,
                &ConversationId::from("conversation-1"),
            )
            .expect("record");
        drop(store);

        let reopened = TrustStore::open(&database_path, secrets() as Arc<dyn SecretStore>)
            .expect("reopen trust store");
        assert_eq!(
            reopened
                .report_origins(&account())
                .expect("origins")
                .get("kit-1"),
            Some(&project.project_id)
        );
        let other = ValidatedAccountId::parse("account-2").expect("second account");
        assert!(reopened
            .report_origins(&other)
            .expect("other origins")
            .is_empty());
    }

    #[test]
    fn a_report_origin_needs_a_project_the_account_holds_and_revoking_it_drops_the_link() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let store = store(app_root.path());
        let project = store
            .create(&account(), project_root.path(), None)
            .expect("project");
        let conversation = ConversationId::from("conversation-1");

        // A project that was never trusted cannot own a link.
        assert!(store
            .record_report_origin(&account(), "kit-0", ProjectId::new(), &conversation)
            .is_err());

        store
            .record_report_origin(&account(), "kit-1", project.project_id, &conversation)
            .expect("record");
        store
            .revoke(&account(), project.project_id)
            .expect("revoke project");
        assert!(store
            .report_origins(&account())
            .expect("origins")
            .is_empty());
    }

    #[test]
    fn a_report_origin_can_be_forgotten_without_touching_the_project() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let store = store(app_root.path());
        let project = store
            .create(&account(), project_root.path(), None)
            .expect("project");
        let conversation = ConversationId::from("conversation-1");
        for kit in ["kit-1", "kit-2"] {
            store
                .record_report_origin(&account(), kit, project.project_id, &conversation)
                .expect("record");
        }

        store
            .forget_report_origin(&account(), "kit-1")
            .expect("forget");
        // Forgetting a kit that has no link is not an error.
        store
            .forget_report_origin(&account(), "kit-9")
            .expect("forget absent");

        let origins = store.report_origins(&account()).expect("origins");
        assert_eq!(origins.len(), 1);
        assert!(origins.contains_key("kit-2"));
        assert!(store
            .get(&account(), project.project_id)
            .expect("read project")
            .is_some());
    }

    #[test]
    fn a_version_two_database_upgrades_in_place_and_keeps_its_bindings() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let database_path = project_trust_db_path(app_root.path(), &account());
        let store = store(app_root.path());
        let project = store
            .create(&account(), project_root.path(), None)
            .expect("project");
        let conversation = ConversationId::from("conversation-1");
        store
            .bind_conversation(&account(), &conversation, project.project_id)
            .expect("bind");
        drop(store);

        // Take the database back to what version 2 shipped: no link table.
        {
            let connection = Connection::open(&database_path).expect("raw open");
            connection
                .execute_batch("DROP TABLE report_kit_projects; PRAGMA user_version = 2;")
                .expect("downgrade");
        }

        let upgraded = TrustStore::open(&database_path, secrets() as Arc<dyn SecretStore>)
            .expect("open upgrades");
        assert_eq!(
            upgraded
                .project_for_conversation(&account(), &conversation)
                .expect("binding kept"),
            Some(project.project_id)
        );
        assert!(upgraded
            .record_report_origin(&account(), "kit-1", project.project_id, &conversation)
            .expect("the new table works"));
    }

    #[test]
    fn a_database_from_a_newer_build_is_refused() {
        let app_root = tempdir().expect("app root");
        let database_path = project_trust_db_path(app_root.path(), &account());
        drop(store(app_root.path()));
        Connection::open(&database_path)
            .expect("raw open")
            .pragma_update(None, "user_version", PROJECT_TRUST_SCHEMA_VERSION + 1)
            .expect("mark newer");

        assert!(TrustStore::open(&database_path, secrets() as Arc<dyn SecretStore>).is_err());
    }

    #[test]
    fn conversation_binding_can_be_removed_without_revoking_the_project() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let store = store(app_root.path());
        let project = store
            .create(&account(), project_root.path(), None)
            .expect("create project");
        let conversation_id = ConversationId::from("conversation-delete");
        store
            .bind_conversation(&account(), &conversation_id, project.project_id)
            .expect("bind conversation");

        store
            .unbind_conversation(&account(), &conversation_id)
            .expect("unbind conversation");

        assert_eq!(
            store
                .project_for_conversation(&account(), &conversation_id)
                .expect("read binding"),
            None
        );
        assert!(store
            .get(&account(), project.project_id)
            .expect("read project")
            .is_some());
    }

    #[test]
    fn the_same_conversation_id_is_isolated_between_accounts() {
        let app_root = tempdir().expect("app root");
        let first_root = tempdir().expect("first project root");
        let second_root = tempdir().expect("second project root");
        let second_account = ValidatedAccountId::parse("account-2").expect("second account");
        let secret_store = Arc::new(InMemorySecretStore::default());
        secret_store
            .put_key(account().as_str(), [0x21; 32])
            .expect("first account key");
        secret_store
            .put_key(second_account.as_str(), [0x22; 32])
            .expect("second account key");
        let store = TrustStore::open(
            app_root.path().join("projects.sqlite3"),
            secret_store as Arc<dyn SecretStore>,
        )
        .expect("open store");
        let first = store
            .create(&account(), first_root.path(), None)
            .expect("first project");
        let second = store
            .create(&second_account, second_root.path(), None)
            .expect("second project");
        let conversation_id = ConversationId::from("shared-conversation-id");

        store
            .bind_conversation(&account(), &conversation_id, first.project_id)
            .expect("bind first account");
        store
            .bind_conversation(&second_account, &conversation_id, second.project_id)
            .expect("bind second account");

        assert_eq!(
            store
                .project_for_conversation(&account(), &conversation_id)
                .expect("first binding"),
            Some(first.project_id)
        );
        assert_eq!(
            store
                .project_for_conversation(&second_account, &conversation_id)
                .expect("second binding"),
            Some(second.project_id)
        );
    }

    fn service(app_root: &Path) -> (ProjectsService, Arc<CurrentAccountScope>) {
        let scope = Arc::new(CurrentAccountScope::default());
        scope.bind_native(ACCOUNT_ID).expect("bind account");
        let service = ProjectsService::new(app_root.to_path_buf(), scope.clone(), secrets());
        (service, scope)
    }

    #[tokio::test]
    async fn service_create_activates_the_new_project_and_list_reflects_it() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());

        let created = service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        assert!(created.active);
        assert_eq!(created.status, ProjectTrustStatus::Trusted);

        let listed = service.list().expect("list");
        assert_eq!(listed.projects.len(), 1);
        assert!(listed.projects[0].active);
    }

    #[tokio::test]
    async fn service_find_by_folder_returns_all_matches_without_changing_projects() {
        let app_root = tempdir().expect("app root");
        let shared_root = tempdir().expect("shared root");
        let other_root = tempdir().expect("other root");
        let (service, _) = service(app_root.path());
        let first = service
            .create(&shared_root.path().to_string_lossy(), Some("First".into()))
            .await
            .expect("first shared project");
        let second = service
            .create(&shared_root.path().to_string_lossy(), Some("Second".into()))
            .await
            .expect("second shared project");
        service
            .create(&other_root.path().to_string_lossy(), None)
            .await
            .expect("other project");
        let before = service.list().expect("before");
        let found = service
            .find_by_folder(&shared_root.path().join(".").to_string_lossy())
            .expect("find canonical alias");
        assert_eq!(found.projects.len(), 2);
        assert!(found.projects.iter().any(|project| project.id == first.id));
        assert!(found.projects.iter().any(|project| project.id == second.id));
        assert!(found.projects.iter().all(|project| !project.active));
        assert_eq!(service.list().expect("after"), before);
        let unselected_root = tempdir().expect("unselected root");
        assert!(service
            .find_by_folder(&unselected_root.path().to_string_lossy())
            .expect("new folder")
            .projects
            .is_empty());
        assert_eq!(service.list().expect("still unchanged"), before);
        service
            .revoke(&first.id)
            .await
            .expect("revoke first project");
        let remaining = service
            .find_by_folder(&shared_root.path().to_string_lossy())
            .expect("remaining matches");
        assert_eq!(remaining.projects.len(), 1);
        assert_eq!(remaining.projects[0].id, second.id);
    }

    #[tokio::test]
    async fn service_find_by_folder_respects_filesystem_case_sensitivity() {
        let app_root = tempdir().expect("app root");
        let parent = tempdir().expect("parent");
        let root = parent.path().join("SharedFolder");
        let lower = parent.path().join("sharedfolder");
        fs::create_dir(&root).expect("root");
        let (service, _) = service(app_root.path());
        let project = service
            .create(&root.to_string_lossy(), None)
            .await
            .expect("project");
        if lower.exists() {
            assert_eq!(
                service
                    .find_by_folder(&lower.to_string_lossy())
                    .expect("case alias")
                    .projects,
                vec![project]
            );
        } else {
            fs::create_dir(&lower).expect("separate case-sensitive folder");
            assert!(service
                .find_by_folder(&lower.to_string_lossy())
                .expect("distinct folder")
                .projects
                .is_empty());
        }
    }

    #[tokio::test]
    async fn service_find_by_folder_is_scoped_to_the_current_account() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let scope = Arc::new(CurrentAccountScope::default());
        let secrets = secrets();
        secrets.put_key("account-2", [0x22; 32]).expect("other key");
        let service = ProjectsService::new(app_root.path().into(), scope.clone(), secrets);
        scope.bind_native(ACCOUNT_ID).expect("first account");
        let first = service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("first");
        scope.bind_native("account-2").expect("second account");
        assert!(service
            .find_by_folder(&project_root.path().to_string_lossy())
            .expect("isolated")
            .projects
            .is_empty());
        let second = service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("second");
        assert_eq!(
            service
                .find_by_folder(&project_root.path().to_string_lossy())
                .expect("second match")
                .projects,
            vec![second]
        );
        scope.bind_native(ACCOUNT_ID).expect("first account again");
        assert_eq!(
            service
                .find_by_folder(&project_root.path().to_string_lossy())
                .expect("first match")
                .projects,
            vec![first]
        );
        scope.clear().expect("sign out");
        assert!(matches!(
            service.find_by_folder(&project_root.path().to_string_lossy()),
            Err(ProjectsServiceError::NoActiveAccount)
        ));
    }

    #[tokio::test]
    async fn service_find_by_folder_rejects_invalid_paths_and_replaced_roots() {
        let app_root = tempdir().expect("app root");
        let parent = tempdir().expect("parent");
        let root = parent.path().join("project");
        fs::create_dir(&root).expect("root");
        let (service, _) = service(app_root.path());
        service
            .create(&root.to_string_lossy(), None)
            .await
            .expect("project");
        fs::rename(&root, parent.path().join("original")).expect("move original");
        fs::create_dir(&root).expect("replacement");
        assert!(service
            .find_by_folder(&root.to_string_lossy())
            .expect("replacement lookup")
            .projects
            .is_empty());
        assert!(service
            .find_by_folder(&parent.path().join("missing").to_string_lossy())
            .is_err());
        let file = parent.path().join("file.txt");
        fs::write(&file, "data").expect("file");
        assert!(service.find_by_folder(&file.to_string_lossy()).is_err());
    }

    #[tokio::test]
    async fn service_deactivate_clears_only_the_current_project_selection() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());
        service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");

        service.deactivate().expect("deactivate");

        let listed = service.list().expect("list after deactivate");
        assert_eq!(listed.projects.len(), 1);
        assert!(!listed.projects[0].active);
    }

    #[tokio::test]
    async fn service_list_reports_unavailable_after_the_root_is_deleted() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());
        service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");

        fs::remove_dir(project_root.path()).expect("delete the trusted root");

        let listed = service.list().expect("list still succeeds");
        assert_eq!(listed.projects.len(), 1);
        assert_eq!(listed.projects[0].status, ProjectTrustStatus::Unavailable);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn confirmed_deletion_persists_before_chat_cleanup_and_survives_restart() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (runtime, _) = service(app_root.path());
        let created = runtime
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        let id = parse_project_id(&created.id).expect("id");
        let conversation = ConversationId::from("conversation-1");
        runtime
            .bind_active_conversation(&account(), &conversation)
            .expect("bind");
        fs::remove_dir(project_root.path()).expect("delete");
        assert_eq!(
            runtime.deleted_projects(&account()).expect("detect"),
            vec![id]
        );
        drop(runtime);
        let (runtime, _) = service(app_root.path());
        assert_eq!(
            runtime
                .deleted_projects(&account())
                .expect("persisted detection"),
            vec![id]
        );
        assert_eq!(
            runtime
                .project_for_conversation(&account(), &conversation)
                .expect("binding"),
            Some(id)
        );
    }

    #[tokio::test]
    async fn renamed_or_moved_root_never_authorizes_chat_deletion() {
        let app_root = tempdir().expect("app root");
        let parent = tempdir().expect("parent");
        let destination = tempdir().expect("destination");
        let root = parent.path().join("project");
        fs::create_dir(&root).expect("root");
        let (runtime, _) = service(app_root.path());
        runtime
            .create(&root.to_string_lossy(), None)
            .await
            .expect("create");
        let renamed = parent.path().join("renamed");
        fs::rename(&root, &renamed).expect("rename same parent");
        assert!(runtime
            .deleted_projects(&account())
            .expect("detect rename")
            .is_empty());
        fs::rename(&renamed, destination.path().join("moved")).expect("move elsewhere");
        assert!(runtime
            .deleted_projects(&account())
            .expect("detect move")
            .is_empty());
        assert_eq!(
            runtime.list().expect("list").projects[0].status,
            ProjectTrustStatus::Unavailable
        );
    }

    #[tokio::test]
    async fn missing_root_after_restart_is_retained_without_definitive_evidence() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (runtime, _) = service(app_root.path());
        runtime
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        drop(runtime);
        fs::remove_dir(project_root.path()).expect("delete while closed");
        let (runtime, _) = service(app_root.path());
        assert!(runtime
            .deleted_projects(&account())
            .expect("detect")
            .is_empty());
        assert_eq!(
            runtime.list().expect("list").projects[0].status,
            ProjectTrustStatus::Unavailable
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn list_observes_existing_roots_after_restart_for_later_deletion() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (runtime, _) = service(app_root.path());
        let created = runtime
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        drop(runtime);
        let (runtime, _) = service(app_root.path());
        runtime.list().expect("observe");
        fs::remove_dir(project_root.path()).expect("delete");
        assert_eq!(
            runtime.deleted_projects(&account()).expect("detect"),
            vec![parse_project_id(&created.id).expect("id")]
        );
    }

    #[tokio::test]
    async fn changed_root_identity_does_not_seed_a_deletion_handle() {
        let app_root = tempdir().expect("app root");
        let parent = tempdir().expect("parent");
        let root = parent.path().join("project");
        fs::create_dir(&root).expect("root");
        let (runtime, _) = service(app_root.path());
        runtime
            .create(&root.to_string_lossy(), None)
            .await
            .expect("create");
        drop(runtime);
        fs::rename(&root, parent.path().join("original")).expect("move original");
        fs::create_dir(&root).expect("replace");
        let (runtime, _) = service(app_root.path());
        runtime.list().expect("revalidate mismatch");
        fs::remove_dir(&root).expect("delete replacement");
        assert!(runtime
            .deleted_projects(&account())
            .expect("detect")
            .is_empty());
    }

    #[tokio::test]
    async fn service_activate_fails_closed_when_the_root_identity_changed() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());
        let created = service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");

        // Keep the original object alive while installing a distinct directory
        // at its path; this also avoids Windows deferred-delete timing.
        fs::rename(project_root.path(), app_root.path().join("original-root"))
            .expect("move original");
        fs::create_dir(project_root.path()).expect("recreate at the same path");

        let error = service
            .activate(&created.id)
            .expect_err("activation must fail closed on identity mismatch");
        assert!(matches!(
            error,
            ProjectsServiceError::Containment(ContainmentError::RootIdentityChanged)
        ));
    }

    #[tokio::test]
    async fn service_revoke_clears_the_active_pointer_and_bumps_cancellation() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());
        let created = service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        let generation_before = service.cancellation().generation();

        service.revoke(&created.id).await.expect("revoke");

        assert_ne!(service.cancellation().generation(), generation_before);
        let listed = service.list().expect("list after revoke");
        assert!(listed.projects.is_empty());
    }

    #[tokio::test]
    async fn service_binds_the_active_project_and_restores_it_for_the_conversation() {
        let app_root = tempdir().expect("app root");
        let first_root = tempdir().expect("first root");
        let second_root = tempdir().expect("second root");
        let (service, _scope) = service(app_root.path());
        let first = service
            .create(
                &first_root.path().to_string_lossy(),
                Some("First".to_owned()),
            )
            .await
            .expect("create first");
        let first_project_id = parse_project_id(&first.id).expect("first project id");
        let conversation_id = ConversationId::from("conversation-1");

        assert_eq!(
            service
                .bind_active_conversation(&account(), &conversation_id)
                .expect("bind active"),
            Some(first_project_id)
        );

        service
            .create(
                &second_root.path().to_string_lossy(),
                Some("Second".to_owned()),
            )
            .await
            .expect("create second");
        assert_ne!(
            service.active_project(&account()),
            service
                .project_for_conversation(&account(), &conversation_id)
                .expect("read binding")
        );
        let frozen_selection_conversation = ConversationId::from("conversation-frozen-selection");
        service
            .bind_conversation(&account(), &frozen_selection_conversation, first_project_id)
            .expect("bind the project selected before ambient activation changed");
        assert_eq!(
            service
                .project_for_conversation(&account(), &frozen_selection_conversation)
                .expect("read frozen binding"),
            Some(first_project_id)
        );

        let restored = service
            .sync_active_to_conversation(&account(), &conversation_id)
            .expect("restore binding");
        assert_eq!(restored, service.active_project(&account()));
    }

    #[tokio::test]
    async fn opening_an_unbound_conversation_clears_the_active_project() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());
        service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");

        let restored = service
            .sync_active_to_conversation(&account(), &ConversationId::from("unbound"))
            .expect("sync unbound");

        assert_eq!(restored, None);
        assert_eq!(service.active_project(&account()), None);
    }

    #[tokio::test]
    async fn cancel_all_projects_clears_the_active_pointer_and_bumps_cancellation() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let (service, _scope) = service(app_root.path());
        service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        let generation_before = service.cancellation().generation();

        crate::security::account_cleanup::ProjectCancellation::cancel_all_projects(
            &service,
            &account(),
        );

        assert_ne!(service.cancellation().generation(), generation_before);
    }

    struct RecordingAuditSink {
        events: Mutex<Vec<crate::audit::AuditEvent>>,
    }

    #[async_trait::async_trait]
    impl crate::audit::AuditSink for RecordingAuditSink {
        async fn publish(
            &self,
            event: &crate::audit::AuditEvent,
        ) -> crate::domain::error::AppResult<crate::audit::AuditPublishStatus> {
            self.events.lock().expect("lock").push(event.clone());
            Ok(crate::audit::AuditPublishStatus::Published)
        }
    }

    fn audit_queue_for_test(app_root: &Path) -> Arc<crate::audit::AuditQueue> {
        let secrets = Arc::new(InMemorySecretStore::default());
        secrets
            .put_key(ACCOUNT_ID, [0x99; 32])
            .expect("account key");
        let journal = Arc::new(
            crate::persistence::Journal::open(app_root.join("audit.sqlite3"), secrets)
                .expect("open audit journal"),
        );
        Arc::new(crate::audit::AuditQueue::new(ACCOUNT_ID, 16, journal).expect("audit queue"))
    }

    /// Closes the review finding that `Task 7` added zero
    /// `AuditSink::publish` calls anywhere in `projects::trust` despite the
    /// plan's own Step 3 explicitly naming "project trust/revocation" as one
    /// of the four required audit categories, and despite `project_create`/
    /// `project_revoke` being genuinely live, Tauri-registered commands
    /// (unlike the other three categories this same finding named, which
    /// have no production caller at all yet).
    #[tokio::test]
    async fn trusting_and_revoking_a_project_publishes_real_audit_events() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let scope = Arc::new(CurrentAccountScope::default());
        scope.bind_native(ACCOUNT_ID).expect("bind account");
        let sink = Arc::new(RecordingAuditSink {
            events: Mutex::new(Vec::new()),
        });
        let service = ProjectsService::new(app_root.path().to_path_buf(), scope, secrets())
            .with_audit(crate::audit::AuditPublisher::new(
                "actor-1",
                "tenant-1",
                "device-1",
                sink.clone(),
                audit_queue_for_test(app_root.path()),
            ));

        let created = service
            .create(&project_root.path().to_string_lossy(), None)
            .await
            .expect("create");
        service.revoke(&created.id).await.expect("revoke");

        let events = sink.events.lock().expect("lock");
        assert_eq!(events.len(), 2, "both trust and revoke must publish");
        assert_eq!(events[0].action(), crate::audit::AuditAction::ProjectTrust);
        assert_eq!(events[0].resource_id(), created.id.as_str());
        assert_eq!(events[1].action(), crate::audit::AuditAction::ProjectRevoke);
        assert_eq!(events[1].resource_id(), created.id.as_str());
    }
}
