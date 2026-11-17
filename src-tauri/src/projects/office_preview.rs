//! One bounded, cancellable Office preview at a time, and saving what the
//! viewer shows. The source is read only through the project's trusted-root
//! reader; the converter sees a temporary copy (see [`super::office_convert`]).
//!
//! A saved file never lands where a caller names: the save dialog opens on this
//! side and the path comes back from the OS (see [`crate::export`]). The PDF that
//! "Save PDF" writes is the one this module produced, held for a short time under
//! the preview's id; the renderer never sends bytes back to be written.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tauri::Manager;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::export::DocumentExportResult;
use xfalcon_contracts::projects::ProjectOfficePreviewStarted;
use xfalcon_contracts::reports::{ReportErrorCode, ReportOperationError};
use xfalcon_contracts::CONTRACT_VERSION;

use crate::app::commands::AppRuntimeState;
use crate::export::{choose_destination, export_outcome, record_export};
use crate::security::account_cleanup::ValidatedAccountId;

use super::commands::{read_scope_id, viewer_target};
use super::office_convert::{self, ConvertError};
use super::sensitive::SecretClassifier;

const CANCELLED: &str = "The Office preview was cancelled.";
const WORKER_STOPPED: &str = "The Office preview worker stopped unexpectedly.";
const PREVIEW_FIRST: &str = "Preview this file first, then save the PDF.";
/// Stands where a project id does in the cache key of a Reports-library file.
const REPORT_CACHE_SCOPE: &str = "report:library";
/// How long the last PDF stays available to "Save PDF".
const PDF_KEEP_FOR: Duration = Duration::from_secs(15 * 60);
/// Finished PDFs reused while their source is byte-identical. In memory only:
/// a derived copy of a project file is never written to disk.
const PDF_CACHE_ENTRIES: usize = 8;
const PDF_CACHE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct PreviewOwner {
    window: String,
    account: ValidatedAccountId,
}

struct ActivePreview {
    id: Uuid,
    owner: PreviewOwner,
    cancel: CancellationToken,
}

/// The PDF of the most recent finished preview.
struct KeptPdf {
    id: Uuid,
    owner: PreviewOwner,
    project_id: String,
    path: String,
    pdf: Arc<Vec<u8>>,
    kept_at: Instant,
}

/// One finished PDF and the exact file version it was made from.
struct CachedPdf {
    account: ValidatedAccountId,
    project_id: String,
    path: String,
    /// SHA-256 of the source bytes: any change to the file is a different version.
    source: [u8; 32],
    pdf: Arc<Vec<u8>>,
}

impl CachedPdf {
    fn is_file(&self, account: &ValidatedAccountId, project_id: &str, path: &str) -> bool {
        &self.account == account && self.project_id == project_id && self.path == path
    }
}

/// The most recently used PDFs, newest first, bounded by count and size.
#[derive(Default)]
struct PdfCache {
    entries: VecDeque<CachedPdf>,
}

impl PdfCache {
    /// The PDF made from exactly this version of this file, if it is still here.
    fn get(
        &mut self,
        account: &ValidatedAccountId,
        project_id: &str,
        path: &str,
        source: &[u8; 32],
    ) -> Option<Arc<Vec<u8>>> {
        let index = self.entries.iter().position(|entry| {
            entry.is_file(account, project_id, path) && &entry.source == source
        })?;
        let entry = self.entries.remove(index)?;
        let pdf = entry.pdf.clone();
        self.entries.push_front(entry);
        Some(pdf)
    }

    fn retain_account(&mut self, account: &ValidatedAccountId) {
        self.entries.retain(|entry| &entry.account == account);
    }

    /// Stores a PDF, replacing any older version of the same file.
    fn put(&mut self, entry: CachedPdf) {
        self.entries
            .retain(|old| !old.is_file(&entry.account, &entry.project_id, &entry.path));
        self.entries.push_front(entry);
        while self.entries.len() > PDF_CACHE_ENTRIES
            || self
                .entries
                .iter()
                .map(|entry| entry.pdf.len())
                .sum::<usize>()
                > PDF_CACHE_BYTES
        {
            self.entries.pop_back();
        }
    }
}

