use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

use tempfile::tempdir;
use xfalcon_contracts::app::{
    CleanupErrorCode, CleanupNextAction, CleanupRetryPolicy, LocalDataClearStatus,
    LocalDataResource,
};

/// Every resource `FixedAccountResources::registry` registers when no project
/// cancellation hook is supplied. `ProjectData` and `DecryptedWorkingFiles`
/// are registered only alongside `with_project_cancellation`, so they are
/// deliberately absent here.
const BASE_REGISTRY_RESOURCES: [LocalDataResource; 7] = [
    LocalDataResource::ActiveWork,
    LocalDataResource::SessionVault,
    LocalDataResource::Journal,
    LocalDataResource::LocalDatabase,
    LocalDataResource::LocalProjections,
    // The `reports/` directory. The arm reads as "cache"; reports live on DA now,
    // so this clears only what earlier builds wrote there (spec: reports).
    LocalDataResource::ArtifactCache,
    LocalDataResource::Export,
];

/// Asserts `cleared` is exactly [`BASE_REGISTRY_RESOURCES`], naming any
/// missing entry. `LocalDataResource` is a wire contract and derives neither
/// `Ord` nor `Hash`, and widening it purely to let a test build a set would
/// be the tail wagging the dog.
fn assert_base_registry_cleared(cleared: &[LocalDataResource]) {
    for resource in BASE_REGISTRY_RESOURCES {
        assert!(
            cleared.contains(&resource),
            "cleanup did not clear {resource:?}; got {cleared:?}"
        );
    }
    assert_eq!(
        cleared.len(),
        BASE_REGISTRY_RESOURCES.len(),
        "cleanup reported an unexpected extra resource: {cleared:?}"
    );
}

use crate::app::commands::AppRuntimeState;
use crate::persistence::{InMemorySecretStore, Journal, NewJournalEvent, SecretStore};
use crate::security::account_cleanup::{
    account_directory_name, AccountCleanup, CleanupHandler, CleanupHandlerFailure,
    CleanupRegistryBuilder, CleanupStartError, CurrentAccountScope, FixedAccountResources,
    JournalSlot, ProjectCancellation, RegistryError, SessionVault, ValidatedAccountId,
};

#[derive(Clone)]
struct RecordingHandler {
    resource: LocalDataResource,
    calls: Arc<Mutex<Vec<LocalDataResource>>>,
    result: Result<(), CleanupHandlerFailure>,
}

impl RecordingHandler {
    fn success(resource: LocalDataResource, calls: Arc<Mutex<Vec<LocalDataResource>>>) -> Self {
        Self {
            resource,
            calls,
            result: Ok(()),
        }
    }

    fn failure(
        resource: LocalDataResource,
        calls: Arc<Mutex<Vec<LocalDataResource>>>,
        code: CleanupErrorCode,
        retryable: bool,
    ) -> Self {
        Self {
            resource,
            calls,
            result: Err(CleanupHandlerFailure::new(code, retryable)),
        }
    }
}

impl CleanupHandler for RecordingHandler {
    fn resource(&self) -> LocalDataResource {
        self.resource
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        self.calls
            .lock()
            .expect("recording lock")
            .push(self.resource);
        self.result
    }
}

#[test]
fn registry_freezes_handlers_in_stable_resource_order_and_rejects_duplicates() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut builder = CleanupRegistryBuilder::new();
    for resource in [
        LocalDataResource::DecryptedWorkingFiles,
        LocalDataResource::Export,
        LocalDataResource::ProjectData,
        LocalDataResource::LocalProjections,
        LocalDataResource::ArtifactCache,
        LocalDataResource::Journal,
        LocalDataResource::ActiveWork,
        LocalDataResource::UploadTemporary,
        LocalDataResource::LocalDatabase,
        LocalDataResource::SessionVault,
    ] {
        builder
            .register(Arc::new(RecordingHandler::success(resource, calls.clone())))
            .expect("unique bounded handler");
    }

    let duplicate = builder
        .register(Arc::new(RecordingHandler::success(
            LocalDataResource::Journal,
            calls,
        )))
        .expect_err("duplicate resource must be rejected");
    assert_eq!(
        duplicate,
        RegistryError::DuplicateResource(LocalDataResource::Journal)
    );

    let registry = builder.freeze().expect("five-handler registry");
    assert_eq!(
        registry.resources(),
        &[
            LocalDataResource::ActiveWork,
            LocalDataResource::SessionVault,
            LocalDataResource::Journal,
            LocalDataResource::LocalDatabase,
            LocalDataResource::LocalProjections,
            LocalDataResource::ProjectData,
            LocalDataResource::UploadTemporary,
            LocalDataResource::ArtifactCache,
            LocalDataResource::Export,
            LocalDataResource::DecryptedWorkingFiles,
        ]
    );
}

struct BlockingHandler {
    entered: mpsc::SyncSender<()>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    completed: Option<mpsc::SyncSender<()>>,
}

struct ScopedDeletionHandler {
    scope: Arc<CurrentAccountScope>,
    secrets: Arc<InMemorySecretStore>,
    root: std::path::PathBuf,
}

