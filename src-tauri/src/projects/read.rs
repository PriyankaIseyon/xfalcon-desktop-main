//! Direct, containment-checked reads of trusted-project files.
//!
//! Every function here goes straight through [`TrustedRoot`] and works
//! correctly whether or not `super::index` has ever run, or is running
//! concurrently: the plan is explicit that "direct containment-checked
//! reads work immediately; indexing runs outside UI/Tauri async core." Only
//! `super::search` reads from the index; this module never does.
//!
//! `.xfalcon/` receives no special-case exclusion here (unlike
//! `super::index`, which excludes it from the walk outright): this module
//! never enumerates a directory's contents beyond one listing level and
//! never guesses a path on its own, so there is no "walk into `.xfalcon/`"
//! behavior to suppress. A caller could still ask this module to read a
//! specific path under `.xfalcon/` by name -- that is unchanged from any
//! other project-relative path and is not this module's concern to police;
//! `crate::skills::discovery` is the module that actually treats
//! `.xfalcon/skills/*` as its own trust-gated namespace when *it* reads
//! that content for skill purposes.
//!
//! Binary and encoding detection use a cheap, dependency-free heuristic
//! rather than a magic-byte-sniffing crate (`infer`, `content_inspector`):
//! a whole-buffer NUL-byte scan (git's and ripgrep's own "binary file"
//! convention) plus a UTF-8-validity check. See
//! `docs/decisions.md`
//! for why this was chosen over adding a dependency.
//!
//! # Security contract
//!
//! A sensitive read (see `super::sensitive`) never returns content --
//! whole-file or partial -- unless [`evaluate`] reports
//! [`SensitiveDecision::Overridden`] for the exact scope presented. Binary,
//! non-UTF-8, and oversized files never return raw bytes at all, sensitive
//! or not (plan: "Binary returns metadata/hash, not bytes").

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Cursor, Read};

use sha2::{Digest, Sha256};

use crate::tools::budget::{json_prefix_end, json_string_len};

use super::domain::{ContainmentError, TrustedRoot, MAX_DIRECTORY_ENTRIES};
use super::path_policy::{PathPolicyError, RelativeProjectPath};
use super::sensitive::{
    evaluate, SecretClassifier, SensitiveDecision, SensitiveOverrideGrant,
    SensitiveOverrideRequest, SensitiveReadScope,
};

/// Whole-file reads return full content only up to this bound; at or past
/// it, a caller must supply a bounded [`LineRange`] instead (plan: "partial
/// line reads"). The 1 MiB bound keeps a whole source-file read predictable;
/// larger files remain available through bounded line ranges.
pub const MAX_FILE_READ_BYTES: u64 = 1024 * 1024;

/// Ceiling on how much of a file [`hash_and_classify`] will hash/classify in
/// one call -- shared by whole-file reads (bounded by
/// [`MAX_FILE_READ_BYTES`], well under this) and `super::index`'s per-file
/// hash. Hashing streams and classification is incremental
/// ([`hash_and_classify_streaming`]), so this bounds *time spent*, not memory:
/// a file past it is reported without an identity rather than stalling a read
/// or an index pass on multiple gigabytes.
pub const MAX_HASHED_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Bytes sniffed from the start of a file when classifying content *without*
/// reading the whole thing (the partial line-range read path only). Matches
/// the order of magnitude of git's own default binary-detection buffer.
pub const SNIFF_BYTES: usize = 8_000;

/// Ceiling on how many lines a single partial (line-range) read may span.
pub const MAX_LINE_RANGE_LINES: u32 = 2_000;

