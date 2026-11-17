use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use uuid::Uuid;
use xfalcon_contracts::chat::ToolStepSummaryV1;
use xfalcon_contracts::{ConversationId, RunId, RunState};

/// Maximum UTF-8 byte length accepted for message text at construction and
/// deserialization boundaries.
///
/// This is the single bound on message text anywhere in the app: the composer
/// refuses a longer prompt through `chat::service`'s `MAX_PROMPT_LEN`, which is
/// defined as this constant rather than as its own number. They were separate
/// numbers once, and when the prompt ceiling was raised on its own an
/// over-length prompt passed the composer's friendly check and then failed here
/// -- after `authenticated_fresh`, `sync_service` and `reconcile_conversation`
/// had already run. Anything that bounds a prompt must derive from this.
///
/// It also bounds hydration: a message read back from DA is rejected past this
/// length, and one oversized stored message fails the whole conversation load.
/// DA holds what the web app wrote as well as what this app sends, so the bound
/// has to leave room for a long model answer, not just a typed prompt: it is
/// the run's own output budget (`crate::agent::output`), so any answer a run is
/// allowed to produce can be stored and loaded back (spec:
/// agent-context-budget). DA itself bounds a message only by its request size.
pub const MAX_MESSAGE_TEXT_BYTES: usize = 512 * 1024;

/// Ceiling on an advertised tool name, matching `mcp::catalog`'s own bound: a
/// name is server-supplied text and lands in the transcript.
pub const MAX_TOOL_NAME_BYTES: usize = 256;
/// Ceiling on a provider tool-call id. Every provider in the allowlist emits a
/// short opaque token; the bound exists so a hostile row cannot carry an
/// unbounded string into the projection.
pub const MAX_TOOL_CALL_ID_BYTES: usize = 256;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ChatError {
    #[error("role and content combination is invalid")]
    InvalidRoleContent,
    #[error("message or event sequence must increase monotonically")]
    SequenceNotMonotonic,
    #[error("completed message content is immutable")]
    MessageImmutable,
    #[error("message text exceeds the maximum allowed size")]
    TextTooLarge,
    #[error("chat resource was not found")]
    NotFound,
    #[error("chat service is unavailable")]
    Unavailable,
    #[error("chat authentication is required")]
    Unauthorized,
    #[error("chat request conflicted with existing state")]
    Conflict,
    #[error("chat payload is too large")]
    PayloadTooLarge,
    #[error("chat validation failed: {0}")]
    Validation(&'static str),
}

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            #[must_use]
            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                self.0.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Ok(Self(Uuid::deserialize(deserializer)?))
            }
        }
    };
}

uuid_id!(MessageId);
uuid_id!(AttachmentId);
uuid_id!(ArtifactId);

/// A provider's own tool-call id.
///
/// A string, not a UUID: this is whatever the model's provider emitted
/// (`call_abc123` on OpenAI, `toolu_...` on Anthropic) and echoed back on the
/// answer. It is the only value that pairs a request to its result, so it is
/// carried verbatim — a UUID here would mean inventing an id and losing the
/// pairing the provider validates.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct ToolCallId(String);

impl ToolCallId {
    pub fn try_new(id: impl Into<String>) -> Result<Self, ChatError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ChatError::Validation("tool call id is required"));
        }
        if id.len() > MAX_TOOL_CALL_ID_BYTES {
            return Err(ChatError::Validation("tool call id is too long"));
        }
        Ok(Self(id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ToolCallId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text {
        text: String,
    },
    AttachmentRef {
        attachment_id: AttachmentId,
        file_name: String,
        mime_type: String,
    },
    ArtifactRef {
        artifact_id: ArtifactId,
        title: String,
        kind: String,
    },
    ToolResultRef {
        tool_call_id: ToolCallId,
    },
}

impl MessageContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn try_text(text: impl Into<String>) -> Result<Self, ChatError> {
        let text = text.into();
        ensure_text_size(&text)?;
        Ok(Self::Text { text })
    }
}

