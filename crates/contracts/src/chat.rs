use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::credentials::{ModelId, ModelView};
use crate::mcp::McpServerId;
use crate::projects::ProjectId;
use crate::{ContractVersion, ConversationId, RunId, RunState, CONTRACT_VERSION};

/// The MCP server scope selected for one chat. `None` means all available servers.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatMcpSelectionView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub server_id: Option<McpServerId>,
}

impl ChatMcpSelectionView {
    #[must_use]
    pub fn new(server_id: Option<McpServerId>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            server_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ChatSyncAvailability {
    Live,
    Stale,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatConversationSummary {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub conversation_id: ConversationId,
    pub title: String,
    pub updated_at: String,
    /// RFC 3339 pin instant, `None` when unpinned — the sidebar's "Pinned"
    /// group and its recency-of-pin ordering, mirroring xfalcon-web.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
    // Device-local project grouping hint. DA never stores or returns this;
    // Desktop enriches workspace summaries from its local binding store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

impl ChatConversationSummary {
    pub fn new(
        conversation_id: ConversationId,
        title: impl Into<String>,
        updated_at: impl Into<String>,
        pinned_at: Option<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            conversation_id,
            title: title.into(),
            updated_at: updated_at.into(),
            pinned_at,
            project_id: None,
        }
    }

    #[must_use]
    pub fn with_project_id(mut self, project_id: Option<ProjectId>) -> Self {
        self.project_id = project_id;
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatWorkspaceBootstrap {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub conversations: Vec<ChatConversationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_notice: Option<String>,
}

impl ChatWorkspaceBootstrap {
    pub fn new(conversations: Vec<ChatConversationSummary>) -> Self {
        Self::with_recovery_notice(conversations, None)
    }

    pub fn with_recovery_notice(
        conversations: Vec<ChatConversationSummary>,
        recovery_notice: Option<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            conversations,
            recovery_notice,
        }
    }
}

/// The redacted model choices for this account and the model that applies to
/// the requested chat surface.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatModelCatalogView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub models: Vec<ModelView>,
    pub selected_model_id: ModelId,
    /// True for an existing conversation. The renderer may display that model
    /// but cannot replace it.
    pub selection_locked: bool,
}

impl ChatModelCatalogView {
    #[must_use]
    pub fn new(
        models: Vec<ModelView>,
        selected_model_id: impl Into<ModelId>,
        selection_locked: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            models,
            selected_model_id: selected_model_id.into(),
            selection_locked,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ChatMessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Whether a tool call a turn requested has an answer in the conversation.
///
/// Only what the durable record proves. A call whose answer is missing is shown
/// as such rather than as a success, because the two are indistinguishable to a
/// reader otherwise and only one of them is a result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ChatToolCallStatus {
    Answered,
    Unanswered,
}

/// Ceiling on one projected tool payload (arguments or result), in UTF-8
/// bytes. The transcript is a viewer, not an export surface: anything larger
/// is truncated at projection time with the matching `*_truncated` flag set,
/// and the execution log (`xf_chat.tool_calls`) stays the authoritative,
/// uncapped record.
pub const TOOL_PAYLOAD_PROJECTION_CAP_BYTES: usize = 32 * 1024;

/// Ceiling on the paths one [`ToolStepSummaryV1`] carries.
///
/// A display budget for one card, not a fact about the run — which is why
/// [`ToolStepSummaryV1::path_total`] exists: a listing of 400 entries still
/// reports 400, so the surface can say "12 of 400" instead of quietly
/// redefining 400 as 12.
pub const TOOL_STEP_PATH_CAP: usize = 12;

/// One file a step touched, as the step shows it.
///
/// Carries no `version` of its own: a path is a leaf inside a versioned parent,
/// and stamping the same constant onto twelve of these per call would put it on
/// the wire once per file for no reader's benefit.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ToolStepPathV1 {
    /// Project-relative, re-validated at projection time — the rows a summary
    /// is built from round-tripped the data plane, so the path is untrusted
    /// input on the way back regardless of who wrote it.
    pub path: String,
    /// The classifier's own verdict, carried so a withheld or refused candidate
    /// can be shown as one. Drawing it identically to an ordinary hit is the
    /// single confusion this data must never create.
    pub is_sensitive: bool,
}

/// What one tool call did, in this app's own words.
///
/// Composed Rust-side from the typed outcome this app itself authored, out of a
/// fixed vocabulary plus integers taken from that outcome. A model's arguments
/// and a server's response text are never interpolated: a label sits in the
/// position a reader trusts for a summary of what happened, so letting either
/// write one would let them write this app's interface copy.
///
/// Absent for any call this app did not execute — an MCP result is text a
/// server chose, under a shape the server chose, naming whatever paths it
/// likes, and a chip drawn from it would lend the app's own voice to
/// server-authored content (`docs/threat-model.md` TM-006: content is
/// never authority).
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ToolStepSummaryV1 {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    /// One short phrase, this app's own: "Listed 14 entries", "Read",
    /// "Blocked — a person must approve reading this file".
    pub label: String,
    /// The paths this step touched, in outcome order, at most
    /// [`TOOL_STEP_PATH_CAP`] of them.
    pub paths: Vec<ToolStepPathV1>,
    /// How many paths the outcome held BEFORE the display budget was applied
    /// and before any path was dropped for failing validation. Carried
    /// separately because a step showing twelve chips and saying nothing else
    /// would read as a call that touched twelve files.
    pub path_total: u32,
}

impl ToolStepSummaryV1 {
    #[must_use]
    pub fn new(label: impl Into<String>, paths: Vec<ToolStepPathV1>, path_total: u32) -> Self {
        Self {
            version: CONTRACT_VERSION,
            label: label.into(),
            paths,
            path_total,
        }
    }
}

/// One tool call an assistant turn requested, as the transcript shows it.
///
/// The call's arguments and result ARE projected — a deliberate narrowing of
/// the earlier "never projected" stance (recorded product sign-off,
/// 2026-08-05; parity with xfalcon-web, whose transcript shows both). The
/// original concern — both fields are attacker-influenced content (an MCP
/// server's response, a model's arguments) — is honored by rule instead of by
/// absence:
///
/// - payloads render as INERT TEXT only: no markdown, no markup, no
///   linkification, no interpretation anywhere in the UI;
/// - payloads are capped at [`TOOL_PAYLOAD_PROJECTION_CAP_BYTES`] with an
///   explicit truncation flag, so the projection stays bounded;
/// - live lifecycle detail still comes from `ToolInvocationEvent`; this is
///   what survives a reload.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatToolCallView {
    /// The provider's own call id — opaque, and the only thing that pairs a
    /// request to its answer.
    pub call_id: String,
    /// The tool name as the server advertised it.
    pub name: String,
    pub status: ChatToolCallStatus,
    /// The JSON argument text the turn advertised for this call, capped.
    /// `None` when the durable row carried none. Defaulted so snapshots
    /// sealed before this field existed still deserialize.
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default)]
    pub arguments_truncated: bool,
    /// The answer row's content, capped. `None` while the call is unanswered.
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub result_truncated: bool,
    /// What the call did, derived from the UNCAPPED outcome before
    /// [`TOOL_PAYLOAD_PROJECTION_CAP_BYTES`] is applied to `result`. `None` for
    /// an unanswered call, for a tool this app did not execute, and for any
    /// outcome it cannot read.
    ///
    /// Defaulted for the same reason its siblings are — a snapshot sealed
    /// before this field existed must still open, or the first launch after
    /// this change destroys every offline conversation. No
    /// `skip_serializing_if`: the key stays on the wire (as `null`) so one
    /// shape is pinned rather than two.
    #[serde(default)]
    pub step: Option<ToolStepSummaryV1>,
}

