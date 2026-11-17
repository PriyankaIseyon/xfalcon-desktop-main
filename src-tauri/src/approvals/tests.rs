use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};

use async_trait::async_trait;
use tempfile::tempdir;
use uuid::Uuid;

use crate::chat::{domain::ChatError, ApprovalDaSync};
use crate::persistence::InMemorySecretStore;
use crate::persistence::SecretStore;
use crate::projects::{
    EditOperation, ProjectId, RelativeProjectPath, RevisionDiff, RevisionDiffLimit,
    ShadowBackupStoreError, TrustStore, MAX_REVISION_DIFF_FILE_BYTES, PROJECT_DATA_DIR,
};
use crate::security::account_cleanup::{account_directory_name, CurrentAccountScope};

use super::policy::ApprovalDecision;
use super::service::{ApprovalOutcome, ApprovalStore, ApprovalsService, ApprovalsServiceError};

const ACCOUNT_ID: &str = "account-1";

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

fn secrets() -> Arc<InMemorySecretStore> {
    let store = Arc::new(InMemorySecretStore::default());
    store.put_key(ACCOUNT_ID, [0x22; 32]).expect("account key");
    store
}

fn trust_project(app_root: &Path, secrets: Arc<dyn SecretStore>, project_root: &Path) -> ProjectId {
    use crate::security::account_cleanup::ValidatedAccountId;
    let db_path = app_root
        .join("accounts")
        .join(account_directory_name(ACCOUNT_ID))
        .join(PROJECT_DATA_DIR)
        .join("projects.sqlite3");
    let store = TrustStore::open(db_path, secrets).expect("open trust store");
    let account = ValidatedAccountId::parse(ACCOUNT_ID).expect("valid account id");
    let record = store
        .create(&account, project_root, None)
        .expect("trust project");
    record.project_id
}

fn service(app_root: &Path) -> (ApprovalsService, Arc<InMemorySecretStore>) {
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native(ACCOUNT_ID).expect("bind account");
    let secrets = secrets();
    let service = ApprovalsService::new(
        app_root.to_path_buf(),
        scope,
        secrets.clone() as Arc<dyn SecretStore>,
    );
    (service, secrets)
}

fn approval_store_path(app_root: &Path) -> PathBuf {
    app_root
        .join("accounts")
        .join(account_directory_name(ACCOUNT_ID))
        .join(PROJECT_DATA_DIR)
        .join("approvals.sqlite3")
}

/// Counts `project_mutation_approval_history` rows for `plan_id_hex` with
/// exactly `status`, by opening the account's `approvals.sqlite3` directly
/// (mirroring how `trust_project` above reconstructs `TrustStore`'s own
/// database path) -- proves [`ApprovalStore::insert_pending`]'s reset
/// archives the row it is about to overwrite rather than silently
/// discarding it (Task 6 review finding).
fn history_row_count(app_root: &Path, plan_id_hex: &str, status: &str) -> i64 {
    let db_path = approval_store_path(app_root);
    let connection = rusqlite::Connection::open(db_path).expect("open approvals db directly");
    connection
        .query_row(
            "SELECT COUNT(*) FROM project_mutation_approval_history
             WHERE plan_id = ?1 AND status = ?2",
            rusqlite::params![plan_id_hex, status],
            |row| row.get(0),
        )
        .expect("query approval history table")
}

fn approval_row_status(app_root: &Path, plan_id_hex: &str) -> String {
    let connection =
        rusqlite::Connection::open(approval_store_path(app_root)).expect("open approvals db");
    connection
        .query_row(
            "SELECT status FROM project_mutation_approvals WHERE plan_id = ?1",
            [plan_id_hex],
            |row| row.get(0),
        )
        .expect("approval status")
}

#[test]
fn concurrent_first_approval_store_opens_initialize_schema_once() {
    const OPENERS: usize = 16;
    let app_root = tempdir().expect("app root");
    let db_path = approval_store_path(app_root.path());
    let shared_secrets: Arc<dyn SecretStore> = secrets();
    let start = Arc::new(Barrier::new(OPENERS));

    let handles = (0..OPENERS)
        .map(|_| {
            let path = db_path.clone();
            let secrets = shared_secrets.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                ApprovalStore::open(path, secrets)
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle
            .join()
            .expect("store opener thread")
            .expect("parallel first open succeeds");
    }

    let connection = rusqlite::Connection::open(db_path).expect("open initialized store");
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read schema version");
    assert_eq!(version, crate::persistence::PROJECT_APPROVAL_SCHEMA_VERSION);
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN (
                 'project_mutation_approvals', 'project_mutation_approval_history'
             )",
            [],
            |row| row.get(0),
        )
        .expect("read initialized schema");
    assert_eq!(tables, 2, "both approval tables must exist after migration");
}

