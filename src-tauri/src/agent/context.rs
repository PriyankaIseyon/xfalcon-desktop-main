//! Bounded, provider-neutral message context for model calls.

use std::fmt;

use uuid::Uuid;

use crate::tools::exact_result::AdmittedToolResultPayload;

/// Role of a context message (provider-neutral).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DurableMessageIdentity {
    pub(crate) id: Uuid,
    pub(crate) role: String,
    pub(crate) sequence_no: i64,
}

/// One structured tool call an assistant turn requested.
///
/// `arguments` stays raw JSON *text*, exactly as the provider streamed it,
/// rather than a parsed `serde_json::Value`. Three reasons, in order of
/// weight:
///
/// 1. **Byte-exact round trip.** OpenAI requires the follow-up request to
///    echo `function.arguments` back as a string; re-serializing a parsed
///    value can reorder keys and change the bytes the model itself produced.
/// 2. **One parse site.** Arguments are attacker-influenceable content
///    (`docs/threat-model.md` TM-006). Parsing exactly once, in the
///    layer that can turn a failure into a typed invalid-arguments outcome,
///    beats parsing defensively at every hop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

impl ContextToolCall {
    #[must_use]
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }
}

/// Identifies the call a [`ContextRole::Tool`] message answers.
///
/// Both fields are load-bearing because the three supported providers key a
/// tool result differently: OpenAI by `tool_call_id`, Anthropic by
/// `tool_use_id`, and Google by function *name* (its `functionResponse` wire
/// shape has no id field at all). Dropping either one would make some
/// provider's follow-up request unconstructible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextToolRef {
    pub call_id: String,
    pub name: String,
}

/// An image type this build knows how to hand to a provider.
///
/// Closed, and a closed set is the point: every provider names the image types
/// it accepts, so carrying a free-form string would move the failure from this
/// boundary — where it can be reported against the file the user chose — to an
/// opaque provider rejection in the middle of a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageMediaType {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageMediaType {
    /// The `image/...` string a provider request carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// Parses a sniffed or declared media type, rejecting anything this build
    /// cannot render rather than passing it through for a provider to refuse.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

/// Something a message carries that is not prose.
///
/// One variant today. The enum is closed so that adding a kind forces a
/// decision about which budget it spends — a document becomes extracted text
/// and is already bounded as prose, while an image is bytes and is not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextAttachment {
    Image {
        media_type: ImageMediaType,
        bytes: Vec<u8>,
    },
}

impl ContextAttachment {
    /// Bytes this attachment costs against the image ceiling.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Image { bytes, .. } => bytes.len(),
        }
    }
}

/// One message slot in the bounded context window.
///
/// `PartialEq` without `Eq`: `reasoning_details` holds opaque provider JSON,
/// and `serde_json::Value` is not `Eq`. Nothing keys a map or set by a message,
/// so total equality was only ever a derive.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextMessage {
    pub role: ContextRole,
    pub content: String,
    /// Set when this message's `content` is not the whole thing. Tool-result
    /// preview construction owns that explicit state. [`ContextBuilder`] never
    /// changes it. Always `false` for messages built via [`Self::new`].
    pub truncated: bool,
    /// [`ContextRole::Assistant`] turns only: the tool calls the model
    /// requested on this turn. Empty for every other role and for a plain
    /// text answer.
    pub tool_calls: Vec<ContextToolCall>,
    /// [`ContextRole::Tool`] messages only: which call this result answers.
    pub responds_to: Option<ContextToolRef>,
    /// Non-prose content this turn carries — today, images the user attached.
    ///
    /// Budgeted apart from `content`; see [`Self::byte_len`] and
    /// [`Self::image_bytes`].
    pub attachments: Vec<ContextAttachment>,
    /// [`ContextRole::Assistant`] turns only: the provider's own reasoning
    /// blocks for this turn, exactly as it streamed them.
    ///
    /// Carried so the next turn of a tool loop can hand them back unchanged.
    /// A provider that reasons over tool results rejects a turn whose thinking
    /// was rebuilt, reordered, or dropped, so this is opaque by construction —
    /// no consumer in this app reads inside it. Empty for every provider that
    /// streams none, which keeps the rendered request byte-identical to what
    /// it was before this field existed.
    pub reasoning_details: Vec<serde_json::Value>,
    /// DA identity used only for durable bootstrap boundaries and digests.
    /// Provider serializers must ignore it.
    pub(crate) durable_identity: Option<DurableMessageIdentity>,
    pub(crate) durable_tool_result_payloads: Vec<AdmittedToolResultPayload>,
}

