//! Journal-first tool invocation state machine.
//!
//! Mirrors `src-tauri/src/agent/run.rs`'s discipline: every status change is
//! validated against an explicit transition table, journaled before the
//! in-memory status changes, and terminal statuses reject further mutation.
//! `InvocationId` (from `xfalcon-contracts`) is the single stable key used
//! for every journaled event, which is what makes replaying the same
//! invocation idempotent (see [`InMemoryInvocationJournal::record`]).

use std::fmt;
use std::time::Instant;

use serde_json::Value;
use sha2::{Digest, Sha256};

use xfalcon_contracts::tools::{
    InvocationId, ToolId, ToolInvocationEvent, ToolInvocationStatus, ToolSource,
};
use xfalcon_contracts::RunId;

use super::budget::{json_prefix_end, json_string_len, ToolResultBudget};
use super::policy::{Actor, PolicyDenial};

/// One call's serialized arguments. Sized for the largest thing a model sends
/// in one call — a patch — at a quarter mebibyte: well past what one reply's
/// output usually carries, and bounded so a runaway call cannot hold megabytes
/// in the journal. `crate::agent::tool_loop` answers a call over it with
/// `invalid_arguments` before it is opened, so the run goes on.
pub(crate) const MAX_ARGUMENTS_BYTES: usize = 256 * 1024;
const MAX_REASON_CODE_BYTES: usize = 128;
const MAX_PROGRESS_SUMMARY_BYTES: usize = 2_048;
/// `pub(crate)` (not private) so `crate::agent::tool_loop`'s
/// `ToolLoop::finalize_failure` can truncate a failure message to this exact
/// bound *before* calling [`ToolInvocation::fail`], rather than risking
/// `InvocationError::FailureMessageTooLarge` on an untrusted, unbounded tool
/// error string — see that call site's own doc comment.
pub(crate) const MAX_FAILURE_MESSAGE_BYTES: usize = 2_048;
/// Exact structured bytes retained until the result is durably persisted.
///
/// This matches the MCP response trust-boundary ceiling. The model never
/// receives these bytes blindly; [`ToolResult::model_preview`] creates the
/// separate bounded representation used in context.
pub const MAX_EXACT_TOOL_RESULT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONTENT_REFS: usize = 64;
/// Column names listed for a previewed table, and the bytes of each.
const PREVIEW_FIELD_LIMIT: usize = 64;
const PREVIEW_NAME_BYTES: usize = 256;
/// Below this, a field or row is not worth a partial rendering: it is named
/// and measured instead.
const MIN_FITTED_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InvocationError {
    #[error("invocation arguments exceed {0} bytes")]
    ArgumentsTooLarge(usize),
    #[error("progress summary exceeds {0} bytes")]
    ProgressSummaryTooLarge(usize),
    #[error("failure message exceeds {0} bytes")]
    FailureMessageTooLarge(usize),
    #[error("denial reason code exceeds {0} bytes")]
    ReasonCodeTooLarge(usize),
    #[error("illegal invocation status transition from {from:?} to {to:?}")]
    IllegalTransition {
        from: ToolInvocationStatus,
        to: ToolInvocationStatus,
    },
    #[error("invocation is in immutable terminal status {0:?}")]
    TerminalImmutable(ToolInvocationStatus),
    #[error("progress may only be journaled while running")]
    ProgressWhileNotRunning,
    /// Carries the underlying [`InvocationJournalError`] (rather than
    /// discarding it) so a genuine sequence conflict remains diagnosable —
    /// it still identifies the invocation and sequence that collided.
    #[error("invocation journal rejected the entry: {0}")]
    Journal(InvocationJournalError),
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InvocationJournalError {
    #[error("invocation {invocation_id:?} sequence {sequence} was already journaled with a different event")]
    SequenceConflict {
        invocation_id: InvocationId,
        sequence: u64,
    },
}

/// Narrow journal port. Persistence adapters bind in a later task; Task 1's
/// unit tests use [`InMemoryInvocationJournal`].
pub trait InvocationJournal: Send {
    fn record(
        &mut self,
        invocation_id: &InvocationId,
        sequence: u64,
        event: ToolInvocationEvent,
    ) -> Result<(), InvocationJournalError>;
}

/// One journaled entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournaledInvocationEntry {
    pub invocation_id: InvocationId,
    pub sequence: u64,
    pub event: ToolInvocationEvent,
}

