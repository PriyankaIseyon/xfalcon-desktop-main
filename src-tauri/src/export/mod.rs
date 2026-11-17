//! Saving a composed document to a place the person chose
//! (spec: lean-report-export).
//!
//! # Why the dialog opens here and not in the webview
//!
//! This is the only command in the app that can write outside app-local data
//! and outside a trusted project root, so the destination must not be
//! something a caller can name. The renderer sends BYTES, a format, and a
//! suggested file name; the native save dialog opens on this side, and the path
//! comes back from the OS. A renderer that has been turned against the user can
//! therefore ask for a write, but it cannot choose where the write lands — a
//! person has to pick the file first, every single time. There is no silent
//! write and no way to accumulate one from repeated calls.
//!
//! It is deliberately ONE command for every file the app hands over — the
//! export document, a grid's CSV, a chart's SVG. A per-surface writer would be
//! a per-surface chance for this rule to be weaker, and the blob-and-anchor
//! download it replaces had no destination step at all.
//!
//! The suggested name is still untrusted input: it seeds a control the OS
//! renders, so [`safe_file_name`] reduces it to a plain base name before it is
//! handed over. Whatever the person then types is the OS's business, except for
//! the extension, which this module re-asserts so a document promised as one
//! format cannot be written under another's suffix.

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri_plugin_dialog::DialogExt;
use thiserror::Error;
use xfalcon_contracts::export::{DocumentExportFormat, DocumentExportResult};

use crate::audit::{emit, AuditAction, AuditOutcome, AuditResourceType};

#[cfg(windows)]
mod pdf;

/// Ceiling on one exported document, well above any conversation this app can
/// hold: every message is already capped on the way into the projection, and
/// the renderer composes from that. A body over this is a bug or an attempt to
/// use the export path as a file-writing primitive, and both should be refused
/// rather than written.
const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

/// Longest suggested base name, before the extension. Windows' own limit is
/// 255 for the whole component; this leaves generous room for the suffix and
/// for a duplicate marker the OS may append.
const MAX_BASE_NAME_CHARS: usize = 120;

/// Used when the suggested name reduces to nothing a file system would accept.
const FALLBACK_BASE_NAME: &str = "document";

/// Names that cannot be used for a file on Windows at any extension.
const RESERVED_BASE_NAMES: [&str; 28] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    // Windows reserves the superscript spellings too, and `is_alphanumeric` keeps them.
    "com¹", "com²", "com³", "lpt¹", "lpt²", "lpt³",
];

#[derive(Debug, Error)]
pub enum DocumentExportError {
    #[error("This document is too large to export.")]
    TooLarge,
    #[error("There is nothing to export.")]
    Empty,
    #[error("The save dialog could not be opened.")]
    DialogUnavailable,
    #[error("That location cannot be written to.")]
    UnusableDestination,
    #[error("The document could not be saved.")]
    WriteFailed,
    #[error("This platform could not write a PDF.")]
    PrintFailed,
    #[error("This platform could not write a PDF.")]
    PrintTimedOut,
    /// Not shown to a person. The renderer treats this exact string as the
    /// signal to fall back to `window.print()` on engines that can open a
    /// real print panel. Windows never returns it.
    #[error("PRINT_VIA_DIALOG")]
    PrintViaDialog,
}

/// Writes `contents` wherever the person points the save dialog, under the
/// extension `format` names.
///
/// Returns a cancelled result — not an error — when the dialog is dismissed:
/// declining to save is an answer, and reporting it as a failure would tell
/// people they broke something by changing their mind.
#[tauri::command]
pub async fn document_export_save(
    app: tauri::AppHandle,
    suggested_name: String,
    contents: String,
    format: DocumentExportFormat,
) -> Result<DocumentExportResult, String> {
    let result = export_document(&app, &suggested_name, contents, format).await;
    record_export(
        &app,
        "document",
        Some(format.extension()),
        export_outcome(&result),
    );
    result.map_err(|error| error.to_string())
}

/// What an export attempt records. Nothing for a dismissed save: declining is
/// a decision, not an export.
pub(crate) fn export_outcome<E>(result: &Result<DocumentExportResult, E>) -> Option<AuditOutcome> {
    match result {
        Ok(saved) => saved.saved.then_some(AuditOutcome::Success),
        Err(_) => Some(AuditOutcome::Failure),
    }
}