impl ContextMessage {
    #[must_use]
    pub fn new(role: ContextRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            truncated: false,
            tool_calls: Vec::new(),
            responds_to: None,
            attachments: Vec::new(),
            reasoning_details: Vec::new(),
            durable_identity: None,
            durable_tool_result_payloads: Vec::new(),
        }
    }

    /// Builds the [`ContextRole::Assistant`] turn in which the model asked to
    /// run one or more tools. `content` is the turn's accompanying text, which
    /// is routinely empty — a model answering purely with tool calls emits no
    /// prose, and that turn still has to enter the context or the next round
    /// loses the calls it must report results against.
    #[must_use]
    pub fn assistant_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ContextToolCall>,
    ) -> Self {
        Self {
            role: ContextRole::Assistant,
            content: content.into(),
            truncated: false,
            tool_calls,
            responds_to: None,
            attachments: Vec::new(),
            reasoning_details: Vec::new(),
            durable_identity: None,
            durable_tool_result_payloads: Vec::new(),
        }
    }

    /// Attaches the provider's reasoning blocks to this assistant turn so the
    /// next round of the tool loop can echo them back unchanged.
    #[must_use]
    pub fn with_reasoning_details(mut self, reasoning_details: Vec<serde_json::Value>) -> Self {
        self.reasoning_details = reasoning_details;
        self
    }

    /// Builds a [`ContextRole::Tool`] message from an already bounded, typed
    /// tool-result preview. This layer does not apply a second hidden prefix
    /// cut: live construction and history restoration must send equivalent
    /// content, while [`ContextBuilder`] still enforces the whole request.
    #[must_use]
    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        Self {
            role: ContextRole::Tool,
            content,
            truncated: false,
            tool_calls: Vec::new(),
            responds_to: Some(ContextToolRef {
                call_id: call_id.into(),
                name: name.into(),
            }),
            attachments: Vec::new(),
            reasoning_details: Vec::new(),
            durable_identity: None,
            durable_tool_result_payloads: Vec::new(),
        }
    }

    /// Builds the user turn that carries attachments.
    ///
    /// `content` may be empty: handing over a picture with nothing to say about
    /// it is the most ordinary way to attach one, and [`Self::is_empty`]
    /// accounts for that.
    #[must_use]
    pub fn user_with_attachments(
        content: impl Into<String>,
        attachments: Vec<ContextAttachment>,
    ) -> Self {
        Self {
            role: ContextRole::User,
            content: content.into(),
            truncated: false,
            tool_calls: Vec::new(),
            responds_to: None,
            attachments,
            reasoning_details: Vec::new(),
            durable_identity: None,
            durable_tool_result_payloads: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_durable_identity(mut self, identity: DurableMessageIdentity) -> Self {
        self.durable_identity = Some(identity);
        self
    }

    #[must_use]
    pub(crate) fn with_durable_tool_result_payloads(
        mut self,
        payloads: Vec<AdmittedToolResultPayload>,
    ) -> Self {
        self.durable_tool_result_payloads = payloads;
        self
    }

    /// Bytes this message costs against the *prose* budget.
    ///
    /// Tool-call payloads count: they are serialized into the provider request
    /// verbatim and the model is billed for them exactly like prose. Counting
    /// only `content` would let a round of large-argument tool calls slip past
    /// [`ContextBuilder`]'s byte budget entirely.
    ///
    /// Reasoning and attachments count because hard admission covers the full
    /// serialized request. Attachments use the exact base64 expansion ceiling.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        let attachments = self.attachments.iter().fold(0usize, |total, attachment| {
            let bytes = attachment.byte_len();
            total.saturating_add(bytes.saturating_add(2) / 3 * 4)
        });
        self.prose_byte_len().saturating_add(attachments)
    }

    /// Bytes of text the provider tokenizes for this message: its prose, tool
    /// calls, and reasoning. Attachments are not text and are counted apart
    /// ([`Self::attachment_count`]).
    #[must_use]
    pub(crate) fn text_bytes(&self) -> usize {
        self.prose_byte_len()
    }

    /// How many attachments this message carries.
    #[must_use]
    pub(crate) fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    fn prose_byte_len(&self) -> usize {
        let structured = self
            .reasoning_details
            .iter()
            .map(|value| serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()))
            .fold(0usize, usize::saturating_add);
        self.content.len()
            + self
                .tool_calls
                .iter()
                .map(|call| call.call_id.len() + call.name.len() + call.arguments.len())
                .sum::<usize>()
            + structured
    }

    /// Bytes this message costs against the image ceiling.
    #[must_use]
    pub fn image_bytes(&self) -> usize {
        self.attachments
            .iter()
            .map(ContextAttachment::byte_len)
            .sum()
    }

    /// Whether this message carries anything a provider can serialize.
    ///
    /// Emptiness is judged across every dimension a message can carry. A turn
    /// with no text is still meaningful when it holds tool calls — the shape of
    /// every tool round trip — or an attachment, which is what "look at this"
    /// with no words looks like.
    #[must_use]
    fn is_empty(&self) -> bool {
        self.content.is_empty() && self.tool_calls.is_empty() && self.attachments.is_empty()
    }
}