/// Ceiling on a single line's own length within a partial read before it is
/// cut, on a character boundary, and the page marked `truncated`. Independent
/// of the page budget so one huge line cannot silently take a whole page.
pub const MAX_LINE_BYTES: usize = 20_000;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Containment(#[from] ContainmentError),
    #[error(transparent)]
    Path(#[from] PathPolicyError),
    #[error("line range must be non-empty and must not exceed {0} lines")]
    InvalidLineRange(u32),
    #[error("io error while reading a trusted-project file: {0}")]
    Io(#[source] std::io::Error),
    #[error("Office document extraction failed: {0}")]
    Office(String),
    #[error("the extracted Office text contains binary data")]
    OfficeNotText,
    #[error("this Office path is not a regular file")]
    OfficeNotAFile,
}

/// A file's classified content shape. `Oversized` means "not read/hashed at
/// all because it exceeds [`MAX_HASHED_FILE_BYTES`]", distinct from
/// `Binary`/`NonUtf8Text`, which *were* read and classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentKind {
    Text,
    Binary,
    NonUtf8Text,
    Oversized,
}

/// A 1-based, inclusive line range for a partial read. Constructed only via
/// [`Self::new`], which enforces [`MAX_LINE_RANGE_LINES`] up front so an
/// invalid range can never reach the actual read loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineRange {
    start_line: u32,
    end_line: u32,
}

impl LineRange {
    pub fn new(start_line: u32, end_line: u32) -> Result<Self, ReadError> {
        if start_line == 0 || end_line < start_line {
            return Err(ReadError::InvalidLineRange(MAX_LINE_RANGE_LINES));
        }
        let span = end_line - start_line + 1;
        if span > MAX_LINE_RANGE_LINES {
            return Err(ReadError::InvalidLineRange(MAX_LINE_RANGE_LINES));
        }
        Ok(Self {
            start_line,
            end_line,
        })
    }

    /// One page beginning at `start_line`: the range a caller means by "read
    /// from here", bounded by the same [`MAX_LINE_RANGE_LINES`] every other
    /// range is. This is how a file of any size is walked -- each page's
    /// [`TextContent::next_line`] is the next call's `start_line`.
    pub fn page_from(start_line: u32) -> Result<Self, ReadError> {
        if start_line == 0 {
            return Err(ReadError::InvalidLineRange(MAX_LINE_RANGE_LINES));
        }
        Self::new(
            start_line,
            start_line.saturating_add(MAX_LINE_RANGE_LINES - 1),
        )
    }

    #[must_use]
    pub fn start_line(&self) -> u32 {
        self.start_line
    }

    #[must_use]
    pub fn end_line(&self) -> u32 {
        self.end_line
    }
}

