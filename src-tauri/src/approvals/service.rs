//! Encrypted local persistence for pending/resolved approval requests, and
//! the Tauri-facing runtime that proposes and resolves them.
//!
//! # Security contract
//!
//! [`ApprovalsService::resolve`] is the *only* production code path that can
//! ever cause [`crate::projects::execute`] to run: it is the only
//! production caller of [`require_sufficient_confirmation`], this crate's
//! sole constructor of [`crate::projects::ApprovalProof`] (see that
//! function's own doc comment). A `deny` decision never reaches
//! [`crate::projects::execute`] at all -- it only ever updates this
//! module's own encrypted status row. Persisted rows follow this codebase's
//! standing encrypted-per-account-row convention
//! (`crate::persistence::crypto`, `crate::chat::local_projection`,
//! `crate::projects::trust::TrustStore`/`crate::projects::index::IndexStore`):
//! AES-256-GCM via a distinct AAD domain (`xfalcon-project-approval-v1`,
//! `crate::persistence::crypto::ApprovalAuthenticatedMetadata`) folding the
//! row's own `plan_id` into the authenticated data, and a fresh
//! `rusqlite::Connection` opened per call. Approval rows live under the same
//! `crate::projects::PROJECT_DATA_DIR` directory
//! `crate::security::account_cleanup::ProjectDataHandler` already wipes
//! wholesale on account cleanup -- no parallel cleanup mechanism is
//! introduced.
//!
//! ## Terminal-decision history
//!
//! [`ApprovalStore::insert_pending`]'s upsert resets a stale terminal row
//! (denied/expired/executed/executed-durability-unconfirmed/failed) back to
//! `pending` when identical plan content is re-proposed (see that method's
//! own doc comment). Every such reset is archived first into the append-only
//! `project_mutation_approval_history` table by a database trigger
//! (`migrations/0006_project_mutation_approval_history.sql`), so the prior
//! decision is never simply lost to a later re-proposal -- this is currently
//! the only durable record any deny/expire/execute/fail decision has
//! anywhere in the system (DA/`crate::audit` publication for approval
//! resolution is not wired until a later task). The history table lives in
//! the same per-account sqlite file as the main table, so it is wiped by the
//! same account-cleanup path with no separate mechanism required.
//!
//! ## Single-flight resolution
//!
//! [`ApprovalStore::claim_pending`] atomically transitions a row from
//! `pending` to a transient `claiming` status via a single
//! `UPDATE ... WHERE status = 'pending'` compare-and-swap: whichever of two
//! concurrent [`ApprovalsService::resolve`] calls for the same plan id wins
//! that race is the only one that ever proceeds to [`crate::projects::execute`];
//! the loser's own claim affects zero rows and it is rejected exactly like
//! any other already-resolved plan. A claim that turns out not to be a real
//! terminal decision (e.g. the second-confirmation policy rejects it) is
//! reverted back to `pending` via [`ApprovalStore::revert_claim`] rather
//! than left stuck, so a caller can still retry it.
//!
//! ## Crash reconciliation
//!
//! A process crash between [`crate::projects::execute`] returning `Ok` and
//! this module's own follow-up status write would otherwise leave a
//! `pending` (or `claiming`) row whose plan already fully executed. Before
//! attempting [`crate::projects::execute`], [`ApprovalsService::resolve`]
//! checks [`crate::projects::plan_already_applied`] against the plan's own
//! trusted root: if every operation's expected post-state is already
//! confirmed on disk, the row is marked `executed` directly rather than
//! re-attempting `execute` (which would otherwise fail on the plan's own
//! now-stale base hashes and be misreported as a fresh `failed`).

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::chat::ApprovalDaSync;
use crate::domain::error::{AppError, AppResult};
use crate::persistence::{
    decrypt_approval, encrypt_approval, ApprovalAuthenticatedMetadata, SecretStore,
    PROJECT_APPROVAL_MIGRATION, PROJECT_APPROVAL_SCHEMA_VERSION,
};
use crate::projects::{
    execute, hash_bytes, plan_already_applied, BackupCapture, BackupRevision, ContainmentError,
    ContentHash, EditOperation, ExecutionOutcome, LoadedBackup, MutationPlan, MutationRiskTier,
    PatchOperation, PlanId, ProjectId, RelativeProjectPath, ShadowBackupStore,
    ShadowBackupStoreError, TrustStore, TrustedRoot, WriterError, WriterExecutionFailure,
    MAX_EDITABLE_FILE_BYTES, PROJECT_DATA_DIR,
};
use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};

use super::policy::{require_sufficient_confirmation, ApprovalDecision, ApprovalPolicyError};

const APPROVAL_DB_FILE: &str = "approvals.sqlite3";