pub(crate) struct OfficePreviewJobs {
    active: Arc<Mutex<Option<ActivePreview>>>,
    worker: Arc<Semaphore>,
    kept: Mutex<Option<KeptPdf>>,
    cache: Arc<Mutex<PdfCache>>,
}

impl Default for OfficePreviewJobs {
    fn default() -> Self {
        Self {
            active: Arc::default(),
            worker: Arc::new(Semaphore::new(1)),
            kept: Mutex::new(None),
            cache: Arc::default(),
        }
    }
}

impl OfficePreviewJobs {
    fn begin(&self, owner: PreviewOwner) -> Result<PreviewLease, String> {
        let id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        // A new preview replaces the PDF kept from the last one.
        self.kept
            .lock()
            .map_err(|_| WORKER_STOPPED.to_owned())?
            .take();
        // PDFs made for another account are of no use to this one and are not
        // kept in memory after the account changes.
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain_account(&owner.account);
        }
        let mut active = self.active.lock().map_err(|_| WORKER_STOPPED.to_owned())?;
        if let Some(previous) = active.replace(ActivePreview {
            id,
            owner,
            cancel: cancel.clone(),
        }) {
            previous.cancel.cancel();
        }
        Ok(PreviewLease {
            id,
            cancel,
            active: self.active.clone(),
            worker: self.worker.clone(),
        })
    }

    fn cancel(&self, owner: &PreviewOwner, id: Uuid) -> Result<(), String> {
        let active = self.active.lock().map_err(|_| WORKER_STOPPED.to_owned())?;
        if let Some(active) = active
            .as_ref()
            .filter(|job| job.id == id && &job.owner == owner)
        {
            active.cancel.cancel();
        }
        drop(active);
        let mut kept = self.kept.lock().map_err(|_| WORKER_STOPPED.to_owned())?;
        if kept
            .as_ref()
            .is_some_and(|pdf| pdf.id == id && &pdf.owner == owner)
        {
            *kept = None;
        }
        // An already-completed id is safe to cancel; no state is allocated for it.
        Ok(())
    }

    fn keep(&self, entry: KeptPdf) -> Result<(), String> {
        *self.kept.lock().map_err(|_| WORKER_STOPPED.to_owned())? = Some(entry);
        Ok(())
    }

    /// The kept PDF, only for the same window and account, project, file and
    /// preview id that produced it, and only for a short while.
    fn kept_pdf(
        &self,
        owner: &PreviewOwner,
        id: Uuid,
        project_id: &str,
        path: &str,
    ) -> Result<Arc<Vec<u8>>, String> {
        let mut kept = self.kept.lock().map_err(|_| WORKER_STOPPED.to_owned())?;
        // An expired PDF is dropped, not just refused, so it does not linger in memory.
        if kept
            .as_ref()
            .is_some_and(|pdf| pdf.kept_at.elapsed() >= PDF_KEEP_FOR)
        {
            *kept = None;
        }
        kept.as_ref()
            .filter(|pdf| {
                pdf.id == id
                    && &pdf.owner == owner
                    && pdf.project_id == project_id
                    && pdf.path == path
            })
            .map(|pdf| pdf.pdf.clone())
            .ok_or_else(|| PREVIEW_FIRST.to_owned())
    }
}

struct PreviewLease {
    id: Uuid,
    cancel: CancellationToken,
    active: Arc<Mutex<Option<ActivePreview>>>,
    worker: Arc<Semaphore>,
}

impl PreviewLease {
    async fn acquire(&self) -> Result<OwnedSemaphorePermit, String> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(CANCELLED.to_owned()),
            permit = self.worker.clone().acquire_owned() => {
                permit.map_err(|_| WORKER_STOPPED.to_owned())
            }
        }
    }
}

impl Drop for PreviewLease {
    fn drop(&mut self) {
        self.cancel.cancel();
        match self.active.lock() {
            Ok(mut active) => {
                if active.as_ref().is_some_and(|job| job.id == self.id) {
                    *active = None;
                }
            }
            Err(_) => {
                tracing::error!("Office preview registration lock was poisoned during cleanup")
            }
        }
    }
}