pub struct ReadFileRequest {
    pub relative_path: RelativeProjectPath,
    pub line_range: Option<LineRange>,
    pub sensitive_override: Option<SensitiveOverrideGrant>,
    /// How much text one page may carry, counted as it is sent — as JSON
    /// (`crate::tools::budget::json_string_len`). The agent passes its run's
    /// page budget. A page never exceeds it.
    pub page_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataOnlyReason {
    Binary,
    NonUtf8Text,
    Oversized,
}

/// A returned partial/whole read's text content plus which 1-based,
/// inclusive line span it actually covers -- can differ from the requested
/// [`LineRange`] when the file has fewer lines than requested, or `truncated`
/// is set when a bound (bytes or a single line's length) was hit.
///
/// `next_line` is what makes a bound a page rather than a dead end: it is
/// `Some` exactly when content remains past `last_line`, and its value is the
/// `start_line` of the call that continues from here. A caller that never
/// looks at it still gets a correct, complete first page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextContent {
    pub content: String,
    pub first_line: u32,
    pub last_line: u32,
    pub truncated: bool,
    pub next_line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReadFileOutcome {
    Text(TextContent),
    MetadataOnly {
        size_bytes: u64,
        content_kind: ContentKind,
        sha256: Option<String>,
        reason: MetadataOnlyReason,
    },
    /// The path names a classified-sensitive file and no matching override
    /// was presented -- no content or hash is included here at all, only
    /// the request a future override UI can act on.
    SensitiveOverrideRequired(SensitiveOverrideRequest),
}

/// Reads one trusted-project file. `scope` and `request.relative_path` MUST
/// name the same path -- see [`super::sensitive::SensitiveReadScope`] -- a
/// caller mismatch is a programmer error caught by the `debug_assert`
/// below, not a security boundary (the actual containment decision comes
/// from resolving `request.relative_path` through `trusted_root`, not from
/// `scope`).
pub fn read_file(
    trusted_root: &TrustedRoot,
    classifier: &SecretClassifier,
    scope: SensitiveReadScope,
    request: ReadFileRequest,
) -> Result<ReadFileOutcome, ReadError> {
    debug_assert_eq!(
        scope.relative_path(),
        &request.relative_path,
        "read_file's scope and request must name the same path"
    );
    let resolved = trusted_root.resolve_existing(request.relative_path.clone())?;

    if let SensitiveDecision::RequiresOverride(pending) =
        evaluate(classifier, scope, request.sensitive_override.as_ref())
    {
        return Ok(ReadFileOutcome::SensitiveOverrideRequired(pending));
    }

    let metadata = trusted_root.file_metadata(&resolved)?;
    let size_bytes = metadata.len();

    if crate::documents::is_office_path(request.relative_path.as_path()) {
        if !metadata.is_file() {
            return Err(ReadError::OfficeNotAFile);
        }
        if size_bytes > crate::documents::MAX_DOCUMENT_BYTES {
            return Err(ReadError::Office(
                crate::documents::DocumentError::InputTooLarge.to_string(),
            ));
        }
        let file = trusted_root.open_file(&resolved)?;
        let file_metadata = file.metadata().map_err(ReadError::Io)?;
        if !file_metadata.is_file() {
            return Err(ReadError::OfficeNotAFile);
        }
        if file_metadata.len() > crate::documents::MAX_DOCUMENT_BYTES {
            return Err(ReadError::Office(
                crate::documents::DocumentError::InputTooLarge.to_string(),
            ));
        }
        let mut bytes = Vec::with_capacity(file_metadata.len() as usize);
        file.take(crate::documents::MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(ReadError::Io)?;
        if bytes.len() as u64 > crate::documents::MAX_DOCUMENT_BYTES {
            return Err(ReadError::Office(
                crate::documents::DocumentError::InputTooLarge.to_string(),
            ));
        }
        let extracted = crate::documents::extract(request.relative_path.as_path(), &bytes)
            .map_err(|error| ReadError::Office(error.to_string()))?;
        if extracted.text.as_bytes().contains(&0) {
            return Err(ReadError::OfficeNotText);
        }
        let range = match request.line_range {
            Some(range) => range,
            None => LineRange::page_from(1)?,
        };
        return match read_partial(
            Cursor::new(extracted.text.into_bytes()),
            range,
            request.page_bytes,
        )? {
            PartialReadOutcome::Text(content) => Ok(ReadFileOutcome::Text(content)),
            PartialReadOutcome::NotText(_) => Err(ReadError::OfficeNotText),
        };
    }

    if let Some(range) = request.line_range {
        let file = trusted_root.open_file(&resolved)?;
        return match read_partial(file, range, request.page_bytes)? {
            PartialReadOutcome::Text(content) => Ok(ReadFileOutcome::Text(content)),
            PartialReadOutcome::NotText(kind) => {
                // `read_partial` only sniffs a prefix and never hashes, so
                // -- exactly like the whole-file path above -- hashing
                // requires its own fresh, from-the-start open through
                // `TrustedRoot` rather than reusing the partially-consumed
                // reader.
                let sha256 = if size_bytes <= MAX_HASHED_FILE_BYTES {
                    let file = trusted_root.open_file(&resolved)?;
                    Some(
                        hash_and_classify_streaming(file, MAX_HASHED_FILE_BYTES)
                            .map_err(ReadError::Io)?
                            .1,
                    )
                } else {
                    None
                };
                let reason = match kind {
                    ContentKind::NonUtf8Text => MetadataOnlyReason::NonUtf8Text,
                    ContentKind::Binary | ContentKind::Oversized => MetadataOnlyReason::Binary,
                    ContentKind::Text => {
                        unreachable!("NotText is never constructed with ContentKind::Text")
                    }
                };
                Ok(ReadFileOutcome::MetadataOnly {
                    size_bytes,
                    content_kind: kind,
                    sha256,
                    reason,
                })
            }
        };
    }

    if size_bytes > MAX_FILE_READ_BYTES {
        // Too long to return whole, but a long *text* file is readable one
        // page at a time -- `read_partial` streams and never holds the file --
        // so return its first page with a cursor rather than refusing it.
        // Only content that cannot be paged at all (binary, non-UTF-8, or too
        // large even to classify) falls through to metadata.
        let file = trusted_root.open_file(&resolved)?;
        return match read_partial(file, LineRange::page_from(1)?, request.page_bytes)? {
            PartialReadOutcome::Text(content) => Ok(ReadFileOutcome::Text(content)),
            PartialReadOutcome::NotText(kind) => {
                // A file too large even to hash is reported as `Oversized`:
                // that is the one case where nothing about its content --
                // not even an identity -- was established.
                let (sha256, reason) = if size_bytes <= MAX_HASHED_FILE_BYTES {
                    let file = trusted_root.open_file(&resolved)?;
                    let hash = hash_and_classify_streaming(file, MAX_HASHED_FILE_BYTES)
                        .map_err(ReadError::Io)?
                        .1;
                    let reason = match kind {
                        ContentKind::NonUtf8Text => MetadataOnlyReason::NonUtf8Text,
                        _ => MetadataOnlyReason::Binary,
                    };
                    (Some(hash), reason)
                } else {
                    (None, MetadataOnlyReason::Oversized)
                };
                Ok(ReadFileOutcome::MetadataOnly {
                    size_bytes,
                    content_kind: kind,
                    sha256,
                    reason,
                })
            }
        };
    }

    let file = trusted_root.open_file(&resolved)?;
    let (kind, sha256, bytes) =
        hash_and_classify(file, MAX_FILE_READ_BYTES).map_err(ReadError::Io)?;
    match kind {
        ContentKind::Text => {
            let total_lines = count_lines(&bytes);
            let content = String::from_utf8(bytes)
                .expect("hash_and_classify only reports Text for a fully valid UTF-8 buffer");
            Ok(ReadFileOutcome::Text(TextContent {
                content,
                first_line: 1,
                last_line: total_lines,
                truncated: false,
                next_line: None,
            }))
        }
        ContentKind::Binary => Ok(ReadFileOutcome::MetadataOnly {
            size_bytes,
            content_kind: kind,
            sha256: Some(sha256),
            reason: MetadataOnlyReason::Binary,
        }),
        ContentKind::NonUtf8Text => Ok(ReadFileOutcome::MetadataOnly {
            size_bytes,
            content_kind: kind,
            sha256: Some(sha256),
            reason: MetadataOnlyReason::NonUtf8Text,
        }),
        ContentKind::Oversized => {
            unreachable!("hash_and_classify never reports Oversized; the size check above does")
        }
    }
}

/// Async wrapper around [`read_file`] that also publishes a best-effort,
/// content-free `crate::audit::AuditAction::ProjectRead` event for the
/// *sensitive-read decision* specifically (Task 7 Step 3's "sensitive-read
/// decisions" audit category — a review finding caught this module had zero
/// `AuditSink::publish` calls anywhere in it). Deliberately does not audit
/// every ordinary non-sensitive read: this is a decision log for the
/// high-salience sensitive-file gate (`super::sensitive::evaluate`), not a
/// blanket access log for every file read `read_file` itself already serves.
/// `read_file` stays untouched (sync, pure, its own test suite unaffected)
/// -- this wrapper independently computes the same classification decision
/// `read_file` computes internally (a second, harmless evaluation of the
/// same pure, deterministic function) purely to decide whether/how to
/// audit, before delegating the actual read to it unchanged.
///
/// No production caller supplies an `audit` yet (no live dispatcher calls
/// `read_file`/`read_file_with_audit` at all — see
/// `crate::agent::tool_loop`'s own "no live tool-execution dispatcher yet"
/// module doc comment) — same disclosed-seam shape as
/// `crate::agent::tool_loop::ToolLoop::with_audit`.
pub async fn read_file_with_audit(
    trusted_root: &TrustedRoot,
    classifier: &SecretClassifier,
    scope: SensitiveReadScope,
    request: ReadFileRequest,
    audit: Option<&crate::audit::AuditPublisher>,
) -> Result<ReadFileOutcome, ReadError> {
    let decision = evaluate(
        classifier,
        scope.clone(),
        request.sensitive_override.as_ref(),
    );
    let outcome = read_file(trusted_root, classifier, scope.clone(), request)?;

    if let Some(audit) = audit {
        let published_outcome = match &decision {
            SensitiveDecision::NotSensitive => None,
            SensitiveDecision::RequiresOverride(_) => {
                Some(crate::audit::AuditOutcome::ApprovalRequired)
            }
            SensitiveDecision::Overridden(_) => Some(crate::audit::AuditOutcome::Allowed),
        };
        if let Some(published_outcome) = published_outcome {
            audit
                .publish(
                    crate::audit::AuditAction::ProjectRead,
                    crate::audit::AuditResourceType::Project,
                    scope.relative_path().as_str(),
                    published_outcome,
                    "projects.sensitive.v1",
                    scope.run_id().as_str(),
                )
                .await;
        }
    }

    Ok(outcome)
}

/// Streams `file`, computing a SHA-256 hash and classifying its content
/// over the *entire* buffer read (never just a prefix -- see the module doc
/// comment's security contract), rejecting a read past `byte_limit` bytes.
/// Every call site first confirms via `TrustedRoot::file_metadata` that the
/// file's actual size does not exceed `byte_limit`, so the early-exit past
/// `byte_limit` below is defense in depth against a TOCTOU grow, not the
/// normal path; a grow past the limit fails rather than returning a partial
/// buffer or hash as a complete read.
pub(super) fn hash_and_classify(
    mut file: impl Read,
    byte_limit: u64,
) -> std::io::Result<(ContentKind, String, Vec<u8>)> {
    let mut buffer = Vec::with_capacity((byte_limit.min(64 * 1024)) as usize);
    let mut chunk = [0u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > byte_limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the file grew past the read limit; read it again in bounded ranges",
            ));
        }
        hasher.update(&chunk[..read]);
        buffer.extend_from_slice(&chunk[..read]);
    }
    let kind = classify_bytes(&buffer);
    Ok((kind, format!("{:x}", hasher.finalize()), buffer))
}

