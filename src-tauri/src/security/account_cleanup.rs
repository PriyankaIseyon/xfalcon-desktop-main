use std::{
    fs, io,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use uuid::Uuid;
use xfalcon_contracts::app::{
    CleanupErrorCode, CleanupFailure, CleanupNextAction, CleanupRetryPolicy, LocalDataClearResult,
    LocalDataResource,
};
use zeroize::{Zeroize, Zeroizing};

use crate::persistence::{Journal, SecretStore};

const MAX_HANDLERS: usize = 10;

pub trait CleanupHandler: Send + Sync + 'static {
    fn resource(&self) -> LocalDataResource;
    fn cleanup(&self) -> Result<(), CleanupHandlerFailure>;
    fn applicable(&self) -> Result<bool, CleanupHandlerFailure> {
        Ok(true)
    }
}

/// Seam a project-trust runtime plugs into so account cleanup can revoke
/// trust and cancel in-flight project operations without `crate::security`
/// depending on `crate::projects`'s concrete types -- mirrors this module's
/// existing `SecretStore`/`CleanupHandler` trait-object injection idiom
/// (`FixedAccountResources`'s `secrets`/`credential_secrets` fields). The
/// composition root (`src-tauri/src/lib.rs`) is the only place that ties
/// `crate::projects::ProjectsService` (which implements this trait) to
/// `FixedAccountResources::with_project_cancellation`.
pub trait ProjectCancellation: Send + Sync + 'static {
    /// Clears the account's in-memory active-project pointer and bumps the
    /// project cancellation generation, signaling any in-flight project
    /// indexing/read/write (Task 5/6; no such loop exists yet) to stop.
    /// Called before the on-disk `project-data/` directory is removed, so
    /// cancellation is signaled even if the subsequent filesystem removal
    /// fails.
    fn cancel_all_projects(&self, account: &ValidatedAccountId);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupHandlerFailure {
    code: CleanupErrorCode,
    retryable: bool,
}

impl CleanupHandlerFailure {
    pub const fn new(code: CleanupErrorCode, retryable: bool) -> Self {
        Self { code, retryable }
    }

    pub const fn code(self) -> CleanupErrorCode {
        self.code
    }