impl<'de> Deserialize<'de> for MessageContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum Raw {
            Text {
                text: String,
            },
            AttachmentRef {
                attachment_id: AttachmentId,
                file_name: String,
                mime_type: String,
            },
            ArtifactRef {
                artifact_id: ArtifactId,
                title: String,
                kind: String,
            },
            ToolResultRef {
                tool_call_id: ToolCallId,
            },
        }

        match Raw::deserialize(deserializer)? {
            Raw::Text { text } => {
                ensure_text_size(&text).map_err(serde::de::Error::custom)?;
                Ok(Self::Text { text })
            }
            Raw::AttachmentRef {
                attachment_id,
                file_name,
                mime_type,
            } => Ok(Self::AttachmentRef {
                attachment_id,
                file_name,
                mime_type,
            }),
            Raw::ArtifactRef {
                artifact_id,
                title,
                kind,
            } => Ok(Self::ArtifactRef {
                artifact_id,
                title,
                kind,
            }),
            Raw::ToolResultRef { tool_call_id } => Ok(Self::ToolResultRef { tool_call_id }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolCallRef {
    pub id: ToolCallId,
    pub name: String,
    /// Whether an answer for this call exists in the conversation.
    ///
    /// Carried rather than derived at render time because the evidence is the
    /// whole message set: the answer is a separate row, and by the time one
    /// message is being rendered the others are no longer in reach.
    pub status: ToolCallStatus,
    /// The JSON argument text the turn advertised, capped at
    /// [`xfalcon_contracts::chat::TOOL_PAYLOAD_PROJECTION_CAP_BYTES`] by
    /// [`ToolCallRef::with_payloads`] — the one place projection payloads are
    /// bounded. `None` when the durable row carried none.
    pub arguments: Option<String>,
    pub arguments_truncated: bool,
    /// The answer row's content, capped the same way. `None` while unanswered.
    pub result: Option<String>,
    pub result_truncated: bool,
    /// What the call did, read from the UNCAPPED outcome by
    /// [`crate::tools::dispatch::tool_step_summary`] before `result` was capped.
    /// The contract type itself rather than a mirror of it: this value is
    /// projected verbatim, and a second declaration of the same four fields
    /// would only give the two a way to disagree.
    pub step: Option<ToolStepSummaryV1>,
}

/// Whether a requested call has an answer in the durable conversation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Answered,
    Unanswered,
}

impl ToolCallRef {
    pub fn try_new(
        id: ToolCallId,
        name: impl Into<String>,
        status: ToolCallStatus,
    ) -> Result<Self, ChatError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ChatError::Validation("tool call name is required"));
        }
        if name.len() > MAX_TOOL_NAME_BYTES {
            return Err(ChatError::Validation("tool call name is too long"));
        }
        Ok(Self {
            id,
            name,
            status,
            arguments: None,
            arguments_truncated: false,
            result: None,
            result_truncated: false,
            step: None,
        })
    }

    /// Attaches the call's projected payloads, applying the projection cap.
    /// Passing already-capped text through again is a no-op, so re-sealing a
    /// snapshot never double-truncates.
    #[must_use]
    pub fn with_payloads(mut self, arguments: Option<String>, result: Option<String>) -> Self {
        let (arguments, arguments_truncated) = cap_payload(arguments);
        let (result, result_truncated) = cap_payload(result);
        self.arguments = arguments;
        self.arguments_truncated = self.arguments_truncated || arguments_truncated;
        self.result = result;
        self.result_truncated = self.result_truncated || result_truncated;
        self
    }

    /// Attaches the step summary read from the call's UNCAPPED outcome.
    ///
    /// Separate from [`Self::with_payloads`] so the call order at the projection
    /// site reads as the rule it encodes: summarize the whole outcome, then cap
    /// the copy kept for the Result panel. Deriving a summary from capped text
    /// would delete it exactly on the calls that most need one — a cut document
    /// no longer parses — and would do it silently.
    #[must_use]
    pub fn with_step(mut self, step: Option<ToolStepSummaryV1>) -> Self {
        self.step = step;
        self
    }

    #[must_use]
    pub fn with_truncation_flags(
        mut self,
        arguments_truncated: bool,
        result_truncated: bool,
    ) -> Self {
        self.arguments_truncated = self.arguments_truncated || arguments_truncated;
        self.result_truncated = self.result_truncated || result_truncated;
        self
    }
}

