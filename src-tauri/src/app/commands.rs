use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_dialog::DialogExt as _;
use xfalcon_contracts::app::{
    AppBootstrap, AppHealth, AppHealthStatus, LocalDataClearResult, Platform,
};
use xfalcon_contracts::projects::{
    FolderGrantView, ProjectId as WireProjectId, ProjectListResult, ProjectView,
};

use crate::audit::{emit, AuditAction, AuditOutcome, AuditResourceType};
use crate::auth::{AuthService, AuthSessionStore};
use crate::chat::ChatService;
use crate::clients::da::model_catalog::ModelCatalogCache;
use crate::credentials::CredentialRuntime;
use crate::datasets::Staging;
use crate::path_grants::{GrantKind, PathGrants, EXPIRED_CHOICE};
use crate::plan_providers::PlanProviders;
use crate::projects::{ProjectsService, ProjectsServiceError};
use crate::reports::ReportsService;
use crate::security::account_cleanup::{AccountCleanup, CurrentAccountScope, JournalSlot};
use crate::security::device_purge::{self, ResetStartError};
use crate::uploads::service::UploadService;
use crate::workspace::WorkspaceService;

pub struct AppRuntimeState {
    cleanup: AccountCleanup,
    accounts: Arc<CurrentAccountScope>,
    journal: Arc<JournalSlot>,
    sessions: Arc<AuthSessionStore>,
    auth: AuthService,
    workspace: WorkspaceService,
    chat: ChatService,
    model_catalog: Arc<ModelCatalogCache>,
    plan_providers: Arc<PlanProviders>,
    credentials: Arc<CredentialRuntime>,
    projects: Arc<ProjectsService>,
    office_previews: crate::projects::OfficePreviewJobs,
    reports: Arc<ReportsService>,
}

