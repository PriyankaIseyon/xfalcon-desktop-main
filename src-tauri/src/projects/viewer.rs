//! The file view's own reads: a person looking at a file in their own trusted
//! project (spec: project-file-editing, "A local file opens at any size up to
//! 10 GiB").
//!
//! Nothing here goes to a model or a server — the bytes reach this device's own
//! webview — so the bounds are about the device, not a context window: a 10 GiB
//! ceiling on what is opened at all, and 8 MiB per range so one IPC reply stays
//! a size the webview handles in one piece. The agent keeps its own paged reader
//! (`super::read`), sized by its run's budget.
//!
//! Every call applies the same gates `super::read` does, fresh: the path is
//! classified for sensitivity before the file is touched, and containment and
//! root identity are revalidated by [`TrustedRoot`] on every open. Size and
//! modified time come from the opened handle, never from a separate lookup of
//! the path, so what is reported is what is read. Nothing is cached between
//! calls that could outlive a changed file.

use std::io::{Read, Seek, SeekFrom};
use std::time::UNIX_EPOCH;

use xfalcon_contracts::RunId;

use super::domain::{ContainmentError, TrustedRoot};
use super::path_policy::RelativeProjectPath;
use super::read::{sniff_content_kind, ContentKind, MetadataOnlyReason, SNIFF_BYTES};
use super::sensitive::{
    evaluate, SecretClass, SecretClassifier, SensitiveDecision, SensitiveReadScope,
};

/// The largest file the view opens. A safety ceiling, not a working limit:
/// past it a file shows its size and is not read.
pub const MAX_VIEWABLE_FILE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// The largest range one read returns. The view assembles a whole file from
/// ranges of this size, and pages a larger one a range at a time.
pub const MAX_READ_RANGE_BYTES: u32 = 8 * 1024 * 1024;

/// The largest document registered for preview from a project file. The
/// preview store's own per-document cap (`crate::preview::PREVIEW_DOC_MAX_BYTES`).
pub const MAX_PROJECT_PREVIEW_BYTES: u64 = crate::preview::PREVIEW_DOC_MAX_BYTES as u64;

#[derive(Debug, thiserror::Error)]
pub enum ViewerError {
    #[error(transparent)]
    Containment(#[from] ContainmentError),
    #[error("this path is classified as sensitive, so it is not read")]
    Sensitive,
    #[error("this path is not a regular file")]
    NotAFile,
    #[error("this file is larger than the {limit}-byte limit for this view")]
    TooLarge { limit: u64 },
    #[error("a read may return at most {MAX_READ_RANGE_BYTES} bytes")]
    RangeTooLong,
    #[error("offset {offset} is past the end of the file ({size} bytes)")]
    OffsetPastEnd { offset: u64, size: u64 },
    #[error("this file is not valid UTF-8 text")]
    NotText,
    #[error("a row of this table is longer than the {limit}-byte limit for one row")]
    RowTooLong { limit: u64 },
    #[error("io error while reading a trusted-project file: {0}")]
    Io(#[source] std::io::Error),
    #[error("Office document extraction failed: {0}")]
    Office(String),
    #[error("the extracted Office text contains binary data")]
    OfficeNotText,
    #[error("this Office file changed while it was read; refresh and try again")]
    ChangedDuringRead,
    #[error("this path is not a supported Office document")]
    NotOfficeDocument,
}

/// Original bytes and metadata read through the project's trusted-root gates.
/// The bytes may be passed to a local renderer, but never to an arbitrary path.
pub(super) struct OfficeSource {
    pub(super) bytes: Vec<u8>,
    pub(super) size_bytes: u64,
    pub(super) modified_ms: Option<i64>,
}

/// What [`stat`] found, before any byte beyond the sniff is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewerStat {
    Text {
        size_bytes: u64,
        modified_ms: Option<i64>,
    },
    NotText {
        size_bytes: u64,
        modified_ms: Option<i64>,
        reason: MetadataOnlyReason,
    },
    TooLarge {
        size_bytes: u64,
    },
    Sensitive(SecretClass),
}

/// Classifies `path` for sensitivity from the path alone, before anything is
/// opened — the same decision `super::read::read_file` makes. A person's read
/// carries no override, so a sensitive path is refused here.
fn sensitive_class(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
) -> Option<SecretClass> {
    let scope = SensitiveReadScope::new(*root.project_id(), path.clone(), run_id);
    match evaluate(classifier, scope, None) {
        SensitiveDecision::RequiresOverride(pending) => Some(pending.class()),
        SensitiveDecision::NotSensitive | SensitiveDecision::Overridden(_) => None,
    }
}

/// Opens `path` as a regular file and returns the handle with the metadata of
/// what was actually opened.
///
/// The path's own metadata is checked first, so a directory or a FIFO is never
/// opened (opening a FIFO for read would block). The handle's metadata is the
/// one callers use: a file swapped in between the two checks cannot pass its
/// size or time off as the one that is read.
fn open_regular(
    root: &TrustedRoot,
    path: &RelativeProjectPath,
) -> Result<(cap_std::fs::File, cap_std::fs::Metadata), ViewerError> {
    let resolved = root.resolve_existing(path.clone())?;
    if !root.file_metadata(&resolved)?.is_file() {
        return Err(ViewerError::NotAFile);
    }
    let file = root.open_file(&resolved)?;
    let metadata = file.metadata().map_err(ViewerError::Io)?;
    if !metadata.is_file() {
        return Err(ViewerError::NotAFile);
    }
    Ok((file, metadata))
}

/// Sniffs the first [`SNIFF_BYTES`] of `file`, then rewinds it.
fn sniff(file: &mut cap_std::fs::File) -> Result<ContentKind, ViewerError> {
    let mut prefix = Vec::with_capacity(SNIFF_BYTES);
    file.by_ref()
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut prefix)
        .map_err(ViewerError::Io)?;
    file.rewind().map_err(ViewerError::Io)?;
    Ok(sniff_content_kind(&prefix))
}