#[derive(Debug, thiserror::Error)]
pub enum ApprovalsServiceError {
    #[error("no account is currently active")]
    NoActiveAccount,
    #[error("the requested project was not found or is no longer trusted")]
    ProjectNotFound,
    #[error("the requested approval plan was not found")]
    PlanNotFound,
    #[error(transparent)]
    Patch(#[from] crate::projects::PatchError),
    #[error(transparent)]
    Containment(#[from] ContainmentError),
    #[error(transparent)]
    Policy(#[from] ApprovalPolicyError),
    #[error(transparent)]
    Execution(#[from] WriterExecutionFailure),
    #[error(transparent)]
    Backup(#[from] ShadowBackupStoreError),
    #[error(
        "{0} is larger than 64 MiB, so xFalcon cannot keep a copy of it before \
         deleting or replacing it; nothing was changed, so ask the person to do \
         it themselves"
    )]
    TooLargeToKeep(String),
    #[error("the local approval store is unavailable")]
    Unavailable,
    #[error("the write was cancelled before it started")]
    Cancelled,
}

impl From<AppError> for ApprovalsServiceError {
    fn from(_: AppError) -> Self {
        Self::Unavailable
    }
}

impl From<rusqlite::Error> for ApprovalsServiceError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Unavailable
    }
}

impl From<crate::projects::TrustError> for ApprovalsServiceError {
    fn from(_: crate::projects::TrustError) -> Self {
        Self::Unavailable
    }
}

/// Outcome of [`ApprovalsService::resolve`].
#[derive(Clone, Debug)]
pub enum ApprovalOutcome {
    Denied,
    Executed(crate::projects::ExecutionOutcome),
}

/// The local decision and its optional remote notifications. Filesystem work
/// and terminal status finish in one owned blocking job, before any async
/// notification can be cancelled. This is not a cross-file transaction.
struct LocalResolution {
    result: Result<ApprovalOutcome, ApprovalsServiceError>,
    approved: Option<bool>,
    audit: Option<(crate::audit::AuditAction, crate::audit::AuditOutcome)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApprovalStatus {
    Pending,
    /// Transient: a `resolve()` call has atomically claimed this row (see
    /// [`ApprovalStore::claim_pending`]) and has not yet persisted a
    /// terminal status. Never observed by a caller -- only ever set and
    /// then immediately replaced (or reverted back to `Pending`) within a
    /// single `resolve()` call.
    Claiming,
    Denied,
    Expired,
    Executed,
    /// The plan's mutation(s) landed (every rename/unlink/create actually
    /// succeeded), but a post-mutation durability step (parent-directory
    /// fsync) could not be confirmed -- distinct from [`Self::Failed`] so
    /// the journal never claims a mutation never happened when it did (see
    /// `crate::projects::writer`'s module doc comment).
    ExecutedDurabilityUnconfirmed,
    Failed,
}

impl ApprovalStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claiming => "claiming",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Executed => "executed",
            Self::ExecutedDurabilityUnconfirmed => "executed_durability_unconfirmed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredOperation {
    kind: String,
    path: Option<String>,
    from: Option<String>,
    to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_base64: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ApprovalPayload {
    project_id: String,
    root_identity_primary: u64,
    root_identity_secondary: u64,
    operations: Vec<StoredOperation>,
    base_hashes: BTreeMap<String, String>,
    proposed_hashes: BTreeMap<String, String>,
    unified_diff: String,
    total_paths: u64,
    total_bytes: u64,
    risk: String,
    created_at_ms: i64,
    expires_at_ms: i64,
}

fn relative(raw: &str) -> Result<RelativeProjectPath, ApprovalsServiceError> {
    RelativeProjectPath::new(raw).map_err(|_| ApprovalsServiceError::Unavailable)
}

fn to_payload(plan: &MutationPlan) -> ApprovalPayload {
    use base64::Engine;
    let operations = plan
        .operations()
        .iter()
        .map(|operation| match operation {
            PatchOperation::Create { path, content } => StoredOperation {
                kind: "create".to_owned(),
                path: Some(path.as_str().to_owned()),
                from: None,
                to: None,
                content_base64: Some(base64::engine::general_purpose::STANDARD.encode(content)),
            },
            PatchOperation::Modify { path, content } => StoredOperation {
                kind: "modify".to_owned(),
                path: Some(path.as_str().to_owned()),
                from: None,
                to: None,
                content_base64: Some(base64::engine::general_purpose::STANDARD.encode(content)),
            },
            PatchOperation::Rename { from, to } => StoredOperation {
                kind: "rename".to_owned(),
                path: None,
                from: Some(from.as_str().to_owned()),
                to: Some(to.as_str().to_owned()),
                content_base64: None,
            },
            PatchOperation::Delete { path } => StoredOperation {
                kind: "delete".to_owned(),
                path: Some(path.as_str().to_owned()),
                from: None,
                to: None,
                content_base64: None,
            },
            PatchOperation::RenameDirectory { from, to } => StoredOperation {
                kind: "rename_directory".to_owned(),
                path: None,
                from: Some(from.as_str().to_owned()),
                to: Some(to.as_str().to_owned()),
                content_base64: None,
            },
            PatchOperation::DeleteDirectory { path } => StoredOperation {
                kind: "delete_directory".to_owned(),
                path: Some(path.as_str().to_owned()),
                from: None,
                to: None,
                content_base64: None,
            },
        })
        .collect();
    let (root_identity_primary, root_identity_secondary) = plan.root_identity().into_parts();
    ApprovalPayload {
        project_id: plan.project_id().to_string(),
        root_identity_primary,
        root_identity_secondary,
        operations,
        base_hashes: hex_map(plan.base_hashes()),
        proposed_hashes: hex_map(plan.proposed_hashes()),
        unified_diff: plan.unified_diff().to_owned(),
        total_paths: plan.total_paths() as u64,
        total_bytes: plan.total_bytes(),
        risk: match plan.risk() {
            MutationRiskTier::Standard => "standard".to_owned(),
            MutationRiskTier::SecondConfirmationRequired => {
                "second_confirmation_required".to_owned()
            }
        },
        created_at_ms: plan.created_at_ms(),
        expires_at_ms: plan.expires_at_ms(),
    }
}

fn hex_map(source: &BTreeMap<String, ContentHash>) -> BTreeMap<String, String> {
    source
        .iter()
        .map(|(path, hash)| (path.clone(), hash.to_hex().to_string()))
        .collect()
}

fn from_payload(
    plan_id: PlanId,
    payload: ApprovalPayload,
) -> Result<MutationPlan, ApprovalsServiceError> {
    use base64::Engine;
    let project_id =
        ProjectId::from_str(&payload.project_id).map_err(|_| ApprovalsServiceError::Unavailable)?;
    let root_identity = crate::projects::RootIdentity::from_parts(
        payload.root_identity_primary,
        payload.root_identity_secondary,
    );
    let mut operations = Vec::with_capacity(payload.operations.len());
    for stored in payload.operations {
        let operation = match stored.kind.as_str() {
            "create" | "modify" => {
                let path = relative(stored.path.as_deref().unwrap_or_default())?;
                let content = base64::engine::general_purpose::STANDARD
                    .decode(stored.content_base64.unwrap_or_default())
                    .map_err(|_| ApprovalsServiceError::Unavailable)?;
                if stored.kind == "create" {
                    PatchOperation::Create { path, content }
                } else {
                    PatchOperation::Modify { path, content }
                }
            }
            "rename" => PatchOperation::Rename {
                from: relative(stored.from.as_deref().unwrap_or_default())?,
                to: relative(stored.to.as_deref().unwrap_or_default())?,
            },
            "delete" => PatchOperation::Delete {
                path: relative(stored.path.as_deref().unwrap_or_default())?,
            },
            "rename_directory" => PatchOperation::RenameDirectory {
                from: relative(stored.from.as_deref().unwrap_or_default())?,
                to: relative(stored.to.as_deref().unwrap_or_default())?,
            },
            "delete_directory" => PatchOperation::DeleteDirectory {
                path: relative(stored.path.as_deref().unwrap_or_default())?,
            },
            _ => return Err(ApprovalsServiceError::Unavailable),
        };
        operations.push(operation);
    }
    let base_hashes = unhex_map(&payload.base_hashes)?;
    let proposed_hashes = unhex_map(&payload.proposed_hashes)?;
    let risk = match payload.risk.as_str() {
        "standard" => MutationRiskTier::Standard,
        "second_confirmation_required" => MutationRiskTier::SecondConfirmationRequired,
        _ => return Err(ApprovalsServiceError::Unavailable),
    };
    Ok(MutationPlan::from_parts(
        plan_id,
        project_id,
        root_identity,
        operations,
        base_hashes,
        proposed_hashes,
        payload.unified_diff,
        payload.total_paths as usize,
        payload.total_bytes,
        risk,
        payload.created_at_ms,
        payload.expires_at_ms,
    ))
}

fn unhex_map(
    source: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ContentHash>, ApprovalsServiceError> {
    source
        .iter()
        .map(|(path, hex)| {
            ContentHash::from_hex(hex)
                .map(|hash| (path.clone(), hash))
                .map_err(|_| ApprovalsServiceError::Unavailable)
        })
        .collect()
}

struct ApprovalRow {
    project_id: String,
    ciphertext_row: CiphertextRow,
    status: String,
}

struct CiphertextRow {
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

/// Encrypted local storage for one account's pending/resolved approval
/// requests. Opens a fresh `rusqlite::Connection` per call, matching
/// `crate::projects::trust::TrustStore`/`crate::projects::index::IndexStore`'s
/// documented idiom.
pub struct ApprovalStore {
    connection: Mutex<Connection>,
    secrets: Arc<dyn SecretStore>,
}

impl ApprovalStore {
    pub fn open(
        path: impl AsRef<Path>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, ApprovalsServiceError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| ApprovalsServiceError::Unavailable)?;
        }
        let connection = open_connection(path).map_err(|_| ApprovalsServiceError::Unavailable)?;
        Ok(Self {
            connection: Mutex::new(connection),
            secrets,
        })
    }

    /// Inserts a fresh `pending` row for `plan`, or -- because [`PlanId`] is
    /// purely content-addressed (`crate::projects::patch::compute_plan_id`),
    /// so a second `propose()` call for byte-identical operations against
    /// unchanged content always collides onto the same id -- resets an
    /// existing row for the same id back to a fresh `pending` state
    /// (new ciphertext/timestamps, cleared `resolved_at_ms`) whenever that
    /// row's current status is a *terminal* one (denied/expired/executed/
    /// executed-durability-unconfirmed/failed). A row already `pending` is left
    /// untouched. A row mid-`claiming` (see [`Self::claim_pending`]) is left
    /// alone too, until its expiry has passed: a claim that outlived it belongs
    /// to a run that died, and resetting it is what lets the same plan be
    /// proposed again instead of being blocked forever. The `WHERE` clause
    /// below never clobbers a resolution that is genuinely in flight. Without
    /// this, a caller who re-proposes identical content after an earlier
    /// proposal was already resolved would silently keep the stale,
    /// terminal row in place while `propose()` itself reports success --
    /// permanently blocking that content from ever being approved again.
    ///
    /// The terminal row's prior status/timestamps/ciphertext are never
    /// simply discarded by this reset: `project_mutation_approvals_archive_on_reset`
    /// (`migrations/0006_project_mutation_approval_history.sql`) is a
    /// `BEFORE UPDATE` trigger on the exact same statement below that copies
    /// the old row into the append-only `project_mutation_approval_history`
    /// table before this `UPDATE` overwrites it, whenever a terminal status
    /// is being reset back to `pending`. This runs at the database layer
    /// (not in this Rust function), so it is not something a future edit to
    /// this method's own logic could accidentally skip, and it covers this
    /// exact `ON CONFLICT ... DO UPDATE` regardless of what future code
    /// calls it. This is the only history any deny/expire/execute/fail
    /// decision has anywhere in the system today (see the module doc
    /// comment; `crate::audit::AuditQueue`/DA publication for approval
    /// resolution is not wired until a later task).
    fn insert_pending(
        &self,
        account: &ValidatedAccountId,
        plan: &MutationPlan,
    ) -> Result<(), ApprovalsServiceError> {
        let plan_id_hex = plan.plan_id().to_hex();
        let project_id_string = plan.project_id().to_string();
        let payload = to_payload(plan);
        let plaintext =
            serde_json::to_vec(&payload).map_err(|_| ApprovalsServiceError::Unavailable)?;
        let sealed = seal(
            self.secrets.as_ref(),
            account.as_str(),
            &project_id_string,
            &plan_id_hex,
            &plaintext,
        )?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| ApprovalsServiceError::Unavailable)?;
        connection.execute(
            "INSERT INTO project_mutation_approvals (
                account_id, project_id, plan_id, aad_schema_version, key_version,
                nonce, ciphertext, status, created_at_ms, expires_at_ms, resolved_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
             ON CONFLICT (account_id, plan_id) DO UPDATE SET
                aad_schema_version = excluded.aad_schema_version,
                key_version = excluded.key_version,
                nonce = excluded.nonce,
                ciphertext = excluded.ciphertext,
                status = excluded.status,
                created_at_ms = excluded.created_at_ms,
                expires_at_ms = excluded.expires_at_ms,
                resolved_at_ms = excluded.resolved_at_ms
             WHERE project_mutation_approvals.status IN (
                'denied', 'expired', 'executed', 'executed_durability_unconfirmed', 'failed'
             ) OR (
                -- A claim is held for the length of one resolve call. One whose
                -- plan expired long ago belongs to a process that died mid-way,
                -- and nothing else will ever release it: without this the same
                -- plan could never be proposed, or applied, again.
                project_mutation_approvals.status = 'claiming'
                AND project_mutation_approvals.expires_at_ms < ?11
             )",
            params![
                account.as_str(),
                project_id_string,
                plan_id_hex,
                i64::from(sealed.aad_schema_version),
                i64::from(sealed.key_version),
                sealed.nonce,
                sealed.ciphertext,
                ApprovalStatus::Pending.as_str(),
                plan.created_at_ms(),
                plan.expires_at_ms(),
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(())
    }

    /// Atomically claims a `pending` row for exclusive resolution,
    /// transitioning it to [`ApprovalStatus::Claiming`] iff it is currently
    /// [`ApprovalStatus::Pending`] -- the single-flight compare-and-swap
    /// that prevents two concurrent [`ApprovalsService::resolve`] calls for
    /// the same plan id from both observing `pending` and both proceeding
    /// to call [`crate::projects::execute`] (or both denying). Returns
    /// `true` iff this call won the claim.
    fn claim_pending(
        &self,
        account: &ValidatedAccountId,
        plan_id: PlanId,
    ) -> Result<bool, ApprovalsServiceError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ApprovalsServiceError::Unavailable)?;
        let rows_changed = connection.execute(
            "UPDATE project_mutation_approvals SET status = ?1
             WHERE account_id = ?2 AND plan_id = ?3 AND status = ?4",
            params![
                ApprovalStatus::Claiming.as_str(),
                account.as_str(),
                plan_id.to_hex(),
                ApprovalStatus::Pending.as_str(),
            ],
        )?;
        Ok(rows_changed == 1)
    }