impl CleanupHandler for ScopedDeletionHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::SessionVault
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        let account = self
            .scope
            .current()?
            .ok_or_else(|| CleanupHandlerFailure::new(CleanupErrorCode::NoActiveAccount, false))?;
        self.secrets
            .delete_account_keys(account.as_str())
            .map_err(|_| CleanupHandlerFailure::new(CleanupErrorCode::VaultUnavailable, true))?;
        fs::remove_file(self.root.join(account.as_str()))
            .map_err(|_| CleanupHandlerFailure::new(CleanupErrorCode::StorageUnavailable, true))
    }
}

impl CleanupHandler for BlockingHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::ActiveWork
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        let _ = self.entered.send(());
        let (lock, condition) = &*self.gate;
        let mut released = lock.lock().expect("gate lock");
        while !*released {
            released = condition.wait(released).expect("gate wait");
        }
        if let Some(completed) = &self.completed {
            let _ = completed.send(());
        }
        Ok(())
    }
}

fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, condition) = &**gate;
    *lock.lock().expect("gate lock") = true;
    condition.notify_all();
}

#[tokio::test]
async fn cleanup_is_single_flight_and_reports_busy_without_starting_another_worker() {
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut builder = CleanupRegistryBuilder::new();
    builder
        .register(Arc::new(BlockingHandler {
            entered: entered_tx,
            gate: gate.clone(),
            completed: None,
        }))
        .expect("blocking handler");
    let cleanup = AccountCleanup::new(builder.freeze().expect("registry"));

    let first = cleanup.start().expect("first cleanup starts");
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first worker entered");
    assert_eq!(
        cleanup.start().expect_err("second cleanup is busy"),
        CleanupStartError::Busy
    );

    release(&gate);
    assert_eq!(first.wait().await.status, LocalDataClearStatus::Cleared);
}

#[tokio::test]
async fn active_cleanup_pins_native_account_and_rejects_concurrent_rebind() {
    let directory = tempdir().expect("app data");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("first account");
    let first_file = directory.path().join("account-1");
    let second_file = directory.path().join("account-2");
    fs::write(&first_file, b"first").expect("first sentinel");
    fs::write(&second_file, b"second").expect("second sentinel");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [1; 32]).expect("first key");
    secrets.put_key("account-2", [2; 32]).expect("second key");

    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut builder = CleanupRegistryBuilder::new();
    builder.pin_account_scope(scope.clone());
    builder
        .register(Arc::new(BlockingHandler {
            entered: entered_tx,
            gate: gate.clone(),
            completed: None,
        }))
        .expect("blocking handler");
    builder
        .register(Arc::new(ScopedDeletionHandler {
            scope: scope.clone(),
            secrets: secrets.clone(),
            root: directory.path().to_path_buf(),
        }))
        .expect("scoped deletion handler");
    let cleanup = AccountCleanup::new(builder.freeze().expect("registry"));

    let waiter = cleanup.start().expect("cleanup starts");
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("cleanup entered");
    let bind_error = scope
        .bind_native("account-2")
        .expect_err("rebind must be serialized");
    assert_eq!(bind_error.code(), CleanupErrorCode::Busy);
    release(&gate);
    assert_eq!(waiter.wait().await.status, LocalDataClearStatus::Cleared);

    assert!(scope.current().expect("scope").is_none());
    assert!(!first_file.exists());
    assert!(second_file.exists());
    assert!(secrets.active_key_version("account-1").is_err());
    assert_eq!(
        secrets.active_key_version("account-2").expect("second key"),
        1
    );
}

#[tokio::test]
async fn dropping_the_waiter_does_not_cancel_owned_cleanup_execution() {
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (completed_tx, completed_rx) = mpsc::sync_channel(1);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut builder = CleanupRegistryBuilder::new();
    builder
        .register(Arc::new(BlockingHandler {
            entered: entered_tx,
            gate: gate.clone(),
            completed: Some(completed_tx),
        }))
        .expect("blocking handler");
    let cleanup = AccountCleanup::new(builder.freeze().expect("registry"));

    let waiter = cleanup.start().expect("cleanup starts");
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker entered");
    drop(waiter);
    release(&gate);

    completed_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("owned worker completed after waiter cancellation");
    let deadline = Instant::now() + Duration::from_secs(2);
    while cleanup.is_busy() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!cleanup.is_busy(), "owned cleanup worker did not finish");
    let retry = cleanup.start().expect("busy state resets after completion");
    assert_eq!(retry.wait().await.status, LocalDataClearStatus::Cleared);
}