/// In-memory journal proving journal-before-status-change ordering and
/// idempotent replay: recording the identical `(invocation_id, sequence)`
/// pair with the identical event is a no-op, so a retried request with a
/// stable invocation ID never double-journals. Recording the same pair with
/// a *different* event is a genuine conflict and is rejected.
#[derive(Clone, Debug, Default)]
pub struct InMemoryInvocationJournal {
    entries: Vec<JournaledInvocationEntry>,
}

impl InMemoryInvocationJournal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[JournaledInvocationEntry] {
        &self.entries
    }

    #[must_use]
    pub fn entries_for(&self, invocation_id: &InvocationId) -> Vec<&JournaledInvocationEntry> {
        self.entries
            .iter()
            .filter(|entry| &entry.invocation_id == invocation_id)
            .collect()
    }
}

impl InvocationJournal for InMemoryInvocationJournal {
    fn record(
        &mut self,
        invocation_id: &InvocationId,
        sequence: u64,
        event: ToolInvocationEvent,
    ) -> Result<(), InvocationJournalError> {
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| &entry.invocation_id == invocation_id && entry.sequence == sequence)
        {
            return if existing.event == event {
                Ok(())
            } else {
                Err(InvocationJournalError::SequenceConflict {
                    invocation_id: invocation_id.clone(),
                    sequence,
                })
            };
        }
        self.entries.push(JournaledInvocationEntry {
            invocation_id: invocation_id.clone(),
            sequence,
            event,
        });
        Ok(())
    }
}

/// Outcome of a validated, journaled invocation transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationTransitionOutcome {
    pub event: ToolInvocationEvent,
    pub sequence: u64,
    pub status: ToolInvocationStatus,
}

/// A live tool invocation. Identity fields (`invocation_id`, `run_id`,
/// `tool_id`, `actor`, `deadline`) are set once at [`Self::open`]; `status`
/// and `sequence` are the only mutable fields, and only through the
/// transition methods below.
pub struct ToolInvocation {
    invocation_id: InvocationId,
    run_id: RunId,
    tool_id: ToolId,
    arguments: Value,
    actor: Actor,
    deadline: Instant,
    status: ToolInvocationStatus,
    sequence: u64,
}

impl fmt::Debug for ToolInvocation {
    /// Deliberately omits `arguments`' content: raw tool arguments must never
    /// reach a log line (see `src-tauri/src/credentials`'s redaction
    /// discipline). Only the byte length is reported.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolInvocation")
            .field("invocation_id", &self.invocation_id)
            .field("run_id", &self.run_id)
            .field("tool_id", &self.tool_id)
            .field("arguments_bytes", &argument_bytes(&self.arguments))
            .field("actor", &self.actor)
            .field("status", &self.status)
            .field("sequence", &self.sequence)
            .finish()
    }
}

impl ToolInvocation {
    /// Constructs an invocation and journals its `Requested` event as the
    /// first, sequence-1 journal entry.
    pub fn open(
        invocation_id: InvocationId,
        run_id: RunId,
        tool_id: ToolId,
        arguments: Value,
        actor: Actor,
        deadline: Instant,
        journal: &mut dyn InvocationJournal,
    ) -> Result<(Self, InvocationTransitionOutcome), InvocationError> {
        let bytes = argument_bytes(&arguments);
        if bytes > MAX_ARGUMENTS_BYTES {
            return Err(InvocationError::ArgumentsTooLarge(MAX_ARGUMENTS_BYTES));
        }

        let mut invocation = Self {
            invocation_id,
            run_id,
            tool_id,
            arguments,
            actor,
            deadline,
            status: ToolInvocationStatus::Requested,
            sequence: 0,
        };

        let event = ToolInvocationEvent::requested(
            invocation.invocation_id.clone(),
            invocation.run_id.clone(),
            invocation.tool_id.clone(),
        );
        journal
            .record(&invocation.invocation_id, 1, event.clone())
            .map_err(InvocationError::Journal)?;
        invocation.sequence = 1;

        let outcome = InvocationTransitionOutcome {
            event,
            sequence: 1,
            status: ToolInvocationStatus::Requested,
        };
        Ok((invocation, outcome))
    }