impl AppRuntimeState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cleanup: AccountCleanup,
        accounts: Arc<CurrentAccountScope>,
        journal: Arc<JournalSlot>,
        auth_sessions: Arc<AuthSessionStore>,
        app_local_data: std::path::PathBuf,
        secrets: Arc<dyn crate::persistence::SecretStore>,
        credential_secrets: Arc<dyn crate::persistence::SecretStore>,
        projects: Arc<ProjectsService>,
        approvals: Arc<crate::approvals::ApprovalsService>,
    ) -> Self {
        let credentials = Arc::new(CredentialRuntime::new(
            auth_sessions.clone(),
            credential_secrets,
        ));
        let reports = Arc::new(ReportsService::new(
            app_local_data.clone(),
            accounts.clone(),
            secrets.clone(),
            auth_sessions.clone(),
        ));
        // One catalogue for the process: the workspace refresh writes it and
        // chat reads it, so the model list is fetched on a workspace refresh
        // rather than before every send.
        let model_catalog = Arc::new(ModelCatalogCache::default());
        // Plan models are device-local rows, merged into every catalogue read
        // rather than fetched from DA. The workspace refresh fills them.
        let plan_providers = Arc::new(PlanProviders::new(app_local_data.clone(), secrets.clone()));
        model_catalog.set_local_rows(plan_providers.model_source());
        // `projects`/`approvals` reach chat so a run can compose its tool
        // session (`crate::agent::build_tool_session`) — chat itself never
        // touches project or approval state.
        let chat = ChatService::new(
            auth_sessions.clone(),
            accounts.clone(),
            app_local_data,
            secrets,
            credentials.clone(),
            model_catalog.clone(),
            projects.clone(),
            approvals,
        );
        chat.set_plan_providers(plan_providers.clone());
        let workspace = WorkspaceService::new(
            auth_sessions.clone(),
            accounts.clone(),
            model_catalog.clone(),
        );
        workspace.set_plan_providers(plan_providers.clone());
        Self {
            cleanup,
            accounts,
            journal,
            auth: AuthService::new(auth_sessions.clone()),
            sessions: auth_sessions,
            workspace,
            chat,
            model_catalog,
            plan_providers,
            credentials,
            projects,
            office_previews: crate::projects::OfficePreviewJobs::default(),
            reports,
        }
    }

    pub fn workspace(&self) -> &WorkspaceService {
        &self.workspace
    }

    pub fn chat(&self) -> &ChatService {
        &self.chat
    }

    /// `pub(crate)`: the sign-out path is the only caller, and the catalogue
    /// is a crate-internal type.
    pub(crate) fn model_catalog(&self) -> &ModelCatalogCache {
        &self.model_catalog
    }

    /// `pub(crate)`: the sign-out and local-data clear paths are the callers,
    /// and they drop the account's plan sessions.
    pub(crate) fn plan_providers(&self) -> &Arc<PlanProviders> {
        &self.plan_providers
    }

    pub fn auth(&self) -> &AuthService {
        &self.auth
    }

    pub fn credentials(&self) -> &CredentialRuntime {
        &self.credentials
    }

    /// The projects service, for a signed-in account only.
    ///
    /// Every project command reaches the service through here, and none of them
    /// reaches a server that would check the bearer — they read and write files
    /// on this device. So this is the gate: see [`Self::is_signed_in`]. A refusal
    /// is the same `NoActiveAccount` a signed-out device already answers with.
    pub fn projects(&self) -> Result<&ProjectsService, ProjectsServiceError> {
        if !self.is_signed_in() {
            return Err(ProjectsServiceError::NoActiveAccount);
        }
        Ok(&self.projects)
    }

    pub(crate) fn office_previews(&self) -> &crate::projects::OfficePreviewJobs {
        &self.office_previews
    }

    /// Whether a command that serves what this device holds for the account may
    /// run: the bound account belongs to a session that can still act for it
    /// (see [`AuthSessionStore::owns_bound_account`]).
    ///
    /// Only for commands no server checks. A command that sends the bearer is
    /// already answered by the server that receives it.
    pub fn is_signed_in(&self) -> bool {
        self.sessions.owns_bound_account(&self.accounts)
    }

    pub fn reports(&self) -> &ReportsService {
        &self.reports
    }

    pub fn health(&self) -> AppHealth {
        let ready = self
            .accounts
            .current()
            .ok()
            .flatten()
            .is_some_and(|account| self.journal.is_ready_for(&account));
        AppHealth::new(if ready {
            AppHealthStatus::Ready
        } else {
            AppHealthStatus::Degraded
        })
    }

    pub fn accounts(&self) -> &CurrentAccountScope {
        &self.accounts
    }

    pub fn journal(&self) -> &JournalSlot {
        &self.journal
    }
}

#[tauri::command]
pub fn app_bootstrap(app: tauri::AppHandle) -> AppBootstrap {
    let session = app.state::<AppRuntimeState>().auth().status();
    AppBootstrap::new(env!("CARGO_PKG_VERSION"), current_platform()).with_session(session)
}

#[tauri::command]
pub fn app_health(app: tauri::AppHandle) -> AppHealth {
    app.state::<AppRuntimeState>().health()
}

/// Full device reset: erases the account, then every local trace of the app,
/// and relaunches it as a fresh install. The app cannot delete files it holds
/// open (logs, audit log, WebView storage), so it starts a helper first, clears
/// the account state, then exits and lets the helper purge and relaunch.
/// Destructive: the webview reaches it only through the pinned capability.
#[tauri::command]
pub async fn app_reset(app: tauri::AppHandle) -> Result<(), String> {
    if let Err(error) = device_purge::spawn_reset_helper(!tauri::is_dev()) {
        // A reset already under way is not a new erase, so it is not audited.
        if error == ResetStartError::Spawn {
            emit::record_with_detail(
                &app,
                AuditAction::DataErase,
                AuditResourceType::LocalData,
                "device",
                AuditOutcome::Failure,
                "not_started",
            );
        }
        return Err(reset_start_message(&error).to_owned());
    }
    emit::record(
        &app,
        AuditAction::DataErase,
        AuditResourceType::LocalData,
        "device",
        AuditOutcome::Allowed,
    );
    // The purge supersedes whatever this leaves behind, so its result is ignored.
    // Bounded, because the exit below is what lets the helper start its purge:
    // a hang here must not keep the app running.
    if tokio::time::timeout(RESET_CLEAR_TIMEOUT, clear_account_state(&app))
        .await
        .is_err()
    {
        tracing::warn!("reset: clearing account state timed out; the purge covers the rest");
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Lets the reply reach the window before the process goes away.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        handle.exit(0);
    });
    Ok(())
}