/// Streams `file`, computing a SHA-256 hash and classifying its content over
/// the *entire* file read, without ever holding it in memory: the NUL scan is
/// per chunk and UTF-8 validity is checked incrementally, carrying only the
/// trailing bytes of a sequence split across a chunk boundary. This is what
/// lets [`MAX_HASHED_FILE_BYTES`] be a time bound rather than a memory one.
///
/// Split from [`hash_and_classify`], which additionally returns the bytes and
/// is therefore only used where the caller needs the content anyway (a
/// whole-file read, bounded by [`MAX_FILE_READ_BYTES`]).
pub(super) fn hash_and_classify_streaming(
    mut file: impl Read,
    byte_limit: u64,
) -> std::io::Result<(ContentKind, String)> {
    let mut chunk = [0u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut has_nul = false;
    let mut valid_utf8 = true;
    // Bytes of one UTF-8 sequence split across a chunk boundary (never more
    // than 3), carried into the next chunk's validation.
    let mut pending: Vec<u8> = Vec::with_capacity(4);
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        // A grow past the checked bound cannot produce a whole-file hash.
        if total > byte_limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the file grew past the hash limit; no complete hash is available",
            ));
        }
        let bytes = &chunk[..read];
        hasher.update(bytes);
        if bytes.contains(&0) {
            has_nul = true;
        }
        if valid_utf8 && !has_nul {
            pending.extend_from_slice(bytes);
            match std::str::from_utf8(&pending) {
                Ok(_) => pending.clear(),
                // `error_len() == None` means the buffer ends mid-sequence,
                // which a following chunk may complete.
                Err(error) if error.error_len().is_none() => {
                    let tail = pending.split_off(error.valid_up_to());
                    pending = tail;
                }
                Err(_) => {
                    valid_utf8 = false;
                    pending.clear();
                }
            }
        }
    }
    // A sequence still pending at end-of-file was never completed.
    let kind = if has_nul {
        ContentKind::Binary
    } else if valid_utf8 && pending.is_empty() {
        ContentKind::Text
    } else {
        ContentKind::NonUtf8Text
    };
    Ok((kind, format!("{:x}", hasher.finalize())))
}