#[tokio::test]
async fn failures_are_aggregated_in_resource_order_without_exposing_raw_details() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut builder = CleanupRegistryBuilder::new();
    builder
        .register(Arc::new(RecordingHandler::failure(
            LocalDataResource::LocalDatabase,
            calls.clone(),
            CleanupErrorCode::StorageUnavailable,
            true,
        )))
        .expect("database handler");
    builder
        .register(Arc::new(RecordingHandler::success(
            LocalDataResource::ActiveWork,
            calls.clone(),
        )))
        .expect("active work handler");
    builder
        .register(Arc::new(RecordingHandler::failure(
            LocalDataResource::SessionVault,
            calls,
            CleanupErrorCode::VaultUnavailable,
            false,
        )))
        .expect("vault handler");
    let cleanup = AccountCleanup::new(builder.freeze().expect("registry"));

    let result = cleanup.start().expect("cleanup starts").wait().await;
    assert_eq!(result.status, LocalDataClearStatus::Partial);
    assert_eq!(result.cleared, vec![LocalDataResource::ActiveWork]);
    assert_eq!(
        result
            .failures
            .iter()
            .map(|failure| (failure.resource, failure.code, failure.retryable))
            .collect::<Vec<_>>(),
        vec![
            (
                LocalDataResource::SessionVault,
                CleanupErrorCode::VaultUnavailable,
                false,
            ),
            (
                LocalDataResource::LocalDatabase,
                CleanupErrorCode::StorageUnavailable,
                true,
            ),
        ]
    );
    assert_eq!(
        result.failures[0].retry_policy,
        CleanupRetryPolicy::DoNotRetry
    );
    assert_eq!(
        result.failures[0].next_action,
        CleanupNextAction::ContactSupport
    );
    assert_eq!(
        result.failures[1].retry_policy,
        CleanupRetryPolicy::AfterRestart
    );
    assert_eq!(
        result.failures[1].next_action,
        CleanupNextAction::RestartAndRetry
    );
    let wire = serde_json::to_string(&result).expect("safe cleanup result");
    uuid::Uuid::parse_str(&result.correlation_id).expect("opaque correlation UUID");
    assert!(!wire.contains('\\'));
    assert!(!wire.contains('/'));
    assert!(!wire.contains("reason"));
}

struct FailOnceHandler {
    attempts: AtomicUsize,
}

struct PanicHandler;

impl CleanupHandler for PanicHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::ActiveWork
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        panic!("sensitive panic detail")
    }
}

impl CleanupHandler for FailOnceHandler {
    fn resource(&self) -> LocalDataResource {
        LocalDataResource::Journal
    }

    fn cleanup(&self) -> Result<(), CleanupHandlerFailure> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(CleanupHandlerFailure::new(
                CleanupErrorCode::StorageUnavailable,
                true,
            ))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn retryable_failure_can_be_retried_idempotently() {
    let mut builder = CleanupRegistryBuilder::new();
    builder
        .register(Arc::new(FailOnceHandler {
            attempts: AtomicUsize::new(0),
        }))
        .expect("journal handler");
    let cleanup = AccountCleanup::new(builder.freeze().expect("registry"));

    let first = cleanup.start().expect("first cleanup starts").wait().await;
    assert_eq!(first.status, LocalDataClearStatus::Failed);
    assert!(first.failures[0].retryable);
    assert!(
        !cleanup.is_busy(),
        "a visible result means the run is over, so the latch must already be clear"
    );

    let second = cleanup.start().expect("retry starts").wait().await;
    assert_eq!(second.status, LocalDataClearStatus::Cleared);
    assert_eq!(second.cleared, vec![LocalDataResource::Journal]);
}

#[tokio::test]
async fn panicking_handler_returns_safe_failure_and_never_wedges_busy_latch() {
    let mut builder = CleanupRegistryBuilder::new();
    builder
        .register(Arc::new(PanicHandler))
        .expect("panic handler");
    let cleanup = AccountCleanup::new(builder.freeze().expect("registry"));

    let first = cleanup.start().expect("first cleanup").wait().await;
    assert_eq!(first.status, LocalDataClearStatus::Failed);
    assert_eq!(first.failures[0].code, CleanupErrorCode::CleanupIncomplete);
    assert!(!serde_json::to_string(&first)
        .expect("safe result")
        .contains("sensitive panic detail"));
    assert!(
        !cleanup.is_busy(),
        "the latch must be released before the result is published, or a caller \
         that retries the moment it sees a failure races the worker's unwind"
    );

    let second = cleanup.start().expect("busy latch reset").wait().await;
    assert_eq!(second.status, LocalDataClearStatus::Failed);
}

#[tokio::test]
async fn fixed_account_resources_clear_all_local_material_and_sqlite_sidecars_idempotently() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let account_root = root
        .join("accounts")
        .join(account_directory_name("tenant:account-1"));
    let neighbor_root = root
        .join("accounts")
        .join(account_directory_name("account-2"));
    let active_work = account_root.join("active-work");
    let projections = account_root.join("local-projections");
    fs::create_dir_all(&active_work).expect("active work directory");
    fs::create_dir_all(&projections).expect("projection directory");
    fs::write(active_work.join("run.bin"), b"active").expect("active work");
    fs::write(projections.join("state.bin"), b"projection").expect("projection");
    for name in [
        "journal.sqlite3",
        "journal.sqlite3-wal",
        "journal.sqlite3-shm",
        "local.sqlite3",
        "local.sqlite3-wal",
        "local.sqlite3-shm",
    ] {
        fs::write(account_root.join(name), b"local").expect("local database file");
    }
    fs::create_dir_all(&neighbor_root).expect("neighbor root");
    let shared_database = neighbor_root.join("local.sqlite3");
    fs::write(&shared_database, b"another account").expect("neighbor database");
    let vault = Arc::new(SessionVault::with_material_for_runtime(vec![1, 2, 3, 4]));
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    secrets
        .put_key("account-2", [0xB6; 32])
        .expect("neighbor key");
    let credential_secrets = Arc::new(InMemorySecretStore::default());
    credential_secrets
        .put_key("tenant:account-1", [0xC7; 32])
        .expect("provider device key");
    credential_secrets
        .put_key("account-2", [0xD8; 32])
        .expect("neighbor provider device key");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        vault.clone(),
        Arc::new(JournalSlot::default()),
        secrets.clone(),
    )
    .with_credential_secrets(credential_secrets.clone())
    .registry()
    .expect("registry");
    let cleanup = AccountCleanup::new(registry);

    let first = cleanup.start().expect("cleanup starts").wait().await;
    assert_eq!(first.status, LocalDataClearStatus::Cleared);
    // Asserted as an exact set rather than a bare count: a count says nothing
    // about *which* resource appeared or vanished, and this list is the whole
    // point of the guard. `Export` joined the base registry after these
    // assertions were written and the drift went unnoticed while the Windows
    // harness could not launch.
    assert_base_registry_cleared(&first.cleared);
    assert!(!account_root.exists());
    assert!(
        shared_database.exists(),
        "shared database must be preserved"
    );
    assert!(vault.is_empty());
    assert!(
        credential_secrets
            .active_key_version("tenant:account-1")
            .is_err(),
        "provider device key must be purged"
    );

    let second = cleanup
        .start()
        .expect("idempotent cleanup starts")
        .wait()
        .await;
    assert_eq!(second.status, LocalDataClearStatus::Failed);
    assert!(second.cleared.is_empty());
    assert_eq!(second.failures[0].code, CleanupErrorCode::NoActiveAccount);
    assert!(shared_database.exists());
    assert_eq!(
        secrets
            .active_key_version("account-2")
            .expect("neighbor key remains"),
        1
    );
    assert_eq!(
        credential_secrets
            .active_key_version("account-2")
            .expect("neighbor provider device key remains"),
        1
    );
}