fn owner(state: &AppRuntimeState, window: &str) -> Result<PreviewOwner, String> {
    state.projects().map_err(|error| error.to_string())?;
    let account = state
        .accounts()
        .current()
        .map_err(|_| CANCELLED.to_owned())?
        .ok_or_else(|| CANCELLED.to_owned())?;
    Ok(PreviewOwner {
        window: window.to_owned(),
        account,
    })
}

/// Returns PDF bytes only after the conversion has ended. The channel announces
/// registration before work, so an early view-close can cancel.
#[tauri::command]
pub async fn project_office_preview(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_id: String,
    path: String,
    on_started: tauri::ipc::Channel<ProjectOfficePreviewStarted>,
) -> Result<tauri::ipc::Response, String> {
    let result = run_project_office_preview(app, window, project_id, path, on_started).await;
    log_preview_failure("project_office_preview", &result);
    result
}

/// The work of [`project_office_preview`].
async fn run_project_office_preview(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_id: String,
    path: String,
    on_started: tauri::ipc::Channel<ProjectOfficePreviewStarted>,
) -> Result<tauri::ipc::Response, String> {
    let state = app.state::<AppRuntimeState>();
    let initial_owner = owner(&state, window.label())?;
    let projects = state.projects().map_err(|error| error.to_string())?;
    let scope = projects.cancellation();
    let generation = scope.generation();
    let (root, relative) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let lease = state.office_previews().begin(initial_owner.clone())?;
    on_started
        .send(ProjectOfficePreviewStarted {
            version: CONTRACT_VERSION,
            render_id: lease.id.to_string(),
        })
        .map_err(|_| CANCELLED.to_owned())?;
    let permit = lease.acquire().await?;
    if scope.generation() != generation || owner(&state, window.label())? != initial_owner {
        return Err(CANCELLED.to_owned());
    }
    let cancel = lease.cancel.clone();
    let cache = state.office_previews().cache.clone();
    let cache_account = initial_owner.account.clone();
    let cache_project = project_id.clone();
    let cache_path = path.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if cancel.is_cancelled() {
            return Err(CANCELLED.to_owned());
        }
        let source = super::viewer::read_office_source(
            &root,
            &SecretClassifier::built_in(),
            &relative,
            read_scope_id(),
        )
        .map_err(|error| error.to_string())?;
        let original_hash = Sha256::digest(&source.bytes);
        let digest: [u8; 32] = original_hash.into();
        // The same file, byte for byte, already has a PDF: reuse it. The source
        // was just read through the trust gates, so a revoked or withheld file
        // never gets here.
        let cached = cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&cache_account, &cache_project, &cache_path, &digest));
        if let Some(pdf) = cached {
            return Ok(pdf);
        }
        let original_size = source.size_bytes;
        let original_modified = source.modified_ms;
        let extension = office_convert::known_extension(relative.as_path())
            .ok_or_else(|| ConvertError::Failed("The preview").to_string())?;
        let pdf = office_convert::convert_to_pdf(&source.bytes, extension, &cancel)
            .map_err(|error| error.to_string())?;
        drop(source);
        if cancel.is_cancelled() {
            return Err(CANCELLED.to_owned());
        }
        // Re-open through the trust boundary. A replaced, revoked, sensitive or
        // changed source cannot publish a preview for the previous file version.
        let after = super::viewer::read_office_source(
            &root,
            &SecretClassifier::built_in(),
            &relative,
            read_scope_id(),
        )
        .map_err(|error| error.to_string())?;
        if after.size_bytes != original_size
            || after.modified_ms != original_modified
            || Sha256::digest(&after.bytes)[..] != digest[..]
        {
            return Err("The Office file changed while its preview was prepared.".to_owned());
        }
        if cancel.is_cancelled() {
            return Err(CANCELLED.to_owned());
        }
        let pdf = Arc::new(pdf);
        match cache.lock() {
            Ok(mut cache) => cache.put(CachedPdf {
                account: cache_account,
                project_id: cache_project,
                path: cache_path,
                source: digest,
                pdf: pdf.clone(),
            }),
            Err(_) => tracing::warn!("Office preview cache lock was poisoned; not caching"),
        }
        Ok(pdf)
    });
    loop {
        tokio::select! {
            result = &mut worker => {
                if lease.cancel.is_cancelled() || scope.generation() != generation
                    || owner(&state, window.label())? != initial_owner {
                    return Err(CANCELLED.to_owned());
                }
                let pdf = result.map_err(|_| WORKER_STOPPED.to_owned())??;
                state.office_previews().keep(KeptPdf {
                    id: lease.id,
                    owner: initial_owner,
                    project_id,
                    path,
                    pdf: pdf.clone(),
                    kept_at: Instant::now(),
                })?;
                return Ok(tauri::ipc::Response::new(pdf.as_ref().clone()));
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if scope.generation() != generation
                    || owner(&state, window.label()).as_ref() != Ok(&initial_owner)
                    || app.get_webview_window(window.label()).is_none() {
                    lease.cancel.cancel();
                }
            }
        }
    }
}