    #[must_use]
    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    #[must_use]
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }

    #[must_use]
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    #[must_use]
    pub fn status(&self) -> ToolInvocationStatus {
        self.status
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn is_deadline_exceeded(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn require_approval(
        &mut self,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let event = ToolInvocationEvent::approval_required(
            self.invocation_id.clone(),
            self.tool_id.clone(),
        );
        self.transition(ToolInvocationStatus::AwaitingApproval, event, journal)
    }

    pub fn approve(
        &mut self,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let event = ToolInvocationEvent::approved(self.invocation_id.clone());
        self.transition(ToolInvocationStatus::Approved, event, journal)
    }

    /// Denies the invocation, journaling `denial.code()` — the stable
    /// machine code — never `denial`'s `Display` text, which may embed a
    /// debug-formatted [`ToolId`]. Accepting `&PolicyDenial` here (rather
    /// than an arbitrary string) is what makes that guarantee structural
    /// instead of a convention callers could get wrong.
    pub fn deny(
        &mut self,
        denial: &PolicyDenial,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        self.deny_with_reason_code(denial.code(), journal)
    }

    /// Test-only escape hatch mirroring
    /// `ToolDescriptor::for_test_with_raw_schema`: every real
    /// [`PolicyDenial::code`] is a short, fixed constant that can never trip
    /// [`InvocationError::ReasonCodeTooLarge`] through the public [`Self::deny`]
    /// API, so this lets a test exercise the bound directly and prove it is
    /// still enforced (defense in depth) rather than dead code.
    #[cfg(test)]
    pub(crate) fn deny_with_raw_reason_code(
        &mut self,
        reason_code: impl Into<String>,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        self.deny_with_reason_code(reason_code, journal)
    }

    fn deny_with_reason_code(
        &mut self,
        reason_code: impl Into<String>,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let reason_code = reason_code.into();
        if reason_code.len() > MAX_REASON_CODE_BYTES {
            return Err(InvocationError::ReasonCodeTooLarge(MAX_REASON_CODE_BYTES));
        }
        let event = ToolInvocationEvent::denied(self.invocation_id.clone(), reason_code);
        self.transition(ToolInvocationStatus::Denied, event, journal)
    }

    pub fn start(
        &mut self,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let event = ToolInvocationEvent::started(self.invocation_id.clone());
        self.transition(ToolInvocationStatus::Running, event, journal)
    }

    /// Journals a progress summary without changing status. Only valid
    /// while [`ToolInvocationStatus::Running`].
    pub fn progress(
        &mut self,
        summary: impl Into<String>,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        if self.status != ToolInvocationStatus::Running {
            return Err(InvocationError::ProgressWhileNotRunning);
        }
        let summary = summary.into();
        if summary.len() > MAX_PROGRESS_SUMMARY_BYTES {
            return Err(InvocationError::ProgressSummaryTooLarge(
                MAX_PROGRESS_SUMMARY_BYTES,
            ));
        }
        let next_sequence = self.sequence + 1;
        let event =
            ToolInvocationEvent::progress(self.invocation_id.clone(), next_sequence, summary);
        journal
            .record(&self.invocation_id, next_sequence, event.clone())
            .map_err(InvocationError::Journal)?;
        self.sequence = next_sequence;
        Ok(InvocationTransitionOutcome {
            event,
            sequence: next_sequence,
            status: self.status,
        })
    }

    pub fn complete(
        &mut self,
        truncated: bool,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let event = ToolInvocationEvent::completed(self.invocation_id.clone(), truncated);
        self.transition(ToolInvocationStatus::Completed, event, journal)
    }

    pub fn fail(
        &mut self,
        message: impl Into<String>,
        retryable: bool,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let message = message.into();
        if message.len() > MAX_FAILURE_MESSAGE_BYTES {
            return Err(InvocationError::FailureMessageTooLarge(
                MAX_FAILURE_MESSAGE_BYTES,
            ));
        }
        let event = ToolInvocationEvent::failed(self.invocation_id.clone(), message, retryable);
        self.transition(ToolInvocationStatus::Failed, event, journal)
    }

    /// Requests cancellation. Idempotent when already
    /// [`ToolInvocationStatus::Cancelled`] (mirrors `Run::request_cancel`).
    pub fn cancel(
        &mut self,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        if self.status == ToolInvocationStatus::Cancelled {
            return Ok(InvocationTransitionOutcome {
                event: ToolInvocationEvent::cancelled(self.invocation_id.clone()),
                sequence: self.sequence,
                status: ToolInvocationStatus::Cancelled,
            });
        }
        let event = ToolInvocationEvent::cancelled(self.invocation_id.clone());
        self.transition(ToolInvocationStatus::Cancelled, event, journal)
    }

    fn transition(
        &mut self,
        to: ToolInvocationStatus,
        event: ToolInvocationEvent,
        journal: &mut dyn InvocationJournal,
    ) -> Result<InvocationTransitionOutcome, InvocationError> {
        let from = self.status;
        if is_terminal(from) {
            return Err(InvocationError::TerminalImmutable(from));
        }
        if !allowed_transition(from, to) {
            return Err(InvocationError::IllegalTransition { from, to });
        }

        let next_sequence = self.sequence + 1;
        journal
            .record(&self.invocation_id, next_sequence, event.clone())
            .map_err(InvocationError::Journal)?;

        self.sequence = next_sequence;
        self.status = to;
        Ok(InvocationTransitionOutcome {
            event,
            sequence: next_sequence,
            status: to,
        })
    }
}

pub(crate) fn argument_bytes(arguments: &Value) -> usize {
    serde_json::to_vec(arguments)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn allowed_transition(from: ToolInvocationStatus, to: ToolInvocationStatus) -> bool {
    use ToolInvocationStatus::{
        Approved, AwaitingApproval, Cancelled, Completed, Denied, Failed, Requested, Running,
    };
    matches!(
        (from, to),
        (Requested, AwaitingApproval)
            | (Requested, Approved)
            | (Requested, Denied)
            | (AwaitingApproval, Approved)
            | (AwaitingApproval, Denied)
            | (AwaitingApproval, Cancelled)
            | (Approved, Running)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Cancelled)
    )
}

fn is_terminal(status: ToolInvocationStatus) -> bool {
    matches!(
        status,
        ToolInvocationStatus::Denied
            | ToolInvocationStatus::Completed
            | ToolInvocationStatus::Failed
            | ToolInvocationStatus::Cancelled
    )
}

/// An opaque reference to bounded/streamed tool content. Real content
/// storage and retrieval belongs to a later task (e.g. an MCP content
/// store); Task 1 only needs a stable handle plus a declared byte length so
/// `ToolResult` can bound how much it retains without holding raw bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentRef {
    id: String,
    byte_length: usize,
}