impl ChatToolCallView {
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        status: ChatToolCallStatus,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            status,
            arguments: None,
            arguments_truncated: false,
            result: None,
            result_truncated: false,
            step: None,
        }
    }

    #[must_use]
    pub fn with_arguments(mut self, arguments: Option<String>, truncated: bool) -> Self {
        self.arguments = arguments;
        self.arguments_truncated = truncated;
        self
    }

    #[must_use]
    pub fn with_result(mut self, result: Option<String>, truncated: bool) -> Self {
        self.result = result;
        self.result_truncated = truncated;
        self
    }

    /// Separate from [`Self::with_result`] so the call order at the projection
    /// site reads as the rule it encodes: summarize the whole outcome, then cap
    /// the copy kept for the Result panel.
    #[must_use]
    pub fn with_step(mut self, step: Option<ToolStepSummaryV1>) -> Self {
        self.step = step;
        self
    }
}

/// A file the person sent with a message, as the transcript names it.
///
/// Identity and label only: the bytes stay in DA and reach the model through
/// the run's own attachment path, never through the transcript.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatAttachmentView {
    pub attachment_id: String,
    pub file_name: String,
    pub mime_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatMessageView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub message_id: String,
    pub role: ChatMessageRole,
    pub text: String,
    pub sequence: u64,
    /// The tool round this turn performed, empty for a turn that ran none. An
    /// assistant turn may carry these and no `text` — a model answering purely
    /// with tool calls emits no prose, and that turn is still what the reader
    /// needs to see.
    pub tool_calls: Vec<ChatToolCallView>,
    /// The files sent with this turn, in the order they were attached. Empty
    /// for a turn that carried none.
    #[serde(default)]
    pub attachments: Vec<ChatAttachmentView>,
}