#[test]
fn concurrent_proposals_preserve_rows_while_a_store_reader_holds_a_snapshot() {
    const PROPOSERS: usize = 12;
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("existing.txt"),
                content: b"existing".to_vec(),
            }],
        )
        .expect("initialize approval store");

    let connection = rusqlite::Connection::open(approval_store_path(app_root.path()))
        .expect("open approval store reader");
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("read journal mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    let transaction = connection
        .unchecked_transaction()
        .expect("start reader snapshot");
    let initial_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM project_mutation_approvals",
            [],
            |row| row.get(0),
        )
        .expect("read initial approval count");

    let service = Arc::new(service);
    let start = Arc::new(Barrier::new(PROPOSERS + 1));
    let handles = (0..PROPOSERS)
        .map(|index| {
            let service = service.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                service.propose(
                    project_id,
                    vec![EditOperation::Create {
                        path: relative(&format!("parallel-{index}.txt")),
                        content: format!("proposal-{index}").into_bytes(),
                    }],
                )
            })
        })
        .collect::<Vec<_>>();
    start.wait();

    let mut plan_ids = Vec::with_capacity(PROPOSERS);
    for handle in handles {
        let plan = handle
            .join()
            .expect("proposal thread")
            .expect("concurrent proposal succeeds");
        plan_ids.push(plan.plan_id().to_hex());
    }

    let snapshot_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM project_mutation_approvals",
            [],
            |row| row.get(0),
        )
        .expect("read stable approval snapshot");
    assert_eq!(snapshot_count, initial_count);
    transaction.commit().expect("finish reader snapshot");

    let final_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM project_mutation_approvals",
            [],
            |row| row.get(0),
        )
        .expect("read all persisted proposals");
    assert_eq!(final_count, initial_count + PROPOSERS as i64);
    for plan_id in plan_ids {
        let status = connection
            .query_row(
                "SELECT status FROM project_mutation_approvals WHERE plan_id = ?1",
                [plan_id],
                |row| row.get::<_, String>(0),
            )
            .expect("read concurrent proposal row");
        assert_eq!(status, "pending");
    }
}

#[tokio::test]
async fn an_aborted_async_resolve_keeps_the_runtime_live_and_terminalizes_its_write() {
    let app_root = tempdir().unwrap();
    let project_root = tempdir().unwrap();
    let (mut service, secrets) = service(app_root.path());
    let project_id = trust_project(app_root.path(), secrets, project_root.path());
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"complete write".to_vec(),
            }],
        )
        .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let hook_entered = entered.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    service.resolution_hook = Some(Arc::new(move |_| {
        hook_entered.notify_one();
        wait.lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("test releases the actual blocking writer");
    }));
    let plan_id = plan.plan_id();
    let pending = tokio::spawn(async move {
        service
            .resolve(plan_id, ApprovalDecision::approve(false), None)
            .await
    });
    // This is a single-thread Tokio runtime. Reaching this while the writer is
    // blocked proves that disk/backup work does not hold its async thread.
    tokio::time::timeout(std::time::Duration::from_secs(3), entered.notified())
        .await
        .expect("runtime stays responsive");
    assert!(!pending.is_finished());
    let plan_id = plan_id.to_hex();
    assert_eq!(approval_row_status(app_root.path(), &plan_id), "claiming");
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if approval_row_status(app_root.path(), &plan_id) == "executed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned job persists its terminal status after caller abort");
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"complete write"
    );
}

#[tokio::test]
async fn a_queued_write_can_stop_without_claiming_or_changing_files() {
    let app_root = tempdir().unwrap();
    let project_root = tempdir().unwrap();
    fs::write(project_root.path().join("notes.txt"), b"original").unwrap();
    let (mut service, secrets) = service(app_root.path());
    let first_id = trust_project(app_root.path(), secrets.clone(), project_root.path());
    // The same folder can have more than one project record. The mutation
    // lock also covers that alias, and nested roots, without trusting ids.
    let second_id = trust_project(app_root.path(), secrets, project_root.path());
    let propose = |id, content: &[u8]| {
        service
            .propose(
                id,
                vec![EditOperation::Modify {
                    path: relative("notes.txt"),
                    content: content.to_vec(),
                }],
            )
            .unwrap()
    };
    let first = propose(first_id, b"first edit");
    let second = propose(second_id, b"second edit");
    let entered = Arc::new(tokio::sync::Notify::new());
    let hook_entered = entered.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    service.resolution_hook = Some(Arc::new(move |id| {
        if id == first_id {
            hook_entered.notify_one();
            wait.lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("release first plan");
        }
    }));
    let first_service = service.clone();
    let first_job = tokio::spawn(async move {
        first_service
            .resolve(first.plan_id(), ApprovalDecision::approve(false), None)
            .await
    });
    entered.notified().await;
    let cancel = tokio_util::sync::CancellationToken::new();
    let account = crate::security::account_cleanup::ValidatedAccountId::parse(ACCOUNT_ID).unwrap();
    let queued = service.apply_cancellable(&account, &second, &cancel);
    tokio::pin!(queued);
    tokio::select! {
        biased;
        result = &mut queued => panic!("second plan bypassed the write lock: {result:?}"),
        () = tokio::task::yield_now() => {}
    }
    cancel.cancel();
    assert!(matches!(
        queued.await,
        Err(ApprovalsServiceError::Cancelled)
    ));
    assert_eq!(
        approval_row_status(app_root.path(), &second.plan_id().to_hex()),
        "pending"
    );
    assert_eq!(
        fs::read(project_root.path().join("notes.txt")).unwrap(),
        b"original"
    );
    // Reading revision metadata remains available while a writer is held.
    assert!(service
        .file_revisions(first_id, &relative("notes.txt"))
        .unwrap()
        .is_empty());
    release.send(()).unwrap();
    assert!(matches!(
        first_job.await.unwrap(),
        Ok(ApprovalOutcome::Executed(_))
    ));
    assert!(matches!(
        service.apply(&account, &second).await,
        Err(ApprovalsServiceError::Execution(failure))
            if matches!(failure.error, crate::projects::WriterError::StaleBaseHash(_))
    ));
    assert_eq!(
        fs::read(project_root.path().join("notes.txt")).unwrap(),
        b"first edit"
    );
    assert_eq!(
        approval_row_status(app_root.path(), &second.plan_id().to_hex()),
        "failed"
    );
    let revisions = service
        .file_revisions(first_id, &relative("notes.txt"))
        .unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(
        service
            .load_revision(revisions[0].backup_id)
            .unwrap()
            .content,
        b"original"
    );
}