/// What the person is told when a Reports-library file cannot be fetched.
fn report_text(error: ReportOperationError) -> String {
    match error.code {
        ReportErrorCode::NotFound => "This report could not be found.",
        ReportErrorCode::Forbidden => "You are not allowed to open this report.",
        ReportErrorCode::TooLargeToOpen => "This file is too large to preview.",
        _ => "This report's file could not be fetched. Check your connection and try again.",
    }
    .to_owned()
}

/// Logs a preview that ended in an error the person sees. A cancellation is
/// routine and is not logged.
fn log_preview_failure<T>(command: &'static str, result: &Result<T, String>) {
    if let Err(message) = result {
        if message != CANCELLED {
            tracing::warn!(
                command,
                error = %crate::diagnostics::sanitize_diagnostics_text(message),
                "office preview failed"
            );
        }
    }
}

/// The PDF preview of a workbook or deck in the Reports library. The file is
/// fetched from DA through the signed-in session and converted from memory; the
/// converter writes it to its own temporary copy as for a project file. A
/// finished PDF is kept, in memory only, for as long as DA reports the same
/// version of the report, so reopening an unchanged one fetches nothing.
/// Cancelling goes through `project_office_preview_cancel`, by the same id.
#[tauri::command]
pub async fn report_office_preview(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    report_id: String,
    on_started: tauri::ipc::Channel<ProjectOfficePreviewStarted>,
) -> Result<tauri::ipc::Response, String> {
    let result = run_report_office_preview(app, window, report_id, on_started).await;
    log_preview_failure("report_office_preview", &result);
    result
}

/// The work of [`report_office_preview`].
async fn run_report_office_preview(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    report_id: String,
    on_started: tauri::ipc::Channel<ProjectOfficePreviewStarted>,
) -> Result<tauri::ipc::Response, String> {
    let state = app.state::<AppRuntimeState>();
    let initial_owner = owner(&state, window.label())?;
    let lease = state.office_previews().begin(initial_owner.clone())?;
    on_started
        .send(ProjectOfficePreviewStarted {
            version: CONTRACT_VERSION,
            render_id: lease.id.to_string(),
        })
        .map_err(|_| CANCELLED.to_owned())?;
    let permit = lease.acquire().await?;
    let cancel = lease.cancel.clone();
    let cache = state.office_previews().cache.clone();
    let reports = state.reports();
    let work = async {
        let version = reports
            .office_file_version(&report_id)
            .await
            .map_err(report_text)?;
        let digest: [u8; 32] = Sha256::digest(version.marker.as_bytes()).into();
        // DA ids differ only in hex case; one report is one cache entry.
        let cache_id = report_id.to_ascii_lowercase();
        let cached = cache.lock().ok().and_then(|mut cache| {
            cache.get(
                &initial_owner.account,
                REPORT_CACHE_SCOPE,
                &cache_id,
                &digest,
            )
        });
        if let Some(pdf) = cached {
            return Ok(pdf);
        }
        let bytes = reports
            .office_file_bytes(&report_id, office_convert::MAX_SOURCE_BYTES)
            .await
            .map_err(report_text)?;
        let extension = version.extension;
        let convert_cancel = cancel.clone();
        let pdf = tokio::task::spawn_blocking(move || {
            // Held until the conversion ends, even if this preview is replaced.
            let _permit = permit;
            office_convert::convert_to_pdf(&bytes, extension, &convert_cancel)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| WORKER_STOPPED.to_owned())??;
        let pdf = Arc::new(pdf);
        match cache.lock() {
            Ok(mut cache) => cache.put(CachedPdf {
                account: initial_owner.account.clone(),
                project_id: REPORT_CACHE_SCOPE.to_owned(),
                path: cache_id,
                source: digest,
                pdf: pdf.clone(),
            }),
            Err(_) => tracing::warn!("Office preview cache lock was poisoned; not caching"),
        }
        Ok::<_, String>(pdf)
    };
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            biased;
            _ = lease.cancel.cancelled() => break Err(CANCELLED.to_owned()),
            result = &mut work => break result,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if owner(&state, window.label()).as_ref() != Ok(&initial_owner)
                    || app.get_webview_window(window.label()).is_none() {
                    lease.cancel.cancel();
                }
            }
        }
    };
    if lease.cancel.is_cancelled() || owner(&state, window.label())? != initial_owner {
        return Err(CANCELLED.to_owned());
    }
    let pdf = result?;
    Ok(tauri::ipc::Response::new(pdf.as_ref().clone()))
}