#[tokio::test]
async fn unbound_cleanup_is_idempotent_and_does_not_touch_app_data() {
    let directory = tempdir().expect("temporary app data");
    let sentinel = directory.path().join("do-not-touch");
    fs::write(&sentinel, b"safe").expect("sentinel");
    let scope = Arc::new(CurrentAccountScope::default());
    let registry = FixedAccountResources::new(
        directory.path().to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        Arc::new(InMemorySecretStore::default()),
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry)
        .start()
        .expect("cleanup starts")
        .wait()
        .await;

    assert_eq!(result.status, LocalDataClearStatus::Failed);
    assert!(result.cleared.is_empty());
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].resource, LocalDataResource::SessionVault);
    assert_eq!(result.failures[0].code, CleanupErrorCode::NoActiveAccount);
    assert!(!result.failures[0].retryable);
    assert_eq!(
        result.failures[0].retry_policy,
        CleanupRetryPolicy::DoNotRetry
    );
    assert_eq!(result.failures[0].next_action, CleanupNextAction::SignIn);
    uuid::Uuid::parse_str(&result.correlation_id).expect("opaque correlation UUID");
    assert!(sentinel.exists());
}

#[tokio::test]
async fn unknown_residual_file_fails_cleanup_and_keeps_account_bound_for_recovery() {
    let directory = tempdir().expect("app data");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("account");
    let root = directory
        .path()
        .join("accounts")
        .join(account_directory_name("account-1"));
    fs::create_dir_all(&root).expect("account root");
    let sentinel = root.join("unknown-residual");
    fs::write(&sentinel, b"must remain").expect("residual");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [1; 32]).expect("key");
    let registry = FixedAccountResources::new(
        directory.path().to_path_buf(),
        scope.clone(),
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(result.status, LocalDataClearStatus::Partial);
    assert_eq!(
        result.failures[0].resource,
        LocalDataResource::LocalProjections
    );
    assert_eq!(
        result.failures[0].code,
        CleanupErrorCode::StorageUnavailable
    );
    assert!(sentinel.exists());
    assert_eq!(
        scope
            .current()
            .expect("scope")
            .expect("account remains")
            .as_str(),
        "account-1"
    );
}