/// Errors when building a bounded context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextError {
    MessageLimitExceeded,
    ByteLimitExceeded,
    EmptyContent,
    /// One attachment exceeds what a single message, or the whole context, may
    /// hold in images. Distinct from [`Self::ByteLimitExceeded`] so a caller can
    /// tell the user which file to replace instead of blaming the conversation.
    AttachmentTooLarge,
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageLimitExceeded => write!(f, "context message limit exceeded"),
            Self::ByteLimitExceeded => write!(f, "context byte limit exceeded"),
            Self::EmptyContent => write!(f, "context message content must not be empty"),
            Self::AttachmentTooLarge => write!(f, "attachment exceeds the context image budget"),
        }
    }
}

impl std::error::Error for ContextError {}

/// Role the run-status turn is sent under.
///
/// Deliberately [`ContextRole::User`] and not [`ContextRole::System`]: the
/// Anthropic adapter hoists every system turn into the top-level `system`
/// string, which would move this out of last position — and last position is
/// the whole point, since the model reads it immediately before choosing its
/// next move. A user turn lands last under all three adapters, and the
/// `<run-status>` wrapper marks it as the harness speaking rather than the
/// person.
const RUN_STATUS_ROLE: ContextRole = ContextRole::User;

/// Builds a message list with strict, no-loss admission.
///
/// Capacity pressure never mutates, truncates, or evicts existing messages.
/// The turn driver performs durable provider-native compaction before it sends
/// an oversized request.
#[derive(Clone, Debug)]
pub struct ContextBuilder {
    max_messages: usize,
    max_bytes: usize,
    /// Ceiling on image bytes held across the whole context.
    max_image_bytes: usize,
    /// Ceiling on any single attachment. Enforced at admission rather than by
    /// truncation — half an image is not an image.
    max_attachment_bytes: usize,
    messages: Vec<ContextMessage>,
    total_bytes: usize,
    /// Image bytes currently held, tracked apart from `total_bytes`.
    total_image_bytes: usize,
    /// The run-status turn, held outside the message list.
    ///
    /// Only its newest form is true — a stale "3 rounds remaining" is worse
    /// than none — so it is replaced rather than appended. Keeping it out of
    /// the list means it occupies no message slot, cannot be evicted to make
    /// room for the very rounds it is reporting on, and is emitted last by
    /// [`Self::build`], which is where the model reads it: immediately before
    /// deciding whether to call another tool. Its bytes are still charged
    /// against the budget.
    run_status: Option<ContextMessage>,
}

