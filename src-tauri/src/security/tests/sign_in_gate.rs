//! The sign-in gate on commands that reach no server which would check the
//! bearer: each one must refuse once the session behind the bound account has
//! ended, even though the account scope is still bound.
//!
//! These drive the real `AppRuntimeState` and, for approvals, the real Tauri
//! command functions over a mock app, so what is proven is the path a webview
//! calling the command directly would take.

use std::sync::Arc;

use base64::Engine as _;
use tauri::Manager as _;
use tempfile::tempdir;
use xfalcon_contracts::auth::{IdentityProjection, SessionProjection};

use super::account_cleanup::runtime_state;
use crate::app::commands::AppRuntimeState;
use crate::approvals::{
    project_file_restore, project_file_revisions, project_save_file, ApprovalsService,
    ApprovalsServiceError,
};
use crate::auth::AuthSessionStore;
use crate::persistence::InMemorySecretStore;
use crate::projects::ProjectsServiceError;
use crate::security::account_cleanup::CurrentAccountScope;

const ACCOUNT: &str = "tenant-1:user-1";

/// Signed in as [`ACCOUNT`] with an hour left and a refresh token, bound
/// exactly as `auth_login` binds it.
fn signed_in() -> (Arc<AuthSessionStore>, Arc<CurrentAccountScope>) {
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            chrono::Utc::now().timestamp_millis() + 3_600_000,
        ),
        "access",
        Some("refresh".to_owned()),
    );
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native(ACCOUNT).expect("bind account");
    (sessions, scope)
}

fn account_secrets() -> Arc<InMemorySecretStore> {
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key(ACCOUNT, [0x22; 32]).expect("account key");
    secrets
}

/// Every project command reaches the service through `projects()`, so this is
/// the gate a webview calling them directly meets. `sessions.clear()` is what
/// a refused renewal leaves behind: the account still bound, no session.
#[test]
fn project_commands_are_refused_once_the_session_behind_the_account_ends() {
    let app_root = tempdir().expect("temporary app data");
    let (sessions, scope) = signed_in();
    let state = runtime_state(
        app_root.path(),
        scope.clone(),
        sessions.clone(),
        account_secrets(),
    );

    assert!(state.is_signed_in());
    let listed = state
        .projects()
        .expect("a signed-in account reaches its projects")
        .list()
        .expect("list the account's projects");
    assert!(listed.projects.is_empty());

    sessions.clear();

    assert!(scope.current().expect("scope").is_some());
    assert!(!state.is_signed_in());
    assert!(matches!(
        state.projects(),
        Err(ProjectsServiceError::NoActiveAccount)
    ));
}

/// A save is a write into the person's own files. It lands while the session
/// lives and is refused — with nothing written — once the session has ended,
/// through the command function the webview invokes.
#[tokio::test]
async fn a_project_save_is_refused_once_the_session_behind_the_account_ends() {
    let app_root = tempdir().expect("temporary app data");
    let project_root = tempdir().expect("project root");
    let file = project_root.path().join("notes.txt");
    std::fs::write(&file, "before").expect("seed file");
    let (sessions, scope) = signed_in();
    let secrets = account_secrets();
    let state = runtime_state(
        app_root.path(),
        scope.clone(),
        sessions.clone(),
        secrets.clone(),
    );
    let project = state
        .projects()
        .expect("signed in")
        .create(project_root.path().to_str().expect("utf-8 path"), None)
        .await
        .expect("trust the project");
    let project_id = project.id.as_str().to_owned();

    let app = tauri::test::mock_app();
    app.manage(state);
    app.manage(Arc::new(ApprovalsService::new(
        app_root.path().to_path_buf(),
        scope,
        secrets,
    )));
    let encode = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);

    project_save_file(
        app.state::<AppRuntimeState>(),
        app.state::<Arc<ApprovalsService>>(),
        project_id.clone(),
        "notes.txt".to_owned(),
        encode("after"),
    )
    .await
    .expect("a signed-in save lands");
    assert_eq!(std::fs::read_to_string(&file).expect("read"), "after");

    sessions.clear();

    let refused = project_save_file(
        app.state::<AppRuntimeState>(),
        app.state::<Arc<ApprovalsService>>(),
        project_id.clone(),
        "notes.txt".to_owned(),
        encode("forged"),
    )
    .await
    .expect_err("no session, no save");
    assert_eq!(refused, ApprovalsServiceError::NoActiveAccount.to_string());
    assert_eq!(std::fs::read_to_string(&file).expect("read"), "after");

    // The sibling commands on the same surface refuse the same way, before
    // their own arguments are looked at.
    let no_account = ApprovalsServiceError::NoActiveAccount.to_string();
    assert_eq!(
        project_file_revisions(
            app.state::<AppRuntimeState>(),
            app.state::<Arc<ApprovalsService>>(),
            project_id,
            "notes.txt".to_owned(),
        )
        .await
        .expect_err("no session, no revisions"),
        no_account
    );
    assert_eq!(
        project_file_restore(
            app.state::<AppRuntimeState>(),
            app.state::<Arc<ApprovalsService>>(),
            uuid::Uuid::new_v4().to_string(),
        )
        .await
        .expect_err("no session, no restore"),
        no_account
    );
}
