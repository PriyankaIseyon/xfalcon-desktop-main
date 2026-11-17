//! Saved reports (spec: reports).
//!
//! A saved report is a DA report kit. The assistant creates it through DA's
//! MCP tools; this module lists, opens (and shows and stores the thumbnail of),
//! renames, pins, shares, hides, deletes, and refreshes it over REST, exports it to a file the reader names, and saves
//! a report as a file into a project folder (a chat answer into its
//! conversation's project, or a library report into one the reader picks).
//! Nothing about a report is kept on this device; the library reads DA.

pub(crate) mod capture;
mod commands;
pub(crate) mod kit_flow;
pub(crate) mod kit_view;
pub(crate) mod origin;
pub(crate) mod project;
#[cfg(test)]
mod tests;

pub use commands::{
    report_delete, report_export, report_list, report_open, report_refresh, report_rename,
    report_save_library_to_project, report_save_to_project, report_set_hidden, report_set_pinned,
    report_set_scope, report_thumbnail, report_thumbnail_capture_open, report_thumbnail_save,
};
/// Directory (under an account's root) where earlier builds kept the local
/// reports store. Nothing writes it now; account cleanup still removes it
/// wholesale (`LocalDataResource::ArtifactCache`) so no old report outlives a
/// clear.
pub const REPORTS_DIR: &str = "reports";

use std::path::PathBuf;
use std::sync::Arc;

use xfalcon_contracts::export::DocumentExportResult;
use xfalcon_contracts::reports::{
    ProjectReportSaveResult, ReportCaptureDocument, ReportDocument, ReportErrorCode,
    ReportListResult, ReportOperationError, ReportRefreshResult, ReportScope, ReportThumbnail,
    ReportView,
};

use crate::approvals::ApprovalsService;
use crate::auth::canonical_account_id;
use crate::auth::{ensure_fresh_session, AuthConfig, AuthSessionStore};
use crate::chat::local_projection::{CHAT_PROJECTION_DB_FILE, LOCAL_PROJECTIONS_DIR};
use crate::chat::EncryptedChatProjection;
use crate::clients::da::kits_api::DaKitsSession;
use crate::export::choose_destination;
use crate::persistence::SecretStore;
use crate::preview::PreviewDocumentStore;
use crate::projects::{ProjectId, ProjectsService};
use crate::security::account_cleanup::{
    account_directory_name, CurrentAccountScope, ValidatedAccountId,
};

use project::ProjectReportRequest;

/// A library report to copy into a project: which report and page, which project,
/// and the name it is saved under.
pub struct LibraryProjectSave<'a> {
    pub report_id: &'a str,
    pub page: Option<&'a str>,
    pub project_id: &'a str,
    pub title: &'a str,
    pub allow_duplicate: bool,
}

/// The runtime layer the Reports Tauri commands call into.
pub struct ReportsService {
    app_root: PathBuf,
    accounts: Arc<CurrentAccountScope>,
    secrets: Arc<dyn SecretStore>,
    sessions: Arc<AuthSessionStore>,
}

impl ReportsService {
    pub fn new(
        app_root: PathBuf,
        accounts: Arc<CurrentAccountScope>,
        secrets: Arc<dyn SecretStore>,
        sessions: Arc<AuthSessionStore>,
    ) -> Self {
        Self {
            app_root,
            accounts,
            secrets,
            sessions,
        }
    }

    /// Saves the `html_index`-th HTML document of one assistant message into
    /// the project its conversation is bound to, instead of the DA store.
    ///
    /// Local only: the project and its folder never leave the device
    /// (spec: project-data-handling), so this makes no network call and needs
    /// no fresh session — only the signed-in account every project command
    /// requires.
    pub async fn save_to_project(
        &self,
        projects: &ProjectsService,
        approvals: &ApprovalsService,
        request: ProjectReportRequest<'_>,
    ) -> Result<ProjectReportSaveResult, ReportOperationError> {
        self.require_signed_in()?;
        let account = self
            .accounts
            .current()
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?
            .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
        let chat = self.chat_projection(&account)?;
        project::save_to_project(&chat, projects, approvals, &account, request).await
    }

    /// Lists the reports in the signed-in account's library, with the project
    /// each was created in where this device knows it.
    ///
    /// The project links are local and only a convenience for the Project
    /// filter, so a local store that cannot be read costs the links and not the
    /// library: the list is returned without them and the failure is logged.
    pub async fn list(
        &self,
        projects: &ProjectsService,
    ) -> Result<ReportListResult, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        let listed = kit_flow::list(&kits).await?;
        self.still_signed_in_as(&account)?;
        let origins = match self.accounts.current() {
            Ok(Some(account)) => match projects.report_origins(&account) {
                Ok(linked) => origin::origins_for_listed(&listed.reports, linked),
                Err(error) => {
                    tracing::warn!(%error, "reports: project links unreadable; listing without them");
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        Ok(listed.with_project_origins(origins))
    }

    /// Resolves one page of a report to a fresh preview-document id.
    pub async fn open(
        &self,
        preview: &PreviewDocumentStore,
        report_id: &str,
        page: Option<&str>,
    ) -> Result<ReportDocument, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::open(&kits, preview, report_id, page, || {
            self.still_signed_in_as(&account)
        })
        .await
    }

    /// The report's stored thumbnail image, for its card.
    pub async fn thumbnail(
        &self,
        report_id: &str,
    ) -> Result<ReportThumbnail, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        let image = kit_flow::thumbnail(&kits, report_id).await?;
        self.still_signed_in_as(&account)?;
        Ok(image)
    }