#[derive(Default)]
struct FakeDaSync {
    calls: Mutex<Vec<(Uuid, bool)>>,
}

#[async_trait]
impl ApprovalDaSync for FakeDaSync {
    async fn record_mutation_approval_decision(
        &self,
        tool_call_id: Uuid,
        approved: bool,
    ) -> Result<(), ChatError> {
        self.calls.lock().unwrap().push((tool_call_id, approved));
        Ok(())
    }
}

#[tokio::test]
async fn propose_persists_a_pending_plan_and_performs_no_mutation() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets.clone() as Arc<dyn SecretStore>,
        project_root.path(),
    );

    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"proposed content".to_vec(),
            }],
        )
        .expect("propose");

    assert!(!project_root.path().join("new.txt").exists());
    assert!(!plan.unified_diff().is_empty());
}

#[tokio::test]
async fn modifying_a_file_captures_the_exact_prior_bytes_before_the_write() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("notes.txt"), b"before\0\xff").expect("seed file");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let path = relative("notes.txt");

    service
        .save(
            project_id,
            vec![EditOperation::Modify {
                path: path.clone(),
                content: b"after".to_vec(),
            }],
        )
        .await
        .expect("save");

    let revisions = service
        .file_revisions(project_id, &path)
        .expect("list revisions");
    assert_eq!(revisions.len(), 1);
    let loaded = service
        .load_revision(revisions[0].backup_id)
        .expect("load revision");
    assert_eq!(loaded.content, b"before\0\xff");
    assert_eq!(
        fs::read(project_root.path().join("notes.txt")).unwrap(),
        b"after"
    );
}

#[tokio::test]
async fn a_persons_save_is_never_refused_for_a_placeholder_line() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("a.py"), b"one()\n").expect("seed file");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );

    // The file viewer, a restore and a saved report all come through `save`.
    service
        .save(
            project_id,
            vec![EditOperation::Modify {
                path: relative("a.py"),
                content: b"one()\n# ... rest unchanged\n".to_vec(),
            }],
        )
        .await
        .expect("a person's words are written as they typed them");

    assert_eq!(
        fs::read(project_root.path().join("a.py")).unwrap(),
        b"one()\n# ... rest unchanged\n"
    );
}

#[tokio::test]
async fn restoring_a_revision_uses_the_writer_and_captures_an_inverse_revision() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("notes.txt"), b"first").expect("seed file");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let path = relative("notes.txt");
    service
        .save(
            project_id,
            vec![EditOperation::Modify {
                path: path.clone(),
                content: b"second".to_vec(),
            }],
        )
        .await
        .expect("first save");
    let first_revision = service.file_revisions(project_id, &path).unwrap()[0].backup_id;

    service
        .restore_revision(first_revision)
        .await
        .expect("restore");

    assert_eq!(
        fs::read(project_root.path().join("notes.txt")).unwrap(),
        b"first"
    );
    let revisions = service.file_revisions(project_id, &path).unwrap();
    assert_eq!(revisions.len(), 2);
    let inverse = service.load_revision(revisions[0].backup_id).unwrap();
    assert_eq!(inverse.content, b"second");
}

#[tokio::test]
async fn a_deleted_file_is_kept_as_a_revision_and_restoring_it_brings_it_back() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::create_dir(project_root.path().join("reports")).expect("seed folder");
    fs::write(project_root.path().join("reports/q3.html"), b"<h1>Q3</h1>").expect("seed file");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let path = relative("reports/q3.html");

    service
        .save(
            project_id,
            vec![EditOperation::Delete { path: path.clone() }],
        )
        .await
        .expect("delete");
    assert!(!project_root.path().join("reports/q3.html").exists());

    let revisions = service.file_revisions(project_id, &path).unwrap();
    assert_eq!(revisions.len(), 1);
    service
        .restore_revision(revisions[0].backup_id)
        .await
        .expect("restore a deleted file");
    assert_eq!(
        fs::read(project_root.path().join("reports/q3.html")).unwrap(),
        b"<h1>Q3</h1>"
    );
}