    /// Reverts a row this call's own [`Self::claim_pending`] just won back
    /// to [`ApprovalStatus::Pending`] -- used when a claimed resolution
    /// turns out not to be a real terminal decision after all (e.g. the
    /// second-confirmation policy rejects it, or a transient environment
    /// error prevents reaching [`crate::projects::execute`] at all), so the
    /// plan remains genuinely retryable rather than stuck in `claiming`
    /// limbo forever.
    fn revert_claim(
        &self,
        account: &ValidatedAccountId,
        plan_id: PlanId,
    ) -> Result<(), ApprovalsServiceError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ApprovalsServiceError::Unavailable)?;
        connection.execute(
            "UPDATE project_mutation_approvals SET status = ?1
             WHERE account_id = ?2 AND plan_id = ?3",
            params![
                ApprovalStatus::Pending.as_str(),
                account.as_str(),
                plan_id.to_hex(),
            ],
        )?;
        Ok(())
    }

    /// Loads a plan by id, returning it together with its current status,
    /// regardless of that status -- callers decide what is actionable.
    fn load(
        &self,
        account: &ValidatedAccountId,
        plan_id: PlanId,
    ) -> Result<Option<(MutationPlan, ApprovalStatusRow)>, ApprovalsServiceError> {
        let plan_id_hex = plan_id.to_hex();
        let connection = self
            .connection
            .lock()
            .map_err(|_| ApprovalsServiceError::Unavailable)?;
        let row = connection
            .query_row(
                "SELECT project_id, aad_schema_version, key_version, nonce, ciphertext, status
                 FROM project_mutation_approvals WHERE account_id = ?1 AND plan_id = ?2",
                params![account.as_str(), plan_id_hex],
                |row| {
                    Ok(ApprovalRow {
                        project_id: row.get(0)?,
                        ciphertext_row: CiphertextRow {
                            aad_schema_version: row.get(1)?,
                            key_version: row.get(2)?,
                            nonce: row.get(3)?,
                            ciphertext: row.get(4)?,
                        },
                        status: row.get(5)?,
                    })
                },
            )
            .optional()?;
        drop(connection);
        let Some(row) = row else {
            return Ok(None);
        };
        let plaintext = open(self.secrets.as_ref(), account.as_str(), &plan_id_hex, &row)?;
        let payload: ApprovalPayload =
            serde_json::from_slice(&plaintext).map_err(|_| ApprovalsServiceError::Unavailable)?;
        let plan = from_payload(plan_id, payload)?;
        Ok(Some((plan, ApprovalStatusRow(row.status))))
    }

    fn set_status(
        &self,
        account: &ValidatedAccountId,
        plan_id: PlanId,
        status: ApprovalStatus,
        resolved_at_ms: i64,
    ) -> Result<(), ApprovalsServiceError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ApprovalsServiceError::Unavailable)?;
        connection.execute(
            "UPDATE project_mutation_approvals SET status = ?1, resolved_at_ms = ?2
             WHERE account_id = ?3 AND plan_id = ?4",
            params![
                status.as_str(),
                resolved_at_ms,
                account.as_str(),
                plan_id.to_hex(),
            ],
        )?;
        Ok(())
    }
}

