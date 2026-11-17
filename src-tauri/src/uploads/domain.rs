//! Upload lifecycle, and the split between what stays in Rust and what the
//! webview may see.
//!
//! Three types, deliberately not one:
//!
//! - [`LocalUploadSource`] — the file on disk. Absolute path, size, sniffed
//!   type. Never leaves this process.
//! - [`UploadRecord`] — the transfer in flight. Adds DA's identifiers and the
//!   presigned targets. Never leaves this process either; a presigned URL is a
//!   bearer credential for tenant object storage, and the webview is the one
//!   process that renders untrusted model output.
//! - `xfalcon_contracts::uploads::UploadProjectionV1` — what the UI gets.
//!
//! Collapsing these into one struct and "just not serializing" the sensitive
//! fields is the failure mode this split exists to prevent: it makes leaking
//! them a one-line mistake instead of a structural impossibility.

use std::path::{Path, PathBuf};

use xfalcon_contracts::uploads::{UploadId, UploadProjectionV1, UploadPurpose, UploadState};

/// Longest display name kept for an upload. Well past any real filename, short
/// enough that a hostile one cannot dominate the UI or a log line.
pub const MAX_DISPLAY_NAME_BYTES: usize = 128;

/// Replacement for a name that sanitizes away to nothing.
pub const FALLBACK_DISPLAY_NAME: &str = "Untitled file";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum UploadTransitionError {
    #[error("an upload cannot move from {from:?} to {to:?}")]
    Illegal { from: UploadState, to: UploadState },
    #[error("bytes sent may not decrease")]
    ProgressWentBackwards,
    #[error("bytes sent may not exceed the file's size")]
    ProgressExceedsSize,
    #[error("a part number was completed more than once")]
    DuplicatePart,
}

/// Which worksheet and which header row DA should ingest.
///
/// Both are optional and both are omitted when the person did not choose, so
/// DA applies its own defaults rather than this app asserting one on their
/// behalf. Both are also omitted when a cleanup recipe ran: the recipe already
/// selected the worksheet and consumed the header row, and what is uploaded is
/// a plain CSV whose first row is the header.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DatasetIngest {
    pub worksheet_index: Option<u32>,
    pub header_row_index: Option<u32>,
}

/// The picked file, as this process sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalUploadSource {
    /// Absolute path on the user's disk. Never projected.
    private_path: PathBuf,
    size: u64,
    /// Content type as detected locally, not as the OS or the user claims.
    detected_type: String,
}

impl LocalUploadSource {
    #[must_use]
    pub fn new(private_path: PathBuf, size: u64, detected_type: impl Into<String>) -> Self {
        Self {
            private_path,
            size,
            detected_type: detected_type.into(),
        }
    }

    #[must_use]
    pub fn private_path(&self) -> &Path {
        &self.private_path
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    #[must_use]
    pub fn detected_type(&self) -> &str {
        &self.detected_type
    }

    /// The name to show for this file, sanitized.
    #[must_use]
    pub fn display_name(&self) -> String {
        sanitize_display_name(
            self.private_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
                .as_str(),
        )
    }
}

/// One transfer in flight.
#[derive(Clone, Debug)]
pub struct UploadRecord {
    id: UploadId,
    purpose: UploadPurpose,
    state: UploadState,
    bytes_sent: u64,
    source: LocalUploadSource,
    display_name: String,
    /// DA's identifier for the dataset/attachment, once presigning assigned
    /// one.
    da_upload_id: Option<String>,
    /// Part numbers already confirmed complete, for a multipart transfer.
    completed_parts: Vec<i64>,
    /// Object storage's own multipart upload id, once one exists. Needed to
    /// abort the upload server-side if it is cancelled part-way.
    multipart_upload_id: Option<String>,
    safe_error: Option<String>,
    conversation_id: Option<String>,
    /// The file's own name, sanitized. Sent to DA as `filename`, which its
    /// presign compares against the declared content type — so it must keep
    /// the real extension even after the person renames the dataset.
    file_name: String,
    /// Which worksheet and header row to ingest. Empty for an attachment.
    ingest: DatasetIngest,
    /// The source is a file this app wrote, not the one the person picked, so
    /// it is deleted once the transfer reaches a terminal state. Set when a
    /// cleanup recipe was applied.
    temporary_source: bool,
}

impl UploadRecord {
    #[must_use]
    pub fn new(id: UploadId, purpose: UploadPurpose, source: LocalUploadSource) -> Self {
        let file_name = source.display_name();
        let display_name = file_name.clone();
        Self {
            id,
            purpose,
            state: UploadState::Selected,
            bytes_sent: 0,
            source,
            display_name,
            da_upload_id: None,
            completed_parts: Vec::new(),
            multipart_upload_id: None,
            safe_error: None,
            conversation_id: None,
            file_name,
            ingest: DatasetIngest::default(),
            temporary_source: false,
        }
    }