#[test]
fn native_account_scope_rejects_path_traversal_and_separator_input() {
    for invalid in ["", ".", "..", "../neighbor", "a/b", "a\\b"] {
        assert!(ValidatedAccountId::parse(invalid).is_err(), "{invalid}");
    }
    assert_eq!(
        ValidatedAccountId::parse("tenant:account_1.prod")
            .expect("valid account")
            .as_str(),
        "tenant:account_1.prod"
    );
    let directory = account_directory_name("CON.");
    assert!(directory.starts_with("acct-"));
    assert_eq!(directory.len(), 69);
    assert!(directory
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
}

/// A real `AppRuntimeState` over `app_root`, `scope`, `sessions`, and
/// `secrets`, composed the way `lib.rs` composes it.
pub(super) fn runtime_state(
    app_root: &std::path::Path,
    scope: Arc<CurrentAccountScope>,
    sessions: Arc<crate::auth::AuthSessionStore>,
    secrets: Arc<InMemorySecretStore>,
) -> AppRuntimeState {
    let journal = Arc::new(JournalSlot::default());
    let app_root = app_root.to_path_buf();
    let registry = FixedAccountResources::new(
        app_root.clone(),
        scope.clone(),
        Arc::new(SessionVault::default()),
        journal.clone(),
        secrets.clone(),
    )
    .registry()
    .expect("registry");
    let projects = Arc::new(crate::projects::ProjectsService::new(
        app_root.clone(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(crate::approvals::ApprovalsService::new(
        app_root.clone(),
        scope.clone(),
        secrets.clone(),
    ));
    AppRuntimeState::new(
        AccountCleanup::new(registry),
        scope,
        journal,
        sessions,
        app_root,
        secrets.clone(),
        secrets,
        projects,
        approvals,
    )
}

#[test]
fn runtime_health_is_degraded_while_account_persistence_is_unbound() {
    let directory = tempdir().expect("temporary app data");
    let state = runtime_state(
        directory.path(),
        Arc::new(CurrentAccountScope::default()),
        Arc::new(crate::auth::AuthSessionStore::default()),
        Arc::new(InMemorySecretStore::default()),
    );

    assert_eq!(
        state.health().status,
        xfalcon_contracts::app::AppHealthStatus::Degraded
    );
}

#[tokio::test]
async fn cleanup_purges_and_closes_the_owned_real_journal_before_removing_account_files() {
    let directory = tempdir().expect("temporary app data");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("valid account");
    let account = ValidatedAccountId::parse("account-1").expect("account");
    let account_root = directory
        .path()
        .join("accounts")
        .join(account_directory_name(account.as_str()));
    fs::create_dir_all(&account_root).expect("account root");
    let neighbor = directory
        .path()
        .join("accounts")
        .join(account_directory_name("account-2"));
    fs::create_dir_all(&neighbor).expect("neighbor root");
    fs::write(neighbor.join("journal.sqlite3"), b"neighbor").expect("neighbor journal");

    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(account.as_str(), [0xA5; 32])
        .expect("account key");
    let database = account_root.join("journal.sqlite3");
    let journal = Journal::open(&database, secrets.clone()).expect("real journal");
    journal
        .append(NewJournalEvent {
            account_id: account.as_str().to_owned(),
            event_id: "event-1".to_owned(),
            run_id: "run-1".to_owned(),
            sequence: 1,
            event_type: "run_event".to_owned(),
            state: "queued".to_owned(),
            payload: b"encrypted".to_vec(),
            created_at_ms: 1,
        })
        .expect("encrypted journal event");
    let slot = Arc::new(JournalSlot::default());
    slot.install(account, journal).expect("owned journal slot");
    let registry = FixedAccountResources::new(
        directory.path().to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        slot,
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(result.status, LocalDataClearStatus::Cleared);
    assert_base_registry_cleared(&result.cleared);
    assert!(!database.exists());
    assert!(neighbor.join("journal.sqlite3").exists());
}

#[derive(Default)]
struct RecordingProjectCancellation {
    calls: Mutex<Vec<String>>,
}

impl ProjectCancellation for RecordingProjectCancellation {
    fn cancel_all_projects(&self, account: &ValidatedAccountId) {
        self.calls
            .lock()
            .expect("recording lock")
            .push(account.as_str().to_owned());
    }
}

#[test]
fn project_resources_are_unregistered_without_an_explicit_project_cancellation() {
    let directory = tempdir().expect("app data");
    let registry = FixedAccountResources::new(
        directory.path().to_path_buf(),
        Arc::new(CurrentAccountScope::default()),
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        Arc::new(InMemorySecretStore::default()),
    )
    .registry()
    .expect("registry");

    assert!(!registry
        .resources()
        .contains(&LocalDataResource::ProjectData));
    assert!(!registry
        .resources()
        .contains(&LocalDataResource::DecryptedWorkingFiles));
}

#[tokio::test]
async fn project_cancellation_wiring_revokes_roots_and_removes_project_directories() {
    let directory = tempdir().expect("temporary app data");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("valid account");
    let account_root = directory
        .path()
        .join("accounts")
        .join(account_directory_name("account-1"));
    let project_data = account_root.join("project-data");
    let decrypted_working_files = account_root.join("decrypted-working-files");
    fs::create_dir_all(&project_data).expect("project data directory");
    fs::write(project_data.join("projects.sqlite3"), b"trust rows").expect("trust store file");
    fs::create_dir_all(project_data.join("index")).expect("project index directory");
    fs::create_dir_all(project_data.join("patch-tmp")).expect("patch temporary directory");
    fs::create_dir_all(&decrypted_working_files).expect("decrypted working files directory");
    fs::write(decrypted_working_files.join("draft.txt"), b"decrypted").expect("decrypted file");

    let cancellation = Arc::new(RecordingProjectCancellation::default());
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [7; 32]).expect("account key");
    let registry = FixedAccountResources::new(
        directory.path().to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .with_project_cancellation(cancellation.clone())
    .registry()
    .expect("registry");
    assert!(registry
        .resources()
        .contains(&LocalDataResource::ProjectData));
    assert!(registry
        .resources()
        .contains(&LocalDataResource::DecryptedWorkingFiles));

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert!(result.cleared.contains(&LocalDataResource::ProjectData));
    assert!(result
        .cleared
        .contains(&LocalDataResource::DecryptedWorkingFiles));
    assert_eq!(
        cancellation
            .calls
            .lock()
            .expect("recording lock")
            .as_slice(),
        ["account-1"]
    );
    assert!(!project_data.exists());
    assert!(!decrypted_working_files.exists());
}

#[tokio::test]
async fn clearing_local_data_forgets_the_accounts_sealed_mcp_server_credentials() {
    // `crate::mcp::credential_store` promises that clearing local data removes a
    // registered server's credential and leaves its definition in DA. That
    // promise is kept by the session-vault handler, which owns every store of
    // material sealed under the device keys it deletes — and if it did not, the
    // leftover directory would also fail the post-handler account-root prune.
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let account_root = root
        .join("accounts")
        .join(account_directory_name("tenant:account-1"));
    let mcp_data = account_root.join(crate::mcp::credential_store::MCP_DATA_DIR);
    fs::create_dir_all(&mcp_data).expect("mcp data directory");
    fs::write(
        mcp_data.join(crate::mcp::credential_store::CREDENTIAL_DB_FILE),
        b"sealed envelope",
    )
    .expect("sealed credential");

    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::with_material_for_runtime(vec![1, 2, 3, 4])),
        Arc::new(JournalSlot::default()),
        Arc::new(InMemorySecretStore::default()),
    )
    .with_credential_secrets(Arc::new(InMemorySecretStore::default()))
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert!(!mcp_data.exists());
    assert!(
        !account_root.exists(),
        "a leftover credential store would block the account-root prune"
    );
}

/// A saved report must not survive the account clear, and the clear must not
/// report itself as partial for having found one.
///
/// Both halves are the same defect. `LocalDataResource::ArtifactCache` had an
/// ordering slot and no handler, so `reports/` survived — and then the
/// post-handler `AccountRootPrune` found the account root non-empty, failed, and
/// `LocalDataClearResult::new` turned a clear that had actually worked into
/// `Partial` for every account that had ever saved a report.
#[tokio::test]
async fn clearing_local_data_removes_saved_reports_and_is_not_partial() {
    use crate::reports::REPORTS_DIR;

    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    let account_root = root
        .join("accounts")
        .join(account_directory_name("tenant:account-1"));

    // What earlier builds left behind: a sealed database and its WAL sidecar.
    // Nothing writes or reads them now, so the clear is the only thing that
    // touches them.
    let reports_dir = account_root.join(REPORTS_DIR);
    fs::create_dir_all(&reports_dir).expect("reports directory");
    fs::write(reports_dir.join("reports.sqlite3"), b"sealed rows").expect("legacy database");
    fs::write(reports_dir.join("reports.sqlite3-wal"), b"wal").expect("legacy wal");
    assert!(reports_dir.exists());

    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::with_material_for_runtime(vec![1, 2, 3, 4])),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry)
        .start()
        .expect("cleanup starts")
        .wait()
        .await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "a clear that removed everything must not report itself partial: {:?}",
        result.failures
    );
    assert!(result.failures.is_empty());
    assert!(
        result.cleared.contains(&LocalDataResource::ArtifactCache),
        "the resource that names these bytes must be reported: {:?}",
        result.cleared
    );
    assert!(!account_root.join(REPORTS_DIR).exists());
    assert!(
        !account_root.exists(),
        "the account root prune must find nothing left behind"
    );
}