/// Classifies a complete, already-in-memory byte buffer using this module's
/// standing binary/text convention: a whole-buffer NUL-byte scan (git's and
/// ripgrep's own "binary file" heuristic) then a UTF-8-validity check.
/// Extracted from [`hash_and_classify`] so `super::patch`'s mutation-plan
/// proposal (which classifies caller-supplied content already held in
/// memory, never a file it needs to stream/hash itself) can reuse the exact
/// same classification rule rather than a second, potentially-diverging
/// implementation. Never reports [`ContentKind::Oversized`] -- that
/// classification is a caller-side size decision (see
/// [`MAX_HASHED_FILE_BYTES`]), not something this function can determine
/// from a buffer it has already been handed.
pub(super) fn classify_bytes(buffer: &[u8]) -> ContentKind {
    if buffer.contains(&0) {
        ContentKind::Binary
    } else {
        match std::str::from_utf8(buffer) {
            Ok(_) => ContentKind::Text,
            Err(_) => ContentKind::NonUtf8Text,
        }
    }
}

/// Cheap, prefix-only classification used solely by the partial (line-range)
/// read path, which deliberately never reads a whole (possibly huge) file
/// just to classify it. Unlike [`hash_and_classify`], this only inspects
/// `prefix` (the first [`SNIFF_BYTES`] actually read); [`read_partial`]
/// treats a classification of [`ContentKind::Text`] here as provisional and
/// still defends against a later invalid byte (see its own doc comment).
pub(super) fn sniff_content_kind(prefix: &[u8]) -> ContentKind {
    if prefix.contains(&0) {
        return ContentKind::Binary;
    }
    match std::str::from_utf8(prefix) {
        Ok(_) => ContentKind::Text,
        Err(error) => {
            // `error_len() == None` means the error is "ran out of bytes
            // mid-sequence", i.e. this prefix's last character was cut off
            // by the prefix boundary, not actually invalid -- treat that as
            // text so far. `Some(_)` means a genuinely invalid byte was
            // found before the end of the prefix.
            if error.error_len().is_none() {
                ContentKind::Text
            } else {
                ContentKind::NonUtf8Text
            }
        }
    }
}

const fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn count_lines(bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u32;
    if bytes.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    }
}

/// [`read_partial`]'s outcome: either the requested text span, or proof the
/// file is not (confidently) text at all, carrying the sniffed
/// [`ContentKind`] so the caller can build a proper
/// [`ReadFileOutcome::MetadataOnly`] instead of a misleading empty "text"
/// result.
enum PartialReadOutcome {
    Text(TextContent),
    NotText(ContentKind),
}

/// Reads `range` from `file` without ever loading the whole file into
/// memory: a bounded prefix sniff decides whether this looks like text at
/// all, then a line-by-line scan skips to `range.start_line` and collects
/// through `range.end_line`, bounded by `page_bytes` as sent and
/// [`MAX_LINE_BYTES`] per line. The page stops *before* a line that would
/// overflow it, so it never exceeds `page_bytes`; only a first line too long
/// for the page on its own is cut to fit. If a line past the sniffed prefix turns out
/// not to be valid UTF-8 (the sniff is a heuristic over a prefix, not a
/// guarantee), the read stops there and reports `truncated: true` rather
/// than returning invalid text or panicking. An encountered NUL instead
/// classifies the file as binary, just as the initial sniff does.
fn read_partial(
    mut file: impl Read,
    range: LineRange,
    page_bytes: usize,
) -> Result<PartialReadOutcome, ReadError> {
    let mut sniff_buffer = vec![0u8; SNIFF_BYTES];
    let sniff_read = file.read(&mut sniff_buffer).map_err(ReadError::Io)?;
    sniff_buffer.truncate(sniff_read);
    let sniffed_kind = sniff_content_kind(&sniff_buffer);
    if !matches!(sniffed_kind, ContentKind::Text) {
        // Not text (or not confidently so): a partial read of binary or
        // non-UTF-8 content is exactly the raw-bytes exposure the plan
        // forbids, so the caller reports it the same way a whole-file read
        // would, rather than this function attempting a line-based read.
        return Ok(PartialReadOutcome::NotText(sniffed_kind));
    }

    let mut reader = BufReader::new(std::io::Cursor::new(sniff_buffer).chain(file));
    let mut current_line: u32 = 0;
    let mut collected = String::new();
    // `collected` as it will be sent, escapes included.
    let mut sent_bytes = 0usize;
    let mut truncated = false;
    let mut first_returned: Option<u32> = None;
    let mut last_returned = range.start_line.saturating_sub(1);
    let mut raw_line: Vec<u8> = Vec::new();
    // `None` means "nothing follows what we returned". Every early exit below
    // sets it, so only reaching end-of-file leaves it unset.
    let mut next_line: Option<u32> = None;

    loop {
        raw_line.clear();
        // Two bytes past the line bound, for a line exactly at it and its
        // `\r\n`: anything longer is cut below either way.
        let (read, contains_nul) =
            read_bounded_line(&mut reader, &mut raw_line, MAX_LINE_BYTES + 2)
                .map_err(ReadError::Io)?;
        if read == 0 {
            break;
        }
        current_line += 1;
        if contains_nul {
            return Ok(PartialReadOutcome::NotText(ContentKind::Binary));
        }
        if current_line < range.start_line {
            continue;
        }
        if current_line > range.end_line {
            // A line exists past the requested range, so this read has a
            // continuation even though it hit no bound.
            next_line = Some(current_line);
            break;
        }

        if raw_line.last() == Some(&b'\n') {
            raw_line.pop();
            if raw_line.last() == Some(&b'\r') {
                raw_line.pop();
            }
        }
        let mut line_bytes: &[u8] = &raw_line;
        let mut line_cut = false;
        if line_bytes.len() > MAX_LINE_BYTES {
            // Back off to a character start, so a cut line is still text.
            let mut end = MAX_LINE_BYTES;
            while end > 0 && is_utf8_continuation(line_bytes[end]) {
                end -= 1;
            }
            line_bytes = &line_bytes[..end];
            line_cut = true;
        }
        let (text, invalid) = match std::str::from_utf8(line_bytes) {
            Ok(text) => (text, false),
            Err(error) => (
                std::str::from_utf8(&line_bytes[..error.valid_up_to()])
                    .expect("valid_up_to always bounds a valid UTF-8 prefix"),
                true,
            ),
        };

        // The line and its newline, as they will be sent.
        let cost = json_string_len(text) + 2;
        if sent_bytes + cost > page_bytes {
            if first_returned.is_some() {
                // This line starts the next page; nothing on this one is cut.
                next_line = Some(current_line);
                break;
            }
            // A first line too long for a page on its own: send what fits,
            // marked, and continue after it.
            let end = json_prefix_end(text, page_bytes.saturating_sub(2));
            first_returned = Some(current_line);
            collected.push_str(&text[..end]);
            collected.push('\n');
            last_returned = current_line;
            truncated = true;
            next_line = current_line.checked_add(1);
            break;
        }

        first_returned.get_or_insert(current_line);
        collected.push_str(text);
        collected.push('\n');
        sent_bytes += cost;
        last_returned = current_line;
        truncated |= line_cut;
        if invalid {
            truncated = true;
            // Continuing from the next line makes forward progress past the
            // invalid bytes rather than re-reading them forever.
            next_line = current_line.checked_add(1);
            break;
        }
    }

    Ok(PartialReadOutcome::Text(TextContent {
        content: collected,
        first_line: first_returned.unwrap_or(range.start_line),
        last_line: last_returned,
        truncated,
        next_line,
    }))
}