#[tauri::command]
pub fn project_office_preview_cancel(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    render_id: String,
) -> Result<(), String> {
    let state = app.state::<AppRuntimeState>();
    let owner = owner(&state, window.label())?;
    let id =
        Uuid::parse_str(&render_id).map_err(|_| "The Office preview id is invalid.".to_owned())?;
    state.office_previews().cancel(&owner, id)
}

/// The name a saved copy starts with: the file's own name without its suffix.
fn suggested_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("document")
        .to_owned()
}

/// Saves the PDF of the preview `render_id` wherever the person chooses.
#[tauri::command]
pub async fn project_office_save_pdf(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_id: String,
    path: String,
    render_id: String,
) -> Result<DocumentExportResult, String> {
    let state = app.state::<AppRuntimeState>();
    let owner = owner(&state, window.label())?;
    let id =
        Uuid::parse_str(&render_id).map_err(|_| "The Office preview id is invalid.".to_owned())?;
    let pdf = state
        .office_previews()
        .kept_pdf(&owner, id, &project_id, &path)?;
    let result = match choose_destination(&app, &suggested_name(&path), "pdf", "PDF")
        .await
        .map_err(|error| error.to_string())?
    {
        None => Ok(DocumentExportResult::cancelled()),
        Some(destination) => destination.write_all(&pdf),
    };
    record_export(&app, "office", Some("pdf"), export_outcome(&result));
    result.map_err(|error| error.to_string())
}