/// Bounds one projected payload at the contract cap, cutting on a char
/// boundary. Returns the (possibly shortened) text and whether it was cut.
fn cap_payload(payload: Option<String>) -> (Option<String>, bool) {
    let Some(text) = payload else {
        return (None, false);
    };
    let cap = xfalcon_contracts::chat::TOOL_PAYLOAD_PROJECTION_CAP_BYTES;
    if text.len() <= cap {
        return (Some(text), false);
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (Some(text[..cut].to_owned()), true)
}

impl<'de> Deserialize<'de> for ToolCallRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            id: ToolCallId,
            name: String,
            status: ToolCallStatus,
            #[serde(default)]
            arguments: Option<String>,
            #[serde(default)]
            arguments_truncated: bool,
            #[serde(default)]
            result: Option<String>,
            #[serde(default)]
            result_truncated: bool,
            /// Defaulted like its siblings: a snapshot sealed by a build with no
            /// step field must still open, or the first launch after this change
            /// destroys every offline conversation.
            #[serde(default)]
            step: Option<ToolStepSummaryV1>,
        }

        let raw = Raw::deserialize(deserializer)?;
        // Re-cap on the way in: sealed snapshots are trusted-ish, but the cap
        // is an invariant of the type, not of one write path. Re-capping cannot
        // reach the summary, which is why a re-sealed call keeps it whole.
        Ok(Self::try_new(raw.id, raw.name, raw.status)
            .map_err(serde::de::Error::custom)?
            .with_step(raw.step)
            .with_payloads(raw.arguments, raw.result)
            .with_truncation_flags(raw.arguments_truncated, raw.result_truncated))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    id: MessageId,
    conversation_id: ConversationId,
    sequence: u64,
    role: MessageRole,
    content: Vec<MessageContent>,
    tool_calls: Vec<ToolCallRef>,
    completed: bool,
}

impl Message {
    pub fn try_new(
        id: MessageId,
        conversation_id: ConversationId,
        sequence: u64,
        role: MessageRole,
        content: Vec<MessageContent>,
        tool_calls: Vec<ToolCallRef>,
    ) -> Result<Self, ChatError> {
        if sequence == 0 {
            return Err(ChatError::Validation(
                "message sequence must be greater than zero",
            ));
        }
        validate_role_content(role, &content, &tool_calls)?;
        for part in &content {
            if let MessageContent::Text { text } = part {
                ensure_text_size(text)?;
            }
        }
        Ok(Self {
            id,
            conversation_id,
            sequence,
            role,
            content,
            tool_calls,
            completed: false,
        })
    }

    /// The message with its attached files listed after its text. Only
    /// attachment references are taken; this never rewrites what was said.
    #[must_use]
    pub fn with_attachment_refs(mut self, files: Vec<MessageContent>) -> Self {
        self.content.extend(
            files
                .into_iter()
                .filter(|part| matches!(part, MessageContent::AttachmentRef { .. })),
        );
        self
    }

    #[must_use]
    pub fn id(&self) -> MessageId {
        self.id
    }

    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn role(&self) -> MessageRole {
        self.role
    }

    #[must_use]
    pub fn content(&self) -> &[MessageContent] {
        &self.content
    }

    #[must_use]
    pub fn tool_calls(&self) -> &[ToolCallRef] {
        &self.tool_calls
    }

    #[must_use]
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub fn replace_content(&mut self, content: Vec<MessageContent>) -> Result<(), ChatError> {
        if self.completed {
            return Err(ChatError::MessageImmutable);
        }
        validate_role_content(self.role, &content, &self.tool_calls)?;
        for part in &content {
            if let MessageContent::Text { text } = part {
                ensure_text_size(text)?;
            }
        }
        self.content = content;
        Ok(())
    }

    pub fn complete(&mut self) {
        self.completed = true;
    }