/// Reads one line, through its newline, keeping at most its first `keep`
/// bytes in `line` and discarding the rest as it goes — so one enormous line
/// (a minified bundle, a one-line export) is never held in memory whole.
/// Returns bytes consumed (`0` only at EOF) and NUL detection over all of them,
/// including bytes discarded past `keep`.
fn read_bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    keep: usize,
) -> std::io::Result<(usize, bool)> {
    let mut consumed = 0;
    let mut contains_nul = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok((consumed, contains_nul));
        }
        let (length, ended) = match available.iter().position(|&byte| byte == b'\n') {
            Some(newline) => (newline + 1, true),
            None => (available.len(), false),
        };
        let room = keep.saturating_sub(line.len());
        line.extend_from_slice(&available[..length.min(room)]);
        contains_nul |= available[..length].contains(&0);
        reader.consume(length);
        consumed += length;
        if ended {
            return Ok((consumed, contains_nul));
        }
    }
}

/// One immediate child of a listed directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntryView {
    pub name: String,
    pub kind: EntryKind,
    pub size_bytes: u64,
    pub is_sensitive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// Lists `relative`'s immediate children (or the trusted root's own, when
/// `relative` is `None` -- see [`TrustedRoot::list_root_entries`]'s doc
/// comment for why the root needs a separate accessor). Never recurses,
/// never reads file content, and never depends on `super::index`. Each
/// entry is labeled `is_sensitive` using the same [`SecretClassifier`] used
/// by reads, so a caller can visually flag a sensitive child without
/// exposing any of its content.
pub fn list_directory(
    trusted_root: &TrustedRoot,
    classifier: &SecretClassifier,
    relative: Option<RelativeProjectPath>,
) -> Result<Vec<DirectoryEntryView>, ReadError> {
    let raw_entries = match &relative {
        None => trusted_root.list_root_entries()?,
        Some(relative_path) => {
            let resolved = trusted_root.resolve_existing(relative_path.clone())?;
            trusted_root.list_entries(&resolved)?
        }
    };
    debug_assert!(raw_entries.len() <= MAX_DIRECTORY_ENTRIES);

    let mut views = Vec::with_capacity(raw_entries.len());
    for (name, metadata) in raw_entries {
        let kind = if metadata.is_symlink() {
            EntryKind::Symlink
        } else if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let name_string = display_name(&name);
        let is_sensitive = classify_child(classifier, relative.as_ref(), &name_string, kind);
        views.push(DirectoryEntryView {
            name: name_string,
            kind,
            size_bytes: metadata.len(),
            is_sensitive,
        });
    }
    Ok(views)
}

