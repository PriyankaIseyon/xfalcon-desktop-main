//! The DA-facing flows of the Reports surface: list, open, thumbnail (show, and
//! capture for the owner), rename, pin, share, hide, delete, refresh, and the
//! two that carry a report off DA onto this device (export, and the page a
//! project copy is written from). Each takes the DA sessions it needs, so the
//! tests can point them at a loopback server.
//!
//! Every id-scoped flow answers a malformed id as `NotFound` before any request
//! leaves the device. Response bodies, tokens, SQL, and HTML are never logged
//! or returned in an error.

use tokio::io::AsyncWriteExt as _;
use xfalcon_contracts::export::DocumentExportResult;
use xfalcon_contracts::reports::{
    ReportCaptureDocument, ReportDatasetFreshness, ReportDocument, ReportErrorCode, ReportId,
    ReportListResult, ReportOperationError, ReportRefreshOutcome, ReportRefreshResult, ReportScope,
    ReportThumbnail, ReportThumbnailFormat, ReportView,
};
use xfalcon_contracts::CONTRACT_VERSION;

use crate::clients::da::generated::{KitPageResponse, KitSummaryResponse, ResponseKind};
use crate::clients::da::kits_api::{
    DaKitsApiError, DaKitsSession, KitPatchFields, ThumbnailFormat,
};
use crate::export::ChosenDestination;
use crate::preview::{PreviewDocumentStore, PreviewKind, RENDERER_PREVIEW_MAX_BYTES};

use super::kit_view::{
    clean_description, clean_title, kit_to_view, map_kits_error, valid_kit_id, valid_page_name,
};

/// The longest dataset failure text handed to the surface, in characters.
const MAX_OUTCOME_ERROR_CHARS: usize = 300;
/// The most an exported workbook or deck may hold. The body streams to disk, so
/// the bound is a disk guard, not a memory one. (A dashboard page needs no bound
/// of its own: the DA client already refuses a body past its 16 MiB cap.)
pub(crate) const EXPORT_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Every kit in the library, of every kind. Kits the reader hid are in the list,
/// flagged, so the surface can show them again.
pub(crate) async fn list(kits: &DaKitsSession) -> Result<ReportListResult, ReportOperationError> {
    let summaries = kits.list_kits().await.map_err(map_kits_error)?;
    let reports = summaries.iter().filter_map(kit_to_view).collect();
    Ok(ReportListResult::new(reports))
}

/// One page of a report, registered with the preview store. Only the id goes
/// back to the renderer.
///
/// `before_register` runs after the page is fetched and checked, and before it
/// is registered: an answer the caller refuses (its account has since changed)
/// must not stay servable from the preview store.
pub(crate) async fn open(
    kits: &DaKitsSession,
    preview: &PreviewDocumentStore,
    report_id: &str,
    page: Option<&str>,
    before_register: impl FnOnce() -> Result<(), ReportOperationError>,
) -> Result<ReportDocument, ReportOperationError> {
    let fetched = fetch_page(kits, report_id, page).await?;
    // Checked before registering, so an oversized page is refused and never
    // truncated. A page the store would serve is one the frame can load.
    if fetched.html.len() > RENDERER_PREVIEW_MAX_BYTES {
        return Err(ReportOperationError::new(ReportErrorCode::TooLargeToOpen));
    }
    before_register()?;
    let document_id = preview
        .register(PreviewKind::Html, fetched.html)
        .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;

    Ok(ReportDocument {
        version: CONTRACT_VERSION,
        id: ReportId::from(report_id.to_string()),
        title: fetched.kit_name,
        // Left empty on purpose: the library list already holds the description,
        // and the viewer does not need a second copy.
        description: None,
        page: fetched.page,
        pages: fetched.pages,
        document_id,
        datasets: fetched
            .datasets
            .unwrap_or_default()
            .into_iter()
            .map(|dataset| ReportDatasetFreshness {
                version: CONTRACT_VERSION,
                dataset_id: dataset.dataset_id,
                executed_at_ms: dataset
                    .executed_at
                    .as_deref()
                    .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                    .map(|parsed| parsed.timestamp_millis()),
                row_count: dataset
                    .row_count
                    .map(|rows| u64::try_from(rows.max(0)).unwrap_or(0)),
            })
            .collect(),
    })
}

/// One assembled page of a dashboard, checked to be a page of this report.
///
/// Shared by open, export, and the project copy, so the three refuse the same
/// answers: an unusable page name or id (a page that does not exist), DA's 422 for
/// a kit with no page, and a 2xx that names another report or carries no page.
async fn fetch_page(
    kits: &DaKitsSession,
    report_id: &str,
    page: Option<&str>,
) -> Result<KitPageResponse, ReportOperationError> {
    // An unusable page name is a page that does not exist, answered like one.
    if !valid_kit_id(report_id) || page.is_some_and(|name| !valid_page_name(name)) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let fetched = kits
        .get_page(report_id, page)
        .await
        .map_err(|err| match err {
            // DA answers 422 for a kit with no page to assemble.
            DaKitsApiError::Validation => ReportOperationError::new(ReportErrorCode::NotFound),
            other => map_kits_error(other),
        })?;
    // The DTO fills absent fields with empty strings, so a 2xx without a page or
    // its name would otherwise register a blank document and show an empty report.
    if fetched.html.is_empty() || fetched.page.is_empty() {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    // The page of another report is not this report's page.
    if !same_kit(&fetched.kit_id, report_id) {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    Ok(fetched)
}

/// Renames a report and sets its description. The description is always sent,
/// so clearing it clears it on DA too.
pub(crate) async fn rename(
    kits: &DaKitsSession,
    report_id: &str,
    title: &str,
    description: Option<&str>,
) -> Result<ReportView, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let title = clean_title(title)?;
    let patch = KitPatchFields {
        name: Some(title),
        description: Some(clean_description(description)?),
        ..KitPatchFields::default()
    };
    let patched = kits
        .patch_kit(report_id, patch)
        .await
        .map_err(|err| match err {
            // On a rename, a 422 is DA refusing the name or the description.
            DaKitsApiError::Validation => ReportOperationError::new(ReportErrorCode::InvalidTitle),
            other => map_kits_error(other),
        })?;
    view_or_not_found(&patched, report_id)
}

/// Pins or unpins a report for this viewer only.
pub(crate) async fn set_pinned(
    kits: &DaKitsSession,
    report_id: &str,
    pinned: bool,
) -> Result<ReportView, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let patch = KitPatchFields {
        pinned: Some(pinned),
        ..KitPatchFields::default()
    };
    let patched = kits
        .patch_kit(report_id, patch)
        .await
        .map_err(map_kits_error)?;
    view_or_not_found(&patched, report_id)
}

/// Shares a report with the organization, or makes it private again. DA allows it
/// to the owner with the save grant; the surface only offers it on `can_manage`.
pub(crate) async fn set_scope(
    kits: &DaKitsSession,
    report_id: &str,
    scope: ReportScope,
) -> Result<ReportView, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let patch = KitPatchFields {
        scope: Some(match scope {
            ReportScope::User => "user",
            ReportScope::Global => "global",
        }),
        ..KitPatchFields::default()
    };
    let patched = kits
        .patch_kit(report_id, patch)
        .await
        .map_err(map_kits_error)?;
    view_or_not_found(&patched, report_id)
}

