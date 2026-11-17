//! The six commands the Reports destination uses, and the one that saves a
//! project chat's dashboard into its project folder instead.
//!
//! Ids and names cross the boundary, never bytes and never paths. A document
//! goes in as `(conversation, message, ordinal)` and a report comes out as a
//! preview document id, so `write_file` and `write_text_file` stay forbidden
//! and the renderer still cannot name a file — see the module doc in `super`.
//! The one exception is a thumbnail: a PNG or WebP image whose signature is
//! checked in the backend, in both directions. It is shown through an `<img>`,
//! which runs nothing, so it needs none of the isolation a page's markup does.
//!
//! Every id-scoped command answers an unknown id, another account's id, and a
//! malformed id identically, because a rename that answered differently from an
//! open would disclose exactly what the fold exists to hide.

use std::sync::Arc;
use tauri::Manager as _;

use xfalcon_contracts::export::DocumentExportResult;
use xfalcon_contracts::reports::{
    ProjectReportSaveResult, ReportCaptureDocument, ReportDocument, ReportErrorCode,
    ReportListResult, ReportOperationError, ReportRefreshResult, ReportScope, ReportThumbnail,
    ReportView,
};
use xfalcon_contracts::ConversationId;

use crate::app::commands::AppRuntimeState;
use crate::approvals::ApprovalsService;
use crate::export::{export_outcome, record_export};
use crate::preview::PreviewDocumentStore;

use super::project::ProjectReportRequest;
use super::LibraryProjectSave;

/// Takes a conversation and a message identity and deliberately no project
/// id: the project is the conversation's own binding, resolved in the backend,
/// so the renderer cannot choose which folder receives the file.
#[tauri::command]
pub async fn report_save_to_project(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    message_id: String,
    html_index: u32,
    title: String,
    allow_duplicate: bool,
) -> Result<ProjectReportSaveResult, ReportOperationError> {
    let runtime = app.state::<AppRuntimeState>();
    let projects = runtime
        .projects()
        .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
    let approvals = app.state::<Arc<ApprovalsService>>();
    runtime
        .reports()
        .save_to_project(
            projects,
            approvals.inner().as_ref(),
            ProjectReportRequest {
                conversation_id,
                message_id: &message_id,
                html_index,
                title: &title,
                allow_duplicate,
            },
        )
        .await
}

#[tauri::command]
pub async fn report_list(app: tauri::AppHandle) -> Result<ReportListResult, ReportOperationError> {
    let runtime = app.state::<AppRuntimeState>();
    let projects = runtime
        .projects()
        .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
    runtime.reports().list(projects).await
}

#[tauri::command]
pub async fn report_open(
    app: tauri::AppHandle,
    report_id: String,
    page: Option<String>,
) -> Result<ReportDocument, ReportOperationError> {
    let preview = app.state::<std::sync::Arc<PreviewDocumentStore>>();
    app.state::<AppRuntimeState>()
        .reports()
        .open(preview.inner().as_ref(), &report_id, page.as_deref())
        .await
}

#[tauri::command]
pub async fn report_thumbnail(
    app: tauri::AppHandle,
    report_id: String,
) -> Result<ReportThumbnail, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .thumbnail(&report_id)
        .await
}

#[tauri::command]
pub async fn report_thumbnail_capture_open(
    app: tauri::AppHandle,
    report_id: String,
) -> Result<ReportCaptureDocument, ReportOperationError> {
    let preview = app.state::<std::sync::Arc<PreviewDocumentStore>>();
    app.state::<AppRuntimeState>()
        .reports()
        .open_for_capture(preview.inner().as_ref(), &report_id)
        .await
}

/// Takes the image the capture frame sent. The bytes are checked in the backend
/// before DA sees them, so the renderer cannot store anything but a bounded PNG.
#[tauri::command]
pub async fn report_thumbnail_save(
    app: tauri::AppHandle,
    report_id: String,
    image_base64: String,
) -> Result<ReportView, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .save_thumbnail(&report_id, &image_base64)
        .await
}

#[tauri::command]
pub async fn report_rename(
    app: tauri::AppHandle,
    report_id: String,
    title: String,
    description: Option<String>,
) -> Result<ReportView, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .rename(&report_id, &title, description.as_deref())
        .await
}

/// Takes the INTENT, not a timestamp: the pin is DA's per-viewer flag, so no
/// surface can invent one and two surfaces cannot disagree about it.
#[tauri::command]
pub async fn report_set_pinned(
    app: tauri::AppHandle,
    report_id: String,
    pinned: bool,
) -> Result<ReportView, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .set_pinned(&report_id, pinned)
        .await
}

/// Takes the INTENT, as a pin does: `user` is private, `global` is the
/// organization. DA enforces who may; the surface offers it on `can_manage`.
#[tauri::command]
pub async fn report_set_scope(
    app: tauri::AppHandle,
    report_id: String,
    scope: ReportScope,
) -> Result<ReportView, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .set_scope(&report_id, scope)
        .await
}

/// The reader's own hide, like a pin: it changes no one else's list.
#[tauri::command]
pub async fn report_set_hidden(
    app: tauri::AppHandle,
    report_id: String,
    hidden: bool,
) -> Result<ReportView, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .set_hidden(&report_id, hidden)
        .await
}

/// Writes a report to a file the reader chooses. Takes no path and no bytes: the
/// save dialog opens in the backend, so nothing a caller sends can aim the write.
#[tauri::command]
pub async fn report_export(
    app: tauri::AppHandle,
    report_id: String,
    page: Option<String>,
) -> Result<DocumentExportResult, ReportOperationError> {
    let result = app
        .state::<AppRuntimeState>()
        .reports()
        .export(&app, &report_id, page.as_deref())
        .await;
    record_export(&app, "report", None, export_outcome(&result));
    result
}

/// Copies a library report's assembled page into a project folder. The project is
/// named by id and resolved to a trusted root in the backend; a path is never an
/// input, and the page's bytes never cross the boundary.
#[tauri::command]
pub async fn report_save_library_to_project(
    app: tauri::AppHandle,
    report_id: String,
    page: Option<String>,
    project_id: String,
    title: String,
    allow_duplicate: bool,
) -> Result<ProjectReportSaveResult, ReportOperationError> {
    let runtime = app.state::<AppRuntimeState>();
    let projects = runtime
        .projects()
        .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
    let approvals = app.state::<Arc<ApprovalsService>>();
    runtime
        .reports()
        .save_library_report_to_project(
            projects,
            approvals.inner().as_ref(),
            LibraryProjectSave {
                report_id: &report_id,
                page: page.as_deref(),
                project_id: &project_id,
                title: &title,
                allow_duplicate,
            },
        )
        .await
}

#[tauri::command]
pub async fn report_delete(
    app: tauri::AppHandle,
    report_id: String,
) -> Result<(), ReportOperationError> {
    let runtime = app.state::<AppRuntimeState>();
    let projects = runtime
        .projects()
        .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
    runtime.reports().delete(projects, &report_id).await
}

#[tauri::command]
pub async fn report_refresh(
    app: tauri::AppHandle,
    report_id: String,
) -> Result<ReportRefreshResult, ReportOperationError> {
    app.state::<AppRuntimeState>()
        .reports()
        .refresh(&report_id)
        .await
}