/// Best-effort display name for a raw directory-entry filename: a
/// non-UTF-8 name is reported lossily rather than silently dropped from the
/// listing (dropping a real entry the filesystem reports would be a silent
/// failure this codebase's error-handling standard forbids).
fn display_name(name: &OsString) -> String {
    name.to_string_lossy().into_owned()
}

/// Classifies one listed child by reconstructing its full project-relative
/// path from `parent` + `name` purely as a *string* (never used for
/// filesystem access -- the listing itself already came from
/// `TrustedRoot::list_entries`/`list_root_entries`, which is the only
/// containment-relevant step here). A name that fails
/// `RelativeProjectPath::new` (vanishingly unlikely for a name the
/// filesystem itself just reported) is treated as not classifiable rather
/// than erroring the whole listing over one child.
///
/// A `kind` of [`EntryKind::Directory`] uses
/// [`SecretClassifier::classify_directory`] instead of
/// [`SecretClassifier::classify`], so a project-root child literally named
/// `.aws`/`.ssh`/`.kube`/`.gnupg`/`.azure` is flagged sensitive too, not only
/// its contents -- see that method's doc comment.
fn classify_child(
    classifier: &SecretClassifier,
    parent: Option<&RelativeProjectPath>,
    name: &str,
    kind: EntryKind,
) -> bool {
    let candidate = match parent {
        None => name.to_owned(),
        Some(parent) => format!("{}/{name}", parent.as_str()),
    };
    let Ok(relative) = RelativeProjectPath::new(&candidate) else {
        return false;
    };
    let classified = if kind == EntryKind::Directory {
        classifier.classify_directory(&relative)
    } else {
        classifier.classify(&relative)
    };
    classified.is_some()
}