/// Hides a report from this reader's own list, or shows it again. Per viewer, as
/// a pin is: no other reader's list changes.
pub(crate) async fn set_hidden(
    kits: &DaKitsSession,
    report_id: &str,
    hidden: bool,
) -> Result<ReportView, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let patch = KitPatchFields {
        hidden: Some(hidden),
        ..KitPatchFields::default()
    };
    let patched = kits
        .patch_kit(report_id, patch)
        .await
        .map_err(map_kits_error)?;
    view_or_not_found(&patched, report_id)
}

/// DA's own thumbnail capture, vendored verbatim from DA (`mcp_server/reports/
/// assets`): its pinned and patched `html-to-image`, then the script that renders
/// the page with it and answers its parent once. Using DA's pair means a report's
/// image looks the same whichever surface made it.
const CAPTURE_RENDERER: &str = include_str!("capture_assets/html-to-image-1.11.13.js");
const CAPTURE_SCRIPT: &str = include_str!("capture_assets/thumbnail-capture.js");
/// The largest PNG DA stores as a thumbnail.
const MAX_THUMBNAIL_PNG_BYTES: usize = 200 * 1024;

/// A dashboard's first page, with DA's capture script, registered for a frame
/// that is never shown. The renderer connects to the script with the returned
/// nonce and hands the one image it sends to [`save_thumbnail`].
///
/// `before_register` is checked as in [`open`].
pub(crate) async fn open_for_capture(
    kits: &DaKitsSession,
    preview: &PreviewDocumentStore,
    report_id: &str,
    before_register: impl FnOnce() -> Result<(), ReportOperationError>,
) -> Result<ReportCaptureDocument, ReportOperationError> {
    let fetched = fetch_page(kits, report_id, None).await?;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let html = with_capture_script(&fetched.html, &nonce);
    if html.len() > RENDERER_PREVIEW_MAX_BYTES {
        return Err(ReportOperationError::new(ReportErrorCode::TooLargeToOpen));
    }
    before_register()?;
    let document_id = preview
        .register(PreviewKind::Html, html)
        .map_err(|_| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
    Ok(ReportCaptureDocument {
        version: CONTRACT_VERSION,
        id: ReportId::from(report_id.to_string()),
        document_id,
        nonce,
    })
}

/// The page with the capture script first in its `<head>`, so the script is
/// installed before any of the page's own and keeps its reply port from them.
/// A page with no `<head>` takes it after `<html>`, and one with neither after
/// its doctype: never before a doctype, which would put the page in quirks mode
/// and change how it lays out.
fn with_capture_script(html: &str, nonce: &str) -> String {
    let script = format!(
        "<script data-capture-nonce=\"{nonce}\">{}\n{}</script>",
        escape_script_end(CAPTURE_RENDERER),
        escape_script_end(CAPTURE_SCRIPT)
    );
    let at = capture_insertion_point(html);
    let mut page = String::with_capacity(html.len() + script.len());
    page.push_str(&html[..at]);
    page.push_str(&script);
    page.push_str(&html[at..]);
    page
}

/// Where the capture script goes: just past the first `<head>` tag, else the
/// first `<html>` tag, else a leading doctype, else the very start.
fn capture_insertion_point(html: &str) -> usize {
    // ASCII lowercasing keeps every byte offset, so positions found here index
    // the original page.
    let lower = html.to_ascii_lowercase();
    let after_tag = |name: &str| {
        let open = format!("<{name}");
        let mut from = 0;
        while let Some(found) = lower[from..].find(&open) {
            let start = from + found;
            let next = lower.as_bytes().get(start + open.len()).copied();
            // `<head` must not match `<header`.
            if matches!(
                next,
                Some(b'>' | b'/') | Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
            ) {
                return lower[start..].find('>').map(|end| start + end + 1);
            }
            from = start + open.len();
        }
        None
    };
    after_tag("head")
        .or_else(|| after_tag("html"))
        .or_else(|| {
            let lead = lower.len() - lower.trim_start().len();
            lower[lead..]
                .starts_with("<!doctype")
                .then(|| lower[lead..].find('>').map(|end| lead + end + 1))
                .flatten()
        })
        .unwrap_or(0)
}

/// A script's text with every `</script` broken, so the source cannot end the
/// element it is inlined in. Case-insensitive, as the HTML parser is.
fn escape_script_end(source: &str) -> String {
    let lower = source.to_ascii_lowercase();
    let mut escaped = String::with_capacity(source.len());
    let mut from = 0;
    while let Some(found) = lower[from..].find("</script") {
        let at = from + found;
        escaped.push_str(&source[from..at]);
        escaped.push_str("<\\/");
        from = at + 2;
    }
    escaped.push_str(&source[from..]);
    escaped
}

/// Stores a captured PNG as a report's thumbnail, for its owner.
///
/// The image comes from a page the reader's own report ran, so it is checked
/// here before anything is sent: standard base64 of a PNG within DA's bound,
/// else `InvalidThumbnail`. DA refuses a report the reader does not own as
/// `NotFound`, and checks the image again itself.
pub(crate) async fn save_thumbnail(
    kits: &DaKitsSession,
    report_id: &str,
    image_base64: &str,
) -> Result<ReportView, ReportOperationError> {
    use base64::Engine as _;

    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let invalid = || ReportOperationError::new(ReportErrorCode::InvalidThumbnail);
    if image_base64.len() > MAX_THUMBNAIL_PNG_BYTES.div_ceil(3) * 4 {
        return Err(invalid());
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(image_base64)
        .map_err(|_| invalid())?;
    if png.is_empty() || png.len() > MAX_THUMBNAIL_PNG_BYTES || !ThumbnailFormat::Png.signs(&png) {
        return Err(invalid());
    }
    let saved = kits
        .put_thumbnail(report_id, png)
        .await
        .map_err(|err| match err {
            DaKitsApiError::Validation | DaKitsApiError::PayloadTooLarge => invalid(),
            other => map_kits_error(other),
        })?;
    if !same_kit(&saved.kit_id, report_id) {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    kit_to_view(&saved)
        .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))
}

/// A report's stored thumbnail image. `NotFound` when DA holds none, so the
/// card draws the report itself instead.
pub(crate) async fn thumbnail(
    kits: &DaKitsSession,
    report_id: &str,
) -> Result<ReportThumbnail, ReportOperationError> {
    use base64::Engine as _;

    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let image = kits
        .get_thumbnail(report_id)
        .await
        .map_err(map_kits_error)?;
    Ok(ReportThumbnail {
        version: CONTRACT_VERSION,
        id: ReportId::from(report_id.to_string()),
        format: match image.format {
            ThumbnailFormat::Png => ReportThumbnailFormat::Png,
            ThumbnailFormat::Webp => ReportThumbnailFormat::Webp,
        },
        data_base64: base64::engine::general_purpose::STANDARD.encode(image.bytes),
    })
}

pub(crate) async fn delete(
    kits: &DaKitsSession,
    report_id: &str,
) -> Result<(), ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    kits.delete_kit(report_id).await.map_err(map_kits_error)
}