/// Loose device-level leftovers go with the account clear, but the directories
/// the running app or a deliberate log still holds stay exactly where they are.
#[tokio::test]
async fn clear_sweeps_loose_device_leftovers_and_keeps_in_use_directories() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path().join("app");
    let temp = directory.path().join("temp");
    fs::create_dir_all(&root).expect("app root");
    fs::create_dir_all(&temp).expect("temp root");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [1; 32]).expect("account key");

    let removed_files = ["keepsakes.json", "settings.json"];
    let removed_dirs = [
        "desktop-updates",
        ".skills-staging",
        "staging",
        "upload-snapshots",
    ];
    let kept_dirs = ["logs", "EBWebView", "migrations", "audit"];
    for name in removed_files {
        fs::write(root.join(name), b"device").expect("device file");
    }
    for name in removed_dirs.iter().chain(kept_dirs.iter()) {
        fs::create_dir_all(root.join(name)).expect("device directory");
        fs::write(root.join(name).join("entry"), b"device").expect("device entry");
    }
    let snapshots = temp.join("xfalcon-upload-snapshots");
    fs::create_dir_all(&snapshots).expect("temporary snapshots");
    fs::write(snapshots.join("attachment"), b"attachment").expect("snapshot");

    let registry = FixedAccountResources::new(
        root.clone(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    // The sweep reports nothing on success: the resource list is unchanged.
    assert_base_registry_cleared(&result.cleared);
    for name in removed_files {
        assert!(!root.join(name).exists(), "{name} must be removed");
    }
    for name in removed_dirs {
        assert!(!root.join(name).exists(), "{name} must be removed");
    }
    assert!(
        snapshots.exists(),
        "unowned legacy temp data must be preserved"
    );
    for name in kept_dirs {
        assert!(
            root.join(name).join("entry").exists(),
            "{name} must be kept by the account clear"
        );
    }
}

/// A device item that cannot be removed safely is reported, not followed. A
/// symlink named like a device directory is unlinked by no one: the sweep
/// refuses it, the target survives, the account stays bound for a retry, and
/// the failure is attributed to `UploadTemporary`.
#[cfg(unix)]
#[tokio::test]
async fn device_sweep_refuses_a_linked_item_and_reports_it_as_upload_temporary() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary app data");
    let root = directory.path().join("app");
    let outside = directory.path().join("outside");
    let temp = directory.path().join("temp");
    fs::create_dir_all(&root).expect("app root");
    fs::create_dir_all(&outside).expect("outside directory");
    fs::create_dir_all(&temp).expect("temp root");
    fs::write(outside.join("keep"), b"not ours").expect("outside file");
    symlink(&outside, root.join("staging")).expect("linked device item");
    fs::write(root.join("settings.json"), b"device").expect("device file");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [1; 32]).expect("account key");

    let registry = FixedAccountResources::new(
        root.clone(),
        scope.clone(),
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(result.status, LocalDataClearStatus::Partial);
    assert_eq!(result.failures.len(), 1, "failures: {:?}", result.failures);
    assert_eq!(
        result.failures[0].resource,
        LocalDataResource::UploadTemporary
    );
    assert_eq!(
        result.failures[0].code,
        CleanupErrorCode::StorageUnavailable
    );
    assert!(!result.cleared.contains(&LocalDataResource::UploadTemporary));
    assert!(outside.join("keep").exists(), "link target must survive");
    assert!(
        fs::symlink_metadata(root.join("staging"))
            .expect("link still present")
            .file_type()
            .is_symlink(),
        "the refused link is left for a person to inspect"
    );
    assert!(
        !root.join("settings.json").exists(),
        "a refused item must not stop the rest of the sweep"
    );
    assert_eq!(
        scope
            .current()
            .expect("scope")
            .expect("account remains bound")
            .as_str(),
        "account-1"
    );
}