impl ContentRef {
    #[must_use]
    pub fn new(id: impl Into<String>, byte_length: usize) -> Self {
        Self {
            id: id.into(),
            byte_length,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn byte_length(&self) -> usize {
        self.byte_length
    }
}

/// A tool's exact result, bounded and provenance-tagged. Raw tool output is
/// untrusted. Construction fails when its structured value exceeds the exact
/// retention ceiling, so an oversized value can never become a successful
/// result with missing data. `truncated` applies only to bounded content-ref
/// metadata supplied alongside the exact structured value.
///
/// Provenance reuses [`ToolSource`] rather than a duplicate enum: "where a
/// result came from" is the same closed question as "where a tool comes
/// from" (see `registry.rs`'s `ToolDescriptor::source`).
#[derive(Clone)]
pub struct ToolResult {
    content_refs: Vec<ContentRef>,
    structured: Option<Value>,
    truncated: bool,
    provenance: ToolSource,
}

/// A trust-boundary rejection while constructing an exact [`ToolResult`].
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ToolResultError {
    #[error("structured tool result is {actual_bytes} bytes; maximum is {max_bytes} bytes")]
    StructuredResultTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
}

/// Why a model-visible preview differs from the exact durable payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolResultPreviewStrategy {
    Complete,
    JsonStructure,
    TextEdges,
    MetadataOnly,
}

impl ToolResultPreviewStrategy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::JsonStructure => "json_structure",
            Self::TextEdges => "text_edges",
            Self::MetadataOnly => "metadata_only",
        }
    }
}