struct ApprovalStatusRow(String);

impl ApprovalStatusRow {
    fn is_pending(&self) -> bool {
        self.0 == ApprovalStatus::Pending.as_str()
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
    plan_id_hex: &str,
    plaintext: &[u8],
) -> AppResult<Sealed> {
    // Mint-on-first-use — see `chat::local_projection::seal`: a seal that only
    // reads the active key fails on a device that has never sealed before.
    let (key_version, key) = secrets.load_or_create_active_key(account_id)?;
    let metadata = ApprovalAuthenticatedMetadata {
        account_id,
        project_id,
        plan_id: plan_id_hex,
    };
    let encrypted = encrypt_approval(&key, key_version, &metadata, plaintext)?;
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
    plan_id_hex: &str,
    row: &ApprovalRow,
) -> AppResult<Vec<u8>> {
    let aad_schema_version = u32::try_from(row.ciphertext_row.aad_schema_version)
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key_version = u32::try_from(row.ciphertext_row.key_version)
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
    let key = secrets.account_key(account_id, key_version)?;
    let metadata = ApprovalAuthenticatedMetadata {
        account_id,
        project_id: &row.project_id,
        plan_id: plan_id_hex,
    };
    decrypt_approval(
        &key,
        aad_schema_version,
        key_version,
        &row.ciphertext_row.nonce,
        &metadata,
        &row.ciphertext_row.ciphertext,
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
    if version > PROJECT_APPROVAL_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_APPROVAL_SCHEMA_VERSION {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > PROJECT_APPROVAL_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_APPROVAL_SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    transaction.execute_batch(PROJECT_APPROVAL_MIGRATION)?;
    transaction.pragma_update(None, "user_version", PROJECT_APPROVAL_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn approval_db_path(app_root: &Path, account: &ValidatedAccountId) -> PathBuf {
    app_root
        .join("accounts")
        .join(account_directory_name(account.as_str()))
        .join(PROJECT_DATA_DIR)
        .join(APPROVAL_DB_FILE)
}

/// The Tauri-facing approvals runtime: proposes new [`MutationPlan`]s
/// against the current account's trusted projects, persists them encrypted,
/// and is the sole path to resolving (approving or denying) one -- see the
/// module doc comment's security contract.
#[derive(Clone)]
pub struct ApprovalsService {
    app_root: PathBuf,
    accounts: Arc<crate::security::account_cleanup::CurrentAccountScope>,
    secrets: Arc<dyn SecretStore>,
    da_sync: Option<Arc<dyn ApprovalDaSync>>,
    /// Best-effort `tool.approve`/`tool.deny`/`tool.execute` audit
    /// publication for approval resolution and mutation outcome — see
    /// [`Self::with_audit`]. `None` (the default) means this service simply
    /// never publishes, exactly like `crate::agent::tool_loop::ToolLoop`
    /// with no `ToolLoopAudit` configured.
    audit: Option<Arc<crate::audit::AuditPublisher>>,
    /// One local mutation at a time also covers duplicate/nested project roots.
    /// Reads and proposals remain parallel. The owned guard stays with the
    /// blocking job through backup capture, apply and terminal status.
    mutation_lock: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    pub(crate) resolution_hook: Option<Arc<dyn Fn(ProjectId) + Send + Sync>>,
}

impl ApprovalsService {
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
            da_sync: None,
            audit: None,
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            resolution_hook: None,
        }
    }

    /// Wires an authenticated DA/MCP-audience sync channel for approval
    /// decisions (`crate::chat::ApprovalDaSync`). No production caller
    /// supplies one yet -- see that trait's doc comment for why.
    #[must_use]
    pub fn with_da_sync(mut self, da_sync: Arc<dyn ApprovalDaSync>) -> Self {
        self.da_sync = Some(da_sync);
        self
    }

    /// Wires a real `crate::audit::AuditSink`/`crate::audit::AuditQueue`
    /// pair for approval-resolution and mutation-outcome events (Task 7
    /// Step 3's "publish content-free DA audit events ... for ... approval
    /// resolution, and mutation outcome" — a review finding caught this had
    /// no `AuditSink::publish` call anywhere in this module, re-opening
    /// rather than closing a Task 6 SOC2 finding). Auditing here covers both
    /// categories at once: this is the only production code path that can
    /// ever call [`crate::projects::execute`] (see the module doc comment's
    /// security contract), so a mutation's own success/failure is exactly
    /// this method's `Executed`/`Failed` outcome. No production caller
    /// supplies one yet — same disclosed-seam shape as [`Self::with_da_sync`].
    #[must_use]
    pub fn with_audit(mut self, audit: crate::audit::AuditPublisher) -> Self {
        self.audit = Some(Arc::new(audit));
        self
    }

    /// Proposes a new [`MutationPlan`] against `project_id`'s current
    /// trusted root and persists it (encrypted, `pending`). Performs no
    /// mutation -- see [`crate::projects::MutationPlan::propose`].
    pub fn propose(
        &self,
        project_id: ProjectId,
        operations: Vec<EditOperation>,
    ) -> Result<MutationPlan, ApprovalsServiceError> {
        let account = self.current_account()?;
        self.propose_for_account(&account, project_id, operations)
    }

    /// Run-scoped proposal entry point. Unlike the UI command wrapper above,
    /// this never re-resolves mutable ambient account state after a run has
    /// frozen its authority.
    pub(crate) fn propose_for_account(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        operations: Vec<EditOperation>,
    ) -> Result<MutationPlan, ApprovalsServiceError> {
        let trusted_root = self.reopen_trusted_root(account, project_id)?;
        let plan = MutationPlan::propose(&trusted_root, operations)?;
        let store = self.approval_store_for(account)?;
        store.insert_pending(account, &plan)?;
        Ok(plan)
    }

    /// The agent's proposal entry point: what a person's
    /// [`Self::propose_for_account`] does, for a plan a model authored, so its
    /// text is also checked for placeholder comments and the paths whose
    /// content already matched are returned with the plan
    /// ([`MutationPlan::propose_reporting`]).
    pub(crate) fn propose_reporting_for_account(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        operations: Vec<EditOperation>,
    ) -> Result<(MutationPlan, Vec<String>), ApprovalsServiceError> {
        let trusted_root = self.reopen_trusted_root(account, project_id)?;
        let (plan, unchanged) = MutationPlan::propose_reporting(&trusted_root, operations)?;
        let store = self.approval_store_for(account)?;
        store.insert_pending(account, &plan)?;
        Ok((plan, unchanged))
    }

    /// Proposes and immediately applies one change a person authored in the
    /// file viewer.
    ///
    /// There is no review step. These are local files, the operator picked the
    /// folder, and they are looking at the change they just typed -- asking
    /// them to approve their own keystrokes reviews nothing. The trust decision
    /// is the project, exactly as it is for [`Self::apply`].
    ///
    /// Everything except the review survives: the plan is proposed against the
    /// current trusted root, persisted, journaled, audited, and
    /// [`crate::projects::execute`] re-validates containment before touching
    /// disk. Removing the human step moves who decides, not whether the write
    /// is legal.
    pub async fn save(
        &self,
        project_id: ProjectId,
        operations: Vec<EditOperation>,
    ) -> Result<ApprovalOutcome, ApprovalsServiceError> {
        let account = self.current_account()?;
        self.save_for_account(&account, project_id, operations)
            .await
    }

    /// [`Self::save`] for a caller that already resolved the account, so every
    /// step of its operation runs under the one account it checked. Saving a
    /// report into a project (`crate::reports::project`) is that caller: the
    /// person chose to keep a document they are looking at, in a folder they
    /// trusted, which is the same consent a viewer save rests on.
    pub(crate) async fn save_for_account(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        operations: Vec<EditOperation>,
    ) -> Result<ApprovalOutcome, ApprovalsServiceError> {
        let service = self.clone();
        let proposal_account = account.clone();
        let plan = tokio::task::spawn_blocking(move || {
            service.propose_for_account(&proposal_account, project_id, operations)
        })
        .await
        .map_err(|_| ApprovalsServiceError::Unavailable)??;
        self.apply(account, &plan).await
    }

    /// Applies `plan` with no review step.
    ///
    /// Every write to a trusted project comes through here: a file-viewer save
    /// whose exact content is on screen, a report the person chose to keep in
    /// their project (see [`Self::save_for_account`]), and an agent's
    /// `project.propose_patch`. Trusting the folder is the consent for all
    /// three -- the trust sheet says the assistant writes without asking --
    /// and every overwritten or deleted file is captured to the revision
    /// history first, so an unwanted agent edit is undone by restoring it.
    ///
    /// This does not skip the rest of the path. The decision goes
    /// through [`Self::resolve_for_account`], so the plan is journaled,
    /// audited, and re-checked for expiry and staleness, and
    /// [`crate::projects::execute`] independently re-validates containment
    /// before touching disk.
    pub(crate) async fn apply(
        &self,
        account: &ValidatedAccountId,
        plan: &MutationPlan,
    ) -> Result<ApprovalOutcome, ApprovalsServiceError> {
        self.resolve_for_account(
            account,
            plan.plan_id(),
            ApprovalDecision::approve(plan.requires_second_confirmation()),
            None,
            None,
        )
        .await
    }

    pub(crate) async fn apply_cancellable(
        &self,
        account: &ValidatedAccountId,
        plan: &MutationPlan,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<ApprovalOutcome, ApprovalsServiceError> {
        self.resolve_for_account(
            account,
            plan.plan_id(),
            ApprovalDecision::approve(plan.requires_second_confirmation()),
            None,
            Some(cancel.clone()),
        )
        .await
    }

    /// Resolves a previously proposed plan. `tool_call_id` is only ever
    /// `Some` once a live agent run has created a DA-tracked tool call for
    /// the originating `project.propose_patch` invocation (Task 7's
    /// agent-loop dispatch wiring, not yet implemented -- see
    /// `crate::chat::ApprovalDaSync`'s doc comment); otherwise the decision
    /// is recorded locally only, never fabricating a DA identifier.
    ///
    /// # Non-confirmable denial
    ///
    /// A `deny` decision only ever updates this store's own status row --
    /// it never reaches [`crate::projects::execute`]. An `approve` decision
    /// first passes [`require_sufficient_confirmation`] (the
    /// second-confirmation *policy*, over an already-valid plan -- see that
    /// function's own doc comment for why it has no authority over
    /// containment), and even after that passes, [`crate::projects::execute`]
    /// independently, unconditionally re-validates containment before doing
    /// anything else. No confirmation-level value anywhere in this call
    /// chain can substitute for, or skip, that re-check.
    ///
    /// # Single-flight and crash reconciliation
    ///
    /// See the module doc comment's "Single-flight resolution" and "Crash
    /// reconciliation" sections: this row is atomically claimed
    /// ([`ApprovalStore::claim_pending`]) before anything else runs, so two
    /// concurrent calls for the same plan id can never both reach
    /// [`crate::projects::execute`]; a claim that turns out not to be a
    /// real terminal decision is reverted back to pending
    /// ([`ApprovalStore::revert_claim`]) rather than left stuck.
    pub async fn resolve(
        &self,
        plan_id: PlanId,
        decision: ApprovalDecision,
        tool_call_id: Option<Uuid>,
    ) -> Result<ApprovalOutcome, ApprovalsServiceError> {
        let account = self.current_account()?;
        self.resolve_for_account(&account, plan_id, decision, tool_call_id, None)
            .await
    }

    /// [`Self::resolve`] against an account already frozen by a run, rather
    /// than re-read from mutable ambient state — the same split
    /// [`Self::propose_for_account`] makes, and for the same reason.
    async fn resolve_for_account(
        &self,
        account: &ValidatedAccountId,
        plan_id: PlanId,
        decision: ApprovalDecision,
        tool_call_id: Option<Uuid>,
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ApprovalOutcome, ApprovalsServiceError> {
        let account = account.clone();
        let service = self.clone();
        let lock = self.mutation_lock.clone().lock_owned();
        let guard = if let Some(cancel) = &cancel {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ApprovalsServiceError::Cancelled),
                guard = lock => guard,
            }
        } else {
            lock.await
        };
        let resolution = tokio::task::spawn_blocking(move || {
            let _mutation_guard = guard;
            if cancel.as_ref().is_some_and(|cancel| cancel.is_cancelled()) {
                return Err(ApprovalsServiceError::Cancelled);
            }
            service.resolve_local(&account, plan_id, decision)
        })
        .await
        .map_err(|_| ApprovalsServiceError::Unavailable)??;
        if let Some(approved) = resolution.approved {
            self.sync_da(tool_call_id, approved).await;
        }
        if let Some((action, outcome)) = resolution.audit {
            self.publish_audit(action, outcome, &plan_id.to_hex()).await;
        }
        resolution.result
    }

    /// Owns the complete local mutation. Dropping an async caller cannot leave
    /// a completed write at `claiming`, or release the mutation lock mid-plan.
    fn resolve_local(
        &self,
        account: &ValidatedAccountId,
        plan_id: PlanId,
        decision: ApprovalDecision,
    ) -> Result<LocalResolution, ApprovalsServiceError> {
        let store = self.approval_store_for(account)?;
        let (plan, status) = store
            .load(account, plan_id)?
            .ok_or(ApprovalsServiceError::PlanNotFound)?;
        if !status.is_pending() {
            return Err(ApprovalsServiceError::PlanNotFound);
        }
        if !store.claim_pending(account, plan_id)? {
            // Lost the race to a concurrent `resolve()` call for the same
            // plan id, which already claimed this row between our `load`
            // and this claim attempt.
            return Err(ApprovalsServiceError::PlanNotFound);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        #[cfg(test)]
        if let Some(hook) = &self.resolution_hook {
            hook(plan.project_id());
        }

        if !decision.is_approve() {
            store.set_status(account, plan_id, ApprovalStatus::Denied, now_ms)?;
            return Ok(LocalResolution {
                result: Ok(ApprovalOutcome::Denied),
                approved: Some(false),
                audit: Some((
                    crate::audit::AuditAction::ToolDeny,
                    crate::audit::AuditOutcome::Denied,
                )),
            });
        }

        if plan.is_expired(now_ms) {
            store.set_status(account, plan_id, ApprovalStatus::Expired, now_ms)?;
            return Err(ApprovalPolicyError::Expired.into());
        }
        let approval = match require_sufficient_confirmation(&plan, &decision, now_ms) {
            Ok(approval) => approval,
            Err(policy_error) => {
                // Not a terminal decision -- release the claim so the plan
                // remains genuinely pending and can be retried (e.g. with a
                // proper second confirmation), rather than stuck in a
                // permanently-claimed limbo state.
                store.revert_claim(account, plan_id)?;
                return Err(policy_error.into());
            }
        };

        let trust_store = match self.trust_store_for(account) {
            Ok(trust_store) => trust_store,
            Err(error) => {
                store.revert_claim(account, plan_id)?;
                return Err(error);
            }
        };
        let record = match trust_store.get(account, plan.project_id()) {
            Ok(Some(record)) => record,
            Ok(None) => {
                store.revert_claim(account, plan_id)?;
                return Err(ApprovalsServiceError::ProjectNotFound);
            }
            Err(error) => {
                store.revert_claim(account, plan_id)?;
                return Err(error.into());
            }
        };

        // Reconcile a crash between a prior `execute` call fully succeeding
        // and this row's status update actually landing (see the module
        // doc comment's "Crash reconciliation" section): only ever marks
        // the row executed on positive confirmation that every operation's
        // expected post-state already holds, never on ambiguity.
        if let Ok(trusted_root) = TrustedRoot::reopen(
            plan.project_id(),
            record.canonical_path.clone(),
            record.identity,
        ) {
            if plan_already_applied(&trusted_root, &plan) {
                let total = plan.operations().len();
                store.set_status(account, plan_id, ApprovalStatus::Executed, now_ms)?;
                return Ok(LocalResolution {
                    result: Ok(ApprovalOutcome::Executed(ExecutionOutcome {
                        plan_id: plan.plan_id(),
                        operations_completed: total,
                        operations_total: total,
                    })),
                    approved: Some(true),
                    audit: Some((
                        crate::audit::AuditAction::ToolExecute,
                        crate::audit::AuditOutcome::Success,
                    )),
                });
            }
        }

        let backup_result = TrustedRoot::reopen(
            plan.project_id(),
            record.canonical_path.clone(),
            record.identity,
        )
        .map_err(ApprovalsServiceError::from)
        .and_then(|trusted_root| {
            self.capture_overwritten_files(account, &trusted_root, &plan, now_ms)
        });
        if let Err(error) = backup_result {
            store.set_status(account, plan_id, ApprovalStatus::Failed, now_ms)?;
            return Ok(LocalResolution {
                result: Err(error),
                approved: None,
                audit: Some((
                    crate::audit::AuditAction::ToolExecute,
                    crate::audit::AuditOutcome::Failure,
                )),
            });
        }

        match execute(&record.canonical_path, &plan, &approval, now_ms) {
            Ok(outcome) => {
                store.set_status(account, plan_id, ApprovalStatus::Executed, now_ms)?;
                Ok(LocalResolution {
                    result: Ok(ApprovalOutcome::Executed(outcome)),
                    approved: Some(true),
                    audit: Some((
                        crate::audit::AuditAction::ToolExecute,
                        crate::audit::AuditOutcome::Success,
                    )),
                })
            }
            Err(failure) => {
                let status = if failure.durability_unconfirmed {
                    ApprovalStatus::ExecutedDurabilityUnconfirmed
                } else {
                    ApprovalStatus::Failed
                };
                store.set_status(account, plan_id, status, now_ms)?;
                Ok(LocalResolution {
                    result: Err(failure.into()),
                    approved: None,
                    audit: Some((
                        crate::audit::AuditAction::ToolExecute,
                        crate::audit::AuditOutcome::Failure,
                    )),
                })
            }
        }
    }

    /// Best-effort, content-free audit publication — never blocks or
    /// reverses a decision this method already durably recorded (same
    /// precedent as [`Self::sync_da`] immediately below). A no-op when
    /// [`Self::with_audit`] was never called.
    async fn publish_audit(
        &self,
        action: crate::audit::AuditAction,
        outcome: crate::audit::AuditOutcome,
        plan_id_hex: &str,
    ) {
        let Some(audit) = self.audit.as_ref() else {
            return;
        };
        audit
            .publish(
                action,
                crate::audit::AuditResourceType::Project,
                plan_id_hex,
                outcome,
                "approvals.resolve.v1",
                uuid::Uuid::new_v4().to_string().as_str(),
            )
            .await;
    }

    async fn sync_da(&self, tool_call_id: Option<Uuid>, approved: bool) {
        if let (Some(da_sync), Some(tool_call_id)) = (&self.da_sync, tool_call_id) {
            // Best-effort: a DA sync failure must never block or reverse a
            // decision this service already durably recorded locally.
            let _ = da_sync
                .record_mutation_approval_decision(tool_call_id, approved)
                .await;
        }
    }

    fn reopen_trusted_root(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
    ) -> Result<TrustedRoot, ApprovalsServiceError> {
        let record = self
            .trust_store_for(account)?
            .get(account, project_id)?
            .ok_or(ApprovalsServiceError::ProjectNotFound)?;
        TrustedRoot::reopen(project_id, record.canonical_path, record.identity)
            .map_err(ApprovalsServiceError::from)
    }

    fn trust_store_for(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<TrustStore, ApprovalsServiceError> {
        // `crate::projects::trust::project_trust_db_path` is private to that
        // module and not re-exported (nothing outside `crate::projects`
        // needed the trust database's own path before this task) -- the
        // exact same directory/file layout is duplicated here rather than
        // widening that module's public surface just for this one path
        // string, matching this codebase's own established precedent for
        // this specific literal (`crate::projects::trust`'s own doc comment
        // on `PROJECT_DATA_DIR` notes `crate::security::account_cleanup`
        // already hard-codes the same `"project-data"` literal directly
        // rather than importing it).
        let path = self
            .app_root
            .join("accounts")
            .join(account_directory_name(account.as_str()))
            .join(PROJECT_DATA_DIR)
            .join("projects.sqlite3");
        TrustStore::open(path, self.secrets.clone()).map_err(|_| ApprovalsServiceError::Unavailable)
    }

    fn approval_store_for(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<ApprovalStore, ApprovalsServiceError> {
        let path = approval_db_path(&self.app_root, account);
        ApprovalStore::open(path, self.secrets.clone())
    }

    fn backup_store_for(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<ShadowBackupStore, ApprovalsServiceError> {
        let path = self
            .app_root
            .join("accounts")
            .join(account_directory_name(account.as_str()))
            .join(PROJECT_DATA_DIR)
            .join("shadow_backups.sqlite3");
        ShadowBackupStore::open(path, self.secrets.clone()).map_err(ApprovalsServiceError::from)
    }

    fn capture_overwritten_files(
        &self,
        account: &ValidatedAccountId,
        trusted_root: &TrustedRoot,
        plan: &MutationPlan,
        created_at_ms: i64,
    ) -> Result<Vec<BackupRevision>, ApprovalsServiceError> {
        let mut captures = Vec::new();
        for operation in plan.operations() {
            // Every operation that destroys bytes keeps them first: a modify's
            // old content, a deleted file, and a file a rename replaces. With
            // no review before an agent's change lands, this history is the
            // only way back from one.
            let (path, replaced_whole) = match operation {
                PatchOperation::Modify { path, .. } => (path, false),
                PatchOperation::Delete { path } => (path, true),
                PatchOperation::Rename { to, .. }
                    if plan.base_hashes().contains_key(to.as_str()) =>
                {
                    (to, true)
                }
                _ => continue,
            };
            let execution_failure = |error: ContainmentError| {
                ApprovalsServiceError::Execution(WriterExecutionFailure {
                    error: WriterError::from(error),
                    operations_completed: 0,
                    operations_total: plan.operations().len(),
                    durability_unconfirmed: false,
                })
            };
            let resolved = trusted_root
                .resolve_existing(path.clone())
                .map_err(execution_failure)?;
            let metadata = trusted_root
                .file_metadata(&resolved)
                .map_err(execution_failure)?;
            if metadata.len() > MAX_EDITABLE_FILE_BYTES {
                // A delete or replacing rename is proposed by digest alone, so
                // it can name a file too large to keep. Refuse it rather than
                // destroy bytes no revision holds.
                return Err(if replaced_whole {
                    ApprovalsServiceError::TooLargeToKeep(path.as_str().to_owned())
                } else {
                    ShadowBackupStoreError::TooLarge.into()
                });
            }
            let mut content = Vec::with_capacity(metadata.len() as usize);
            trusted_root
                .open_file(&resolved)
                .map_err(execution_failure)?
                .take(MAX_EDITABLE_FILE_BYTES.saturating_add(1))
                .read_to_end(&mut content)
                .map_err(|_| ApprovalsServiceError::Unavailable)?;
            if content.len() as u64 > MAX_EDITABLE_FILE_BYTES {
                return Err(ShadowBackupStoreError::TooLarge.into());
            }
            let expected = plan
                .base_hashes()
                .get(path.as_str())
                .ok_or(ApprovalsServiceError::Unavailable)?;
            if &hash_bytes(&content) != expected {
                return Err(WriterExecutionFailure {
                    error: WriterError::StaleBaseHash(path.as_str().to_owned()),
                    operations_completed: 0,
                    operations_total: plan.operations().len(),
                    durability_unconfirmed: false,
                }
                .into());
            }
            captures.push(BackupCapture {
                project_id: plan.project_id(),
                relative_path: path.clone(),
                content,
            });
        }
        if captures.is_empty() {
            return Ok(Vec::new());
        }
        self.backup_store_for(account)?
            .capture_batch(account, &captures, created_at_ms)
            .map_err(ApprovalsServiceError::from)
    }

    pub fn file_revisions(
        &self,
        project_id: ProjectId,
        relative_path: &RelativeProjectPath,
    ) -> Result<Vec<BackupRevision>, ApprovalsServiceError> {
        let account = self.current_account()?;
        self.reopen_trusted_root(&account, project_id)?;
        self.backup_store_for(&account)?
            .list(&account, project_id, relative_path)
            .map_err(ApprovalsServiceError::from)
    }

    /// What changed in `relative_path` from revision `backup_id` to `against`
    /// -- another revision of the same file -- or, when `against` is `None`,
    /// to the file as it is now (spec: project-file-patch-inspector).
    ///
    /// Both revisions are looked up in this file's own history, so an id from
    /// another file or project is not found rather than compared against the
    /// wrong file. Stored sizes are checked before anything is decrypted.
    pub fn revision_diff(
        &self,
        project_id: ProjectId,
        relative_path: &RelativeProjectPath,
        backup_id: Uuid,
        against: Option<Uuid>,
    ) -> Result<crate::projects::RevisionDiff, ApprovalsServiceError> {
        let account = self.current_account()?;
        let root = self.reopen_trusted_root(&account, project_id)?;
        let store = self.backup_store_for(&account)?;
        let history = store.list(&account, project_id, relative_path)?;
        let stored_size = |id: Uuid| {
            history
                .iter()
                .find(|revision| revision.backup_id == id)
                .map(|revision| revision.original_bytes)
                .ok_or(ShadowBackupStoreError::NotFound)
        };
        let revision_bytes = stored_size(backup_id)?;
        let load = |id: Uuid| -> Result<Vec<u8>, ApprovalsServiceError> {
            Ok(store.load(&account, id)?.content)
        };
        let target = match against {
            Some(against) => crate::projects::DiffTarget::Revision {
                bytes: stored_size(against)?,
                load: Box::new(move || load(against)) as Box<dyn FnOnce() -> _>,
            },
            None => crate::projects::DiffTarget::CurrentFile,
        };
        crate::projects::revision_diff(
            &root,
            &crate::projects::SecretClassifier::built_in(),
            relative_path,
            crate::projects::read_scope_id(),
            revision_bytes,
            || load(backup_id),
            target,
        )
    }

    pub fn load_revision(&self, backup_id: Uuid) -> Result<LoadedBackup, ApprovalsServiceError> {
        let account = self.current_account()?;
        let revision = self.backup_store_for(&account)?.load(&account, backup_id)?;
        self.reopen_trusted_root(&account, revision.project_id)?;
        Ok(revision)
    }

    pub async fn restore_revision(
        &self,
        backup_id: Uuid,
    ) -> Result<LoadedBackup, ApprovalsServiceError> {
        let account = self.current_account()?;
        let service = self.clone();
        let restore_account = account.clone();
        let (revision, plan) =
            tokio::task::spawn_blocking(move || -> Result<_, ApprovalsServiceError> {
                let revision = service
                    .backup_store_for(&restore_account)?
                    .load(&restore_account, backup_id)?;
                // A deleted file is restored with Create; both paths still
                // use the same current-root checks and backed-up writer.
                let root = service.reopen_trusted_root(&restore_account, revision.project_id)?;
                let exists = match root.resolve_existing(revision.relative_path.clone()) {
                    Ok(_) => true,
                    Err(ContainmentError::NotFound) => false,
                    Err(error) => return Err(error.into()),
                };
                let path = revision.relative_path.clone();
                let content = revision.content.clone();
                let operation = if exists {
                    EditOperation::Modify { path, content }
                } else {
                    EditOperation::Create { path, content }
                };
                let plan = service.propose_for_account(
                    &restore_account,
                    revision.project_id,
                    vec![operation],
                )?;
                Ok((revision, plan))
            })
            .await
            .map_err(|_| ApprovalsServiceError::Unavailable)??;
        let outcome = self.apply(&account, &plan).await?;
        match outcome {
            ApprovalOutcome::Executed(execution)
                if execution.operations_completed == execution.operations_total =>
            {
                Ok(revision)
            }
            ApprovalOutcome::Executed(_) | ApprovalOutcome::Denied => {
                Err(ApprovalsServiceError::Unavailable)
            }
        }
    }

    fn current_account(&self) -> Result<ValidatedAccountId, ApprovalsServiceError> {
        self.accounts
            .current()
            .map_err(|_| ApprovalsServiceError::Unavailable)?
            .ok_or(ApprovalsServiceError::NoActiveAccount)
    }
}