/// Per-account files no handler used to name: the chosen chat model, the custom
/// model settings, the Claude effort each chat keeps, the imported skill library
/// and its staging folder. Left
/// behind, they kept the account directory non-empty, so the clear reported
/// `Partial` for every account that had ever used them.
#[tokio::test]
async fn clear_removes_named_per_account_files_and_leaves_no_account_directory() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let account_root = root
        .join("accounts")
        .join(account_directory_name("tenant:account-1"));
    fs::create_dir_all(account_root.join("skills").join("y")).expect("skill folder");
    fs::write(
        account_root.join("skills").join("y").join("SKILL.md"),
        b"skill",
    )
    .expect("skill file");
    fs::create_dir_all(account_root.join(".skills-staging").join("z")).expect("staging folder");
    fs::write(account_root.join("chat-model.json"), b"{}").expect("model preference");
    fs::write(account_root.join("custom-model-settings.json"), b"{}").expect("custom settings");
    fs::write(account_root.join("claude-chat-effort.json"), b"{}").expect("chat effort levels");
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert_base_registry_cleared(&result.cleared);
    assert!(
        !account_root.exists(),
        "no per-account file may keep the account directory alive"
    );
}

/// The ChatGPT plan sign-in keeps sealed session files in the account's
/// `chatgpt-plan/` directory. The session-vault handler removes that directory
/// before the account keys, so a clear leaves no ciphertext behind and the
/// account directory is pruned.
#[tokio::test]
async fn clear_removes_the_chatgpt_plan_directory_and_leaves_no_account_directory() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let account_root = root
        .join("accounts")
        .join(account_directory_name("tenant:account-1"));
    let plan_dir = account_root.join(crate::providers::chatgpt_plan::store::PLAN_DIR);
    fs::create_dir_all(&plan_dir).expect("plan sign-in directory");
    fs::write(plan_dir.join("meta.json"), b"{}").expect("plan metadata");
    fs::write(plan_dir.join("session-x.bin"), b"sealed").expect("sealed session");
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert_base_registry_cleared(&result.cleared);
    assert!(!plan_dir.exists(), "sealed plan sign-in must be removed");
    assert!(
        !account_root.exists(),
        "no sealed plan file may keep the account directory alive"
    );
}

/// The plan-provider setting is device-wide, not owned by an account, so the
/// device sweep removes it with the other loose app-root items.
#[tokio::test]
async fn clear_sweeps_the_device_plan_provider_setting() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let setting = root.join(crate::plan_providers::settings::PLAN_SETTINGS_FILE);
    fs::write(&setting, b"{}").expect("device plan setting");
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert_base_registry_cleared(&result.cleared);
    assert!(!setting.exists(), "the device plan setting must be removed");
}

/// A plan-settings write that crashed leaves a temporary sibling at the app
/// root, under a random name. The device sweep removes it by prefix. A
/// directory with the same prefix is not a temporary this app wrote, so it stays.
#[tokio::test]
async fn clear_sweeps_a_crashed_plan_settings_temporary() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let prefix = crate::plan_providers::settings::TEMPORARY_PREFIX;
    let temporary = root.join(format!("{prefix}x7Kq2m"));
    fs::write(&temporary, b"{}").expect("crashed plan settings temporary");
    let directory_with_prefix = root.join(format!("{prefix}folder"));
    fs::create_dir_all(&directory_with_prefix).expect("directory with the prefix");
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert_base_registry_cleared(&result.cleared);
    assert!(
        !temporary.exists(),
        "a crashed plan settings temporary must be removed"
    );
    assert!(
        directory_with_prefix.is_dir(),
        "a directory is not a settings temporary and must be left alone"
    );
}