    fn contract(self, resource: LocalDataResource) -> CleanupFailure {
        let (retry_policy, next_action) = if matches!(self.code, CleanupErrorCode::NoActiveAccount)
        {
            (CleanupRetryPolicy::DoNotRetry, CleanupNextAction::SignIn)
        } else if !self.retryable {
            (
                CleanupRetryPolicy::DoNotRetry,
                CleanupNextAction::ContactSupport,
            )
        } else if matches!(self.code, CleanupErrorCode::Busy) {
            (
                CleanupRetryPolicy::UserInitiated,
                CleanupNextAction::RetryClearLocalData,
            )
        } else {
            (
                CleanupRetryPolicy::AfterRestart,
                CleanupNextAction::RestartAndRetry,
            )
        };
        CleanupFailure::new(resource, self.code, retry_policy, next_action)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RegistryError {
    #[error("a cleanup resource was registered more than once")]
    DuplicateResource(LocalDataResource),
    #[error("the cleanup registry exceeds its fixed capacity")]
    CapacityExceeded,
}

#[derive(Default)]
pub struct CleanupRegistryBuilder {
    handlers: Vec<Arc<dyn CleanupHandler>>,
    completion_scope: Option<Arc<CurrentAccountScope>>,
    account_root_prune: Option<AccountRootPrune>,
    device_sweep: Option<DeviceSweep>,
}

impl CleanupRegistryBuilder {
    pub const fn new() -> Self {
        Self {
            handlers: Vec::new(),
            completion_scope: None,
            account_root_prune: None,
            device_sweep: None,
        }
    }

    pub fn pin_account_scope(&mut self, scope: Arc<CurrentAccountScope>) {
        self.completion_scope = Some(scope);
    }

    /// Registers the account directory itself for removal *after* every
    /// handler has run.
    ///
    /// The root is the container the resources live in, not a resource, so
    /// no single handler can own it: whichever one tried would run at its own
    /// position in the frozen order and find the directory still populated by
    /// everything sorted after it. That is exactly what happened while this
    /// was bolted onto the `LocalProjections` handler — any account with
    /// trusted projects reported `Partial` from the account clear, because
    /// `LocalProjections` sorts ahead of `ProjectData` and
    /// `DecryptedWorkingFiles`.
    pub fn prune_account_root(&mut self, app_root: PathBuf, scope: Arc<CurrentAccountScope>) {
        self.account_root_prune = Some(AccountRootPrune { app_root, scope });
    }

    /// Registers the loose device-level leftovers for removal *after* the
    /// account root has been pruned. See [`DeviceSweep`].
    pub fn sweep_device_leftovers(&mut self, app_root: PathBuf) {
        self.device_sweep = Some(DeviceSweep { app_root });
    }

    pub fn register(&mut self, handler: Arc<dyn CleanupHandler>) -> Result<(), RegistryError> {
        let resource = handler.resource();
        if self
            .handlers
            .iter()
            .any(|registered| registered.resource() == resource)
        {
            return Err(RegistryError::DuplicateResource(resource));
        }
        if self.handlers.len() == MAX_HANDLERS {
            return Err(RegistryError::CapacityExceeded);
        }
        self.handlers.push(handler);
        Ok(())
    }

    pub fn freeze(mut self) -> Result<CleanupRegistry, RegistryError> {
        self.handlers
            .sort_by_key(|handler| resource_order(handler.resource()));
        let resources = self
            .handlers
            .iter()
            .map(|handler| handler.resource())
            .collect();
        Ok(CleanupRegistry {
            handlers: self.handlers.into(),
            resources,
            completion_scope: self.completion_scope,
            account_root_prune: self.account_root_prune,
            device_sweep: self.device_sweep,
        })
    }
}

pub struct CleanupRegistry {
    handlers: Arc<[Arc<dyn CleanupHandler>]>,
    resources: Vec<LocalDataResource>,
    completion_scope: Option<Arc<CurrentAccountScope>>,
    account_root_prune: Option<AccountRootPrune>,
    device_sweep: Option<DeviceSweep>,
}

/// Removes the now-empty per-account directory once every handler has
/// finished. See [`CleanupRegistryBuilder::prune_account_root`].
struct AccountRootPrune {
    app_root: PathBuf,
    scope: Arc<CurrentAccountScope>,
}

impl AccountRootPrune {
    fn run(&self) -> Result<(), CleanupHandlerFailure> {
        let account = required_account(&self.scope)?;
        let root = checked_account_root(&self.app_root, &account)?;
        remove_empty_directory(&root).map_err(|_| storage_failure())
    }
}

/// Loose device-level items a clear removes after the account is gone.
///
/// None of these belong to one account, and no handler names them, so without
/// this sweep they would survive the account clear indefinitely. Deliberately
/// absent: `logs/`, `EBWebView/` (the WebView profile, in use by the running
/// app), `migrations/` (storage-migration markers) and `audit/` (the security
/// audit log, kept on purpose). Those are removed only by the headless
/// `--purge-local-data` mode (`security::device_purge`).
///
/// Failures are reported as `LocalDataResource::UploadTemporary`, the resource
/// that owns the app-local snapshot directory and has no handler of its own.
/// The sweep never adds to `cleared`, so a clear that removes only the account
/// keeps the same resource list it always reported. Shared system-temp paths
/// are excluded because this process cannot prove their ownership.
struct DeviceSweep {
    app_root: PathBuf,
}

const DEVICE_LEFTOVERS: [&str; 10] = [
    "keepsakes.json",
    "settings.json",
    // Device-level plan-provider setting, not owned by any account.
    crate::plan_providers::settings::PLAN_SETTINGS_FILE,
    // Claude turn folders, which hold a prompt and a loopback token.
    crate::plan_providers::WORK_ROOT_DIR,
    crate::tools::standing::STANDING_APPROVALS_FILE,
    // Pre-account skill library at the app root. Adoption moves it into the
    // first account that opens, so it survives only when no account has.
    crate::skills::store::DEVICE_LIBRARY,
    "desktop-updates",
    ".skills-staging",
    "staging",
    "upload-snapshots",
];

impl DeviceSweep {
    /// Attempts every item even after one fails, so one stuck item does not
    /// strand the rest; the outcome is a single failure if any item failed.
    /// The plan-settings temporaries a crashed write left are swept by prefix,
    /// because their names carry a random part.
    fn run(&self) -> Result<(), CleanupHandlerFailure> {
        reject_link(&self.app_root)?;
        let paths = DEVICE_LEFTOVERS.iter().map(|name| self.app_root.join(name));
        let mut failed = false;
        for path in paths {
            if remove_if_present(&path).is_err() {
                failed = true;
            }
        }
        if remove_settings_temporaries(&self.app_root).is_err() {
            failed = true;
        }
        if failed {
            Err(storage_failure())
        } else {
            Ok(())
        }
    }
}

impl CleanupRegistry {
    pub fn resources(&self) -> &[LocalDataResource] {
        &self.resources
    }

    fn execute(&self) -> LocalDataClearResult {
        let correlation_id = Uuid::new_v4().to_string();
        let mut account_guard = if let Some(scope) = &self.completion_scope {
            match scope.begin_cleanup() {
                Ok(guard) => Some(guard),
                Err(failure) => {
                    let failure = failure.contract(LocalDataResource::SessionVault);
                    return LocalDataClearResult::new(correlation_id, Vec::new(), vec![failure]);
                }
            }
        } else {
            None
        };
        let mut cleared = Vec::with_capacity(self.handlers.len());
        let mut failures = Vec::new();
        for handler in self.handlers.iter() {
            match handler.applicable() {
                Ok(false) => continue,
                Ok(true) => {}
                Err(failure) => {
                    failures.push(failure.contract(handler.resource()));
                    continue;
                }
            }
            let resource = handler.resource();
            match handler.cleanup() {
                Ok(()) => cleared.push(resource),
                Err(failure) => failures.push(failure.contract(resource)),
            }
        }
        // Only once every handler has emptied it — and only if they all
        // succeeded, since a leftover file means data still on disk and the
        // removal would fail anyway.
        if failures.is_empty() {
            if let Some(prune) = self.account_root_prune.as_ref() {
                if let Err(failure) = prune.run() {
                    // Attributed to `LocalProjections`, the handler that owned
                    // this sweep before it moved here: `LocalDataResource` is a
                    // wire contract with no "account container" variant, and
                    // inventing one to re-attribute a failure that only occurs
                    // when the directory is unexpectedly non-empty is not worth
                    // a breaking change to the projection React consumes.
                    failures.push(failure.contract(LocalDataResource::LocalProjections));
                }
            }
        }
        // Last, and only once the account itself is gone. See [`DeviceSweep`].
        if failures.is_empty() {
            if let Some(sweep) = self.device_sweep.as_ref() {
                if let Err(failure) = sweep.run() {
                    failures.push(failure.contract(LocalDataResource::UploadTemporary));
                }
            }
        }
        if failures.is_empty() {
            if let Some(guard) = account_guard.as_mut() {
                guard.unbind_on_drop();
            }
        }
        LocalDataClearResult::new(correlation_id, cleared, failures)
    }
}

const fn resource_order(resource: LocalDataResource) -> u8 {
    match resource {
        LocalDataResource::ActiveWork => 0,
        LocalDataResource::SessionVault => 1,
        LocalDataResource::Journal => 2,
        LocalDataResource::LocalDatabase => 3,
        LocalDataResource::LocalProjections => 4,
        LocalDataResource::ProjectData => 5,
        LocalDataResource::UploadTemporary => 6,
        LocalDataResource::ArtifactCache => 7,
        LocalDataResource::Export => 8,
        LocalDataResource::DecryptedWorkingFiles => 9,
    }
}

#[derive(Clone)]
pub struct AccountCleanup {
    inner: Arc<AccountCleanupInner>,
}

struct AccountCleanupInner {
    registry: CleanupRegistry,
    busy: AtomicBool,
}

struct BusyReset(Arc<AccountCleanupInner>);

impl Drop for BusyReset {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}

impl AccountCleanup {
    pub fn new(registry: CleanupRegistry) -> Self {
        Self {
            inner: Arc::new(AccountCleanupInner {
                registry,
                busy: AtomicBool::new(false),
            }),
        }
    }

    pub fn start(&self) -> Result<CleanupWaiter, CleanupStartError> {
        self.inner
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CleanupStartError::Busy)?;
        let inner = self.inner.clone();
        let (result_tx, result_rx) = oneshot::channel();
        if thread::Builder::new()
            .name("xfalcon-account-cleanup".to_owned())
            .spawn(move || {
                // The guard is scoped to end BEFORE the result is published.
                // Sending on the oneshot wakes the awaiting caller, and a
                // caller that immediately calls `start()` again -- which is
                // exactly what a retry after a failed cleanup does -- would
                // otherwise race this thread's unwind and be told `Busy` by a
                // run that had already finished. Releasing the latch first and
                // sending second makes the store visible to the receiver
                // through the channel's own happens-before, so the retry
                // observes an idle latch deterministically.
                let result = {
                    let _busy_reset = BusyReset(inner.clone());
                    catch_unwind(AssertUnwindSafe(|| inner.registry.execute()))
                        .unwrap_or_else(|_| failure_result(CleanupErrorCode::CleanupIncomplete))
                };
                let _ = result_tx.send(result);
            })
            .is_err()
        {
            self.inner.busy.store(false, Ordering::Release);
            return Err(CleanupStartError::WorkerUnavailable);
        }
        Ok(CleanupWaiter { result: result_rx })
    }

    pub async fn clear(&self) -> LocalDataClearResult {
        match self.start() {
            Ok(waiter) => waiter.wait().await,
            Err(CleanupStartError::Busy) => failure_result(CleanupErrorCode::Busy),
            Err(CleanupStartError::WorkerUnavailable) => {
                failure_result(CleanupErrorCode::CleanupIncomplete)
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.inner.busy.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupStartError {
    Busy,
    WorkerUnavailable,
}

#[derive(Debug)]
pub struct CleanupWaiter {
    result: oneshot::Receiver<LocalDataClearResult>,
}

impl CleanupWaiter {
    pub async fn wait(self) -> LocalDataClearResult {
        self.result
            .await
            .unwrap_or_else(|_| failure_result(CleanupErrorCode::CleanupIncomplete))
    }
}

fn failure_result(code: CleanupErrorCode) -> LocalDataClearResult {
    let failure = CleanupHandlerFailure::new(code, true).contract(LocalDataResource::ActiveWork);
    LocalDataClearResult::new(Uuid::new_v4().to_string(), Vec::new(), vec![failure])
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedAccountId(String);

impl ValidatedAccountId {
    pub fn parse(value: &str) -> Result<Self, CleanupHandlerFailure> {
        if value.is_empty()
            || value.len() > 160
            || matches!(value, "." | "..")
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(CleanupHandlerFailure::new(
                CleanupErrorCode::CleanupIncomplete,
                false,
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn account_directory_name(account_id: &str) -> String {
    let digest = Sha256::digest(account_id.as_bytes());
    format!("acct-{digest:x}")
}

#[derive(Default)]
struct AccountScopeState {
    current: Option<ValidatedAccountId>,
    cleanup_pin: Option<ValidatedAccountId>,
}

#[derive(Default)]
pub struct CurrentAccountScope {
    state: Mutex<AccountScopeState>,
}

impl CurrentAccountScope {
    pub fn bind_native(&self, account_id: &str) -> Result<(), CleanupHandlerFailure> {
        let validated = ValidatedAccountId::parse(account_id)?;
        let mut state = self.state.lock().map_err(|_| state_failure())?;
        if state.cleanup_pin.is_some() {
            return Err(CleanupHandlerFailure::new(CleanupErrorCode::Busy, true));
        }
        state.current = Some(validated);
        Ok(())
    }

    pub fn clear(&self) -> Result<(), CleanupHandlerFailure> {
        let mut state = self.state.lock().map_err(|_| state_failure())?;
        if state.cleanup_pin.is_some() {
            return Err(CleanupHandlerFailure::new(CleanupErrorCode::Busy, true));
        }
        state.current = None;
        Ok(())
    }

    pub fn current(&self) -> Result<Option<ValidatedAccountId>, CleanupHandlerFailure> {
        self.state
            .lock()
            .map_err(|_| state_failure())
            .map(|state| state.current.clone())
    }

    fn begin_cleanup(self: &Arc<Self>) -> Result<AccountCleanupGuard, CleanupHandlerFailure> {
        let pinned = {
            let mut state = self.state.lock().map_err(|_| state_failure())?;
            if state.cleanup_pin.is_some() {
                return Err(CleanupHandlerFailure::new(CleanupErrorCode::Busy, true));
            }
            let pinned = state.current.clone().ok_or_else(|| {
                CleanupHandlerFailure::new(CleanupErrorCode::NoActiveAccount, false)
            })?;
            state.cleanup_pin = Some(pinned.clone());
            pinned
        };
        Ok(AccountCleanupGuard {
            scope: self.clone(),
            pinned,
            unbind: false,
        })
    }
}

struct AccountCleanupGuard {
    scope: Arc<CurrentAccountScope>,
    pinned: ValidatedAccountId,
    unbind: bool,
}

impl AccountCleanupGuard {
    fn unbind_on_drop(&mut self) {
        self.unbind = true;
    }
}

impl Drop for AccountCleanupGuard {
    fn drop(&mut self) {
        let mut state = match self.scope.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if self.unbind && state.current.as_ref() == Some(&self.pinned) {
            state.current = None;
        }
        if state.cleanup_pin.as_ref() == Some(&self.pinned) {
            state.cleanup_pin = None;
        }
    }
}

fn state_failure() -> CleanupHandlerFailure {
    CleanupHandlerFailure::new(CleanupErrorCode::CleanupIncomplete, true)
}

#[derive(Default)]
pub struct JournalSlot {
    journal: Mutex<Option<(ValidatedAccountId, Journal)>>,
}

impl JournalSlot {
    pub fn install(
        &self,
        account_id: ValidatedAccountId,
        journal: Journal,
    ) -> Result<(), CleanupHandlerFailure> {
        let mut slot = self.journal.lock().map_err(|_| state_failure())?;
        if slot.is_some() {
            return Err(state_failure());
        }
        *slot = Some((account_id, journal));
        Ok(())
    }

    pub fn is_ready_for(&self, account_id: &ValidatedAccountId) -> bool {
        self.journal
            .lock()
            .map(|slot| slot.as_ref().is_some_and(|(owner, _)| owner == account_id))
            .unwrap_or(false)
    }

    fn take_for(
        &self,
        account_id: &ValidatedAccountId,
    ) -> Result<Option<Journal>, CleanupHandlerFailure> {
        let mut slot = self.journal.lock().map_err(|_| state_failure())?;
        match slot.as_ref() {
            Some((owner, _)) if owner != account_id => Err(state_failure()),
            Some(_) => Ok(slot.take().map(|(_, journal)| journal)),
            None => Ok(None),
        }
    }
}

#[derive(Default)]
pub struct SessionVault {
    material: Mutex<Zeroizing<Vec<u8>>>,
}

impl SessionVault {
    pub fn with_material_for_runtime(material: Vec<u8>) -> Self {
        Self {
            material: Mutex::new(Zeroizing::new(material)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.material
            .lock()
            .map(|material| material.is_empty())
            .unwrap_or(false)
    }

    fn clear(&self) -> Result<(), CleanupHandlerFailure> {
        let mut material = self
            .material
            .lock()
            .map_err(|_| CleanupHandlerFailure::new(CleanupErrorCode::VaultUnavailable, true))?;
        material.zeroize();
        material.clear();
        Ok(())
    }
}

pub struct FixedAccountResources {
    app_root: PathBuf,
    scope: Arc<CurrentAccountScope>,
    vault: Arc<SessionVault>,
    journal: Arc<JournalSlot>,
    secrets: Arc<dyn SecretStore>,
    credential_secrets: Option<Arc<dyn SecretStore>>,
    project_cancellation: Option<Arc<dyn ProjectCancellation>>,
}

impl FixedAccountResources {
    pub fn new(
        app_root: PathBuf,
        scope: Arc<CurrentAccountScope>,
        vault: Arc<SessionVault>,
        journal: Arc<JournalSlot>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            app_root,
            scope,
            vault,
            journal,
            secrets,
            credential_secrets: None,
            project_cancellation: None,
        }
    }

    pub fn with_credential_secrets(mut self, credential_secrets: Arc<dyn SecretStore>) -> Self {
        self.credential_secrets = Some(credential_secrets);
        self
    }

    /// Wires a live project-trust runtime into cleanup: revokes trust roots,
    /// cancels in-flight project operations, and removes project-data /
    /// decrypted-working-copies directories. Without this, `registry()`
    /// leaves `LocalDataResource::ProjectData`/`DecryptedWorkingFiles`
    /// unregistered (`applicable`/`cleanup` for them never run) -- the
    /// production composition root (`src-tauri/src/lib.rs`) always calls
    /// this; tests that don't exercise projects may omit it.
    pub fn with_project_cancellation(
        mut self,
        project_cancellation: Arc<dyn ProjectCancellation>,
    ) -> Self {
        self.project_cancellation = Some(project_cancellation);
        self
    }

    pub fn registry(self) -> Result<CleanupRegistry, RegistryError> {
        let mut builder = CleanupRegistryBuilder::new();
        builder.pin_account_scope(self.scope.clone());
        builder.register(self.path_handler(LocalDataResource::ActiveWork, "active-work", false))?;
        builder.register(Arc::new(SessionVaultHandler {
            app_root: self.app_root.clone(),
            scope: self.scope.clone(),
            vault: self.vault.clone(),
            secrets: self.secrets.clone(),
            credential_secrets: self.credential_secrets.clone(),
        }))?;
        builder.register(Arc::new(JournalHandler {
            app_root: self.app_root.clone(),
            scope: self.scope.clone(),
            slot: self.journal.clone(),
        }))?;
        builder.register(self.sqlite_handler(LocalDataResource::LocalDatabase, "local.sqlite3"))?;
        // Removes `local-projections/` including chat_projection.sqlite3 (+ WAL/SHM)
        // written by EncryptedChatProjection, and the per-account files that no
        // other handler names: the chosen chat model, custom model settings, the
        // Claude effort each chat keeps, the imported skill library and its
        // staging folder. Every name is a direct
        // child of the account directory. A file in that directory that is not
        // named here still fails the clear, which is the safety net for data
        // this module does not know about.
        //
        // This slot is the one that owns the account's own files because it
        // already sorts after every sealed store and before the directories
        // the project and report handlers own. No new resource variant.
        builder.register(Arc::new(FixedPathHandler {
            resource: LocalDataResource::LocalProjections,
            app_root: self.app_root.clone(),
            scope: self.scope.clone(),
            names: vec![
                "local-projections".to_owned(),
                crate::chat::model_preference::MODEL_PREFERENCE_FILE.to_owned(),
                crate::chat::custom_model_settings::CUSTOM_MODEL_SETTINGS_FILE.to_owned(),
                crate::plan_providers::chat_effort::CHAT_EFFORT_FILE.to_owned(),
                crate::skills::store::ACCOUNT_LIBRARY_DIR.to_owned(),
                crate::skills::store::ACCOUNT_STAGING_DIR.to_owned(),
            ],
            remove_account_root: false,
        }))?;
        // Saved reports (`accounts/<id>/reports/`), including the reports.sqlite3
        // (+ WAL/SHM) that earlier builds wrote.
        //
        // `ArtifactCache` is the arm that names these bytes. It had an ordering
        // slot in `resource_order` and no handler, which failed twice over: a
        // report survived a clear that claimed to remove it, AND the
        // post-handler `AccountRootPrune` then found the account root
        // non-empty and failed, so `LocalDataClearResult::new` turned a clear
        // that actually worked into `Partial` for every account that had ever
        // saved a report. No new `LocalDataResource` variant: adding an arm is
        // a breaking change to a projection React consumes, which is the
        // argument this module already makes for itself above.
        //
        // Read the arm's name with care. "Cache" is what the wire enum says,
        // but for a report that earlier builds only ever saved here, this
        // directory holds the only copy of its document. Reports now live on DA
        // and nothing reads this directory, so a clear removes it as it always
        // did; it is not reclaimable space to anything but that clear.
        builder.register(self.path_handler(
            LocalDataResource::ArtifactCache,
            crate::reports::REPORTS_DIR,
            false,
        ))?;
        // Conversation exports (`accounts/<id>/export/`). Nothing writes here
        // any more — an export now goes wherever the person points the save
        // dialog (`crate::export`), which is outside app-local data by design
        // and therefore outside what a clear can reach. The handler stays so a
        // clear still removes what earlier builds left behind, and for the same
        // reason `decrypted-working-files` below is registered ahead of its
        // writer: the directory a clear must cover is named here, not wherever
        // the writer happens to live.
        builder.register(self.path_handler(LocalDataResource::Export, "export", false))?;
        if let Some(project_cancellation) = self.project_cancellation.clone() {
            builder.register(Arc::new(ProjectDataHandler {
                app_root: self.app_root.clone(),
                scope: self.scope.clone(),
                cancellation: project_cancellation,
            }))?;
            // Decrypted working copies of trusted-project files (Task 5/6;
            // no writer exists yet, but the directory this removes is named
            // now so cleanup already covers it once one does).
            builder.register(self.path_handler(
                LocalDataResource::DecryptedWorkingFiles,
                "decrypted-working-files",
                false,
            ))?;
        }
        builder.prune_account_root(self.app_root.clone(), self.scope.clone());
        builder.sweep_device_leftovers(self.app_root.clone());
        builder.freeze()
    }

    fn path_handler(
        &self,
        resource: LocalDataResource,
        relative: &'static str,
        remove_account_root: bool,
    ) -> Arc<dyn CleanupHandler> {
        Arc::new(FixedPathHandler {
            resource,
            app_root: self.app_root.clone(),
            scope: self.scope.clone(),
            names: vec![relative.to_owned()],
            remove_account_root,
        })
    }

    fn sqlite_handler(
        &self,
        resource: LocalDataResource,
        name: &'static str,
    ) -> Arc<dyn CleanupHandler> {
        Arc::new(FixedPathHandler {
            resource,
            app_root: self.app_root.clone(),
            scope: self.scope.clone(),
            names: ["", "-wal", "-shm"]
                .into_iter()
                .map(|suffix| format!("{name}{suffix}"))
                .collect(),
            remove_account_root: false,
        })
    }
}

/// Clears everything holding this account's sealed secret material: the
/// in-memory session vault, both keyrings' account keys, and the on-disk
/// ciphertext sealed under those keys.
///
/// The MCP per-server credential store belongs here rather than in a handler of
/// its own because it is the *same* material: those rows are envelopes under the
/// very device keys this handler deletes, and `LocalDataResource` has no arm
/// that describes them. Removing them anywhere else would also leave the
/// account directory non-empty for the post-handler prune.
///
/// The ChatGPT plan sign-in is the same kind of material: its `chatgpt-plan/`
/// directory holds sessions sealed under these account keys, so it is removed
/// here for the same reason and before the keys are deleted.
struct SessionVaultHandler {
    app_root: PathBuf,
    scope: Arc<CurrentAccountScope>,
    vault: Arc<SessionVault>,
    secrets: Arc<dyn SecretStore>,
    credential_secrets: Option<Arc<dyn SecretStore>>,
}

impl CleanupHandler for SessionVaultHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::SessionVault
    }

    fn applicable(&self) -> Result<bool, CleanupHandlerFailure> {
        self.scope.current().map(|account| account.is_some())
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        let account = required_account(&self.scope)?;
        let root = checked_account_root(&self.app_root, &account)?;
        // The sealed rows go before the keys that open them: if key deletion
        // then fails and the user retries, the ciphertext is already gone —
        // whereas losing the keys first would leave a file this handler can
        // still be asked to remove but nothing can ever read.
        remove_named(
            &root,
            &[
                crate::mcp::credential_store::MCP_DATA_DIR,
                crate::providers::chatgpt_plan::store::PLAN_DIR,
            ],
        )?;
        self.vault.clear()?;
        self.secrets
            .delete_account_keys(account.as_str())
            .map_err(|_| CleanupHandlerFailure::new(CleanupErrorCode::VaultUnavailable, true))?;
        if let Some(credential_secrets) = &self.credential_secrets {
            credential_secrets
                .delete_account_keys(account.as_str())
                .map_err(|_| {
                    CleanupHandlerFailure::new(CleanupErrorCode::VaultUnavailable, true)
                })?;
        }
        Ok(())
    }
}

struct JournalHandler {
    app_root: PathBuf,
    scope: Arc<CurrentAccountScope>,
    slot: Arc<JournalSlot>,
}

impl CleanupHandler for JournalHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::Journal
    }

    fn applicable(&self) -> Result<bool, CleanupHandlerFailure> {
        self.scope.current().map(|account| account.is_some())
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        let account = required_account(&self.scope)?;
        if let Some(journal) = self.slot.take_for(&account)? {
            journal
                .purge_account(account.as_str())
                .map_err(|_| storage_failure())?;
            journal.close().map_err(|_| storage_failure())?;
        }
        let root = checked_account_root(&self.app_root, &account)?;
        remove_named(
            &root,
            &[
                "journal.sqlite3",
                "journal.sqlite3-wal",
                "journal.sqlite3-shm",
            ],
        )
    }
}

/// Revokes trust and cancels in-flight project operations for the current
/// account, then removes `project-data/` (the trust store database plus,
/// once Task 5/6 exist, the project index and patch-temporary files) --
/// mirrors `JournalHandler`'s "cancel/close the live resource, then remove
/// its files" shape.
struct ProjectDataHandler {
    app_root: PathBuf,
    scope: Arc<CurrentAccountScope>,
    cancellation: Arc<dyn ProjectCancellation>,
}

impl CleanupHandler for ProjectDataHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::ProjectData
    }

    fn applicable(&self) -> Result<bool, CleanupHandlerFailure> {
        self.scope.current().map(|account| account.is_some())
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        let account = required_account(&self.scope)?;
        // Signal cancellation before touching disk, so in-flight project
        // work is told to stop even if the filesystem removal below fails.
        self.cancellation.cancel_all_projects(&account);
        let root = checked_account_root(&self.app_root, &account)?;
        remove_named(&root, &["project-data"])
    }
}

struct FixedPathHandler {
    resource: LocalDataResource,
    app_root: PathBuf,
    scope: Arc<CurrentAccountScope>,
    names: Vec<String>,
    remove_account_root: bool,
}

impl CleanupHandler for FixedPathHandler {
    fn resource(&self) -> LocalDataResource {
        self.resource
    }

    fn applicable(&self) -> Result<bool, CleanupHandlerFailure> {
        self.scope.current().map(|account| account.is_some())
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        let account = required_account(&self.scope)?;
        let root = checked_account_root(&self.app_root, &account)?;
        let names = self.names.iter().map(String::as_str).collect::<Vec<_>>();
        remove_named(&root, &names)?;
        if self.remove_account_root {
            remove_empty_directory(&root).map_err(|_| storage_failure())?;
        }
        Ok(())
    }
}

fn required_account(
    scope: &CurrentAccountScope,
) -> Result<ValidatedAccountId, CleanupHandlerFailure> {
    scope.current()?.ok_or_else(state_failure)
}

fn checked_account_root(
    app_root: &Path,
    account: &ValidatedAccountId,
) -> Result<PathBuf, CleanupHandlerFailure> {
    reject_link(app_root)?;
    let accounts = app_root.join("accounts");
    reject_link(&accounts)?;
    let account_root = accounts.join(account_directory_name(account.as_str()));
    reject_link(&account_root)?;
    Ok(account_root)
}

fn reject_link(path: &Path) -> Result<(), CleanupHandlerFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => Err(storage_failure()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(storage_failure()),
    }
}

fn remove_named(root: &Path, names: &[&str]) -> Result<(), CleanupHandlerFailure> {
    for name in names {
        remove_if_present(&root.join(name)).map_err(|_| storage_failure())?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "link"))
        }
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Removes the temporaries a crashed plan-settings write left at the app root.
/// Only a regular file is removed. A link is refused, as the named items
/// refuse one, and anything else with the prefix is not ours, so it is left
/// alone. Every entry is attempted, and any failure is reported at the end.
fn remove_settings_temporaries(root: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut failed = false;
    for entry in entries {
        let Ok(entry) = entry else {
            failed = true;
            continue;
        };
        let is_temporary = entry.file_name().to_str().is_some_and(|name| {
            name.starts_with(crate::plan_providers::settings::TEMPORARY_PREFIX)
        });
        if is_temporary && remove_settings_temporary(&entry.path()).is_err() {
            failed = true;
        }
    }
    if failed {
        Err(io::Error::other(
            "a plan settings temporary could not be removed",
        ))
    } else {
        Ok(())
    }
}

fn remove_settings_temporary(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "link"))
        }
        Ok(metadata) if metadata.is_file() => fs::remove_file(path),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn remove_empty_directory(path: &Path) -> io::Result<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn storage_failure() -> CleanupHandlerFailure {
    CleanupHandlerFailure::new(CleanupErrorCode::StorageUnavailable, true)
}
