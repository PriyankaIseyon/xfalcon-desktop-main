use std::sync::Arc;

use serde::Deserialize;
use tempfile::tempdir;

use crate::persistence::InMemorySecretStore;

use super::capture::{find_html_documents, has_external_reference};

const ACCOUNT: &str = "tenant:reports-account";

fn secrets() -> Arc<InMemorySecretStore> {
    let store = Arc::new(InMemorySecretStore::default());
    store.put_key(ACCOUNT, [0x5A; 32]).expect("account key");
    store
}

/// Cases shared with `src/features/chat/richContent/detect.test.ts`. Rust
/// reaches into the frontend tree here the same way
/// `crates/contracts/tests/contract_snapshots.rs` reaches into
/// `src/lib/generated/contracts.ts`.
const HTML_DOCUMENT_FIXTURES: &str =
    include_str!("../../../src/features/chat/richContent/htmlDocumentFixtures.json");

#[derive(Deserialize)]
struct FixtureFile {
    cases: Vec<FixtureCase>,
}

#[derive(Deserialize)]
struct FixtureCase {
    name: String,
    text: String,
    documents: Vec<String>,
}

#[test]
fn both_fence_scanners_agree_on_which_document_was_saved() {
    let fixtures: FixtureFile =
        serde_json::from_str(HTML_DOCUMENT_FIXTURES).expect("fixtures parse");
    assert!(
        fixtures.cases.len() >= 10,
        "the fixture set is the only thing pinning the two scanners together"
    );
    for case in &fixtures.cases {
        assert_eq!(
            find_html_documents(&case.text),
            case.documents,
            "case: {}",
            case.name
        );
    }
}

#[test]
fn an_external_script_or_stylesheet_is_warned_about_but_never_blocking() {
    assert!(has_external_reference(
        "<!doctype html><html><head><script src=\"https://cdn.jsdelivr.net/npm/chart.js\"></script></head></html>"
    ));
    assert!(has_external_reference(
        "<!doctype html><html><head><link rel=\"stylesheet\" href=\"https://unpkg.com/x.css\"></head></html>"
    ));
    // A self-contained document is the shape the prompt contract now demands.
    assert!(!has_external_reference(
        "<!doctype html><html><head><style>body{color:#111}</style><script>const a=1;</script></head></html>"
    ));
}

/// The library is DA's, and DA is reached only with a bearer, so a dead session
/// leaves nothing to read and no device copy to fall back on.
async fn assert_reads_are_refused(sessions: Arc<crate::auth::AuthSessionStore>) {
    let app_root = tempdir().expect("app root");
    let secrets = secrets();
    let accounts = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
    accounts.bind_native(ACCOUNT).expect("bind account");
    let projects = crate::projects::ProjectsService::new(
        app_root.path().to_path_buf(),
        accounts.clone(),
        secrets.clone(),
    );
    let service =
        super::ReportsService::new(app_root.path().to_path_buf(), accounts, secrets, sessions);

    let listed = service
        .list(&projects)
        .await
        .expect_err("no session, no library");
    assert_eq!(
        listed.code,
        xfalcon_contracts::reports::ReportErrorCode::StorageUnavailable
    );
    let id = "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01";
    let storage = xfalcon_contracts::reports::ReportErrorCode::StorageUnavailable;
    let opened = service
        .open(&crate::preview::PreviewDocumentStore::default(), id, None)
        .await
        .expect_err("no session, no report");
    assert_eq!(opened.code, storage);
    // The writes send the bearer too, so they stop at the same gate.
    assert_eq!(
        service.rename(id, "Name", None).await.unwrap_err().code,
        storage,
        "rename"
    );
    assert_eq!(
        service.set_pinned(id, true).await.unwrap_err().code,
        storage,
        "pin"
    );
    assert_eq!(
        service.delete(&projects, id).await.unwrap_err().code,
        storage,
        "delete"
    );
    assert_eq!(
        service.refresh(id).await.unwrap_err().code,
        storage,
        "refresh"
    );
    assert_eq!(
        service
            .set_scope(id, xfalcon_contracts::reports::ReportScope::Global)
            .await
            .unwrap_err()
            .code,
        storage,
        "share"
    );
    assert_eq!(
        service.set_hidden(id, true).await.unwrap_err().code,
        storage,
        "hide"
    );
}