#[tokio::test]
async fn a_rename_onto_an_existing_file_keeps_the_replaced_file() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("draft.md"), b"new").expect("seed source");
    fs::write(project_root.path().join("final.md"), b"old").expect("seed target");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );

    service
        .save(
            project_id,
            vec![EditOperation::Rename {
                from: relative("draft.md"),
                to: relative("final.md"),
            }],
        )
        .await
        .expect("rename");

    assert_eq!(
        fs::read(project_root.path().join("final.md")).unwrap(),
        b"new"
    );
    let revisions = service
        .file_revisions(project_id, &relative("final.md"))
        .unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(
        service
            .load_revision(revisions[0].backup_id)
            .unwrap()
            .content,
        b"old"
    );
    // A plain move destroys nothing, so the source keeps no revision.
    assert!(service
        .file_revisions(project_id, &relative("draft.md"))
        .unwrap()
        .is_empty());
}

/// A seeded project whose `path` was saved once over `before`, so its one
/// revision holds `before` and the file on disk holds `after`.
async fn project_with_one_revision(
    app_root: &Path,
    project_root: &Path,
    path: &str,
    before: &[u8],
    after: &[u8],
) -> (ApprovalsService, ProjectId, Uuid) {
    fs::write(project_root.join(path), before).expect("seed file");
    let (service, secrets) = service(app_root);
    let project_id = trust_project(app_root, secrets as Arc<dyn SecretStore>, project_root);
    service
        .save(
            project_id,
            vec![EditOperation::Modify {
                path: relative(path),
                content: after.to_vec(),
            }],
        )
        .await
        .expect("save");
    let backup_id = service.file_revisions(project_id, &relative(path)).unwrap()[0].backup_id;
    (service, project_id, backup_id)
}

#[tokio::test]
async fn a_revision_diff_shows_what_changed_since_that_revision() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "notes.txt",
        b"alpha\nbeta\n",
        b"alpha\ngamma\n",
    )
    .await;

    let diff = service
        .revision_diff(project_id, &relative("notes.txt"), backup_id, None)
        .expect("diff");

    let RevisionDiff::Changes {
        unified_diff,
        added_lines,
        removed_lines,
    } = diff
    else {
        panic!("expected changes, got {diff:?}");
    };
    assert_eq!((added_lines, removed_lines), (1, 1));
    assert!(unified_diff.contains("\n-beta\n"));
    assert!(unified_diff.contains("\n+gamma\n"));
}

#[tokio::test]
async fn a_revision_against_the_next_one_shows_only_that_edit() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, project_id, _) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "notes.txt",
        b"one\n",
        b"one\ntwo\n",
    )
    .await;
    service
        .save(
            project_id,
            vec![EditOperation::Modify {
                path: relative("notes.txt"),
                content: b"one\ntwo\nthree\n".to_vec(),
            }],
        )
        .await
        .expect("second save");
    // Newest first: [before the second save, before the first save].
    let history = service
        .file_revisions(project_id, &relative("notes.txt"))
        .unwrap();
    let (next, first) = (history[0].backup_id, history[1].backup_id);

    let edit = service
        .revision_diff(project_id, &relative("notes.txt"), first, Some(next))
        .expect("diff");
    let since = service
        .revision_diff(project_id, &relative("notes.txt"), first, None)
        .expect("diff");

    let RevisionDiff::Changes {
        unified_diff,
        added_lines: 1,
        removed_lines: 0,
    } = edit
    else {
        panic!("expected the first edit alone, got {edit:?}");
    };
    assert!(unified_diff.contains("\n+two\n"));
    assert!(!unified_diff.contains("three"));
    assert!(matches!(
        since,
        RevisionDiff::Changes {
            added_lines: 2,
            removed_lines: 0,
            ..
        }
    ));
}

#[tokio::test]
async fn a_comparison_revision_from_another_file_is_not_found() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("other.txt"), b"other\n").expect("seed other");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "notes.txt",
        b"first\n",
        b"second\n",
    )
    .await;
    service
        .save(
            project_id,
            vec![EditOperation::Modify {
                path: relative("other.txt"),
                content: b"changed\n".to_vec(),
            }],
        )
        .await
        .expect("save other");
    let foreign = service
        .file_revisions(project_id, &relative("other.txt"))
        .unwrap()[0]
        .backup_id;

    let outcome =
        service.revision_diff(project_id, &relative("notes.txt"), backup_id, Some(foreign));
    assert!(
        matches!(
            outcome,
            Err(ApprovalsServiceError::Backup(
                ShadowBackupStoreError::NotFound
            ))
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_revision_the_file_has_returned_to_is_unchanged() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "notes.txt",
        b"first\n",
        b"second\n",
    )
    .await;
    fs::write(project_root.path().join("notes.txt"), b"first\n").expect("revert by hand");

    assert_eq!(
        service
            .revision_diff(project_id, &relative("notes.txt"), backup_id, None)
            .expect("diff"),
        RevisionDiff::Unchanged
    );
}

#[tokio::test]
async fn a_deleted_file_diffs_against_nothing() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "notes.txt",
        b"kept\nlines\n",
        b"other\n",
    )
    .await;
    fs::remove_file(project_root.path().join("notes.txt")).expect("delete");

    let diff = service
        .revision_diff(project_id, &relative("notes.txt"), backup_id, None)
        .expect("diff");
    assert!(
        matches!(
            diff,
            RevisionDiff::Changes {
                added_lines: 0,
                removed_lines: 2,
                ..
            }
        ),
        "{diff:?}"
    );
}