/// Records one export that wrote a file or failed. `format` is the file
/// extension, recorded only when known; never a path or a file name.
pub(crate) fn record_export<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    kind: &str,
    format: Option<&str>,
    outcome: Option<AuditOutcome>,
) {
    let Some(outcome) = outcome else {
        return;
    };
    match format {
        Some(format) => emit::record_with_detail(
            app,
            AuditAction::DataExport,
            AuditResourceType::Export,
            kind,
            outcome,
            format,
        ),
        None => emit::record(
            app,
            AuditAction::DataExport,
            AuditResourceType::Export,
            kind,
            outcome,
        ),
    }
}

/// Writes a PDF of the calling webview's print sheet wherever the person
/// points the save dialog.
///
/// The renderer sends only a suggested name. The destination comes from the
/// OS, the same rule as [`document_export_save`]. On Windows the webview's
/// own print engine writes the file (`PrintToPdf`), which does not block the
/// renderer on a print preview. Other platforms return [`PrintViaDialog`] so
/// the renderer can fall back to a native print panel.
#[tauri::command]
pub async fn document_export_pdf(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    suggested_name: String,
) -> Result<DocumentExportResult, String> {
    let result = export_pdf(&app, &window, &suggested_name).await;
    record_export(&app, "pdf", Some("pdf"), pdf_export_outcome(&result));
    result.map_err(|error| error.to_string())
}

/// The print hand-off is not an attempt that failed. The renderer then opens
/// its own print panel, which writes the file outside this command.
fn pdf_export_outcome(
    result: &Result<DocumentExportResult, DocumentExportError>,
) -> Option<AuditOutcome> {
    if matches!(result, Err(DocumentExportError::PrintViaDialog)) {
        None
    } else {
        export_outcome(result)
    }
}

async fn export_pdf(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    suggested_name: &str,
) -> Result<DocumentExportResult, DocumentExportError> {
    #[cfg(not(windows))]
    {
        let _ = (app, window, suggested_name);
        Err(DocumentExportError::PrintViaDialog)
    }
    #[cfg(windows)]
    {
        let file_name = format!("{}.pdf", safe_file_name(suggested_name, FALLBACK_BASE_NAME));
        let Some(chosen) = ask_where_to_save_with_filter(app, &file_name, "PDF", "pdf").await?
        else {
            return Ok(DocumentExportResult::cancelled());
        };
        let destination = with_suffix(chosen, "pdf");
        pdf::write_pdf(window, &destination).await?;
        tracing::info!("document export: pdf written");
        Ok(DocumentExportResult::written(display_file_name(
            &destination,
            &file_name,
        )))
    }
}

async fn export_document(
    app: &tauri::AppHandle,
    suggested_name: &str,
    contents: String,
    format: DocumentExportFormat,
) -> Result<DocumentExportResult, DocumentExportError> {
    if contents.trim().is_empty() {
        return Err(DocumentExportError::Empty);
    }
    if contents.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentExportError::TooLarge);
    }

    let file_name = format!(
        "{}.{}",
        safe_file_name(suggested_name, FALLBACK_BASE_NAME),
        format.extension()
    );
    let Some(chosen) =
        ask_where_to_save_with_filter(app, &file_name, format.filter_label(), format.extension())
            .await?
    else {
        return Ok(DocumentExportResult::cancelled());
    };
    let destination = with_expected_extension(chosen, format);

    write_document(&destination, contents.as_bytes(), format.extension())?;
    tracing::info!(
        format = format.extension(),
        "document export: document saved"
    );
    Ok(DocumentExportResult::written(display_file_name(
        &destination,
        &file_name,
    )))
}

/// A place the reader picked for a file the caller composes or downloads itself
/// (a saved report), rather than one whose text the renderer sent.
///
/// The same rule as the document export applies: the dialog opens on this side
/// and the path comes back from the OS, so nothing a caller passes can aim the
/// write. Only the file's name is ever handed back to the renderer.
pub(crate) struct ChosenDestination {
    path: PathBuf,
    file_name: String,
    extension: &'static str,
}

/// Opens the save dialog for a file named after `suggested_name` and returns the
/// reader's choice, or `None` when the dialog was dismissed.
pub(crate) async fn choose_destination(
    app: &tauri::AppHandle,
    suggested_name: &str,
    extension: &'static str,
    filter_label: &str,
) -> Result<Option<ChosenDestination>, DocumentExportError> {
    let file_name = format!(
        "{}.{extension}",
        safe_file_name(suggested_name, FALLBACK_BASE_NAME)
    );
    let Some(chosen) =
        ask_where_to_save_with_filter(app, &file_name, filter_label, extension).await?
    else {
        return Ok(None);
    };
    let path = with_suffix(chosen, extension);
    Ok(Some(ChosenDestination {
        file_name: display_file_name(&path, &file_name),
        path,
        extension,
    }))
}