fn session_for_account(
    expires_at_ms: i64,
    refresh_token: Option<&str>,
) -> Arc<crate::auth::AuthSessionStore> {
    let sessions = Arc::new(crate::auth::AuthSessionStore::default());
    sessions.set_authenticated(
        xfalcon_contracts::auth::SessionProjection::authenticated(
            xfalcon_contracts::auth::IdentityProjection::new(
                "reports-account",
                "tenant",
                "User",
                None,
            ),
            expires_at_ms,
        ),
        "access",
        refresh_token.map(str::to_owned),
    );
    sessions
}

/// What a refused renewal leaves behind: the account bound, the session gone.
#[tokio::test]
async fn report_operations_are_refused_once_the_session_behind_the_account_ends() {
    let sessions = session_for_account(
        chrono::Utc::now().timestamp_millis() + 3_600_000,
        Some("refresh"),
    );
    sessions.clear();

    assert_reads_are_refused(sessions).await;
}

#[tokio::test]
async fn report_operations_are_refused_to_a_dead_session() {
    assert_reads_are_refused(session_for_account(
        chrono::Utc::now().timestamp_millis() - 3_600_000,
        None,
    ))
    .await;
}

fn service_for(
    sessions: Arc<crate::auth::AuthSessionStore>,
) -> (tempfile::TempDir, super::ReportsService) {
    let app_root = tempdir().expect("app root");
    let accounts = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
    accounts.bind_native(ACCOUNT).expect("bind account");
    let service =
        super::ReportsService::new(app_root.path().to_path_buf(), accounts, secrets(), sessions);
    (app_root, service)
}

#[test]
fn a_read_that_outlived_its_account_is_refused() {
    let sessions = session_for_account(chrono::Utc::now().timestamp_millis() + 3_600_000, None);
    let (_root, service) = service_for(sessions.clone());
    let account = service.bound_account_id().expect("bound account");
    assert_eq!(account, ACCOUNT);

    service
        .still_signed_in_as(&account)
        .expect("the same account, still signed in");
    // Another account is bound by the time the answer comes back.
    assert!(service.still_signed_in_as("tenant:someone-else").is_err());
    // Or the session ended while the request was out.
    sessions.clear();
    assert!(service.still_signed_in_as(&account).is_err());
}

/// The library's project copy judges what it can judge on this device, in a fixed
/// order, before any request leaves it: the session, the name, then the project.
/// The codes tell the orders apart, so a reorder that reached for DA first would
/// answer `StorageUnavailable` and fail here.
#[tokio::test]
async fn a_library_project_save_judges_its_inputs_before_it_reaches_for_the_network() {
    use xfalcon_contracts::reports::ReportErrorCode;

    let app_root = tempdir().expect("app root");
    let secrets = secrets();
    let accounts = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
    accounts.bind_native(ACCOUNT).expect("bind account");
    let projects = crate::projects::ProjectsService::new(
        app_root.path().to_path_buf(),
        accounts.clone(),
        secrets.clone(),
    );
    let approvals = crate::approvals::ApprovalsService::new(
        app_root.path().to_path_buf(),
        accounts.clone(),
        secrets.clone(),
    );
    let report = "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01";
    let request = |project_id: &'static str, title: &'static str| super::LibraryProjectSave {
        report_id: report,
        page: None,
        project_id,
        title,
        allow_duplicate: false,
    };

    // No session: the gate answers first, whatever the inputs.
    let dead = session_for_account(chrono::Utc::now().timestamp_millis() - 3_600_000, None);
    let service = super::ReportsService::new(
        app_root.path().to_path_buf(),
        accounts.clone(),
        secrets.clone(),
        dead,
    );
    assert_eq!(
        service
            .save_library_report_to_project(&projects, &approvals, request("", ""))
            .await
            .unwrap_err()
            .code,
        ReportErrorCode::StorageUnavailable
    );

    // A live session: a blank name is refused as one, and so is a project id that
    // names nothing, each without a request to DA.
    let live = session_for_account(chrono::Utc::now().timestamp_millis() + 3_600_000, None);
    let service =
        super::ReportsService::new(app_root.path().to_path_buf(), accounts, secrets, live);
    assert_eq!(
        service
            .save_library_report_to_project(&projects, &approvals, request("not-a-project", "  "))
            .await
            .unwrap_err()
            .code,
        ReportErrorCode::InvalidTitle
    );
    assert_eq!(
        service
            .save_library_report_to_project(
                &projects,
                &approvals,
                request("not-a-project", "Weekly")
            )
            .await
            .unwrap_err()
            .code,
        ReportErrorCode::ProjectUnavailable
    );
}