/// A linked settings temporary is refused like any other linked device item:
/// the link and the file it points at both survive, and the failure is reported.
#[cfg(unix)]
#[tokio::test]
async fn device_sweep_refuses_a_linked_plan_settings_temporary() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary app data");
    let root = directory.path().join("app");
    let outside = directory.path().join("outside");
    fs::create_dir_all(&root).expect("app root");
    fs::create_dir_all(&outside).expect("outside directory");
    fs::write(outside.join("keep"), b"not ours").expect("outside file");
    let link = root.join(format!(
        "{}x7Kq2m",
        crate::plan_providers::settings::TEMPORARY_PREFIX
    ));
    symlink(outside.join("keep"), &link).expect("linked temporary");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [1; 32]).expect("account key");

    let registry = FixedAccountResources::new(
        root.clone(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(result.status, LocalDataClearStatus::Partial);
    assert_eq!(result.failures.len(), 1, "failures: {:?}", result.failures);
    assert_eq!(
        result.failures[0].resource,
        LocalDataResource::UploadTemporary
    );
    assert_eq!(
        result.failures[0].code,
        CleanupErrorCode::StorageUnavailable
    );
    assert!(
        outside.join("keep").exists(),
        "the link target must survive"
    );
    assert!(
        fs::symlink_metadata(&link)
            .expect("link still present")
            .file_type()
            .is_symlink(),
        "the refused link is left for a person to inspect"
    );
}

/// Claude turn folders hold a prompt and a loopback token, so the device sweep
/// removes the `claude-turns/` root with the other loose app-root items.
#[tokio::test]
async fn clear_sweeps_the_claude_turn_folders() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path();
    let work_root = root.join(crate::plan_providers::WORK_ROOT_DIR);
    let turn = work_root.join("turn-x");
    fs::create_dir_all(&turn).expect("claude turn folder");
    fs::write(turn.join("prompt.txt"), b"prompt").expect("turn prompt");
    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native("tenant:account-1")
        .expect("valid native account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("tenant:account-1", [0xA5; 32])
        .expect("account key");
    let registry = FixedAccountResources::new(
        root.to_path_buf(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert_base_registry_cleared(&result.cleared);
    assert!(!turn.exists(), "the Claude turn folder must be removed");
    assert!(!work_root.exists(), "the claude-turns root must be removed");
}

/// Account clearing leaves legacy temporary snapshots alone because their
/// ownership cannot be proved from a shared system temp directory.
#[tokio::test]
async fn account_clear_preserves_unowned_legacy_temp_snapshots() {
    let build = || {
        let directory = tempdir().expect("temporary app data");
        let scope = Arc::new(CurrentAccountScope::default());
        scope.bind_native("account-1").expect("account");
        let secrets = Arc::new(InMemorySecretStore::default());
        secrets.put_key("account-1", [1; 32]).expect("account key");
        let resources = FixedAccountResources::new(
            directory.path().to_path_buf(),
            scope,
            Arc::new(SessionVault::default()),
            Arc::new(JournalSlot::default()),
            secrets,
        );
        (directory, resources.registry().expect("registry"))
    };

    let unnamed_temp = tempdir().expect("stand-in temp root");
    let unnamed_snapshots = unnamed_temp.path().join("xfalcon-upload-snapshots");
    fs::create_dir_all(&unnamed_snapshots).expect("planted snapshots");
    let (_app, registry) = build();
    let result = AccountCleanup::new(registry).clear().await;
    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert!(
        unnamed_snapshots.exists(),
        "without a named temp dir the sweep must not touch any temp folder"
    );
}

/// Standing tool approvals and the pre-account root skill library are device
/// leftovers the account clear must remove, while the directories the app still
/// owns beside them stay put.
#[tokio::test]
async fn clear_sweeps_standing_approvals_and_the_legacy_root_skill_library() {
    let directory = tempdir().expect("temporary app data");
    let root = directory.path().join("app");
    fs::create_dir_all(&root).expect("app root");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native("account-1").expect("account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [1; 32]).expect("account key");
    fs::write(
        root.join(crate::tools::standing::STANDING_APPROVALS_FILE),
        b"{}",
    )
    .expect("standing approvals");
    fs::create_dir_all(root.join("skills").join("legacy")).expect("legacy library");
    fs::write(
        root.join("skills").join("legacy").join("SKILL.md"),
        b"skill",
    )
    .expect("legacy skill");
    fs::create_dir_all(root.join("logs")).expect("logs");
    fs::write(root.join("logs").join("app.log"), b"log").expect("log file");

    let registry = FixedAccountResources::new(
        root.clone(),
        scope,
        Arc::new(SessionVault::default()),
        Arc::new(JournalSlot::default()),
        secrets,
    )
    .registry()
    .expect("registry");

    let result = AccountCleanup::new(registry).clear().await;

    assert_eq!(
        result.status,
        LocalDataClearStatus::Cleared,
        "failures: {:?}",
        result.failures
    );
    assert!(!root
        .join(crate::tools::standing::STANDING_APPROVALS_FILE)
        .exists());
    assert!(!root.join("skills").exists());
    assert!(root.join("logs").join("app.log").exists(), "logs are kept");
}