/// The longest the reset waits for the account state to clear before it exits.
const RESET_CLEAR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The message a reset gets when its helper does not start. The helper may
/// already be running from an earlier call, which then closes the app itself.
fn reset_start_message(error: &ResetStartError) -> &'static str {
    match error {
        ResetStartError::AlreadyStarted => {
            "A reset is already under way. xFalcon will close and reopen on its own."
        }
        ResetStartError::Spawn => "xFalcon could not start the reset. Nothing was removed.",
    }
}

/// Clears the account's upload, staging, path-grant and cleanup state. Reset
/// runs it before the purge; the purge covers whatever it leaves behind.
async fn clear_account_state(app: &tauri::AppHandle) -> LocalDataClearResult {
    app.state::<Arc<UploadService>>()
        .clear_account_scope()
        .await;
    app.state::<Arc<Staging>>().clear_account_scope();
    app.state::<Arc<PathGrants>>().clear();
    let cleanup = app.state::<AppRuntimeState>().cleanup.clone();
    // Before the cleanup deletes the files a cached ChatGPT session was read from.
    app.state::<AppRuntimeState>()
        .plan_providers()
        .forget_account();
    cleanup.clear().await
}

/// Opens the native folder picker and records the choice as a grant.
///
/// The renderer gets the grant id and the text to show for the folder, never a
/// path it can act on. Cancelling is `None`, not an error.
#[tauri::command]
pub async fn project_pick_folder(app: tauri::AppHandle) -> Result<Option<FolderGrantView>, String> {
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|error| error.to_string())?;
    let display_path = path.to_string_lossy().into_owned();
    let display_name = path.file_name().map_or_else(
        || display_path.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    let grant_id = app
        .state::<Arc<PathGrants>>()
        .issue(GrantKind::Folder, vec![path])
        .ok_or_else(|| "The chosen folder could not be recorded.".to_owned())?;
    Ok(Some(FolderGrantView::new(
        grant_id,
        display_name,
        display_path,
    )))
}

/// Trusts the folder a grant names. The grant comes from `project_pick_folder`;
/// see `crate::projects::TrustStore::create` for the
/// canonicalize-then-`cap-std`-open sequence and
/// `docs/threat-model.md` (TM-007) for the containment guarantee
/// this establishes.
#[tauri::command]
pub async fn project_create(
    app: tauri::AppHandle,
    grant_id: String,
    display_name: Option<String>,
) -> Result<ProjectView, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let path = resolve_folder(&app.state::<Arc<PathGrants>>(), &grant_id)?;
    projects
        .create(folder_text(&path)?, display_name)
        .await
        .map_err(|error| error.to_string())
}

/// Lists every project the current account has trusted, revalidating each
/// root's identity fresh (see `crate::projects::ProjectsService::list`)
/// rather than returning a cached "still valid" flag.
#[tauri::command]
pub async fn project_list(app: tauri::AppHandle) -> Result<ProjectListResult, String> {
    let state = app.state::<AppRuntimeState>();
    // Gated before the cleanup, not only before the list: the cleanup works
    // on the same account's projects.
    let projects = state.projects().map_err(|error| error.to_string())?;
    state
        .chat()
        .cleanup_deleted_projects(Some(app.clone()))
        .await
        .map_err(|error| error.to_string())?;
    projects.list().map_err(|error| error.to_string())
}

/// Finds current-account projects using the folder a grant names, without
/// creating or activating a project.
#[tauri::command]
pub async fn project_find_by_folder(
    app: tauri::AppHandle,
    grant_id: String,
) -> Result<ProjectListResult, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let path = resolve_folder(&app.state::<Arc<PathGrants>>(), &grant_id)?;
    projects
        .find_by_folder(folder_text(&path)?)
        .map_err(|error| error.to_string())
}