    #[must_use]
    pub fn id(&self) -> &UploadId {
        &self.id
    }

    #[must_use]
    pub fn state(&self) -> UploadState {
        self.state
    }

    #[must_use]
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    #[must_use]
    pub fn source(&self) -> &LocalUploadSource {
        &self.source
    }

    #[must_use]
    pub fn da_upload_id(&self) -> Option<&str> {
        self.da_upload_id.as_deref()
    }

    pub fn set_da_upload_id(&mut self, id: impl Into<String>) {
        self.da_upload_id = Some(id.into());
    }

    #[must_use]
    pub fn multipart_upload_id(&self) -> Option<&str> {
        self.multipart_upload_id.as_deref()
    }

    pub fn set_multipart_upload_id(&mut self, id: impl Into<String>) {
        self.multipart_upload_id = Some(id.into());
    }

    /// The name to send DA as the dataset's own. Sanitized, and never empty:
    /// a blank name would leave the catalog with an unnameable row.
    pub fn set_display_name(&mut self, name: &str) {
        self.display_name = sanitize_display_name(name);
    }

    /// Sets the filename DA validates against the declared content type.
    ///
    /// App-owned upload snapshots use an opaque on-disk name. The original
    /// sanitized filename stays here so the snapshot path never needs to
    /// contain renderer-controlled text.
    pub fn set_file_name(&mut self, name: &str) {
        self.file_name = sanitize_display_name(name);
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    #[must_use]
    pub fn ingest(&self) -> DatasetIngest {
        self.ingest
    }

    pub fn set_ingest(&mut self, ingest: DatasetIngest) {
        self.ingest = ingest;
    }

    #[must_use]
    pub fn has_temporary_source(&self) -> bool {
        self.temporary_source
    }

    pub fn mark_temporary_source(&mut self) {
        self.temporary_source = true;
    }

    pub fn set_conversation_id(&mut self, id: impl Into<String>) {
        self.conversation_id = Some(id.into());
    }

    #[must_use]
    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    #[must_use]
    pub fn purpose(&self) -> UploadPurpose {
        self.purpose
    }

    #[must_use]
    pub fn completed_parts(&self) -> &[i64] {
        &self.completed_parts
    }

    /// Records a part as complete.
    ///
    /// A repeated part number is rejected rather than ignored: S3's
    /// CompleteMultipartUpload takes one entry per part, and a duplicate means
    /// the caller lost track of which parts are outstanding — silently
    /// deduplicating would hide that until the finalize failed.
    pub fn complete_part(&mut self, part_number: i64) -> Result<(), UploadTransitionError> {
        if self.completed_parts.contains(&part_number) {
            return Err(UploadTransitionError::DuplicatePart);
        }
        self.completed_parts.push(part_number);
        Ok(())
    }

    /// Advances progress.
    ///
    /// Monotonic within an attempt, and never past the size measured before
    /// the transfer began: a server or a racing writer claiming more bytes
    /// than the file has means the two disagree about what is being sent, and
    /// reporting it as progress would show a bar past 100% at best.
    pub fn advance(&mut self, bytes_sent: u64) -> Result<(), UploadTransitionError> {
        if bytes_sent < self.bytes_sent {
            return Err(UploadTransitionError::ProgressWentBackwards);
        }
        if bytes_sent > self.source.size() {
            return Err(UploadTransitionError::ProgressExceedsSize);
        }
        self.bytes_sent = bytes_sent;
        Ok(())
    }

    /// Moves to `next`, or rejects the move.
    ///
    /// A retry (`Failed -> Preparing`) resets progress: the transfer starts
    /// over from a fresh presign, and leaving the old byte count would show
    /// progress the new attempt has not made.
    pub fn transition(&mut self, next: UploadState) -> Result<(), UploadTransitionError> {
        if !is_legal_transition(self.state, next) {
            return Err(UploadTransitionError::Illegal {
                from: self.state,
                to: next,
            });
        }
        if self.state == UploadState::Failed && next == UploadState::Preparing {
            self.bytes_sent = 0;
            self.completed_parts.clear();
            self.multipart_upload_id = None;
            self.safe_error = None;
        }
        self.state = next;
        Ok(())
    }

    /// Cancels the upload.
    ///
    /// Idempotent, and a no-op once terminal: cancelling a finished upload
    /// must not un-finish it, and cancelling twice is what a user
    /// double-clicking produces.
    pub fn cancel(&mut self) {
        if self.state.is_terminal() {
            return;
        }
        self.state = UploadState::Cancelled;
    }

    /// Fails the upload with a message safe to show a person.
    pub fn fail(&mut self, safe_error: impl Into<String>) {
        if self.state.is_terminal() {
            return;
        }
        self.state = UploadState::Failed;
        self.safe_error = Some(safe_error.into());
    }

    /// The bounded view the webview may have.
    #[must_use]
    pub fn projection(&self) -> UploadProjectionV1 {
        let mut projection = UploadProjectionV1::new(
            self.id.clone(),
            self.purpose,
            self.state,
            self.bytes_sent,
            self.source.size(),
            self.display_name.clone(),
        );
        if let Some(conversation_id) = self.conversation_id.as_deref() {
            projection = projection.with_conversation_id(conversation_id);
        }
        if self.state == UploadState::Ready {
            if let Some(resource_id) = self.da_upload_id.clone() {
                projection = projection.with_resource_id(resource_id);
            }
        }
        match self.safe_error.as_ref() {
            Some(error) => projection.with_error(error.clone()),
            None => projection,
        }
    }
}

const fn is_legal_transition(from: UploadState, to: UploadState) -> bool {
    use UploadState::{
        Cancelled, Failed, Finalizing, Preparing, Ready, Resigning, Selected, Uploading,
    };
    matches!(
        (from, to),
        (Selected, Preparing)
            | (Preparing, Uploading)
            | (Uploading, Finalizing)
            | (Uploading, Resigning)
            | (Resigning, Uploading)
            | (Finalizing, Ready)
            // Anything in flight can fail or be cancelled.
            | (Preparing | Uploading | Resigning | Finalizing, Failed | Cancelled)
            // A picked-but-unstarted upload can still be dropped.
            | (Selected, Cancelled)
            // Retry starts over from a fresh presign, never mid-transfer.
            | (Failed, Preparing)
            | (Failed, Cancelled)
    )
}

/// Makes a filename safe to render and bounded in length.
///
/// A filename is attacker-influenced content: it comes from a file the user
/// may have received from anywhere, and it lands in UI text and log lines.
/// Control characters (including the bidirectional overrides that let
/// `report.fdp.exe` render as `report.exe.pdf`) and path separators are
/// removed; the rest is truncated on a character boundary.
#[must_use]
pub fn sanitize_display_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(character, '/' | '\\')
                // Unicode bidi controls: LRE/RLE/PDF/LRO/RLO, LRI/RLI/FSI/PDI,
                // and ALM. `is_control` does not cover these.
                && !matches!(*character as u32, 0x202A..=0x202E | 0x2066..=0x2069 | 0x061C)
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return FALLBACK_DISPLAY_NAME.to_owned();
    }
    if trimmed.len() <= MAX_DISPLAY_NAME_BYTES {
        return trimmed.to_owned();
    }
    let mut boundary = MAX_DISPLAY_NAME_BYTES;
    while boundary > 0 && !trimmed.is_char_boundary(boundary) {
        boundary -= 1;
    }
    trimmed[..boundary].to_owned()
}