/// Re-runs every dataset of a report on DA. A dataset that fails is an outcome,
/// not an error, so the others still show their new rows.
pub(crate) async fn refresh(
    kits: &DaKitsSession,
    report_id: &str,
) -> Result<ReportRefreshResult, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let refreshed = kits.refresh_kit(report_id).await.map_err(|err| match err {
        // DA answers 422 when the report has no stored queries to re-run.
        DaKitsApiError::Validation => ReportOperationError::new(ReportErrorCode::NotRefreshable),
        other => map_kits_error(other),
    })?;
    // The outcomes describe the kit DA names; they must be this report's.
    if !same_kit(&refreshed.kit.kit_id, report_id) {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    let report = kit_to_view(&refreshed.kit)
        .ok_or_else(|| ReportOperationError::new(ReportErrorCode::StorageUnavailable))?;
    let outcomes = refreshed
        .outcomes
        .into_iter()
        .map(|outcome| ReportRefreshOutcome {
            version: CONTRACT_VERSION,
            dataset_id: outcome.dataset_id,
            ok: outcome.ok,
            rows: outcome.rows.unwrap_or(0).max(0) as u64,
            duration_ms: duration_ms(outcome.seconds),
            data_source: outcome
                .data_source_name
                .map(|name| bounded_error_text(&name))
                .filter(|name| !name.is_empty()),
            error: outcome.error.map(|text| bounded_error_text(&text)),
        })
        .collect();
    Ok(ReportRefreshResult {
        version: CONTRACT_VERSION,
        report,
        outcomes,
    })
}

/// DA reports a dataset's time as fractional seconds. A negative or non-finite
/// value is not a duration, so it reads as none rather than a wrapped number.
fn duration_ms(seconds: Option<f64>) -> u64 {
    match seconds {
        Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
            // Rounded, and clamped so a hostile value cannot wrap: one day is
            // already far past any refresh DA allows.
            (seconds * 1000.0).round().min(86_400_000.0) as u64
        }
        _ => 0,
    }
}

/// What an export will write, fetched before the save dialog opens so a failure
/// never opens a dialog for nothing.
pub(crate) enum ExportPlan {
    /// A dashboard page, already fetched: the assembled, self-contained HTML.
    Page { title: String, html: String },
    /// A workbook or deck. Its file is downloaded after the reader has chosen a
    /// place, because a connection held open while they choose would go stale.
    File {
        title: String,
        extension: &'static str,
        filter_label: &'static str,
    },
}

/// Decides what exporting a report writes, and fetches it when it is a page.
pub(crate) async fn plan_export(
    kits: &DaKitsSession,
    report_id: &str,
    page: Option<&str>,
) -> Result<ExportPlan, ReportOperationError> {
    if !valid_kit_id(report_id) || page.is_some_and(|name| !valid_page_name(name)) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let kit = kits.get_kit(report_id).await.map_err(map_kits_error)?;
    if !same_kit(&kit.kit_id, report_id) {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    match kit.kind {
        ResponseKind::Dashboard => {
            let fetched = fetch_page(kits, report_id, page).await?;
            Ok(ExportPlan::Page {
                title: fetched.kit_name,
                html: fetched.html,
            })
        }
        ResponseKind::Workbook => Ok(ExportPlan::File {
            title: kit.name,
            extension: "xlsx",
            filter_label: "Excel workbook",
        }),
        ResponseKind::Presentation => Ok(ExportPlan::File {
            title: kit.name,
            extension: "pptx",
            filter_label: "PowerPoint presentation",
        }),
    }
}

/// One dashboard page for a project copy: the assembled HTML as DA holds it,
/// with the title DA names it by.
pub(crate) async fn page_for_project(
    kits: &DaKitsSession,
    report_id: &str,
    page: Option<&str>,
) -> Result<(String, String), ReportOperationError> {
    let fetched = fetch_page(kits, report_id, page).await?;
    Ok((fetched.kit_name, fetched.html))
}

/// Streams a workbook's or deck's filled file into the reader's chosen place.
///
/// The body goes to a temporary file beside the destination, which replaces it
/// only once every byte has arrived and the byte ceiling held. Any failure
/// removes the temporary file and leaves an existing destination untouched.
pub(crate) async fn download_file(
    kits: &DaKitsSession,
    report_id: &str,
    destination: &ChosenDestination,
) -> Result<DocumentExportResult, ReportOperationError> {
    download_file_within(kits, report_id, destination, EXPORT_MAX_FILE_BYTES).await
}

/// [`download_file`] with its byte ceiling named, so a test can reach it without
/// a quarter-gigabyte body.
async fn download_file_within(
    kits: &DaKitsSession,
    report_id: &str,
    destination: &ChosenDestination,
    max_bytes: u64,
) -> Result<DocumentExportResult, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let failed = || ReportOperationError::new(ReportErrorCode::ExportFailed);
    let mut download = kits.open_file(report_id).await.map_err(map_kits_error)?;
    if download
        .declared_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(failed());
    }
    let temporary = destination.temp_file().map_err(|_| failed())?;
    let mut sink = tokio::fs::File::from_std(temporary.reopen().map_err(|_| failed())?);
    let declared = download.declared_length();
    let mut written: u64 = 0;
    let mut head = Vec::with_capacity(ZIP_MAGIC.len());
    // A connection that fails mid-body is a failed export, not a storage problem.
    while let Some(chunk) = download.chunk().await.map_err(|_| failed())? {
        written += chunk.len() as u64;
        if written > max_bytes {
            return Err(failed());
        }
        if head.len() < ZIP_MAGIC.len() {
            let take = (ZIP_MAGIC.len() - head.len()).min(chunk.len());
            head.extend_from_slice(&chunk[..take]);
        }
        sink.write_all(&chunk).await.map_err(|_| failed())?;
    }
    // An empty answer is not a workbook, and writing it would replace a good file
    // with nothing. Neither is a page of HTML a proxy or a sign-in wall sent in its
    // place: a workbook and a deck are ZIP packages, so anything else is refused.
    if written == 0 || head != ZIP_MAGIC {
        return Err(failed());
    }
    // A body shorter or longer than the length it announced was cut or altered.
    if declared.is_some_and(|length| length != written) {
        return Err(failed());
    }
    sink.flush().await.map_err(|_| failed())?;
    sink.sync_all().await.map_err(|_| failed())?;
    drop(sink);
    destination
        .finish(temporary.into_temp_path())
        .map_err(|_| failed())
}

/// The first bytes of every `.xlsx` and `.pptx`: both are ZIP packages.
const ZIP_MAGIC: [u8; 4] = *b"PK\x03\x04";

/// Which file a workbook or deck report is, and which version of it DA holds.
pub(crate) struct OfficeFileVersion {
    pub(crate) extension: &'static str,
    /// Names this exact version: when DA last changed the kit and how much it
    /// holds. A different marker is a different file, so a preview made from the
    /// same marker can be reused without fetching the file again.
    pub(crate) marker: String,
}

/// Reads what a preview needs to know about a workbook or deck report, without
/// its file. A dashboard has no file, so it reads as not found.
pub(crate) async fn office_file_version(
    kits: &DaKitsSession,
    report_id: &str,
) -> Result<OfficeFileVersion, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let kit = kits.get_kit(report_id).await.map_err(map_kits_error)?;
    if !same_kit(&kit.kit_id, report_id) {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    let extension = match kit.kind {
        ResponseKind::Workbook => "xlsx",
        ResponseKind::Presentation => "pptx",
        ResponseKind::Dashboard => {
            return Err(ReportOperationError::new(ReportErrorCode::NotFound));
        }
    };
    Ok(OfficeFileVersion {
        extension,
        marker: format!(
            "{}|{}|{}",
            kit.kit_id.to_ascii_lowercase(),
            kit.updated_at.as_deref().unwrap_or_default(),
            kit.total_payload_bytes
        ),
    })
}