impl ChatMessageView {
    pub fn new(
        message_id: impl Into<String>,
        role: ChatMessageRole,
        text: impl Into<String>,
        sequence: u64,
        tool_calls: Vec<ChatToolCallView>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            message_id: message_id.into(),
            role,
            text: text.into(),
            sequence,
            tool_calls,
            attachments: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_attachments(mut self, attachments: Vec<ChatAttachmentView>) -> Self {
        self.attachments = attachments;
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatConversationView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub conversation_id: ConversationId,
    pub title: String,
    pub availability: ChatSyncAvailability,
    pub messages: Vec<ChatMessageView>,
    pub active_run_id: Option<RunId>,
}

impl ChatConversationView {
    pub fn new(
        conversation_id: ConversationId,
        title: impl Into<String>,
        availability: ChatSyncAvailability,
        messages: Vec<ChatMessageView>,
        active_run_id: Option<RunId>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            conversation_id,
            title: title.into(),
            availability,
            messages,
            active_run_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum ChatProjectionEvent {
    ConversationRemoved {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        account_id: String,
        conversation_id: ConversationId,
    },
    SnapshotReplaced {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        conversation: ChatConversationView,
    },
    RunStateChanged {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        conversation_id: ConversationId,
        run_id: RunId,
        state: RunState,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ChatSubmitStatus {
    Submitted,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatSubmitResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub status: ChatSubmitStatus,
    pub conversation_id: Option<ConversationId>,
    pub run_id: Option<String>,
    pub message: String,
}

impl ChatSubmitResult {
    pub fn submitted(conversation_id: ConversationId, run_id: impl Into<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status: ChatSubmitStatus::Submitted,
            conversation_id: Some(conversation_id),
            run_id: Some(run_id.into()),
            message: "Your message was sent to the tenant agent.".to_owned(),
        }
    }

    /// A retry that found the run already answered (spec: chat-run-recovery).
    /// `run_id` stays `None` deliberately: there is no new run for the
    /// frontend to adopt, and adopting the finished one would strand the UI
    /// in a pending state no event will ever resolve.
    pub fn already_settled(conversation_id: ConversationId) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status: ChatSubmitStatus::Submitted,
            conversation_id: Some(conversation_id),
            run_id: None,
            message: "This message was already answered — showing the existing response."
                .to_owned(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status: ChatSubmitStatus::Failed,
            conversation_id: None,
            run_id: None,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatMessageFeedback {
    pub message_id: String,
    pub rating: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatFollowUpsResult {
    pub suggestions: Vec<String>,
}

/// Why a chat operation failed, as a code the frontend can act on.
///
/// The message beside it is already user-facing prose; this code exists for the
/// decisions copy cannot carry: self-healing a conversation the server has
/// authoritatively dropped, and telling a failure that will pass on its own
/// apart from one that will not.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ChatErrorCode {
    /// No signed-in session.
    AuthRequired,
    /// Auth is not configured on this device.
    NotConfigured,
    /// The session was refused and must be renewed.
    SessionExpired,
    /// No active account is selected.
    NoActiveAccount,
    /// A send is already in flight for this conversation.
    SendInFlight,
    /// The per-account run ceiling is occupied.
    TooManyRuns,
    /// The server authoritatively answered that the row does not exist. The
    /// surface self-heals on this rather than showing a retry the user cannot win.
    NotFound,
    /// The workspace could not be reached, or answered 5xx, timed out, or rate
    /// limited. Distinct from [`Self::ServiceUnavailable`] because it passes on
    /// its own: the one failure where "try again in a moment" is true advice.
    TemporarilyUnavailable,
    /// The workspace answered outside its contract. Retrying reaches the same
    /// answer, so the copy does not invite one.
    ServiceUnavailable,
    /// A specific refusal whose reason the message already states in full.
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: ChatErrorCode,
    /// User-facing prose. Always populated, and never a machine token: the
    /// frontend renders it directly.
    pub message: String,
}

impl ChatOperationError {
    pub fn new(code: ChatErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
            message: message.into(),
        }
    }
}