    #[must_use]
    pub fn into_completed(mut self) -> Self {
        self.completed = true;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MessageSequence {
    messages: Vec<Message>,
    head_sequence: u64,
}

impl MessageSequence {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, message: Message) -> Result<(), ChatError> {
        if message.sequence() <= self.head_sequence {
            return Err(ChatError::SequenceNotMonotonic);
        }
        self.head_sequence = message.sequence();
        self.messages.push(message);
        Ok(())
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
    pub fn head_sequence(&self) -> u64 {
        self.head_sequence
    }

    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantDraft {
    conversation_id: ConversationId,
    sequence: u64,
    text: String,
    committed: bool,
}

impl AssistantDraft {
    pub fn new(conversation_id: ConversationId, sequence: u64) -> Result<Self, ChatError> {
        if sequence == 0 {
            return Err(ChatError::Validation(
                "draft sequence must be greater than zero",
            ));
        }
        Ok(Self {
            conversation_id,
            sequence,
            text: String::new(),
            committed: false,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    pub fn try_restore(
        conversation_id: ConversationId,
        sequence: u64,
        text: impl Into<String>,
    ) -> Result<Self, ChatError> {
        if sequence == 0 {
            return Err(ChatError::Validation(
                "draft sequence must be greater than zero",
            ));
        }
        let text = text.into();
        ensure_text_size(&text)?;
        Ok(Self {
            conversation_id,
            sequence,
            text,
            committed: false,
        })
    }

    pub fn append_text(&mut self, delta: &str) -> Result<(), ChatError> {
        if self.committed {
            return Err(ChatError::MessageImmutable);
        }
        let mut next = String::with_capacity(self.text.len() + delta.len());
        next.push_str(&self.text);
        next.push_str(delta);
        ensure_text_size(&next)?;
        self.text = next;
        Ok(())
    }

    pub fn commit(mut self) -> Result<Message, ChatError> {
        if self.committed {
            return Err(ChatError::MessageImmutable);
        }
        self.committed = true;
        let message = Message::try_new(
            MessageId::new(),
            self.conversation_id,
            self.sequence,
            MessageRole::Assistant,
            vec![MessageContent::try_text(self.text)?],
            vec![],
        )?;
        Ok(message.into_completed())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conversation {
    id: ConversationId,
    title: String,
}

impl Conversation {
    #[must_use]
    pub fn new(id: ConversationId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
        }
    }

    #[must_use]
    pub fn id(&self) -> &ConversationId {
        &self.id
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationSummary {
    id: ConversationId,
    title: String,
    /// RFC 3339 pin instant from the durable row, `None` when unpinned. The
    /// sidebar groups and orders pinned conversations by it, exactly as web.
    pinned_at: Option<String>,
    /// RFC 3339 last-update instant from the durable row; empty when the
    /// source (a local snapshot) does not carry one.
    updated_at: String,
}

impl ConversationSummary {
    #[must_use]
    pub fn from_conversation(conversation: &Conversation) -> Self {
        Self {
            id: conversation.id.clone(),
            title: conversation.title.clone(),
            pinned_at: None,
            updated_at: String::new(),
        }
    }

    #[must_use]
    pub fn with_row_metadata(mut self, pinned_at: Option<String>, updated_at: String) -> Self {
        self.pinned_at = pinned_at;
        self.updated_at = updated_at;
        self
    }

    #[must_use]
    pub fn pinned_at(&self) -> Option<&str> {
        self.pinned_at.as_deref()
    }

    #[must_use]
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }

    #[must_use]
    pub fn id(&self) -> &ConversationId {
        &self.id
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    run_id: RunId,
    conversation_id: ConversationId,
    state: RunState,
    sequence: u64,
}

impl RunRecord {
    pub fn try_new(
        run_id: RunId,
        conversation_id: ConversationId,
        state: RunState,
        sequence: u64,
    ) -> Result<Self, ChatError> {
        if sequence == 0 {
            return Err(ChatError::Validation(
                "run sequence must be greater than zero",
            ));
        }
        Ok(Self {
            run_id,
            conversation_id,
            state,
            sequence,
        })
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn state(&self) -> &RunState {
        &self.state
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Monotonic ordering cursor for conversation messages and persisted run events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncCursor {
    head_sequence: u64,
}

impl SyncCursor {
    #[must_use]
    pub const fn new(head_sequence: u64) -> Self {
        Self { head_sequence }
    }

    #[must_use]
    pub const fn head_sequence(&self) -> u64 {
        self.head_sequence
    }

    pub fn advance_to(&mut self, sequence: u64) -> Result<(), ChatError> {
        if sequence <= self.head_sequence {
            return Err(ChatError::SequenceNotMonotonic);
        }
        self.head_sequence = sequence;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatBootstrap {
    conversation: Conversation,
    messages: Vec<Message>,
    runs: Vec<RunRecord>,
    cursor: SyncCursor,
}

impl ChatBootstrap {
    #[must_use]
    pub fn new(
        conversation: Conversation,
        messages: Vec<Message>,
        runs: Vec<RunRecord>,
        cursor: SyncCursor,
    ) -> Self {
        Self {
            conversation,
            messages,
            runs,
            cursor,
        }
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    #[must_use]
    pub fn runs(&self) -> &[RunRecord] {
        &self.runs
    }

    #[must_use]
    pub fn cursor(&self) -> &SyncCursor {
        &self.cursor
    }

    #[must_use]
    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChatBootstrap {
    conversations: Vec<ConversationSummary>,
    cursor: SyncCursor,
}

impl WorkspaceChatBootstrap {
    #[must_use]
    pub fn new(conversations: Vec<ConversationSummary>, cursor: SyncCursor) -> Self {
        Self {
            conversations,
            cursor,
        }
    }

    #[must_use]
    pub fn conversations(&self) -> &[ConversationSummary] {
        &self.conversations
    }

    #[must_use]
    pub fn cursor(&self) -> &SyncCursor {
        &self.cursor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSnapshot {
    conversation: Conversation,
    messages: Vec<Message>,
    draft: Option<AssistantDraft>,
    runs: Vec<RunRecord>,
    cursor: SyncCursor,
}

impl ChatSnapshot {
    #[must_use]
    pub fn new(
        conversation: Conversation,
        messages: Vec<Message>,
        draft: Option<AssistantDraft>,
        runs: Vec<RunRecord>,
        cursor: SyncCursor,
    ) -> Self {
        Self {
            conversation,
            messages,
            draft,
            runs,
            cursor,
        }
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    #[must_use]
    pub fn draft(&self) -> Option<&AssistantDraft> {
        self.draft.as_ref()
    }

    #[must_use]
    pub fn runs(&self) -> &[RunRecord] {
        &self.runs
    }

    #[must_use]
    pub fn cursor(&self) -> &SyncCursor {
        &self.cursor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewConversation {
    pub title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewMessage {
    conversation_id: ConversationId,
    role: MessageRole,
    content: Vec<MessageContent>,
    tool_calls: Vec<ToolCallRef>,
}

impl NewMessage {
    pub fn try_new(
        conversation_id: ConversationId,
        role: MessageRole,
        content: Vec<MessageContent>,
        tool_calls: Vec<ToolCallRef>,
    ) -> Result<Self, ChatError> {
        validate_role_content(role, &content, &tool_calls)?;
        for part in &content {
            if let MessageContent::Text { text } = part {
                ensure_text_size(text)?;
            }
        }
        Ok(Self {
            conversation_id,
            role,
            content,
            tool_calls,
        })
    }

    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn role(&self) -> MessageRole {
        self.role
    }

    #[must_use]
    pub fn content(&self) -> &[MessageContent] {
        &self.content
    }

    #[must_use]
    pub fn tool_calls(&self) -> &[ToolCallRef] {
        &self.tool_calls
    }
}

/// Authoritative cloud/journal run event for persistence ports.
///
/// Sequence monotonicity for these events is enforced by callers via
/// [`SyncCursor`] (and a future dedicated run-sequence helper if needed), not
/// by a separate aggregate on this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedRunEvent {
    run_id: RunId,
    conversation_id: ConversationId,
    sequence: u64,
    state: RunState,
}

impl PersistedRunEvent {
    pub fn try_new(
        run_id: RunId,
        conversation_id: ConversationId,
        sequence: u64,
        state: RunState,
    ) -> Result<Self, ChatError> {
        if sequence == 0 {
            return Err(ChatError::Validation(
                "run event sequence must be greater than zero",
            ));
        }
        Ok(Self {
            run_id,
            conversation_id,
            sequence,
            state,
        })
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn state(&self) -> &RunState {
        &self.state
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatProjectionEvent {
    UpsertConversation(Conversation),
    AppendMessage(Message),
    UpsertDraft(AssistantDraft),
    ClearDraft { conversation_id: ConversationId },
    UpsertRun(RunRecord),
    ReplaceSnapshot(ChatSnapshot),
    AdvanceCursor(SyncCursor),
    RemoveConversation { conversation_id: ConversationId },
}

fn validate_role_content(
    role: MessageRole,
    content: &[MessageContent],
    tool_calls: &[ToolCallRef],
) -> Result<(), ChatError> {
    // A turn must say *something*. An assistant answering purely with tool calls
    // says it with the calls and no prose, which is a real turn and the one the
    // transcript most needs to show — so emptiness is judged across both.
    if content.is_empty() && tool_calls.is_empty() {
        return Err(ChatError::InvalidRoleContent);
    }

    match role {
        MessageRole::User => {
            if !tool_calls.is_empty() || content.is_empty() {
                return Err(ChatError::InvalidRoleContent);
            }
            if content.iter().any(|part| {
                matches!(
                    part,
                    MessageContent::ToolResultRef { .. } | MessageContent::ArtifactRef { .. }
                )
            }) {
                return Err(ChatError::InvalidRoleContent);
            }
        }
        MessageRole::Assistant | MessageRole::System => {
            if content
                .iter()
                .any(|part| matches!(part, MessageContent::ToolResultRef { .. }))
            {
                return Err(ChatError::InvalidRoleContent);
            }
            if role == MessageRole::System && (!tool_calls.is_empty() || content.is_empty()) {
                return Err(ChatError::InvalidRoleContent);
            }
        }
        MessageRole::Tool => {
            if !tool_calls.is_empty() {
                return Err(ChatError::InvalidRoleContent);
            }
            if content.len() != 1 || !matches!(content[0], MessageContent::ToolResultRef { .. }) {
                return Err(ChatError::InvalidRoleContent);
            }
        }
    }
    Ok(())
}

fn ensure_text_size(text: &str) -> Result<(), ChatError> {
    if text.len() > MAX_MESSAGE_TEXT_BYTES {
        return Err(ChatError::TextTooLarge);
    }
    Ok(())
}