/// A text file the view may read, opened: the sensitivity gate, a regular
/// file, the view's size cap, and a text sniff. Returns the handle, rewound,
/// and its own metadata.
pub(super) fn open_text(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
) -> Result<(cap_std::fs::File, cap_std::fs::Metadata), ViewerError> {
    if sensitive_class(root, classifier, path, run_id).is_some() {
        return Err(ViewerError::Sensitive);
    }
    let (mut file, metadata) = open_regular(root, path)?;
    if metadata.len() > MAX_VIEWABLE_FILE_BYTES {
        return Err(ViewerError::TooLarge {
            limit: MAX_VIEWABLE_FILE_BYTES,
        });
    }
    match sniff(&mut file)? {
        ContentKind::Text => Ok((file, metadata)),
        ContentKind::NonUtf8Text | ContentKind::Binary | ContentKind::Oversized => {
            Err(ViewerError::NotText)
        }
    }
}

/// Reads original Office bytes through fresh sensitivity, trusted-root and
/// regular-file gates. Metadata from the opened handle brackets the read.
pub(super) fn read_office_source(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
) -> Result<OfficeSource, ViewerError> {
    if !super::office_convert::is_convertible_path(path.as_path()) {
        return Err(ViewerError::NotOfficeDocument);
    }
    if sensitive_class(root, classifier, path, run_id).is_some() {
        return Err(ViewerError::Sensitive);
    }
    let (mut file, before) = open_regular(root, path)?;
    let limit = crate::documents::MAX_DOCUMENT_BYTES;
    if before.len() > limit {
        return Err(ViewerError::TooLarge { limit });
    }
    let before_modified = modified_ms(&before);
    let mut bytes = Vec::with_capacity(usize::try_from(before.len()).unwrap_or(0));
    file.by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(ViewerError::Io)?;
    if bytes.len() as u64 > limit {
        return Err(ViewerError::TooLarge { limit });
    }
    let after = file.metadata().map_err(ViewerError::Io)?;
    let after_modified = modified_ms(&after);
    if bytes.len() as u64 != before.len()
        || !after.is_file()
        || before.len() != after.len()
        || before_modified != after_modified
    {
        return Err(ViewerError::ChangedDuringRead);
    }
    Ok(OfficeSource {
        bytes,
        size_bytes: before.len(),
        modified_ms: before_modified,
    })
}