/// A workbook's or deck's filled file, held in memory for a preview. Held to
/// the same checks as a download: a ZIP package of the length DA announced, and
/// never more than `max_bytes`, so a preview cannot take more memory than the
/// bound.
pub(crate) async fn read_office_file(
    kits: &DaKitsSession,
    report_id: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, ReportOperationError> {
    if !valid_kit_id(report_id) {
        return Err(ReportOperationError::new(ReportErrorCode::NotFound));
    }
    let too_large = || ReportOperationError::new(ReportErrorCode::TooLargeToOpen);
    let unusable = || ReportOperationError::new(ReportErrorCode::StorageUnavailable);
    let mut download = kits.open_file(report_id).await.map_err(map_kits_error)?;
    let declared = download.declared_length();
    if declared.is_some_and(|length| length > max_bytes) {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(declared.map_or(0, |length| length as usize));
    while let Some(chunk) = download.chunk().await.map_err(|_| unusable())? {
        if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty()
        || !bytes.starts_with(&ZIP_MAGIC)
        || declared.is_some_and(|length| length != bytes.len() as u64)
    {
        return Err(unusable());
    }
    Ok(bytes)
}

/// Whether DA's answer names the kit that was asked for. DA mints lowercase
/// UUIDs and a caller's id may differ only in hex case.
fn same_kit(answered: &str, requested: &str) -> bool {
    answered.eq_ignore_ascii_case(requested)
}

/// A dataset's failure text is DA's and unbounded; the surface shows a line of it.
fn bounded_error_text(text: &str) -> String {
    let mut chars = text.trim().chars();
    let head: String = chars.by_ref().take(MAX_OUTCOME_ERROR_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn view_or_not_found(
    kit: &KitSummaryResponse,
    requested: &str,
) -> Result<ReportView, ReportOperationError> {
    // The answer must be for the kit that was patched: a card is never updated
    // with another report's title and state.
    if !same_kit(&kit.kit_id, requested) {
        return Err(ReportOperationError::new(
            ReportErrorCode::StorageUnavailable,
        ));
    }
    // A kit that is not a dashboard is not a report, so it reads as not found.
    kit_to_view(kit).ok_or_else(|| ReportOperationError::new(ReportErrorCode::NotFound))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{any, body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use xfalcon_contracts::reports::ReportKind;

    /// DA mints kit ids as UUIDs, and the flows refuse anything else.
    const KIT: &str = "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01";
    const WORKBOOK: &str = "0c5a1d2e-7b3f-4e8a-8d26-9a41c7e0b5f2";

    fn kit_json(kit_id: &str) -> serde_json::Value {
        serde_json::json!({
            "kit_id": kit_id,
            "name": "Pipeline",
            "kind": "dashboard",
            "scope": "user",
            "owner_user_id": "user-1",
            "is_owner": true,
            "refreshable": true,
            "can_refresh": true,
            "can_manage": true,
            "can_delete": true,
            "page_count": 1,
            "dataset_count": 1,
            "total_payload_bytes": 10,
            "pinned": false
        })
    }

    /// A server that must receive no request at all.
    async fn silent_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn a_malformed_id_is_not_found_without_a_request() {
        let server = silent_server().await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();

        // DA ids are UUIDs: `.` and `..` would be dropped by the URL builder and
        // name a different route, and `kit-1` names nothing DA ever issued.
        for id in ["", "kit\n1", "kit-1", ".", "..", "%2e%2e", "a/../b"] {
            assert_eq!(
                open(&kits, &preview, id, None, || Ok(()))
                    .await
                    .unwrap_err()
                    .code,
                ReportErrorCode::NotFound,
                "open {id:?}"
            );
            assert_eq!(
                rename(&kits, id, "Name", None).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "rename {id:?}"
            );
            assert_eq!(
                set_pinned(&kits, id, true).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "pin {id:?}"
            );
            assert_eq!(
                delete(&kits, id).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "delete {id:?}"
            );
            assert_eq!(
                thumbnail(&kits, id).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "thumbnail {id:?}"
            );
            assert_eq!(
                open_for_capture(&kits, &preview, id, || Ok(()))
                    .await
                    .unwrap_err()
                    .code,
                ReportErrorCode::NotFound,
                "capture {id:?}"
            );
            assert_eq!(
                save_thumbnail(&kits, id, "iVBORw0KGgo=")
                    .await
                    .unwrap_err()
                    .code,
                ReportErrorCode::NotFound,
                "save thumbnail {id:?}"
            );
            assert_eq!(
                refresh(&kits, id).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "refresh {id:?}"
            );
            assert_eq!(
                set_scope(&kits, id, ReportScope::Global)
                    .await
                    .unwrap_err()
                    .code,
                ReportErrorCode::NotFound,
                "share {id:?}"
            );
            assert_eq!(
                set_hidden(&kits, id, true).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "hide {id:?}"
            );
            assert_eq!(
                plan_export(&kits, id, None).await.err().map(|e| e.code),
                Some(ReportErrorCode::NotFound),
                "export {id:?}"
            );
            assert_eq!(
                page_for_project(&kits, id, None).await.unwrap_err().code,
                ReportErrorCode::NotFound,
                "project copy {id:?}"
            );
            let destination =
                ChosenDestination::at(std::env::temp_dir().join("never-written.xlsx"), "xlsx");
            assert_eq!(
                download_file(&kits, id, &destination)
                    .await
                    .unwrap_err()
                    .code,
                ReportErrorCode::NotFound,
                "download {id:?}"
            );
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn a_stored_thumbnail_reaches_the_card_as_base64_and_a_missing_one_is_not_found() {
        use base64::Engine as _;

        let png = b"\x89PNG\r\n\x1a\nIHDR".to_vec();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/thumbnail")))
            .respond_with(ResponseTemplate::new(200).set_body_raw(png.clone(), "image/png"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/user/report-kits/{WORKBOOK}/thumbnail"
            )))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();

        let shown = thumbnail(&kits, KIT).await.expect("thumbnail");
        assert_eq!(shown.id.as_str(), KIT);
        assert_eq!(shown.format, ReportThumbnailFormat::Png);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(shown.data_base64)
                .expect("standard base64"),
            png
        );
        assert_eq!(
            thumbnail(&kits, WORKBOOK).await.unwrap_err().code,
            ReportErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn an_empty_page_body_is_a_failed_open_not_a_blank_report() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/pages")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();
        assert_eq!(
            open(&kits, &preview, KIT, None, || Ok(()))
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::StorageUnavailable
        );
    }

    #[tokio::test]
    async fn an_unusable_page_name_is_not_found_without_a_request() {
        let server = silent_server().await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();
        let overlong = "p".repeat(257);
        for page in ["", "a\nb", overlong.as_str()] {
            assert_eq!(
                open(&kits, &preview, KIT, Some(page), || Ok(()))
                    .await
                    .unwrap_err()
                    .code,
                ReportErrorCode::NotFound,
                "{page:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_422_means_what_the_operation_means_by_it() {
        let server = MockServer::start().await;
        for suffix in ["", "/pages", "/refresh"] {
            for verb in ["GET", "PATCH", "POST"] {
                Mock::given(method(verb))
                    .and(path(format!("/api/v1/user/report-kits/{KIT}{suffix}")))
                    .respond_with(ResponseTemplate::new(422))
                    .mount(&server)
                    .await;
            }
        }
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();

        // A kit with no stored queries cannot be refreshed.
        assert_eq!(
            refresh(&kits, KIT).await.unwrap_err().code,
            ReportErrorCode::NotRefreshable
        );
        // A name or description DA refuses is an invalid name.
        assert_eq!(
            rename(&kits, KIT, "Name", Some("text"))
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::InvalidTitle
        );
        // A kit with nothing to assemble has no such page.
        assert_eq!(
            open(&kits, &preview, KIT, None, || Ok(()))
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::NotFound
        );
        // On a pin it is not a name problem: it reads as storage.
        assert_eq!(
            set_pinned(&kits, KIT, true).await.unwrap_err().code,
            ReportErrorCode::StorageUnavailable
        );
    }

    #[tokio::test]
    async fn a_description_over_the_bound_is_refused_before_any_request() {
        let server = silent_server().await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let long = "d".repeat(4001);
        assert_eq!(
            rename(&kits, KIT, "Name", Some(&long))
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::InvalidTitle
        );
    }

    #[tokio::test]
    async fn open_registers_the_fetched_page_and_omits_its_description() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/pages")))
            .and(wiremock::matchers::query_param("page", "detail.html"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kit_name": "Pipeline",
                "page": "detail.html",
                "pages": ["index.html", "detail.html"],
                "html": "<h1>Detail</h1>",
                "size_bytes": 15,
                "sha256": "abc"
            })))
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();
        let document = open(&kits, &preview, KIT, Some("detail.html"), || Ok(()))
            .await
            .unwrap();
        assert_eq!(document.title, "Pipeline");
        assert_eq!(document.description, None);
        assert_eq!(document.page, "detail.html");
        assert_eq!(document.pages, ["index.html", "detail.html"]);
        let (_, served) = preview.serve(&document.document_id).expect("registered");
        assert_eq!(served, "<h1>Detail</h1>");
    }

    #[tokio::test]
    async fn open_refuses_a_page_over_the_preview_ceiling() {
        let server = MockServer::start().await;
        let oversized = "x".repeat(RENDERER_PREVIEW_MAX_BYTES + 1);
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/pages")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kit_name": "Pipeline",
                "page": "index.html",
                "pages": ["index.html"],
                "html": oversized,
                "size_bytes": oversized.len(),
                "sha256": "abc"
            })))
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();
        assert_eq!(
            open(&kits, &preview, KIT, None, || Ok(()))
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::TooLargeToOpen
        );
    }

    #[tokio::test]
    async fn rename_always_sends_the_description_so_clearing_clears_it() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}")))
            .and(body_partial_json(serde_json::json!({
                "name": "Renamed",
                "description": null
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json(KIT)))
            .expect(1)
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let view = rename(&kits, KIT, "  Renamed ", Some("   ")).await.unwrap();
        assert_eq!(view.title, "Pipeline");
        server.verify().await;
    }

    #[tokio::test]
    async fn an_empty_name_is_refused_before_any_request() {
        let server = silent_server().await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        assert_eq!(
            rename(&kits, KIT, "   ", None).await.unwrap_err().code,
            ReportErrorCode::InvalidTitle
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn refresh_reports_each_dataset_outcome() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/refresh")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kind": "dashboard",
                "outcomes": [
                    {"dataset_id": "sales", "ok": true, "rows": 12},
                    {"dataset_id": "stock", "ok": false, "error": "timeout"}
                ],
                "kit": kit_json(KIT)
            })))
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let result = refresh(&kits, KIT).await.unwrap();
        assert_eq!(result.report.id.as_str(), KIT);
        assert_eq!(result.outcomes.len(), 2);
        assert!(result.outcomes[0].ok);
        assert_eq!(result.outcomes[0].rows, 12);
        assert!(!result.outcomes[1].ok);
        assert_eq!(result.outcomes[1].rows, 0);
        assert_eq!(result.outcomes[1].error.as_deref(), Some("timeout"));
    }

    const OTHER: &str = "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a99";

    async fn page_server(kit_id: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/pages")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": kit_id,
                "kit_name": "Pipeline",
                "page": "index.html",
                "pages": ["index.html"],
                "html": "<h1>Page</h1>",
                "size_bytes": 13,
                "sha256": "abc"
            })))
            .mount(&server)
            .await;
        server
    }

    #[test]
    fn the_vendored_renderer_is_the_one_da_pinned() {
        use sha2::Digest as _;

        let pin: serde_json::Value = serde_json::from_str(include_str!(
            "capture_assets/html-to-image-1.11.13.provenance.json"
        ))
        .expect("provenance");
        assert_eq!(
            sha2::Sha256::digest(CAPTURE_RENDERER.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            pin["sha256"].as_str().expect("pinned digest")
        );
    }

    #[test]
    fn the_capture_script_goes_first_in_the_head_and_never_before_a_doctype() {
        let at = |html: &str| {
            let page = with_capture_script(html, "n0");
            let script = page
                .find("<script data-capture-nonce=\"n0\">")
                .expect("injected");
            (page, script)
        };
        let (page, script) =
            at("<!DOCTYPE html><html><HEAD lang=x><script>author()</script></head></html>");
        assert_eq!(&page[..script], "<!DOCTYPE html><html><HEAD lang=x>");
        assert!(script < page.find("author()").expect("author script"));
        // `<header>` is not the head.
        let (page, script) = at("<!doctype html><html><body><header>Top</header></body></html>");
        assert_eq!(&page[..script], "<!doctype html><html>");
        let (page, script) = at("  <!doctype html><p>No head or html</p>");
        assert_eq!(&page[..script], "  <!doctype html>");
        let (_, script) = at("<h1>Fragment</h1>");
        assert_eq!(script, 0);
        // The page itself is unchanged around the script.
        let html = "<html><head><title>T</title></head><body>B</body></html>";
        let page = with_capture_script(html, "n0");
        let end = page
            .find("</script><title>")
            .expect("script closes once, right before the title");
        assert_eq!(
            format!(
                "{}{}",
                &page[..page.find("<script").unwrap()],
                &page[end + 9..]
            ),
            html
        );
    }

    #[test]
    fn an_inlined_script_cannot_end_its_own_element() {
        assert_eq!(
            escape_script_end("a</script>b</SCRIPT >c"),
            "a<\\/script>b<\\/SCRIPT >c"
        );
        let page = with_capture_script("<head></head>", "n0");
        assert_eq!(page.to_ascii_lowercase().matches("</script").count(), 1);
    }

    #[tokio::test]
    async fn a_capture_page_carries_the_script_and_a_fresh_nonce() {
        let server = page_server(KIT).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();

        let first = open_for_capture(&kits, &preview, KIT, || Ok(()))
            .await
            .expect("capture page");
        let second = open_for_capture(&kits, &preview, KIT, || Ok(()))
            .await
            .expect("capture page");
        assert_eq!(first.id.as_str(), KIT);
        assert_eq!(first.nonce.len(), 32);
        assert_ne!(first.nonce, second.nonce);
        let (kind, html) = preview.serve(&first.document_id).expect("registered");
        assert_eq!(kind, PreviewKind::Html);
        assert!(html.starts_with(&format!("<script data-capture-nonce=\"{}\">", first.nonce)));
        assert!(html.ends_with("</script><h1>Page</h1>"));
        assert!(html.contains("ida-thumbnail-connect"));
    }

    #[tokio::test]
    async fn a_captured_image_is_checked_before_it_is_sent() {
        use base64::Engine as _;

        // Nothing may reach DA for any of these.
        let server = silent_server().await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let png_header = b"\x89PNG\r\n\x1a\n".to_vec();
        for image in [
            String::new(),
            "not base64!".to_owned(),
            encode(b"RIFF\x10\x00\x00\x00WEBPVP8 "),
            encode(b"<svg onload=alert(1)>"),
            encode(&[png_header.clone(), vec![0; MAX_THUMBNAIL_PNG_BYTES]].concat()),
            "A".repeat(MAX_THUMBNAIL_PNG_BYTES.div_ceil(3) * 4 + 4),
        ] {
            assert_eq!(
                save_thumbnail(&kits, KIT, &image).await.unwrap_err().code,
                ReportErrorCode::InvalidThumbnail
            );
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn a_saved_thumbnail_returns_the_report_with_its_new_image_version() {
        use base64::Engine as _;

        let png = b"\x89PNG\r\n\x1a\nIHDR".to_vec();
        let mut kit = kit_json(KIT);
        kit["thumbnail_updated_at"] = serde_json::json!("2026-10-12T10:00:00Z");
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/thumbnail")))
            .and(wiremock::matchers::body_bytes(png.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!(
                "/api/v1/user/report-kits/{WORKBOOK}/thumbnail"
            )))
            .respond_with(ResponseTemplate::new(422))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let image = base64::engine::general_purpose::STANDARD.encode(&png);

        let saved = save_thumbnail(&kits, KIT, &image).await.expect("saved");
        assert_eq!(saved.id.as_str(), KIT);
        assert_eq!(saved.thumbnail_updated_at_ms, Some(1_791_799_200_000));
        // DA refusing the image is the image's fault, said as such.
        assert_eq!(
            save_thumbnail(&kits, WORKBOOK, &image)
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::InvalidThumbnail
        );
    }

    #[tokio::test]
    async fn a_page_that_belongs_to_another_report_is_not_opened() {
        let server = page_server(OTHER).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();

        let error = open(&kits, &preview, KIT, None, || Ok(()))
            .await
            .unwrap_err();

        assert_eq!(error.code, ReportErrorCode::StorageUnavailable);
        assert_eq!(preview.resident_bytes(), 0);
    }

    #[tokio::test]
    async fn an_open_the_caller_refuses_leaves_nothing_registered() {
        let server = page_server(KIT).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();

        let error = open(&kits, &preview, KIT, None, || {
            Err(ReportOperationError::new(
                ReportErrorCode::StorageUnavailable,
            ))
        })
        .await
        .unwrap_err();

        assert_eq!(error.code, ReportErrorCode::StorageUnavailable);
        assert_eq!(preview.resident_bytes(), 0);
    }

    #[tokio::test]
    async fn a_refresh_that_answers_for_another_report_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/refresh")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": OTHER,
                "kind": "dashboard",
                "outcomes": [{"dataset_id": "sales", "ok": true, "rows": 1}],
                "kit": kit_json(OTHER)
            })))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();

        let error = refresh(&kits, KIT).await.unwrap_err();

        assert_eq!(error.code, ReportErrorCode::StorageUnavailable);
    }

    #[test]
    fn a_long_dataset_error_is_cut_with_an_ellipsis() {
        assert_eq!(bounded_error_text("  timeout  "), "timeout");
        let exact = "e".repeat(MAX_OUTCOME_ERROR_CHARS);
        assert_eq!(bounded_error_text(&exact), exact);
        let cut = bounded_error_text(&"e".repeat(MAX_OUTCOME_ERROR_CHARS + 50));
        assert_eq!(cut.chars().count(), MAX_OUTCOME_ERROR_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[tokio::test]
    async fn refresh_hands_the_surface_a_bounded_dataset_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/refresh")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kind": "dashboard",
                "outcomes": [
                    {"dataset_id": "sales", "ok": false, "error": "x".repeat(5000)}
                ],
                "kit": kit_json(KIT)
            })))
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let result = refresh(&kits, KIT).await.unwrap();
        let error = result.outcomes[0].error.as_deref().unwrap();
        assert_eq!(error.chars().count(), MAX_OUTCOME_ERROR_CHARS + 1);
    }

    #[tokio::test]
    async fn list_keeps_every_kind_and_asks_for_hidden_kits_too() {
        let server = MockServer::start().await;
        let mut workbook = kit_json(WORKBOOK);
        workbook["kind"] = serde_json::json!("workbook");
        let mut hidden = kit_json(OTHER);
        hidden["hidden"] = serde_json::json!(true);
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(wiremock::matchers::query_param("include_hidden", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [kit_json(KIT), workbook, hidden],
                "total": 3,
                "limit": 100,
                "offset": 0
            })))
            .expect(1)
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let result = list(&kits).await.unwrap();
        assert_eq!(result.reports.len(), 3);
        assert_eq!(result.reports[0].kind, ReportKind::Dashboard);
        assert_eq!(result.reports[1].kind, ReportKind::Workbook);
        assert!(!result.reports[0].hidden);
        assert!(result.reports[2].hidden);
        server.verify().await;
    }

    #[tokio::test]
    async fn sharing_and_hiding_send_exactly_their_own_field() {
        let server = MockServer::start().await;
        for body in [
            serde_json::json!({ "scope": "global" }),
            serde_json::json!({ "scope": "user" }),
            serde_json::json!({ "hidden": true }),
            serde_json::json!({ "hidden": false }),
        ] {
            Mock::given(method("PATCH"))
                .and(path(format!("/api/v1/user/report-kits/{KIT}")))
                .and(wiremock::matchers::body_json(body))
                .respond_with(ResponseTemplate::new(200).set_body_json(kit_json(KIT)))
                .expect(1)
                .mount(&server)
                .await;
        }
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        set_scope(&kits, KIT, ReportScope::Global).await.unwrap();
        set_scope(&kits, KIT, ReportScope::User).await.unwrap();
        set_hidden(&kits, KIT, true).await.unwrap();
        set_hidden(&kits, KIT, false).await.unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn sharing_a_report_the_reader_does_not_own_is_forbidden() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        assert_eq!(
            set_scope(&kits, KIT, ReportScope::Global)
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::Forbidden
        );
    }

    #[tokio::test]
    async fn open_reports_when_each_dataset_was_stored() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/pages")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kit_name": "Pipeline",
                "page": "index.html",
                "pages": ["index.html"],
                "html": "<h1>Page</h1>",
                "size_bytes": 13,
                "sha256": "abc",
                "datasets": [
                    {"dataset_id": "sales", "executed_at": "2026-10-01T10:00:00Z", "row_count": 42},
                    {"dataset_id": "returns", "executed_at": null, "row_count": null},
                    {"dataset_id": "odd", "executed_at": "yesterday-ish", "row_count": -5}
                ]
            })))
            .mount(&server)
            .await;

        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();
        let document = open(&kits, &preview, KIT, None, || Ok(())).await.unwrap();
        assert_eq!(document.datasets.len(), 3);
        assert_eq!(document.datasets[0].dataset_id, "sales");
        assert_eq!(document.datasets[0].executed_at_ms, Some(1_790_848_800_000));
        assert_eq!(document.datasets[0].row_count, Some(42));
        // A dataset DA never stored reads as unknown, never as the epoch or zero.
        assert_eq!(document.datasets[1].executed_at_ms, None);
        assert_eq!(document.datasets[1].row_count, None);
        // A time or count DA sent badly is unknown or zero, never a wrapped number.
        assert_eq!(document.datasets[2].executed_at_ms, None);
        assert_eq!(document.datasets[2].row_count, Some(0));
    }

    #[tokio::test]
    async fn a_static_page_has_no_dataset_freshness() {
        let server = page_server(KIT).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let preview = PreviewDocumentStore::default();
        let document = open(&kits, &preview, KIT, None, || Ok(())).await.unwrap();
        assert!(document.datasets.is_empty());
    }

    #[tokio::test]
    async fn refresh_hands_over_each_datasets_time_and_source() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/refresh")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kind": "dashboard",
                "outcomes": [
                    {"dataset_id": "sales", "ok": true, "rows": 12, "seconds": 1.2504,
                     "data_source_name": "warehouse"},
                    {"dataset_id": "stock", "ok": true, "rows": 3, "seconds": -4.0,
                     "data_source_name": "   "},
                    {"dataset_id": "none", "ok": true}
                ],
                "kit": kit_json(KIT)
            })))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let result = refresh(&kits, KIT).await.unwrap();
        assert_eq!(result.outcomes[0].duration_ms, 1250);
        assert_eq!(result.outcomes[0].data_source.as_deref(), Some("warehouse"));
        // A negative time is no duration, and a blank source is no source.
        assert_eq!(result.outcomes[1].duration_ms, 0);
        assert_eq!(result.outcomes[1].data_source, None);
        assert_eq!(result.outcomes[2].duration_ms, 0);
        assert_eq!(result.outcomes[2].data_source, None);
    }

    #[test]
    fn a_duration_never_wraps_or_goes_negative() {
        assert_eq!(duration_ms(Some(0.0004)), 0);
        assert_eq!(duration_ms(Some(2.0)), 2000);
        assert_eq!(duration_ms(Some(f64::NAN)), 0);
        assert_eq!(duration_ms(Some(f64::INFINITY)), 0);
        // Finite but absurd is clamped to a day, not wrapped.
        assert_eq!(duration_ms(Some(f64::MAX)), 86_400_000);
        assert_eq!(duration_ms(Some(1.0e12)), 86_400_000);
        assert_eq!(duration_ms(None), 0);
    }

    async fn kit_of_kind_server(kind: &str) -> MockServer {
        let server = MockServer::start().await;
        let mut kit = kit_json(KIT);
        kit["kind"] = serde_json::json!(kind);
        kit["name"] = serde_json::json!("Quarterly numbers");
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn exporting_a_dashboard_fetches_its_assembled_page_first() {
        let server = kit_of_kind_server("dashboard").await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/pages")))
            .and(wiremock::matchers::query_param("page", "detail.html"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": KIT,
                "kit_name": "Pipeline",
                "page": "detail.html",
                "pages": ["index.html", "detail.html"],
                "html": "<h1>Detail</h1>",
                "size_bytes": 15,
                "sha256": "abc"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        match plan_export(&kits, KIT, Some("detail.html")).await.ok() {
            Some(ExportPlan::Page { title, html }) => {
                assert_eq!(title, "Pipeline");
                assert_eq!(html, "<h1>Detail</h1>");
            }
            _ => panic!("a dashboard exports its page"),
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn a_patch_answered_for_another_kit_updates_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json(OTHER)))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let wrong = ReportErrorCode::StorageUnavailable;

        assert_eq!(
            rename(&kits, KIT, "Name", None).await.unwrap_err().code,
            wrong
        );
        assert_eq!(set_pinned(&kits, KIT, true).await.unwrap_err().code, wrong);
        assert_eq!(
            set_scope(&kits, KIT, ReportScope::Global)
                .await
                .unwrap_err()
                .code,
            wrong
        );
        assert_eq!(set_hidden(&kits, KIT, true).await.unwrap_err().code, wrong);
    }

    #[tokio::test]
    async fn exporting_a_workbook_or_deck_names_its_file_and_fetches_nothing_yet() {
        for (kind, extension) in [("workbook", "xlsx"), ("presentation", "pptx")] {
            let server = kit_of_kind_server(kind).await;
            let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
            match plan_export(&kits, KIT, None).await.ok() {
                Some(ExportPlan::File {
                    title,
                    extension: planned,
                    ..
                }) => {
                    assert_eq!(title, "Quarterly numbers");
                    assert_eq!(planned, extension);
                }
                _ => panic!("{kind} exports its file"),
            }
            // Only the kit itself was read: the file is downloaded after the reader
            // has chosen where it goes.
            let received = server.received_requests().await.unwrap();
            assert_eq!(received.len(), 1, "{kind}");
        }
    }

    #[tokio::test]
    async fn an_export_that_answers_for_another_report_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json(OTHER)))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        assert_eq!(
            plan_export(&kits, KIT, None).await.err().map(|e| e.code),
            Some(ReportErrorCode::StorageUnavailable)
        );
    }

    #[tokio::test]
    async fn a_project_copy_takes_the_assembled_page_of_this_report() {
        let server = page_server(KIT).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let (title, html) = page_for_project(&kits, KIT, None).await.unwrap();
        assert_eq!(title, "Pipeline");
        assert_eq!(html, "<h1>Page</h1>");

        let other = page_server(OTHER).await;
        let kits = DaKitsSession::for_test(&other.uri(), "token").unwrap();
        assert_eq!(
            page_for_project(&kits, KIT, None).await.unwrap_err().code,
            ReportErrorCode::StorageUnavailable
        );
    }

    #[tokio::test]
    async fn a_preview_names_the_file_kind_and_a_version_without_fetching_the_file() {
        for (kind, extension) in [("workbook", "xlsx"), ("presentation", "pptx")] {
            let server = kit_of_kind_server(kind).await;
            let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
            let version = office_file_version(&kits, KIT).await.unwrap();
            assert_eq!(version.extension, extension);
            assert!(version.marker.starts_with(&KIT.to_ascii_lowercase()));
            assert_eq!(server.received_requests().await.unwrap().len(), 1, "{kind}");
        }
        // A dashboard has no file to preview, and a kit that answers for another
        // report is refused.
        let dashboard = kit_of_kind_server("dashboard").await;
        let kits = DaKitsSession::for_test(&dashboard.uri(), "token").unwrap();
        assert_eq!(
            office_file_version(&kits, KIT).await.err().map(|e| e.code),
            Some(ReportErrorCode::NotFound)
        );
    }

    #[tokio::test]
    async fn a_new_version_of_the_kit_is_a_different_marker() {
        let versions: Vec<String> = {
            let mut markers = Vec::new();
            for updated in ["2026-10-02T10:00:00+00:00", "2026-10-03T10:00:00+00:00"] {
                let server = MockServer::start().await;
                let mut kit = kit_json(KIT);
                kit["kind"] = serde_json::json!("presentation");
                kit["updated_at"] = serde_json::json!(updated);
                Mock::given(method("GET"))
                    .and(path(format!("/api/v1/user/report-kits/{KIT}")))
                    .respond_with(ResponseTemplate::new(200).set_body_json(kit))
                    .mount(&server)
                    .await;
                let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
                markers.push(office_file_version(&kits, KIT).await.unwrap().marker);
            }
            markers
        };
        assert_ne!(versions[0], versions[1]);
    }

    #[tokio::test]
    async fn a_preview_reads_a_whole_package_into_memory_within_its_bound() {
        let body = b"PK\x03\x04 the filled deck".to_vec();
        let server = file_server(body.clone()).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        assert_eq!(read_office_file(&kits, KIT, 1024).await.unwrap(), body);

        // Over the bound: refused as too large, before it is all read.
        assert_eq!(
            read_office_file(&kits, KIT, 10).await.unwrap_err().code,
            ReportErrorCode::TooLargeToOpen
        );
    }

    #[tokio::test]
    async fn a_preview_refuses_a_body_that_is_empty_or_not_a_package() {
        for body in [Vec::new(), b"<html>sign in</html>".to_vec()] {
            let server = file_server(body).await;
            let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
            assert_eq!(
                read_office_file(&kits, KIT, 1024).await.unwrap_err().code,
                ReportErrorCode::StorageUnavailable
            );
        }
    }

    /// A DA that serves `body` for the kit's file through a signed redirect.
    async fn file_server(body: Vec<u8>) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/file")))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/signed/file", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/signed/file"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
        server
    }

    fn directory_entries(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn a_downloaded_file_replaces_the_destination_only_once_whole() {
        let server = file_server(b"PK\x03\x04 the filled workbook".to_vec()).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Quarterly numbers.xlsx");
        std::fs::write(&target, b"old copy").unwrap();
        let destination = ChosenDestination::at(target.clone(), "xlsx");

        let result = download_file(&kits, KIT, &destination).await.unwrap();
        assert!(result.saved);
        assert_eq!(result.file_name.as_deref(), Some("Quarterly numbers.xlsx"));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"PK\x03\x04 the filled workbook"
        );
        // No temporary file is left beside it.
        assert_eq!(directory_entries(dir.path()), ["Quarterly numbers.xlsx"]);
    }

    #[tokio::test]
    async fn a_declared_length_over_the_ceiling_is_refused_before_anything_is_written() {
        let server = file_server(vec![7u8; 64]).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("deck.pptx");
        std::fs::write(&target, b"old copy").unwrap();
        let destination = ChosenDestination::at(target.clone(), "pptx");

        let error = download_file_within(&kits, KIT, &destination, 10)
            .await
            .unwrap_err();
        assert_eq!(error.code, ReportErrorCode::ExportFailed);
        assert_eq!(std::fs::read(&target).unwrap(), b"old copy");
        assert_eq!(directory_entries(dir.path()), ["deck.pptx"]);
    }

    /// A server that answers `/file` with a redirect to a body sent in chunks with
    /// NO declared length, so only the streaming ceiling can stop it.
    async fn chunked_file_server(chunks: Vec<Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let target = format!("{origin}/signed/file");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let target = target.clone();
                let chunks = chunks.clone();
                tokio::spawn(async move {
                    // The whole request head, however many reads it arrives in.
                    let mut head = Vec::new();
                    let mut buffer = [0u8; 1024];
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => break,
                            Ok(read) => head.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let request = String::from_utf8_lossy(&head).into_owned();
                    if request.contains("/signed/file") {
                        let _ = socket
                            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                            .await;
                        for chunk in chunks {
                            let _ = socket
                                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                                .await;
                            let _ = socket.write_all(&chunk).await;
                            let _ = socket.write_all(b"\r\n").await;
                        }
                        let _ = socket.write_all(b"0\r\n\r\n").await;
                    } else {
                        let reply = format!(
                            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = socket.write_all(reply.as_bytes()).await;
                    }
                });
            }
        });
        origin
    }

    #[tokio::test]
    async fn a_body_with_no_declared_length_is_stopped_by_the_streaming_ceiling() {
        let mut chunks = vec![vec![1u8; 8]; 4];
        chunks[0][..4].copy_from_slice(b"PK\x03\x04");
        let whole: Vec<u8> = chunks.concat();
        let origin = chunked_file_server(chunks).await;
        let kits = DaKitsSession::for_test(&origin, "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("deck.pptx");
        std::fs::write(&target, b"old copy").unwrap();
        let destination = ChosenDestination::at(target.clone(), "pptx");

        // 32 bytes arrive in four chunks against a 20-byte ceiling.
        let error = download_file_within(&kits, KIT, &destination, 20)
            .await
            .unwrap_err();
        assert_eq!(error.code, ReportErrorCode::ExportFailed);
        assert_eq!(std::fs::read(&target).unwrap(), b"old copy");
        assert_eq!(directory_entries(dir.path()), ["deck.pptx"]);

        // The same body within the ceiling is written whole.
        download_file_within(&kits, KIT, &destination, 64)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), whole);
    }

    #[tokio::test]
    async fn an_empty_download_never_replaces_a_good_file() {
        let server = file_server(Vec::new()).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("empty.xlsx");
        std::fs::write(&target, b"good copy").unwrap();
        let destination = ChosenDestination::at(target.clone(), "xlsx");

        assert_eq!(
            download_file(&kits, KIT, &destination)
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::ExportFailed
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"good copy");
        assert_eq!(directory_entries(dir.path()), ["empty.xlsx"]);
    }

    #[tokio::test]
    async fn a_page_of_html_in_place_of_the_file_never_replaces_a_good_one() {
        // A proxy or a sign-in wall answered 200 with a page.
        let server = file_server(b"<!doctype html><title>Sign in</title>".to_vec()).await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Quarterly.xlsx");
        std::fs::write(&target, b"good copy").unwrap();
        let destination = ChosenDestination::at(target.clone(), "xlsx");

        assert_eq!(
            download_file(&kits, KIT, &destination)
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::ExportFailed
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"good copy");
        assert_eq!(directory_entries(dir.path()), ["Quarterly.xlsx"]);
    }

    /// A target that announces `announced` bytes, sends `sent`, and closes.
    async fn short_file_server(announced: usize, sent: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let target = format!("{origin}/signed/file");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let target = target.clone();
                let sent = sent.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut buffer = [0u8; 1024];
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => break,
                            Ok(read) => head.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let request = String::from_utf8_lossy(&head).into_owned();
                    if request.contains("/signed/file") {
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {announced}\r\nConnection: close\r\n\r\n"
                        );
                        let _ = socket.write_all(reply.as_bytes()).await;
                        let _ = socket.write_all(&sent).await;
                    } else {
                        let reply = format!(
                            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = socket.write_all(reply.as_bytes()).await;
                    }
                });
            }
        });
        origin
    }

    #[tokio::test]
    async fn a_body_cut_short_is_a_failed_export_and_leaves_the_good_file() {
        let origin = short_file_server(4096, b"PK\x03\x04 only the start".to_vec()).await;
        let kits = DaKitsSession::for_test(&origin, "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("deck.pptx");
        std::fs::write(&target, b"good copy").unwrap();
        let destination = ChosenDestination::at(target.clone(), "pptx");

        // The connection ends mid-body: reported as a failed export, not as a
        // storage problem, and nothing replaces the old file.
        assert_eq!(
            download_file(&kits, KIT, &destination)
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::ExportFailed
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"good copy");
        assert_eq!(directory_entries(dir.path()), ["deck.pptx"]);
    }

    #[tokio::test]
    async fn a_report_with_no_file_says_it_was_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/user/report-kits/{KIT}/file")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let kits = DaKitsSession::for_test(&server.uri(), "token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let destination = ChosenDestination::at(dir.path().join("x.xlsx"), "xlsx");
        assert_eq!(
            download_file(&kits, KIT, &destination)
                .await
                .unwrap_err()
                .code,
            ReportErrorCode::NotFound
        );
        assert!(directory_entries(dir.path()).is_empty());
    }
}