/// Bounded model input derived from one exact result.
pub struct ToolResultPreview {
    value: Value,
    exact_byte_length: usize,
    sha256: String,
    complete: bool,
    strategy: ToolResultPreviewStrategy,
}

impl ToolResultPreview {
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub const fn exact_byte_length(&self) -> usize {
        self.exact_byte_length
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    #[must_use]
    pub const fn complete(&self) -> bool {
        self.complete
    }

    #[must_use]
    pub const fn strategy(&self) -> ToolResultPreviewStrategy {
        self.strategy
    }
}

impl ToolResult {
    /// Constructs a result without ever degrading an oversized structured
    /// value into `None`. Callers must propagate the typed rejection as a tool
    /// execution failure.
    pub fn new(
        mut content_refs: Vec<ContentRef>,
        structured: Option<Value>,
        provenance: ToolSource,
    ) -> Result<Self, ToolResultError> {
        let mut truncated = false;
        if content_refs.len() > MAX_CONTENT_REFS {
            content_refs.truncate(MAX_CONTENT_REFS);
            truncated = true;
        }
        if let Some(value) = structured.as_ref() {
            let size = serde_json::to_vec(&value)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX);
            if size > MAX_EXACT_TOOL_RESULT_BYTES {
                return Err(ToolResultError::StructuredResultTooLarge {
                    actual_bytes: size,
                    max_bytes: MAX_EXACT_TOOL_RESULT_BYTES,
                });
            }
        }
        Ok(Self {
            content_refs,
            structured,
            truncated,
            provenance,
        })
    }

    #[must_use]
    pub fn content_refs(&self) -> &[ContentRef] {
        &self.content_refs
    }

    #[must_use]
    pub fn structured(&self) -> Option<&Value> {
        self.structured.as_ref()
    }

    /// Canonical exact bytes persisted for later range retrieval.
    ///
    /// JSON is the tool-result storage contract. `serde_json::Value` has no
    /// non-finite numbers and its map representation is deterministic, so the
    /// digest and uploaded bytes cannot drift between prepare and finalize.
    #[must_use]
    pub fn exact_json_bytes(&self) -> Vec<u8> {
        match self.structured.as_ref() {
            Some(value) => {
                serde_json::to_vec(value).expect("serde_json::Value serialization cannot fail")
            }
            None => b"null".to_vec(),
        }
    }

    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    #[must_use]
    pub fn provenance(&self) -> ToolSource {
        self.provenance
    }