#[tokio::test]
async fn a_revision_over_the_size_limit_is_refused_before_it_is_diffed() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let large = vec![b'a'; usize::try_from(MAX_REVISION_DIFF_FILE_BYTES).unwrap() + 1];
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "large.txt",
        &large,
        b"small\n",
    )
    .await;

    assert_eq!(
        service
            .revision_diff(project_id, &relative("large.txt"), backup_id, None)
            .expect("diff"),
        RevisionDiff::TooLarge(RevisionDiffLimit::FileSize)
    );
}

#[tokio::test]
async fn a_current_file_over_the_size_limit_is_refused() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "grown.txt",
        b"small\n",
        b"also small\n",
    )
    .await;
    let large = vec![b'a'; usize::try_from(MAX_REVISION_DIFF_FILE_BYTES).unwrap() + 1];
    fs::write(project_root.path().join("grown.txt"), large).expect("grow");

    assert_eq!(
        service
            .revision_diff(project_id, &relative("grown.txt"), backup_id, None)
            .expect("diff"),
        RevisionDiff::TooLarge(RevisionDiffLimit::FileSize)
    );
}

#[tokio::test]
async fn a_sensitive_path_is_withheld_without_reading_either_side() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        ".env",
        b"TOKEN=old\n",
        b"TOKEN=new\n",
    )
    .await;

    assert!(matches!(
        service
            .revision_diff(project_id, &relative(".env"), backup_id, None)
            .expect("diff"),
        RevisionDiff::Sensitive(_)
    ));
}

#[tokio::test]
async fn a_revision_of_another_file_is_not_found_for_this_one() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("other.txt"), b"other\n").expect("seed other");
    let (service, project_id, backup_id) = project_with_one_revision(
        app_root.path(),
        project_root.path(),
        "notes.txt",
        b"first\n",
        b"second\n",
    )
    .await;

    let outcome = service.revision_diff(project_id, &relative("other.txt"), backup_id, None);
    assert!(
        matches!(
            outcome,
            Err(ApprovalsServiceError::Backup(
                ShadowBackupStoreError::NotFound
            ))
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn denying_a_plan_never_mutates_the_filesystem() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"proposed content".to_vec(),
            }],
        )
        .expect("propose");

    let outcome = service
        .resolve(plan.plan_id(), ApprovalDecision::deny(), None)
        .await
        .expect("deny resolves without error");

    assert!(matches!(outcome, ApprovalOutcome::Denied));
    assert!(!project_root.path().join("new.txt").exists());
}

#[tokio::test]
async fn approving_a_standard_risk_plan_without_second_confirmation_executes_it() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");

    let outcome = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect("approve resolves");

    assert!(matches!(
        outcome,
        ApprovalOutcome::Executed(execution) if execution.operations_completed == 1
    ));
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"content"
    );
}

#[tokio::test]
async fn approving_a_plan_requiring_second_confirmation_without_it_is_rejected() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("gone.txt"), b"bye").expect("seed");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Delete {
                path: relative("gone.txt"),
            }],
        )
        .expect("propose delete (requires second confirmation)");

    let error = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect_err("must reject without second confirmation");

    assert!(matches!(
        error,
        ApprovalsServiceError::Policy(
            crate::approvals::ApprovalPolicyError::SecondConfirmationRequired
        )
    ));
    assert!(project_root.path().join("gone.txt").exists());
}

#[tokio::test]
async fn approving_a_plan_requiring_second_confirmation_with_it_executes_it() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("gone.txt"), b"bye").expect("seed");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Delete {
                path: relative("gone.txt"),
            }],
        )
        .expect("propose delete");

    let outcome = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(true), None)
        .await
        .expect("approve with second confirmation resolves");

    assert!(matches!(outcome, ApprovalOutcome::Executed(_)));
    assert!(!project_root.path().join("gone.txt").exists());
}

#[tokio::test]
async fn resolving_an_already_resolved_plan_is_rejected() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");
    service
        .resolve(plan.plan_id(), ApprovalDecision::deny(), None)
        .await
        .expect("first resolve (deny)");

    let error = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect_err("a second resolve against an already-resolved plan must be rejected");
    assert!(matches!(error, ApprovalsServiceError::PlanNotFound));
}

#[tokio::test]
async fn resolving_an_unknown_plan_id_is_rejected() {
    let app_root = tempdir().expect("app root");
    let (service, _secrets) = service(app_root.path());

    let bogus_plan_id =
        crate::projects::PlanId::from_hex(&"0".repeat(64)).expect("well-formed hex");
    let error = service
        .resolve(bogus_plan_id, ApprovalDecision::approve(false), None)
        .await
        .expect_err("an unknown plan id must be rejected");
    assert!(matches!(error, ApprovalsServiceError::PlanNotFound));
}