/// Saves a copy of the Office file itself wherever the person chooses. The
/// bytes are read again through the project's trusted-root gates, so a file the
/// project withholds cannot be copied out this way.
#[tauri::command]
pub async fn project_office_save_original(
    app: tauri::AppHandle,
    project_id: String,
    path: String,
) -> Result<DocumentExportResult, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let (root, relative) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let extension = office_convert::known_extension(relative.as_path())
        .ok_or_else(|| "This path is not a supported Office document.".to_owned())?;
    let bytes = tokio::task::spawn_blocking(move || {
        super::viewer::read_office_source(
            &root,
            &SecretClassifier::built_in(),
            &relative,
            read_scope_id(),
        )
        .map(|source| source.bytes)
    })
    .await
    .map_err(|_| WORKER_STOPPED.to_owned())?
    .map_err(|error| error.to_string())?;
    let result = match choose_destination(&app, &suggested_name(&path), extension, "Office file")
        .await
        .map_err(|error| error.to_string())?
    {
        None => Ok(DocumentExportResult::cancelled()),
        Some(destination) => destination.write_all(&bytes),
    };
    record_export(&app, "office", Some(extension), export_outcome(&result));
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_owner(window: &str, account: &str) -> PreviewOwner {
        PreviewOwner {
            window: window.to_owned(),
            account: ValidatedAccountId::parse(account).unwrap(),
        }
    }

    fn kept(id: Uuid, owner: PreviewOwner) -> KeptPdf {
        KeptPdf {
            id,
            owner,
            project_id: "project-1".to_owned(),
            path: "deck.pptx".to_owned(),
            pdf: Arc::new(b"%PDF-1.7".to_vec()),
            kept_at: Instant::now(),
        }
    }

    #[tokio::test]
    async fn replacement_cancels_old_work_but_waits_for_its_worker_to_finish() {
        let jobs = OfficePreviewJobs::default();
        let first = jobs.begin(test_owner("main", "account-a")).unwrap();
        let worker = first.acquire().await.unwrap();
        let next = jobs.begin(test_owner("main", "account-a")).unwrap();
        assert!(first.cancel.is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), next.acquire())
                .await
                .is_err()
        );
        drop(first);
        assert!(!next.cancel.is_cancelled());
        drop(worker);
        let _next_worker = tokio::time::timeout(Duration::from_secs(1), next.acquire())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancellation_is_bound_to_the_registered_window_and_account() {
        let jobs = OfficePreviewJobs::default();
        let owner = test_owner("main", "account-a");
        let lease = jobs.begin(owner.clone()).unwrap();
        jobs.cancel(&test_owner("other", "account-a"), lease.id)
            .unwrap();
        jobs.cancel(&test_owner("main", "account-b"), lease.id)
            .unwrap();
        jobs.cancel(&owner, Uuid::new_v4()).unwrap();
        assert!(!lease.cancel.is_cancelled());
        jobs.cancel(&owner, lease.id).unwrap();
        assert!(lease.acquire().await.is_err());
    }

    #[test]
    fn dropped_lease_cancels_worker_and_releases_registration() {
        let jobs = OfficePreviewJobs::default();
        let lease = jobs.begin(test_owner("main", "account-a")).unwrap();
        let cancel = lease.cancel.clone();
        drop(lease);
        assert!(cancel.is_cancelled());
        assert!(jobs.active.lock().unwrap().is_none());
    }

    #[test]
    fn a_kept_pdf_is_only_for_the_preview_that_made_it() {
        let jobs = OfficePreviewJobs::default();
        let owner = test_owner("main", "account-a");
        let id = Uuid::new_v4();
        jobs.keep(kept(id, owner.clone())).unwrap();

        assert_eq!(
            jobs.kept_pdf(&owner, id, "project-1", "deck.pptx")
                .unwrap()
                .as_slice(),
            b"%PDF-1.7"
        );
        for (who, which, project, path) in [
            (
                test_owner("other", "account-a"),
                id,
                "project-1",
                "deck.pptx",
            ),
            (
                test_owner("main", "account-b"),
                id,
                "project-1",
                "deck.pptx",
            ),
            (owner.clone(), Uuid::new_v4(), "project-1", "deck.pptx"),
            (owner.clone(), id, "project-2", "deck.pptx"),
            (owner.clone(), id, "project-1", "other.pptx"),
        ] {
            assert_eq!(
                jobs.kept_pdf(&who, which, project, path).unwrap_err(),
                PREVIEW_FIRST
            );
        }
    }

    #[test]
    fn a_kept_pdf_expires_and_is_dropped_by_a_new_preview_or_a_cancel() {
        let jobs = OfficePreviewJobs::default();
        let owner = test_owner("main", "account-a");

        let id = Uuid::new_v4();
        let mut stale = kept(id, owner.clone());
        stale.kept_at = Instant::now() - PDF_KEEP_FOR - Duration::from_secs(1);
        jobs.keep(stale).unwrap();
        assert!(jobs.kept_pdf(&owner, id, "project-1", "deck.pptx").is_err());

        jobs.keep(kept(id, owner.clone())).unwrap();
        jobs.cancel(&owner, id).unwrap();
        assert!(jobs.kept_pdf(&owner, id, "project-1", "deck.pptx").is_err());

        jobs.keep(kept(id, owner.clone())).unwrap();
        let _next = jobs.begin(owner.clone()).unwrap();
        assert!(jobs.kept_pdf(&owner, id, "project-1", "deck.pptx").is_err());
    }

    fn cached(account: &str, project: &str, path: &str, source: u8, size: usize) -> CachedPdf {
        CachedPdf {
            account: ValidatedAccountId::parse(account).unwrap(),
            project_id: project.to_owned(),
            path: path.to_owned(),
            source: [source; 32],
            pdf: Arc::new(vec![0; size]),
        }
    }

    fn lookup(
        cache: &mut PdfCache,
        account: &str,
        project: &str,
        path: &str,
        source: u8,
    ) -> Option<Arc<Vec<u8>>> {
        cache.get(
            &ValidatedAccountId::parse(account).unwrap(),
            project,
            path,
            &[source; 32],
        )
    }

    #[test]
    fn a_new_preview_drops_pdfs_made_for_another_account() {
        let jobs = OfficePreviewJobs::default();
        {
            let mut cache = jobs.cache.lock().unwrap();
            cache.put(cached("account-a", "project-1", "deck.pptx", 1, 10));
            cache.put(cached("account-b", "project-1", "deck.pptx", 1, 10));
        }
        let _lease = jobs.begin(test_owner("main", "account-b")).unwrap();
        let mut cache = jobs.cache.lock().unwrap();
        assert!(lookup(&mut cache, "account-a", "project-1", "deck.pptx", 1).is_none());
        assert!(lookup(&mut cache, "account-b", "project-1", "deck.pptx", 1).is_some());
    }

    #[test]
    fn a_cached_pdf_is_reused_only_for_the_same_file_version_and_account() {
        let mut cache = PdfCache::default();
        cache.put(cached("account-a", "project-1", "deck.pptx", 1, 10));

        assert!(lookup(&mut cache, "account-a", "project-1", "deck.pptx", 1).is_some());
        // Any change to the bytes, or another file, project or account, misses.
        assert!(lookup(&mut cache, "account-a", "project-1", "deck.pptx", 2).is_none());
        assert!(lookup(&mut cache, "account-a", "project-1", "other.pptx", 1).is_none());
        assert!(lookup(&mut cache, "account-a", "project-2", "deck.pptx", 1).is_none());
        assert!(lookup(&mut cache, "account-b", "project-1", "deck.pptx", 1).is_none());
    }

    #[test]
    fn a_new_version_of_a_file_replaces_its_old_pdf() {
        let mut cache = PdfCache::default();
        cache.put(cached("account-a", "project-1", "deck.pptx", 1, 10));
        cache.put(cached("account-a", "project-1", "deck.pptx", 2, 10));

        assert!(lookup(&mut cache, "account-a", "project-1", "deck.pptx", 1).is_none());
        assert!(lookup(&mut cache, "account-a", "project-1", "deck.pptx", 2).is_some());
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn the_cache_drops_its_least_recently_used_pdf_past_its_count_or_size_limit() {
        let mut cache = PdfCache::default();
        for index in 0..PDF_CACHE_ENTRIES {
            cache.put(cached(
                "account-a",
                "project-1",
                &format!("f{index}.docx"),
                1,
                10,
            ));
        }
        // Using f0 makes f1 the oldest.
        assert!(lookup(&mut cache, "account-a", "project-1", "f0.docx", 1).is_some());
        cache.put(cached("account-a", "project-1", "extra.docx", 1, 10));
        assert_eq!(cache.entries.len(), PDF_CACHE_ENTRIES);
        assert!(lookup(&mut cache, "account-a", "project-1", "f1.docx", 1).is_none());
        assert!(lookup(&mut cache, "account-a", "project-1", "f0.docx", 1).is_some());

        let mut by_size = PdfCache::default();
        let big = PDF_CACHE_BYTES / 2 + 1;
        by_size.put(cached("account-a", "project-1", "a.pptx", 1, big));
        by_size.put(cached("account-a", "project-1", "b.pptx", 1, big));
        assert_eq!(by_size.entries.len(), 1);
        assert!(lookup(&mut by_size, "account-a", "project-1", "b.pptx", 1).is_some());
    }

    #[test]
    fn the_saved_name_starts_from_the_files_own_name() {
        assert_eq!(suggested_name("reports/Q3 deck.pptx"), "Q3 deck");
        assert_eq!(suggested_name("a.b.docx"), "a.b");
        assert_eq!(suggested_name(""), "document");
    }
}