impl ContextBuilder {
    /// A builder with no room for images. Retained for callers that predate
    /// attachments, so every existing construction keeps its exact behavior.
    #[must_use]
    pub fn new(max_messages: usize, max_bytes: usize) -> Self {
        Self::with_image_budget(max_messages, max_bytes, 0, 0)
    }

    #[must_use]
    pub fn with_image_budget(
        max_messages: usize,
        max_bytes: usize,
        max_image_bytes: usize,
        max_attachment_bytes: usize,
    ) -> Self {
        Self {
            max_messages: max_messages.max(1),
            max_bytes: max_bytes.max(1),
            max_image_bytes,
            max_attachment_bytes,
            messages: Vec::new(),
            total_bytes: 0,
            total_image_bytes: 0,
            run_status: None,
        }
    }

    /// Replaces the run-status turn with `content`, or clears it when `content`
    /// is empty.
    ///
    /// Fails only if the status alone cannot fit the budget, which would mean
    /// the budget is smaller than one sentence. Eviction is never triggered on
    /// its behalf: a status turn that displaced real history to describe that
    /// history would be self-defeating.
    pub fn set_run_status(&mut self, content: impl Into<String>) -> Result<(), ContextError> {
        let content = content.into();
        let previous = self.run_status.take().map_or(0, |status| status.byte_len());
        self.total_bytes = self.total_bytes.saturating_sub(previous);
        if content.is_empty() {
            return Ok(());
        }
        let status = ContextMessage::new(RUN_STATUS_ROLE, content);
        let incoming = status.byte_len();
        if self.total_bytes + incoming > self.max_bytes {
            return Err(ContextError::ByteLimitExceeded);
        }
        self.total_bytes += incoming;
        self.run_status = Some(status);
        Ok(())
    }

    /// The messages held, without the run-status turn [`Self::build`] appends.
    #[must_use]
    pub(crate) fn messages(&self) -> &[ContextMessage] {
        &self.messages
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Image bytes currently held.
    #[must_use]
    pub fn total_image_bytes(&self) -> usize {
        self.total_image_bytes
    }

    /// Pushes a message only when the complete raw value fits.
    pub fn push(&mut self, message: ContextMessage) -> Result<(), ContextError> {
        // Deliberately `is_empty()`, not `content.is_empty()`: an assistant
        // turn that is purely tool calls has no text, and it is the single
        // most common shape in a tool round trip. Rejecting it would make the
        // feature unrepresentable. A message empty in *every* dimension is
        // still an error.
        if message.is_empty() {
            return Err(ContextError::EmptyContent);
        }

        // An oversized single attachment is refused rather than trimmed: bytes
        // cut from the middle of an image do not make a smaller image, they make
        // a corrupt one, and a provider would reject it after the run had
        // already paid to upload it.
        let incoming_images = message.image_bytes();
        if message
            .attachments
            .iter()
            .any(|attachment| attachment.byte_len() > self.max_attachment_bytes)
            || incoming_images > self.max_image_bytes
        {
            return Err(ContextError::AttachmentTooLarge);
        }

        let incoming = message.prose_byte_len();
        if incoming > self.max_bytes {
            return Err(ContextError::ByteLimitExceeded);
        }

        if self.messages.len() >= self.max_messages {
            return Err(ContextError::MessageLimitExceeded);
        }
        if self.total_bytes.saturating_add(incoming) > self.max_bytes {
            return Err(ContextError::ByteLimitExceeded);
        }
        if self.total_image_bytes.saturating_add(incoming_images) > self.max_image_bytes {
            return Err(ContextError::AttachmentTooLarge);
        }

        self.total_bytes += incoming;
        self.total_image_bytes += incoming_images;
        self.messages.push(message);
        Ok(())
    }

    /// Consumes the builder into the bounded message list (oldest → newest),
    /// with the run-status turn last when one is set.
    #[must_use]
    pub fn build(mut self) -> Vec<ContextMessage> {
        if let Some(status) = self.run_status.take() {
            self.messages.push(status);
        }
        self.messages
    }
}