impl ChosenDestination {
    /// Writes `contents` beside the destination and replaces it atomically.
    pub(crate) fn write_all(
        &self,
        contents: &[u8],
    ) -> Result<DocumentExportResult, DocumentExportError> {
        write_document(&self.path, contents, self.extension)?;
        Ok(DocumentExportResult::written(self.file_name.clone()))
    }

    /// A temporary file beside the destination, for a body that arrives in
    /// pieces. It is removed on every path that does not reach [`Self::finish`].
    pub(crate) fn temp_file(&self) -> Result<tempfile::NamedTempFile, DocumentExportError> {
        create_export_temp(&self.path, self.extension)
    }

    /// Replaces the destination with the finished temporary file.
    pub(crate) fn finish(
        &self,
        temporary: tempfile::TempPath,
    ) -> Result<DocumentExportResult, DocumentExportError> {
        persist_export(temporary, &self.path)?;
        Ok(DocumentExportResult::written(self.file_name.clone()))
    }

    #[cfg(test)]
    pub(crate) fn at(path: PathBuf, extension: &'static str) -> Self {
        let file_name = display_file_name(&path, "file");
        Self {
            path,
            file_name,
            extension,
        }
    }
}

/// Writes beside the chosen destination, then atomically replaces it.
///
/// A direct write can truncate an existing document before a later disk error.
/// Keeping the temporary file on the same filesystem means a failed export
/// leaves the previous file byte-for-byte intact.
fn write_document(
    destination: &Path,
    contents: &[u8],
    extension: &str,
) -> Result<(), DocumentExportError> {
    let mut temporary = create_export_temp(destination, extension)?;
    temporary
        .write_all(contents)
        .and_then(|_| temporary.flush())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(log_write_failure)?;
    persist_export(temporary.into_temp_path(), destination)
}

pub(super) fn create_export_temp(
    destination: &Path,
    extension: &str,
) -> Result<tempfile::NamedTempFile, DocumentExportError> {
    let parent = destination
        .parent()
        .ok_or(DocumentExportError::UnusableDestination)?;
    tempfile::Builder::new()
        .prefix(".xfalcon-export-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(parent)
        .map_err(log_write_failure)
}

pub(super) fn persist_export(
    temporary: tempfile::TempPath,
    destination: &Path,
) -> Result<(), DocumentExportError> {
    // A temporary file is created readable by its owner alone. The reader chose
    // this place to keep the file, so it takes an ordinary document's mode.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644))
            .map_err(log_write_failure)?;
    }
    temporary
        .persist(destination)
        .map_err(|error| log_write_failure(error.error))
}

pub(super) fn log_write_failure(error: std::io::Error) -> DocumentExportError {
    // The path is deliberately absent: this line goes to a log file the
    // person may share, and where they keep their documents is not the app's
    // story to tell. The io error names the cause on its own.
    tracing::warn!(%error, "document export: write failed");
    DocumentExportError::WriteFailed
}

/// Opens the native save dialog and resolves to the chosen path, or `None` when
/// the person dismissed it.
///
/// The callback form rather than `blocking_save_file`: on some platforms the
/// dialog must be driven from the main thread, and blocking a command's task on
/// it risks deadlocking against the very loop that would dismiss it.
async fn ask_where_to_save_with_filter(
    app: &tauri::AppHandle,
    file_name: &str,
    filter_label: &str,
    extension: &str,
) -> Result<Option<PathBuf>, DocumentExportError> {
    let (reply, chosen) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save file")
        .set_file_name(file_name)
        .add_filter(filter_label, &[extension])
        .save_file(move |path| {
            let _ = reply.send(path);
        });
    let Some(path) = chosen
        .await
        .map_err(|_| DocumentExportError::DialogUnavailable)?
    else {
        return Ok(None);
    };
    path.into_path()
        .map(Some)
        .map_err(|_| DocumentExportError::UnusableDestination)
}

/// Re-asserts the format's suffix on whatever the person typed.
///
/// Appends rather than replaces when some other extension is present:
/// `report.2026.txt` becomes `report.2026.txt.md`, which keeps every character
/// they chose and still names the format honestly. Silently rewriting the tail
/// of a name someone typed is the worse surprise.
fn with_expected_extension(path: PathBuf, format: DocumentExportFormat) -> PathBuf {
    with_suffix(path, format.extension())
}