    /// A dashboard's first page with DA's capture script, for a frame that is
    /// never shown.
    pub async fn open_for_capture(
        &self,
        preview: &PreviewDocumentStore,
        report_id: &str,
    ) -> Result<ReportCaptureDocument, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::open_for_capture(&kits, preview, report_id, || {
            self.still_signed_in_as(&account)
        })
        .await
    }

    /// Stores a captured image as the thumbnail of a report the reader owns.
    pub async fn save_thumbnail(
        &self,
        report_id: &str,
        image_base64: &str,
    ) -> Result<ReportView, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        self.still_signed_in_as(&account)?;
        kit_flow::save_thumbnail(&kits, report_id, image_base64).await
    }

    pub async fn rename(
        &self,
        report_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<ReportView, ReportOperationError> {
        self.require_signed_in()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::rename(&kits, report_id, title, description).await
    }

    pub async fn set_pinned(
        &self,
        report_id: &str,
        pinned: bool,
    ) -> Result<ReportView, ReportOperationError> {
        self.require_signed_in()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::set_pinned(&kits, report_id, pinned).await
    }

    /// Shares a report with the organization, or makes it private again.
    pub async fn set_scope(
        &self,
        report_id: &str,
        scope: ReportScope,
    ) -> Result<ReportView, ReportOperationError> {
        self.require_signed_in()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::set_scope(&kits, report_id, scope).await
    }

    /// Hides a report from this reader's list, or shows it again.
    pub async fn set_hidden(
        &self,
        report_id: &str,
        hidden: bool,
    ) -> Result<ReportView, ReportOperationError> {
        self.require_signed_in()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::set_hidden(&kits, report_id, hidden).await
    }

    /// Writes a report to a file the reader names in the native save dialog: a
    /// dashboard's assembled page as `.html`, a workbook or deck as the file DA
    /// built. The renderer names no path and sends no bytes.
    ///
    /// A cancelled dialog is an answer, not a failure. The account is checked
    /// again after every wait (the fetch and the dialog), so an answer that
    /// outlived its account is never written for whoever is signed in now.
    pub async fn export(
        &self,
        app: &tauri::AppHandle,
        report_id: &str,
        page: Option<&str>,
    ) -> Result<DocumentExportResult, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        let plan = kit_flow::plan_export(&kits, report_id, page).await?;
        self.still_signed_in_as(&account)?;
        let export_failed = |_| ReportOperationError::new(ReportErrorCode::ExportFailed);
        match plan {
            kit_flow::ExportPlan::Page { title, html } => {
                let Some(destination) = choose_destination(app, &title, "html", "HTML document")
                    .await
                    .map_err(export_failed)?
                else {
                    return Ok(DocumentExportResult::cancelled());
                };
                self.still_signed_in_as(&account)?;
                destination
                    .write_all(html.as_bytes())
                    .map_err(export_failed)
            }
            kit_flow::ExportPlan::File {
                title,
                extension,
                filter_label,
            } => {
                let Some(destination) = choose_destination(app, &title, extension, filter_label)
                    .await
                    .map_err(export_failed)?
                else {
                    return Ok(DocumentExportResult::cancelled());
                };
                self.still_signed_in_as(&account)?;
                // The reader may have taken minutes to choose: the file is asked
                // for now, on a session renewed if it needs it.
                let kits = self.fresh_kits_session().await?;
                kit_flow::download_file(&kits, report_id, &destination).await
            }
        }
    }

    /// Which file a workbook or deck report is, and which version of it DA
    /// holds, for a preview. The file itself is not fetched.
    pub(crate) async fn office_file_version(
        &self,
        report_id: &str,
    ) -> Result<kit_flow::OfficeFileVersion, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        let version = kit_flow::office_file_version(&kits, report_id).await?;
        self.still_signed_in_as(&account)?;
        Ok(version)
    }

    /// A workbook's or deck's file in memory, within `max_bytes`, for a preview.
    pub(crate) async fn office_file_bytes(
        &self,
        report_id: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, ReportOperationError> {
        self.require_signed_in()?;
        let account = self.bound_account_id()?;
        let kits = self.fresh_kits_session().await?;
        let bytes = kit_flow::read_office_file(&kits, report_id, max_bytes).await?;
        self.still_signed_in_as(&account)?;
        Ok(bytes)
    }

    /// Writes a library report's assembled page into `reports/` of a project the
    /// reader picked. The page carries the rows as of the last refresh; the copy
    /// keeps no link to the report and does not refresh.
    pub async fn save_library_report_to_project(
        &self,
        projects: &ProjectsService,
        approvals: &ApprovalsService,
        request: LibraryProjectSave<'_>,
    ) -> Result<ProjectReportSaveResult, ReportOperationError> {
        self.require_signed_in()?;
        let account_id = self.bound_account_id()?;
        let title = project::project_report_title(request.title)?;
        // A project id that is not one is a project that is not there.
        let project_id: ProjectId = request
            .project_id
            .parse()
            .map_err(|_| ReportOperationError::new(ReportErrorCode::ProjectUnavailable))?;
        let kits = self.fresh_kits_session().await?;
        let (_, html) = kit_flow::page_for_project(&kits, request.report_id, request.page).await?;
        self.still_signed_in_as(&account_id)?;
        let account = self
            .accounts
            .current()
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?
            .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
        project::write_project_report(
            projects,
            approvals,
            &account,
            project_id,
            title,
            html,
            request.allow_duplicate,
        )
        .await
    }

    pub async fn delete(
        &self,
        projects: &ProjectsService,
        report_id: &str,
    ) -> Result<(), ReportOperationError> {
        self.require_signed_in()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::delete(&kits, report_id).await?;
        // The report is gone; so is the link. A failure here leaves a link to
        // nothing, which the list never shows, so it is logged and not raised.
        if let Ok(Some(account)) = self.accounts.current() {
            if let Err(error) = projects.forget_report_origin(&account, report_id) {
                tracing::warn!(%error, "reports: could not drop a deleted report's project link");
            }
        }
        Ok(())
    }

    /// Re-runs a report's datasets on DA.
    pub async fn refresh(
        &self,
        report_id: &str,
    ) -> Result<ReportRefreshResult, ReportOperationError> {
        self.require_signed_in()?;
        let kits = self.fresh_kits_session().await?;
        kit_flow::refresh(&kits, report_id).await
    }

    /// Refuses a read unless the bound account belongs to a session that can
    /// still act for it (`AuthSessionStore::owns_bound_account`), with the
    /// `StorageUnavailable` a device with no active account answers with.
    ///
    /// Every report operation sends the bearer to DA, so a refused renewal or a
    /// cancelled sign-in stops it here. A renewal that could not be reached leaves
    /// the stored bearer in place, and DA answers for it.
    fn require_signed_in(&self) -> Result<(), ReportOperationError> {
        if self.sessions.owns_bound_account(&self.accounts) {
            Ok(())
        } else {
            Err(ReportOperationError::new(
                ReportErrorCode::StorageUnavailable,
            ))
        }
    }

    fn bound_account_id(&self) -> Result<String, ReportOperationError> {
        self.accounts
            .current()
            .ok()
            .flatten()
            .map(|account| account.as_str().to_owned())
            .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))
    }

    /// A read can take most of a minute. If the user signed out, or another
    /// account took the device, while it ran, its answer belongs to nobody: it is
    /// refused rather than handed to whoever is signed in now.
    fn still_signed_in_as(&self, account: &str) -> Result<(), ReportOperationError> {
        self.require_signed_in()?;
        if self.bound_account_id()? == account {
            Ok(())
        } else {
            Err(ReportOperationError::new(
                ReportErrorCode::StorageUnavailable,
            ))
        }
    }

    /// The DA kit client for a fresh session. Every library call goes through here.
    async fn fresh_kits_session(&self) -> Result<DaKitsSession, ReportOperationError> {
        let _ = ensure_fresh_session(&self.sessions).await;
        let (config, token) = self.authenticated()?;
        self.kits_session(&config, &token)
    }

    /// The config and bearer of the signed-in session, once it is confirmed to
    /// belong to the account this device is bound to.
    fn authenticated(&self) -> Result<(AuthConfig, String), ReportOperationError> {
        let config = AuthConfig::from_env()
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
        let snapshot = self
            .sessions
            .authenticated_snapshot()
            .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
        let account_id = canonical_account_id(&snapshot.identity)
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
        let account = self
            .accounts
            .current()
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?
            .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
        if account.as_str() != account_id {
            return Err(ReportOperationError::new(
                ReportErrorCode::StorageUnavailable,
            ));
        }
        Ok((config, snapshot.access_token.to_string()))
    }

    fn kits_session(
        &self,
        config: &AuthConfig,
        token: &str,
    ) -> Result<DaKitsSession, ReportOperationError> {
        DaKitsSession::new(config.da_base_url.as_str(), token)
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))
    }

    fn chat_projection(
        &self,
        account: &ValidatedAccountId,
    ) -> Result<EncryptedChatProjection, ReportOperationError> {
        let path = self
            .app_root
            .join("accounts")
            .join(account_directory_name(account.as_str()))
            .join(LOCAL_PROJECTIONS_DIR)
            .join(CHAT_PROJECTION_DB_FILE);
        EncryptedChatProjection::open(path, self.secrets.clone(), account.clone())
            .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))
    }
}