    /// Builds the bounded representation used in model context without
    /// changing the exact structured value retained for persistence. It fits
    /// `budget`'s result share, so the rendered outcome around it fits the
    /// whole budget (spec: agent-context-budget).
    #[must_use]
    pub fn model_preview(&self, budget: ToolResultBudget) -> ToolResultPreview {
        let limit = budget.result_bytes();
        let Some(structured) = self.structured.as_ref() else {
            let exact = b"null";
            return ToolResultPreview {
                value: Value::Null,
                exact_byte_length: exact.len(),
                sha256: sha256_hex(exact),
                complete: !self.truncated,
                strategy: ToolResultPreviewStrategy::MetadataOnly,
            };
        };

        let exact =
            serde_json::to_vec(structured).expect("serde_json::Value serialization cannot fail");
        if exact.len() <= limit {
            return ToolResultPreview {
                value: structured.clone(),
                exact_byte_length: exact.len(),
                sha256: sha256_hex(&exact),
                complete: true,
                strategy: ToolResultPreviewStrategy::Complete,
            };
        }

        let mut value = fit(structured, exact.len(), limit);
        if serialized_len(&value) > limit {
            value = serde_json::json!({
                "kind": value_kind(structured),
                "note": "Exact result is available through its durable payload reference.",
            });
        }
        ToolResultPreview {
            value,
            exact_byte_length: exact.len(),
            sha256: sha256_hex(&exact),
            complete: false,
            strategy: match structured {
                Value::String(_) => ToolResultPreviewStrategy::TextEdges,
                Value::Null | Value::Bool(_) | Value::Number(_) => {
                    ToolResultPreviewStrategy::Complete
                }
                Value::Array(_) | Value::Object(_) => ToolResultPreviewStrategy::JsonStructure,
            },
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn bounded_name(value: &str) -> String {
    let cut = floor_char_boundary(value, PREVIEW_NAME_BYTES);
    if cut == value.len() {
        value.to_owned()
    } else {
        format!("{}…", &value[..cut])
    }
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// The start of the longest suffix of `text` whose escaped length is at most
/// `limit`, on a character boundary, never before `floor`.
fn escaped_suffix_start(text: &str, limit: usize, floor: usize) -> usize {
    let mut used = 0;
    let mut start = text.len();
    for (index, character) in text[floor..].char_indices().rev() {
        let mut buffer = [0u8; 4];
        used += json_string_len(character.encode_utf8(&mut buffer));
        if used > limit {
            break;
        }
        start = floor + index;
    }
    start
}

/// `value` reduced to at most `limit` serialized bytes, keeping as much of it
/// as fits: leading rows, whole small fields, the head and tail of a string.
/// Every reduction says what it dropped, so the model never mistakes a part
/// for the whole.
///
/// `size` is `value`'s serialized length, which every caller already has.
/// When not even a reduction fits, the value is named and measured instead
/// ([`omitted`]); a caller with less room than that marker charges it at its
/// real size.
fn fit(value: &Value, size: usize, limit: usize) -> Value {
    if size <= limit {
        return value.clone();
    }
    let reduced = match value {
        Value::String(text) => fit_text(text, limit),
        Value::Array(items) => fit_array(items, limit),
        Value::Object(fields) => fit_object(fields, limit),
        Value::Null | Value::Bool(_) | Value::Number(_) => return omitted(value, size),
    };
    if serialized_len(&reduced) <= limit {
        reduced
    } else {
        omitted(value, size)
    }
}

fn omitted(value: &Value, size: usize) -> Value {
    serde_json::json!({
        "kind": "omitted",
        "type": value_kind(value),
        "byte_length": size,
    })
}

/// Head and tail, three quarters to the head: the start of a document says
/// what it is, and the end says where it stopped.
fn fit_text(text: &str, limit: usize) -> Value {
    let preview = |head: &str, tail: &str, omitted_bytes: usize| {
        serde_json::json!({
            "kind": "text",
            "head": head,
            "tail": tail,
            "omitted_bytes": omitted_bytes,
            "total_bytes": text.len(),
        })
    };
    // The envelope at its largest: the omitted count only shrinks as text is
    // kept, so what fits beside this fits beside the real one.
    let room = limit.saturating_sub(serialized_len(&preview("", "", text.len())));
    let head_end = json_prefix_end(text, room / 4 * 3);
    let tail_room = room - json_string_len(&text[..head_end]);
    let tail_start = escaped_suffix_start(text, tail_room, head_end);
    preview(
        &text[..head_end],
        &text[tail_start..],
        tail_start - head_end,
    )
}

/// Leading rows, whole, while they fit; the first row that does not fit is
/// itself fitted when nothing else was kept, so a result of one huge row
/// still shows part of it.
fn fit_array(items: &[Value], limit: usize) -> Value {
    let mut columns = items
        .iter()
        .filter_map(Value::as_object)
        .flat_map(|row| row.keys())
        .map(|name| bounded_name(name))
        .collect::<Vec<_>>();
    columns.sort();
    columns.dedup();
    columns.truncate(PREVIEW_FIELD_LIMIT);
    let header = serde_json::json!({
        "kind": if columns.is_empty() { "array" } else { "table" },
        "item_count": items.len(),
        "columns": columns,
        "items": [],
        "omitted_items": items.len(),
    });
    // Every kept row costs its bytes and a separator. The omitted count only
    // shrinks as rows are kept, so the header never outgrows this measure.
    let mut room = limit.saturating_sub(serialized_len(&header));
    let mut kept = Vec::new();
    for item in items {
        let size = serialized_len(item);
        if size < room {
            room -= size + 1;
            kept.push(item.clone());
            continue;
        }
        if kept.is_empty() && room > MIN_FITTED_BYTES {
            let fitted = fit(item, size, room - 1);
            if serialized_len(&fitted) < room {
                kept.push(fitted);
            }
        }
        break;
    }
    let omitted_items = items.len() - kept.len();
    let mut preview = header;
    preview["items"] = Value::Array(kept);
    preview["omitted_items"] = omitted_items.into();
    preview
}

/// Small fields whole, then the large ones fitted into what is left, shared
/// evenly, in the object's own field order. A field too large for any useful
/// share is named and measured; one with no room even for that is dropped,
/// and then the object is wrapped so the count of dropped fields is said.
fn fit_object(fields: &serde_json::Map<String, Value>, limit: usize) -> Value {
    // A field costs its escaped key, the key's quotes, a colon, and a comma.
    let entry_cost = |name: &str| json_string_len(name) + 4;
    let entries = fields
        .iter()
        .map(|(name, value)| (name, value, serialized_len(value)))
        .collect::<Vec<_>>();
    let wrapped = |kept: serde_json::Map<String, Value>, omitted_fields: usize| {
        serde_json::json!({
            "kind": "object",
            "field_count": entries.len(),
            "fields": kept,
            "omitted_fields": omitted_fields,
        })
    };
    // Room for the wrapper is reserved up front, so dropping a field never
    // needs a second pass over the ones already fitted.
    let mut room = limit.saturating_sub(serialized_len(&wrapped(
        serde_json::Map::new(),
        entries.len(),
    )));

    // Cheapest first; the sort is stable, so ties keep the object's order.
    let mut order = (0..entries.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| entry_cost(entries[index].0) + entries[index].2);
    let mut chosen: Vec<Option<Value>> = vec![None; entries.len()];
    let mut whole = 0;
    for &index in &order {
        let (name, value, size) = entries[index];
        let cost = entry_cost(name) + size;
        if cost > room {
            break;
        }
        room -= cost;
        chosen[index] = Some(value.clone());
        whole += 1;
    }
    let rest = &order[whole..];
    for (done, &index) in rest.iter().enumerate() {
        let (name, value, size) = entries[index];
        let key = entry_cost(name);
        let share = room / (rest.len() - done);
        let Some(value_room) = share.checked_sub(key) else {
            continue;
        };
        let fitted = if value_room >= MIN_FITTED_BYTES {
            fit(value, size, value_room)
        } else {
            omitted(value, size)
        };
        let cost = key + serialized_len(&fitted);
        if cost <= share {
            room -= cost;
            chosen[index] = Some(fitted);
        }
    }

    let dropped = chosen.iter().filter(|choice| choice.is_none()).count();
    let kept = entries
        .iter()
        .zip(chosen)
        .filter_map(|((name, _, _), choice)| choice.map(|value| ((*name).clone(), value)))
        .collect::<serde_json::Map<_, _>>();
    if dropped == 0 {
        Value::Object(kept)
    } else {
        wrapped(kept, dropped)
    }
}

const fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl fmt::Debug for ToolResult {
    /// Deliberately omits `structured`'s content: raw tool output is
    /// untrusted and must never reach a log line unredacted.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolResult")
            .field("content_ref_count", &self.content_refs.len())
            .field("structured_present", &self.structured.is_some())
            .field("truncated", &self.truncated)
            .field("provenance", &self.provenance)
            .finish()
    }
}