#[tokio::test]
async fn a_file_changed_since_propose_is_rejected_at_resolve_time_and_marks_the_row_failed() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("doc.txt"), b"v1").expect("seed");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Modify {
                path: relative("doc.txt"),
                content: b"v2".to_vec(),
            }],
        )
        .expect("propose");

    fs::write(project_root.path().join("doc.txt"), b"raced").expect("out-of-band change");

    let error = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect_err("a stale base hash must be rejected");
    assert!(matches!(
        error,
        ApprovalsServiceError::Execution(failure)
        if matches!(failure.error, crate::projects::WriterError::StaleBaseHash(_))
    ));
    assert_eq!(
        fs::read(project_root.path().join("doc.txt")).unwrap(),
        b"raced"
    );

    // The row is now `failed`, not `pending` -- a second resolve attempt
    // must not be able to retry it silently.
    let retry = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect_err("a failed plan is no longer pending");
    assert!(matches!(retry, ApprovalsServiceError::PlanNotFound));
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_escape_planted_after_propose_is_rejected_regardless_of_second_confirmation() {
    use std::os::unix::fs::symlink;

    use crate::projects::ContainmentError;

    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let outside = tempdir().expect("outside target");
    fs::write(outside.path().join("secret.txt"), b"outside").expect("outside file");
    fs::write(project_root.path().join("escape.txt"), b"placeholder").expect("placeholder");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Modify {
                path: relative("escape.txt"),
                content: b"attacker-controlled".to_vec(),
            }],
        )
        .expect("propose against the placeholder file");

    fs::remove_file(project_root.path().join("escape.txt")).expect("remove placeholder");
    symlink(
        outside.path().join("secret.txt"),
        project_root.path().join("escape.txt"),
    )
    .expect("plant an escaping symlink");

    let error = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(true), None)
        .await
        .expect_err("a symlink escape must be rejected even with second confirmation given");
    assert!(matches!(
        error,
        ApprovalsServiceError::Execution(failure)
        if matches!(
            failure.error,
            crate::projects::WriterError::Containment(ContainmentError::LinkOrReparse)
        )
    ));
    assert_eq!(
        fs::read(outside.path().join("secret.txt")).unwrap(),
        b"outside"
    );
}

#[tokio::test]
async fn resolving_with_a_wired_da_sync_channel_and_a_tool_call_id_calls_it() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let da_sync = Arc::new(FakeDaSync::default());
    let service = service.with_da_sync(da_sync.clone());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");
    let tool_call_id = Uuid::new_v4();

    service
        .resolve(
            plan.plan_id(),
            ApprovalDecision::approve(false),
            Some(tool_call_id),
        )
        .await
        .expect("resolve");

    assert_eq!(*da_sync.calls.lock().unwrap(), vec![(tool_call_id, true)]);
}

struct RecordingAuditSink {
    events: Mutex<Vec<crate::audit::AuditEvent>>,
}

#[async_trait]
impl crate::audit::AuditSink for RecordingAuditSink {
    async fn publish(
        &self,
        event: &crate::audit::AuditEvent,
    ) -> crate::domain::error::AppResult<crate::audit::AuditPublishStatus> {
        self.events.lock().unwrap().push(event.clone());
        Ok(crate::audit::AuditPublishStatus::Published)
    }
}

fn audit_queue_for_test(app_root: &Path) -> Arc<crate::audit::AuditQueue> {
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT_ID, [0x88; 32])
        .expect("account key");
    let journal = Arc::new(
        crate::persistence::Journal::open(app_root.join("audit.sqlite3"), secrets)
            .expect("open audit journal"),
    );
    Arc::new(crate::audit::AuditQueue::new(ACCOUNT_ID, 16, journal).expect("audit queue"))
}

/// Closes the review finding that `Task 7` added zero `AuditSink::publish`
/// calls anywhere in `approvals::service` despite the plan's own Step 3
/// explicitly naming "approval resolution" and "mutation outcome" as two of
/// the four required audit categories. `resolve` is the only production
/// code path that can ever call `crate::projects::execute` (see this
/// module's own security contract), so auditing here covers both named
/// categories in one place: a denial publishes once, and a successful
/// execution publishes once more (not the same event duplicated).
#[tokio::test]
async fn resolving_a_plan_publishes_real_audit_events_for_both_denial_and_execution() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let sink = Arc::new(RecordingAuditSink {
        events: Mutex::new(Vec::new()),
    });
    let service = service.with_audit(crate::audit::AuditPublisher::new(
        "actor-1",
        "tenant-1",
        "device-1",
        sink.clone(),
        audit_queue_for_test(app_root.path()),
    ));
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );

    let denied_plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("denied.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");
    service
        .resolve(denied_plan.plan_id(), ApprovalDecision::deny(), None)
        .await
        .expect("deny resolves");

    let executed_plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("executed.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");
    service
        .resolve(
            executed_plan.plan_id(),
            ApprovalDecision::approve(false),
            None,
        )
        .await
        .expect("approve resolves");

    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 2, "exactly one audit event per resolution");
    assert_eq!(events[0].action(), crate::audit::AuditAction::ToolDeny);
    assert_eq!(events[0].outcome(), crate::audit::AuditOutcome::Denied);
    assert_eq!(events[0].resource_id(), denied_plan.plan_id().to_hex());
    assert_eq!(events[1].action(), crate::audit::AuditAction::ToolExecute);
    assert_eq!(events[1].outcome(), crate::audit::AuditOutcome::Success);
    assert_eq!(events[1].resource_id(), executed_plan.plan_id().to_hex());
}