fn with_suffix(path: PathBuf, expected: &str) -> PathBuf {
    let already_correct = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected));
    if already_correct {
        return path;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{expected}"));
    path.with_file_name(name)
}

/// What to call the saved file in the confirmation the renderer shows. Falls
/// back to the name that was requested when the OS hands back a path whose
/// final component is not valid UTF-8, so the message is never empty.
fn display_file_name(destination: &Path, requested: &str) -> String {
    destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(requested)
        .to_owned()
}

/// Reduces an untrusted title to a plain file base name.
///
/// Keeps letters, digits, spaces, hyphens and underscores and drops the rest,
/// which removes path separators, `..`, NUL, and the characters Windows
/// forbids in a component in one pass rather than by blocklist. Runs of
/// whitespace collapse so a title full of newlines does not become a name full
/// of gaps, and a name that is reserved on Windows, empty, or all dots falls
/// back to `fallback`.
fn safe_file_name(raw: &str, fallback: &str) -> String {
    let mut base = String::new();
    let mut kept = 0usize;
    let mut pending_space = false;
    for character in raw.chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_') {
            // A separator carries its own character in, so the budget is checked
            // for BOTH before either is pushed. Checking afterwards lets the pair
            // land one past the cap, which is a name a filesystem may refuse.
            let needs_space = pending_space && !base.is_empty();
            let width = if needs_space { 2 } else { 1 };
            if kept + width > MAX_BASE_NAME_CHARS {
                break;
            }
            if needs_space {
                base.push(' ');
                kept += 1;
            }
            pending_space = false;
            base.push(character);
            kept += 1;
            continue;
        }
        // Every rejected character — a separator, a comma, a newline — becomes
        // at most one space, so words stay words instead of running together.
        pending_space = true;
    }

    let trimmed = base.trim();
    if trimmed.is_empty() || RESERVED_BASE_NAMES.contains(&trimmed.to_ascii_lowercase().as_str()) {
        return fallback.to_owned();
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_superscript_device_name_is_reserved_like_its_plain_spelling() {
        for name in ["COM¹", "lpt²", "Com³"] {
            assert_eq!(safe_file_name(name, "fallback"), "fallback", "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_exported_file_is_an_ordinary_documents_mode_not_its_temporary_ones() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("report.html");
        let temporary = create_export_temp(&target, "html").unwrap();
        std::fs::write(&temporary, b"<p>x</p>").unwrap();
        persist_export(temporary.into_temp_path(), &target).unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }

    #[test]
    fn a_title_becomes_a_plain_base_name() {
        assert_eq!(
            safe_file_name("Quarterly review", "fallback"),
            "Quarterly review"
        );
        assert_eq!(safe_file_name("  padded  ", "fallback"), "padded");
        assert_eq!(
            safe_file_name("keep-hyphen_and_underscore", "fallback"),
            "keep-hyphen_and_underscore"
        );
    }

    #[test]
    fn separators_and_traversal_cannot_survive_a_title() {
        for hostile in [
            "../../etc/passwd",
            "..\\..\\Windows\\System32\\drivers\\etc\\hosts",
            "/absolute/path",
            "C:\\Users\\someone\\startup.bat",
            "\\\\server\\share\\file",
        ] {
            let safe = safe_file_name(hostile, "fallback");
            assert!(
                !safe.contains('/'),
                "{hostile} kept a forward slash: {safe}"
            );
            assert!(!safe.contains('\\'), "{hostile} kept a backslash: {safe}");
            assert!(!safe.contains(".."), "{hostile} kept a traversal: {safe}");
            assert!(
                !safe.contains(':'),
                "{hostile} kept a drive separator: {safe}"
            );
        }
    }

    #[test]
    fn control_characters_and_wildcards_are_dropped() {
        assert_eq!(safe_file_name("a\0b\nc\td", "fallback"), "a b c d");
        assert_eq!(
            safe_file_name("wild*card?name\"quoted\"", "fallback"),
            "wild card name quoted"
        );
    }

    #[test]
    fn a_name_with_nothing_usable_in_it_falls_back() {
        for empty in ["", "   ", "...", "///", "***", "\n\n"] {
            assert_eq!(safe_file_name(empty, "fallback"), "fallback");
        }
    }

    #[test]
    fn windows_reserved_names_fall_back_whatever_their_case() {
        for reserved in ["con", "CON", "NUL", "com1", "LPT9", " aux "] {
            assert_eq!(
                safe_file_name(reserved, "fallback"),
                "fallback",
                "{reserved} must not become a file name"
            );
        }
        // Only the whole name is reserved; the word inside a longer title is not.
        assert_eq!(
            safe_file_name("Consulting notes", "fallback"),
            "Consulting notes"
        );
    }

    #[test]
    fn a_long_title_is_capped_without_splitting_a_character() {
        let long = "é".repeat(MAX_BASE_NAME_CHARS * 2);
        let safe = safe_file_name(&long, "fallback");
        assert_eq!(safe.chars().count(), MAX_BASE_NAME_CHARS);
        assert!(safe.chars().all(|character| character == 'é'));
    }

    #[test]
    fn the_cap_counts_the_spaces_it_inserts() {
        // A title of single letters separated by punctuation must not exceed the
        // cap by counting only the letters it kept.
        let alternating = "a.".repeat(MAX_BASE_NAME_CHARS);
        assert!(safe_file_name(&alternating, "fallback").chars().count() <= MAX_BASE_NAME_CHARS);
    }

    #[test]
    fn each_format_asserts_its_own_extension() {
        for (format, expected) in [
            (DocumentExportFormat::Markdown, "md"),
            (DocumentExportFormat::Csv, "csv"),
            (DocumentExportFormat::Svg, "svg"),
            (DocumentExportFormat::Html, "html"),
        ] {
            assert_eq!(format.extension(), expected);
            assert_eq!(
                with_expected_extension(PathBuf::from("/tmp/notes"), format),
                PathBuf::from(format!("/tmp/notes.{expected}"))
            );
        }
        assert_eq!(
            with_suffix(PathBuf::from("/tmp/notes"), "pdf"),
            PathBuf::from("/tmp/notes.pdf")
        );
        assert_eq!(
            with_suffix(PathBuf::from("/tmp/notes.PDF"), "pdf"),
            PathBuf::from("/tmp/notes.PDF")
        );
    }

    #[test]
    fn the_expected_extension_is_asserted_on_whatever_was_typed() {
        let markdown = DocumentExportFormat::Markdown;
        assert_eq!(
            with_expected_extension(PathBuf::from("/tmp/notes.md"), markdown),
            PathBuf::from("/tmp/notes.md")
        );
        assert_eq!(
            with_expected_extension(PathBuf::from("/tmp/notes.MD"), markdown),
            PathBuf::from("/tmp/notes.MD"),
            "an uppercase suffix is already Markdown and must not be doubled"
        );
        assert_eq!(
            with_expected_extension(PathBuf::from("/tmp/report.2026.txt"), markdown),
            PathBuf::from("/tmp/report.2026.txt.md"),
            "an unrelated extension is kept and suffixed, never rewritten"
        );
        assert_eq!(
            with_expected_extension(PathBuf::from("/tmp/rows.csv"), DocumentExportFormat::Svg),
            PathBuf::from("/tmp/rows.csv.svg"),
            "a csv name under an svg export is still written as an svg"
        );
    }

    #[test]
    fn the_reported_name_is_the_one_that_was_written() {
        assert_eq!(
            display_file_name(
                &PathBuf::from("/home/a/Quarterly review.md"),
                "requested.md"
            ),
            "Quarterly review.md"
        );
        assert_eq!(
            display_file_name(&PathBuf::from("/"), "requested.md"),
            "requested.md",
            "a path with no final component falls back to what was asked for"
        );
    }

    #[test]
    fn an_abandoned_export_leaves_the_existing_destination_unchanged() {
        let directory = tempfile::tempdir().expect("temporary export directory");
        let destination = directory.path().join("report.md");
        std::fs::write(&destination, b"existing document").expect("existing document");

        {
            let mut temporary = create_export_temp(&destination, "md").expect("temporary export");
            temporary
                .write_all(b"incomplete replacement")
                .expect("partial export");
            assert_eq!(temporary.path().parent(), Some(directory.path()));
        }

        assert_eq!(
            std::fs::read(&destination).expect("preserved document"),
            b"existing document"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("export directory")
                .count(),
            1,
            "the abandoned sibling is removed"
        );
    }

    #[test]
    fn a_completed_export_atomically_replaces_the_existing_destination() {
        let directory = tempfile::tempdir().expect("temporary export directory");
        let destination = directory.path().join("report.md");
        std::fs::write(&destination, b"existing document").expect("existing document");

        write_document(&destination, b"complete replacement", "md").expect("completed export");

        assert_eq!(
            std::fs::read(&destination).expect("replacement document"),
            b"complete replacement"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("export directory")
                .count(),
            1,
            "only the chosen destination remains"
        );
    }

    #[test]
    fn a_chosen_destination_writes_whole_names_only_the_file_and_leaves_no_scrap() {
        let directory = tempfile::tempdir().expect("temporary export directory");
        let path = directory.path().join("Quarterly review.html");
        std::fs::write(&path, b"old page").expect("existing page");
        let destination = ChosenDestination::at(path.clone(), "html");

        let result = destination
            .write_all(b"<h1>new page</h1>")
            .expect("written");

        assert!(result.saved);
        // The renderer is told the file's name and nothing about where it is.
        assert_eq!(result.file_name.as_deref(), Some("Quarterly review.html"));
        assert_eq!(std::fs::read(&path).expect("page"), b"<h1>new page</h1>");
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("export directory")
                .count(),
            1
        );
    }

    #[test]
    fn a_streamed_destination_is_replaced_only_when_finished() {
        let directory = tempfile::tempdir().expect("temporary export directory");
        let path = directory.path().join("book.xlsx");
        std::fs::write(&path, b"old").expect("existing file");
        let destination = ChosenDestination::at(path.clone(), "xlsx");

        let mut temporary = destination.temp_file().expect("temp file");
        std::io::Write::write_all(&mut temporary, b"half of the new file").expect("write");
        // Until it is finished the destination still holds the old file.
        assert_eq!(std::fs::read(&path).expect("file"), b"old");
        // Dropping it unfinished (a failed download) removes the scrap.
        drop(temporary);
        assert_eq!(std::fs::read(&path).expect("file"), b"old");
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("export directory")
                .count(),
            1
        );

        let mut temporary = destination.temp_file().expect("temp file");
        std::io::Write::write_all(&mut temporary, b"the whole new file").expect("write");
        let result = destination
            .finish(temporary.into_temp_path())
            .expect("finished");
        assert!(result.saved);
        assert_eq!(std::fs::read(&path).expect("file"), b"the whole new file");
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use crate::audit::emit::test_hub;
    use tempfile::tempdir;

    #[test]
    fn a_cancelled_export_records_nothing() {
        let root = tempdir().expect("temp dir");
        let app = tauri::test::mock_app();
        let hub = test_hub(app.handle(), root.path());

        let cancelled: Result<DocumentExportResult, DocumentExportError> =
            Ok(DocumentExportResult::cancelled());
        record_export(
            app.handle(),
            "document",
            Some("md"),
            export_outcome(&cancelled),
        );

        assert!(hub.store().recent(10).expect("read audit log").is_empty());
    }

    #[test]
    fn a_written_export_records_its_format_and_never_the_file_name() {
        let root = tempdir().expect("temp dir");
        let app = tauri::test::mock_app();
        let hub = test_hub(app.handle(), root.path());

        let written: Result<DocumentExportResult, DocumentExportError> =
            Ok(DocumentExportResult::written("Q3 secret plan.md"));
        record_export(
            app.handle(),
            "document",
            Some("md"),
            export_outcome(&written),
        );

        let records = hub.store().recent(10).expect("read audit log");
        assert_eq!(records.len(), 1);
        let event = records[0].event();
        assert_eq!(event.action(), AuditAction::DataExport);
        assert_eq!(event.resource_type(), AuditResourceType::Export);
        assert_eq!(event.resource_id(), "document");
        assert_eq!(event.outcome(), AuditOutcome::Success);
        assert_eq!(event.detail(), Some("md"));
        let serialized = serde_json::to_string(event).expect("serialize event");
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn a_failed_export_is_a_failure_and_the_print_hand_off_is_not_an_attempt() {
        let failed: Result<DocumentExportResult, DocumentExportError> =
            Err(DocumentExportError::WriteFailed);
        assert_eq!(export_outcome(&failed), Some(AuditOutcome::Failure));

        let hand_off: Result<DocumentExportResult, DocumentExportError> =
            Err(DocumentExportError::PrintViaDialog);
        assert_eq!(pdf_export_outcome(&hand_off), None);
        assert_eq!(pdf_export_outcome(&failed), Some(AuditOutcome::Failure));
    }
}