/// The folder a folder grant names, or the refusal when the id is unknown,
/// expired, or was issued as something else.
pub(crate) fn resolve_folder(grants: &PathGrants, grant_id: &str) -> Result<PathBuf, String> {
    grants
        .resolve(GrantKind::Folder, grant_id)
        .and_then(|paths| paths.into_iter().next())
        .ok_or_else(|| EXPIRED_CHOICE.to_owned())
}

/// The folder as the text the projects service takes. A name that is not
/// text is refused, not converted lossily into a different folder.
fn folder_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "The chosen folder has a name this app cannot read.".to_owned())
}

/// Activates a previously trusted project, failing closed if its root's
/// identity no longer matches what was recorded at trust time.
#[tauri::command]
pub async fn project_activate(
    app: tauri::AppHandle,
    project_id: WireProjectId,
) -> Result<ProjectView, String> {
    app.state::<AppRuntimeState>()
        .projects()
        .map_err(|error| error.to_string())?
        .activate(&project_id)
        .map_err(|error| error.to_string())
}

/// Clears the current project context. Trust and existing project-chat
/// bindings remain intact; the next new conversation is an ordinary chat.
#[tauri::command]
pub async fn project_deactivate(app: tauri::AppHandle) -> Result<(), String> {
    app.state::<AppRuntimeState>()
        .projects()
        .map_err(|error| error.to_string())?
        .deactivate()
        .map_err(|error| error.to_string())
}

/// Revokes a project's trust, clearing the active-project pointer if it was
/// the active project and cancelling any in-flight project operation (the
/// same cancellation seam `crate::security::account_cleanup`'s
/// `ProjectDataHandler` uses during account-wide cleanup).
#[tauri::command]
pub async fn project_revoke(
    app: tauri::AppHandle,
    project_id: WireProjectId,
) -> Result<(), String> {
    app.state::<AppRuntimeState>()
        .projects()
        .map_err(|error| error.to_string())?
        .revoke(&project_id)
        .await
        .map_err(|error| error.to_string())
}

const fn current_platform() -> Platform {
    #[cfg(target_os = "windows")]
    {
        Platform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
    #[cfg(target_os = "linux")]
    {
        Platform::Linux
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Platform::Unknown
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;

    #[test]
    fn a_folder_grant_resolves_to_the_folder_it_names() {
        let grants = PathGrants::default();
        let id = grants
            .issue(GrantKind::Folder, vec![PathBuf::from("/chosen/research")])
            .expect("within the grant limit");

        assert_eq!(
            resolve_folder(&grants, &id),
            Ok(PathBuf::from("/chosen/research"))
        );
    }

    #[test]
    fn an_unknown_or_wrong_kind_folder_grant_is_refused_with_the_choice_message() {
        let grants = PathGrants::default();
        let skills = grants
            .issue(GrantKind::SkillSources, vec![PathBuf::from("/chosen/a.md")])
            .expect("within the grant limit");

        for id in ["00000000000000000000000000000000", skills.as_str()] {
            assert_eq!(
                resolve_folder(&grants, id),
                Err("That choice expired. Choose it again.".to_owned()),
                "{id} must be refused"
            );
        }
    }

    #[test]
    fn a_folder_name_that_is_not_text_is_refused_rather_than_converted() {
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;

            let not_utf8 = PathBuf::from(OsStr::from_bytes(b"/chosen/\xff"));
            assert!(folder_text(&not_utf8).is_err());
        }
        assert_eq!(
            folder_text(Path::new("/chosen/research")),
            Ok("/chosen/research")
        );
    }
}

#[cfg(test)]
mod reset_start_tests {
    use super::*;

    #[test]
    fn a_reset_already_under_way_gets_its_own_message() {
        assert_eq!(
            reset_start_message(&ResetStartError::AlreadyStarted),
            "A reset is already under way. xFalcon will close and reopen on its own."
        );
    }

    #[test]
    fn a_helper_that_does_not_start_keeps_the_nothing_removed_message() {
        assert_eq!(
            reset_start_message(&ResetStartError::Spawn),
            "xFalcon could not start the reset. Nothing was removed."
        );
    }
}