#[tokio::test]
async fn reproposing_identical_content_after_the_first_proposal_was_denied_is_resolvable() {
    // `PlanId` is purely content-addressed (`crate::projects::patch::compute_plan_id`),
    // so a second `propose()` call for byte-identical operations against
    // unchanged content collides onto the exact same plan id as the first,
    // already-denied proposal. Without `insert_pending`'s upsert-reset
    // (Task 6 review finding), the stale `denied` row would be left
    // untouched while `propose()` still reported success, permanently
    // blocking this content from ever being approved.
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let operations = || {
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"content".to_vec(),
        }]
    };

    let first_plan = service
        .propose(project_id, operations())
        .expect("first propose");
    service
        .resolve(first_plan.plan_id(), ApprovalDecision::deny(), None)
        .await
        .expect("deny the first proposal");

    let second_plan = service
        .propose(project_id, operations())
        .expect("second propose");
    assert_eq!(
        second_plan.plan_id(),
        first_plan.plan_id(),
        "identical operations against unchanged content must collide onto the same plan id"
    );
    assert_eq!(
        history_row_count(app_root.path(), &first_plan.plan_id().to_hex(), "denied"),
        1,
        "the first proposal's denial must be archived to history before the reset overwrites it"
    );

    let outcome = service
        .resolve(second_plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect(
            "the second proposal's plan must actually be resolvable, not stuck behind the first denial",
        );

    assert!(matches!(outcome, ApprovalOutcome::Executed(_)));
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"content"
    );
}

#[tokio::test]
async fn a_crash_between_execute_succeeding_and_the_status_update_landing_is_reconciled() {
    // Simulates `resolve()` having already called `execute()` successfully
    // and then crashing before it could persist the row's terminal status
    // -- the row is still `pending` in the store, but the mutation already
    // fully landed on disk. A naive retry would re-attempt `execute()`,
    // which would fail (the target now already exists) and be misreported
    // as a fresh `Failed`, even though the plan already succeeded.
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");

    let approval = crate::projects::ApprovalProof::new(plan.plan_id(), false);
    crate::projects::execute(project_root.path(), &plan, &approval, 0)
        .expect("simulate resolve()'s own execute() having already succeeded");
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"content"
    );

    let outcome = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect("a retry against an already-applied plan must reconcile, not fail");

    assert!(matches!(
        outcome,
        ApprovalOutcome::Executed(execution)
        if execution.operations_completed == 1 && execution.operations_total == 1
    ));
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"content"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_resolve_calls_against_the_same_plan_id_only_one_executes() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");
    let plan_id = plan.plan_id();

    let service = Arc::new(service);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .resolve(plan_id, ApprovalDecision::approve(false), None)
                .await
        }));
    }

    let mut executed = 0u32;
    let mut not_found = 0u32;
    for handle in handles {
        match handle.await.expect("resolve task must not panic") {
            Ok(ApprovalOutcome::Executed(_)) => executed += 1,
            Err(ApprovalsServiceError::PlanNotFound) => not_found += 1,
            other => panic!("unexpected concurrent resolve() outcome: {other:?}"),
        }
    }

    assert_eq!(
        executed, 1,
        "exactly one concurrent resolve() call must execute the plan"
    );
    assert_eq!(
        not_found, 1,
        "the losing concurrent resolve() call must observe the claim already taken, not re-execute"
    );
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"content"
    );
}

#[tokio::test]
async fn resolving_without_a_tool_call_id_never_calls_the_da_sync_channel() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let da_sync = Arc::new(FakeDaSync::default());
    let service = service.with_da_sync(da_sync.clone());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let plan = service
        .propose(
            project_id,
            vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
        )
        .expect("propose");

    service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect("resolve");

    assert!(da_sync.calls.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------
// Direct application for content already visible in the file viewer
// ---------------------------------------------------------------------

mod direct_file_viewer_writes {
    use super::*;
    use crate::security::account_cleanup::ValidatedAccountId;

    fn bound_service(app_root: &Path) -> ApprovalsService {
        let scope = Arc::new(CurrentAccountScope::default());
        scope.bind_native(ACCOUNT_ID).expect("bind account");
        ApprovalsService::new(
            app_root.to_path_buf(),
            scope,
            secrets() as Arc<dyn SecretStore>,
        )
    }

    fn account() -> ValidatedAccountId {
        ValidatedAccountId::parse(ACCOUNT_ID).expect("valid account id")
    }

    #[tokio::test]
    async fn a_plan_lands_on_disk_without_anyone_being_asked() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let project_id = trust_project(
            app_root.path(),
            secrets() as Arc<dyn SecretStore>,
            project_root.path(),
        );
        let service = bound_service(app_root.path());
        let plan = service
            .propose(
                project_id,
                vec![EditOperation::Create {
                    path: relative("written.txt"),
                    content: b"applied".to_vec(),
                }],
            )
            .expect("propose");

        let outcome = service.apply(&account(), &plan).await.expect("apply");

        assert!(matches!(outcome, ApprovalOutcome::Executed(_)));
        assert_eq!(
            fs::read(project_root.path().join("written.txt")).unwrap(),
            b"applied"
        );
    }

    /// A file-viewer save already shows the exact bytes the person authored.
    /// It still has to satisfy the immutable plan's risk classification.
    #[tokio::test]
    async fn a_delete_applies_too() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        fs::write(project_root.path().join("doomed.txt"), b"here").expect("seed");
        let project_id = trust_project(
            app_root.path(),
            secrets() as Arc<dyn SecretStore>,
            project_root.path(),
        );
        let service = bound_service(app_root.path());
        let plan = service
            .propose(
                project_id,
                vec![EditOperation::Delete {
                    path: relative("doomed.txt"),
                }],
            )
            .expect("propose");
        assert!(plan.requires_second_confirmation());

        let outcome = service.apply(&account(), &plan).await.expect("apply");

        assert!(matches!(outcome, ApprovalOutcome::Executed(_)));
        assert!(!project_root.path().join("doomed.txt").exists());
    }

    /// The single-flight claim still holds for the direct-save helper: a plan
    /// already applied cannot be applied a second time.
    #[tokio::test]
    async fn the_same_plan_cannot_be_applied_twice() {
        let app_root = tempdir().expect("app root");
        let project_root = tempdir().expect("project root");
        let project_id = trust_project(
            app_root.path(),
            secrets() as Arc<dyn SecretStore>,
            project_root.path(),
        );
        let service = bound_service(app_root.path());
        let plan = service
            .propose(
                project_id,
                vec![EditOperation::Create {
                    path: relative("once.txt"),
                    content: b"once".to_vec(),
                }],
            )
            .expect("propose");
        service.apply(&account(), &plan).await.expect("apply");

        let error = service
            .apply(&account(), &plan)
            .await
            .expect_err("an already-applied plan is no longer pending");

        assert!(matches!(error, ApprovalsServiceError::PlanNotFound));
    }
}