/// Reads one bounded Office document through the same fresh trust gates as
/// `read_office_source`, then extracts text without changing the source bytes.
pub fn read_office_document(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
) -> Result<(String, u64, Option<i64>), ViewerError> {
    // Text extraction covers Word and PowerPoint files; the wider set that can be
    // previewed as a PDF includes Excel, which has no text reader here.
    if !crate::documents::is_office_path(path.as_path()) {
        return Err(ViewerError::NotOfficeDocument);
    }
    let source = read_office_source(root, classifier, path, run_id)?;
    let text = match crate::documents::extract(path.as_path(), &source.bytes) {
        Ok(extracted) => extracted.text,
        Err(crate::documents::DocumentError::NoText) => String::new(),
        Err(error) => return Err(ViewerError::Office(error.to_string())),
    };
    if text.as_bytes().contains(&0) {
        return Err(ViewerError::OfficeNotText);
    }
    Ok((text, source.size_bytes, source.modified_ms))
}

fn modified_ms(metadata: &cap_std::fs::Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?.into_std();
    let since_epoch = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

/// Size, modified time, and kind. Reads at most the sniff prefix.
pub fn stat(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
) -> Result<ViewerStat, ViewerError> {
    if let Some(class) = sensitive_class(root, classifier, path, run_id) {
        return Ok(ViewerStat::Sensitive(class));
    }
    let (mut file, metadata) = open_regular(root, path)?;
    let size_bytes = metadata.len();
    if size_bytes > MAX_VIEWABLE_FILE_BYTES {
        return Ok(ViewerStat::TooLarge { size_bytes });
    }
    let modified_ms = modified_ms(&metadata);
    Ok(match sniff(&mut file)? {
        ContentKind::Text => ViewerStat::Text {
            size_bytes,
            modified_ms,
        },
        ContentKind::NonUtf8Text => ViewerStat::NotText {
            size_bytes,
            modified_ms,
            reason: MetadataOnlyReason::NonUtf8Text,
        },
        ContentKind::Binary | ContentKind::Oversized => ViewerStat::NotText {
            size_bytes,
            modified_ms,
            reason: MetadataOnlyReason::Binary,
        },
    })
}

/// Up to `length` raw bytes from `offset`. Fewer only at the end of the file;
/// none at exactly the end. The caller decodes and aligns to lines — a range
/// may start or end inside a line, or inside a character.
pub fn read_range(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
    offset: u64,
    length: u32,
) -> Result<Vec<u8>, ViewerError> {
    if length > MAX_READ_RANGE_BYTES {
        return Err(ViewerError::RangeTooLong);
    }
    if sensitive_class(root, classifier, path, run_id).is_some() {
        return Err(ViewerError::Sensitive);
    }
    let (mut file, metadata) = open_regular(root, path)?;
    let size = metadata.len();
    if size > MAX_VIEWABLE_FILE_BYTES {
        return Err(ViewerError::TooLarge {
            limit: MAX_VIEWABLE_FILE_BYTES,
        });
    }
    if offset > size {
        return Err(ViewerError::OffsetPastEnd { offset, size });
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(ViewerError::Io)?;
    let wanted = u64::from(length).min(size - offset);
    let mut bytes = Vec::with_capacity(usize::try_from(wanted).unwrap_or(0));
    file.take(wanted)
        .read_to_end(&mut bytes)
        .map_err(ViewerError::Io)?;
    Ok(bytes)
}

/// A whole HTML or SVG file as text, for registering with the preview store,
/// up to [`MAX_PROJECT_PREVIEW_BYTES`].
pub fn read_preview_text(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
) -> Result<String, ViewerError> {
    if sensitive_class(root, classifier, path, run_id).is_some() {
        return Err(ViewerError::Sensitive);
    }
    let (file, metadata) = open_regular(root, path)?;
    if metadata.len() > MAX_PROJECT_PREVIEW_BYTES {
        return Err(ViewerError::TooLarge {
            limit: MAX_PROJECT_PREVIEW_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    // One past the bound, so a file that grew since the metadata read is
    // refused rather than cut.
    file.take(MAX_PROJECT_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ViewerError::Io)?;
    if bytes.len() as u64 > MAX_PROJECT_PREVIEW_BYTES {
        return Err(ViewerError::TooLarge {
            limit: MAX_PROJECT_PREVIEW_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|_| ViewerError::NotText)
}