#[tokio::test]
async fn a_plan_whose_claim_was_left_behind_by_a_crash_can_be_proposed_and_applied_again() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let operations = || {
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"content".to_vec(),
        }]
    };
    let plan = service.propose(project_id, operations()).expect("propose");

    // The process died after claiming the row and before finishing: the row is
    // `claiming`, and its plan has long since expired.
    let db_path = app_root
        .path()
        .join("accounts")
        .join(account_directory_name(ACCOUNT_ID))
        .join(PROJECT_DATA_DIR)
        .join("approvals.sqlite3");
    let connection = rusqlite::Connection::open(db_path).expect("open approvals db");
    connection
        .execute(
            "UPDATE project_mutation_approvals SET status = 'claiming', expires_at_ms = 1
             WHERE plan_id = ?1",
            rusqlite::params![plan.plan_id().to_hex()],
        )
        .expect("simulate the crash");

    let again = service
        .propose(project_id, operations())
        .expect("propose again");
    let outcome = service
        .resolve(again.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect("the stale claim no longer blocks this plan");

    assert!(matches!(outcome, ApprovalOutcome::Executed(_)));
    assert_eq!(
        fs::read(project_root.path().join("new.txt")).unwrap(),
        b"content"
    );
}

#[tokio::test]
async fn a_claim_still_inside_its_plans_lifetime_is_never_taken_over() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    let operations = || {
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"content".to_vec(),
        }]
    };
    let plan = service.propose(project_id, operations()).expect("propose");
    let db_path = app_root
        .path()
        .join("accounts")
        .join(account_directory_name(ACCOUNT_ID))
        .join(PROJECT_DATA_DIR)
        .join("approvals.sqlite3");
    rusqlite::Connection::open(db_path)
        .expect("open approvals db")
        .execute(
            "UPDATE project_mutation_approvals SET status = 'claiming' WHERE plan_id = ?1",
            rusqlite::params![plan.plan_id().to_hex()],
        )
        .expect("a resolve is in flight");

    service
        .propose(project_id, operations())
        .expect("propose again");

    // A resolve that is genuinely running keeps its claim.
    let error = service
        .resolve(plan.plan_id(), ApprovalDecision::approve(false), None)
        .await
        .expect_err("the in-flight claim was not reset");
    assert!(matches!(error, ApprovalsServiceError::PlanNotFound));
}

/// Diagnostic, not a gate: how long listing a file's revisions holds the
/// caller. Each revision is about 2 MiB of incompressible bytes, so the
/// decrypt cost is realistic rather than flattered by compression. Run with
/// `cargo test --lib file_revisions_timing -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn file_revisions_timing_with_twelve_large_revisions() {
    const REVISIONS: u64 = 12;
    const REVISION_BYTES: usize = 2 * 1024 * 1024;
    let body = |seed: u64| -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..REVISION_BYTES)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    };
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let path = "large.bin";
    fs::write(project_root.path().join(path), body(0)).expect("seed file");
    let (service, secrets) = service(app_root.path());
    let project_id = trust_project(
        app_root.path(),
        secrets as Arc<dyn SecretStore>,
        project_root.path(),
    );
    // Each save keeps the version it replaces, so 12 saves leave 12 revisions.
    for seed in 1..=REVISIONS {
        service
            .save(
                project_id,
                vec![EditOperation::Modify {
                    path: relative(path),
                    content: body(seed),
                }],
            )
            .await
            .expect("save");
    }

    let started = std::time::Instant::now();
    let revisions = service
        .file_revisions(project_id, &relative(path))
        .expect("list revisions");
    let elapsed = started.elapsed();

    assert_eq!(revisions.len(), REVISIONS as usize);
    eprintln!(
        "file_revisions: {} revisions of {} bytes each took {:?}",
        revisions.len(),
        REVISION_BYTES,
        elapsed
    );
}
