use std::collections::HashMap;

use async_trait::async_trait;
use uuid::Uuid;
use xfalcon_contracts::{ConversationId, RunId, RunState};

use serde::Deserialize;

use crate::clients::da::chat_api::{DaChatApiError, DaChatSession};
use crate::clients::da::generated::{ChatAttachment, ChatMessageToolCalls, ConversationBootstrap};
use crate::tools::dispatch::tool_step_summary;

use super::domain::{
    AttachmentId, ChatBootstrap, ChatError, Conversation, ConversationSummary, Message,
    MessageContent, MessageId, MessageRole, NewConversation, NewMessage, PersistedRunEvent,
    RunRecord, SyncCursor, ToolCallId, ToolCallRef, ToolCallStatus, WorkspaceChatBootstrap,
};
use super::repository::CloudChatRepository;

pub struct DaChatRepository {
    session: DaChatSession,
    expected_tenant_id: Option<String>,
}

impl DaChatRepository {
    // Wired into ChatService/Tauri in a later task; kept callable for tests and adapters.
    #[allow(dead_code)]
    pub(crate) fn new(session: DaChatSession) -> Self {
        Self {
            session,
            expected_tenant_id: None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_expected_tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.expected_tenant_id = Some(tenant_id.into());
        self
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn expected_tenant_id(&self) -> Option<&str> {
        self.expected_tenant_id.as_deref()
    }
}

/// Routes a `crate::approvals` mutation-plan approval decision through
/// whatever authenticated DA/MCP-audience channel is already pinned for this
/// account -- never a new provider or control-token path. A trait (rather
/// than `crate::approvals::service` depending on the concrete
/// [`DaChatRepository`]/`DaChatSession` types directly) so:
///
/// 1. `crate::approvals` never needs to know `DaChatSession`/`reqwest`
///    exist, mirroring how [`CloudChatRepository`] already decouples
///    `ChatService` from the concrete DA HTTP client; and
/// 2. `crate::approvals::tests` can exercise the "a DA sync channel is
///    present and is actually called" path with a lightweight in-memory
///    fake, without standing up a real DA HTTP fixture for a capability
///    Task 6 has no live caller for yet (see the impl's own doc comment).
#[async_trait]
pub trait ApprovalDaSync: Send + Sync {
    async fn record_mutation_approval_decision(
        &self,
        tool_call_id: Uuid,
        approved: bool,
    ) -> Result<(), ChatError>;
}

#[async_trait]
impl ApprovalDaSync for DaChatRepository {
    /// Routes one patch-approval decision through the pinned DA tool-call
    /// approval endpoint (`DaChatSession::update_tool_call_approval`), over
    /// the exact same authenticated DA/MCP-audience channel (`self.session`)
    /// every other method on this type already uses -- no new provider or
    /// control-token path is introduced for this.
    ///
    /// `tool_call_id` is DA's own identifier for the `project.propose_patch`
    /// tool call this plan was proposed under. Nothing in this task creates
    /// one: that requires a live agent run actually invoking the tool
    /// through DA-tracked tool-call rows, which is Task 7's agent-loop
    /// dispatch wiring (the same disclosed-seam pattern already used by
    /// `crate::projects::index::ProjectIndexer::start` and
    /// `crate::tools::registry::project_read_tool_descriptors`, both landed
    /// with "no caller yet" documented in their own doc comments).
    /// `crate::approvals::service::ApprovalsService::resolve` calls this
    /// only when it already holds a `tool_call_id` for the plan being
    /// resolved, and treats a plan with none as "local approval only" --
    /// never fabricating one.
    async fn record_mutation_approval_decision(
        &self,
        tool_call_id: Uuid,
        approved: bool,
    ) -> Result<(), ChatError> {
        let status = if approved { "approved" } else { "denied" };
        self.session
            .update_tool_call_approval(tool_call_id, status)
            .await
            .map_err(map_da_error)
    }
}

#[async_trait]
impl CloudChatRepository for DaChatRepository {
    async fn workspace_bootstrap(&self) -> Result<WorkspaceChatBootstrap, ChatError> {
        let bootstrap = self
            .session
            .workspace_bootstrap()
            .await
            .map_err(map_da_error)?;
        let conversations = bootstrap
            .conversations
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| parse_conversation_summary(&value))
            .collect::<Vec<_>>();
        let head = conversations.len() as u64;
        Ok(WorkspaceChatBootstrap::new(
            conversations,
            SyncCursor::new(head),
        ))
    }

    async fn conversation_bootstrap(&self, id: ConversationId) -> Result<ChatBootstrap, ChatError> {
        let conversation_uuid = parse_uuid(id.as_str())?;
        let bootstrap = self
            .session
            .conversation_bootstrap(conversation_uuid)
            .await
            .map_err(map_da_error)?;
        let mapped =
            map_conversation_bootstrap(bootstrap, &id, self.expected_tenant_id.as_deref())?;
        // The transcript names the files each turn was sent with. The
        // bootstrap carries message rows only, so the list is read beside it;
        // a failure costs the file labels, never the conversation.
        let attachments = match self
            .session
            .list_conversation_attachments(conversation_uuid)
            .await
        {
            Ok(attachments) => attachments,
            Err(error) => {
                tracing::warn!(%error, conversation = %id.as_str(), "bootstrap: attachment labels could not be read");
                Vec::new()
            }
        };
        Ok(with_attachments(mapped, &attachments))
    }

    async fn create_conversation(&self, input: NewConversation) -> Result<Conversation, ChatError> {
        let created = self
            .session
            .create_conversation(input.title.as_deref(), None, None, true)
            .await
            .map_err(map_da_error)?;
        self.ensure_tenant(Some(&created.tenant_id))?;
        let title = input.title.unwrap_or(created.title).trim().to_owned();
        Ok(Conversation::new(
            ConversationId::from(created.id.to_string()),
            if title.is_empty() {
                "New conversation".to_owned()
            } else {
                title
            },
        ))
    }

    async fn append_message(&self, input: NewMessage) -> Result<Message, ChatError> {
        let conversation_uuid = parse_uuid(input.conversation_id().as_str())?;
        let text = input
            .content()
            .iter()
            .find_map(|part| match part {
                MessageContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .ok_or(ChatError::Validation("text content is required"))?;
        let idempotency_key = Uuid::new_v4().to_string();
        let inserted = self
            .session
            .create_user_message_idempotent(
                conversation_uuid,
                text,
                Some(idempotency_key.as_str()),
                None,
            )
            .await
            .map_err(map_da_error)?;
        let sequence = u64::try_from(inserted.sequence_no)
            .map_err(|_| ChatError::Validation("invalid message sequence"))?;
        Message::try_new(
            MessageId::from_uuid(inserted.id),
            input.conversation_id().clone(),
            sequence,
            input.role(),
            input.content().to_vec(),
            input.tool_calls().to_vec(),
        )
        .map(Message::into_completed)
    }

    async fn record_run_event(&self, event: PersistedRunEvent) -> Result<(), ChatError> {
        let run_uuid = parse_uuid(event.run_id().as_str())?;
        let status = match event.state() {
            RunState::Draft => "draft",
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::AwaitingApproval => "awaiting_approval",
            RunState::Completed => "completed",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Interrupted => "interrupted",
            RunState::Recovering => "recovering",
        };
        // DA status patch does not accept client sequence numbers; sequence remains
        // a local/journal concern and is not invented on the wire.
        self.session
            .update_run_status(run_uuid, status)
            .await
            .map_err(map_da_error)
    }
}

impl DaChatRepository {
    fn ensure_tenant(&self, tenant_id: Option<&str>) -> Result<(), ChatError> {
        ensure_expected_tenant(self.expected_tenant_id.as_deref(), tenant_id)
    }
}

pub(crate) fn ensure_expected_tenant(
    expected_tenant: Option<&str>,
    tenant_id: Option<&str>,
) -> Result<(), ChatError> {
    let Some(expected) = expected_tenant else {
        return Ok(());
    };
    match tenant_id {
        Some(actual) if actual == expected => Ok(()),
        Some(_) | None => Err(ChatError::Validation("account mismatch")),
    }
}

fn map_da_error(error: DaChatApiError) -> ChatError {
    match error {
        DaChatApiError::Unauthorized => ChatError::Unauthorized,
        DaChatApiError::NotFound => ChatError::NotFound,
        DaChatApiError::NoModel => ChatError::Validation("no chat model is available"),
        DaChatApiError::CustomModelUnavailable => {
            ChatError::Validation("the selected private model is no longer available")
        }
        DaChatApiError::CustomModelCapacityUnsupported => {
            ChatError::Validation("the selected private model has insufficient input capacity")
        }
        DaChatApiError::InvalidResponse => ChatError::Unavailable,
        DaChatApiError::RequestFailed
        | DaChatApiError::Conflict
        | DaChatApiError::ContextConflict
        | DaChatApiError::ContextSnapshotStale
        | DaChatApiError::IdempotencyConflict
        | DaChatApiError::PayloadTooLarge
        | DaChatApiError::StorageQuotaExceeded
        | DaChatApiError::StorageUnavailable
        | DaChatApiError::PayloadNotReady
        | DaChatApiError::PayloadIntegrityFailed
        | DaChatApiError::Validation
        | DaChatApiError::RateLimited
        | DaChatApiError::Timeout
        | DaChatApiError::UnsupportedCompactionFormat => ChatError::Unavailable,
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, ChatError> {
    Uuid::parse_str(value).map_err(|_| ChatError::Validation("invalid conversation id"))
}

pub(crate) fn map_conversation_bootstrap(
    bootstrap: ConversationBootstrap,
    fallback_id: &ConversationId,
    expected_tenant: Option<&str>,
) -> Result<ChatBootstrap, ChatError> {
    let conversation_value =
        serde_json::to_value(&bootstrap.conversation).unwrap_or(serde_json::Value::Null);
    let tenant_id = conversation_value
        .get("tenant_id")
        .and_then(|value| value.as_str());
    ensure_expected_tenant(expected_tenant, tenant_id)?;

    if let Some(payload_id) = conversation_value.get("id").and_then(|value| {
        value
            .as_str()
            .map(|s| ConversationId::from(s.to_owned()))
            .or_else(|| {
                serde_json::from_value::<Uuid>(value.clone())
                    .ok()
                    .map(|id| ConversationId::from(id.to_string()))
            })
    }) {
        if &payload_id != fallback_id {
            return Err(ChatError::Validation("conversation id mismatch"));
        }
    }

    let title = conversation_value
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("Conversation")
        .to_owned();
    // Trust the requested bootstrap path id; keep title from the payload.
    let conversation = Conversation::new(fallback_id.clone(), title);

    let rows = bootstrap
        .chats
        .unwrap_or_default()
        .into_iter()
        .filter_map(
            |value| match serde_json::from_value::<BootstrapChatRow>(value) {
                Ok(row) => Some(row),
                // One hostile or unrecognized row must not make a conversation
                // unreadable — but a dropped row must never be silent either:
                // an all-rows drop renders a reopened conversation empty, which
                // reads as data loss (the exact bug this WARN was added for).
                Err(error) => {
                    tracing::warn!(%error, "bootstrap: dropped unparseable chat row");
                    None
                }
            },
        )
        .collect::<Vec<_>>();
    // Which calls have an answer — and what it said — decided once over the
    // whole set: the answer is a separate row, and a single message being
    // mapped cannot see it. First answer wins (a second row for one call id is
    // a duplicate write; `chat::history` applies the same rule).
    let mut answered: HashMap<&str, &BootstrapChatRow> = HashMap::new();
    for row in rows
        .iter()
        .filter(|row| row.role.eq_ignore_ascii_case("tool"))
    {
        if let Some(call_id) = row.tool_call_id.as_deref() {
            answered.entry(call_id).or_insert(row);
        }
    }
    let mut messages = rows
        .iter()
        .filter_map(|row| map_chat_message(row, &answered).ok())
        .collect::<Vec<_>>();
    messages.sort_by_key(Message::sequence);

    let runs = bootstrap
        .latest_run
        .and_then(|run| serde_json::to_value(run).ok())
        .and_then(|value| parse_run_record(&value, conversation.id()).ok())
        .into_iter()
        .collect::<Vec<_>>();

    let head = messages
        .last()
        .map(Message::sequence)
        .or_else(|| runs.last().map(RunRecord::sequence))
        .unwrap_or(0);

    Ok(ChatBootstrap::new(
        conversation,
        messages,
        runs,
        SyncCursor::new(head),
    ))
}

/// Lists each user turn's files after its text, so a reopened or reconciled
/// conversation shows what was attached, not just what was typed.
pub(crate) fn with_attachments(
    bootstrap: ChatBootstrap,
    attachments: &[ChatAttachment],
) -> ChatBootstrap {
    let mut by_message: HashMap<Uuid, Vec<MessageContent>> = HashMap::new();
    for attachment in attachments {
        if let Some(message_id) = attachment.message_id {
            by_message
                .entry(message_id)
                .or_default()
                .push(MessageContent::AttachmentRef {
                    attachment_id: AttachmentId::from_uuid(attachment.id),
                    file_name: attachment.file_name.clone(),
                    mime_type: attachment.mime_type.clone(),
                });
        }
    }
    if by_message.is_empty() {
        return bootstrap;
    }
    let messages = bootstrap
        .messages()
        .iter()
        .cloned()
        .map(|message| match by_message.remove(message.id().as_uuid()) {
            Some(files) if message.role() == MessageRole::User => {
                message.with_attachment_refs(files)
            }
            _ => message,
        })
        .collect();
    bootstrap.with_messages(messages)
}

fn parse_conversation_summary(value: &serde_json::Value) -> Option<ConversationSummary> {
    let id = value.get("id").and_then(|v| {
        v.as_str()
            .map(|id| ConversationId::from(id.to_owned()))
            .or_else(|| {
                serde_json::from_value::<Uuid>(v.clone())
                    .ok()
                    .map(|id| ConversationId::from(id.to_string()))
            })
    })?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Conversation")
        .to_owned();
    let pinned_at = value
        .get("pinned_at")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let updated_at = value
        .get("updated_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    Some(
        ConversationSummary::from_conversation(&Conversation::new(id, title))
            .with_row_metadata(pinned_at, updated_at),
    )
}

/// One row of `get_chat_bootstrap`'s `chats` array.
///
/// The RPC projects a SUBSET of `xf_chat.chats` — notably WITHOUT `tenant_id`
/// (the bootstrap is already tenant-scoped by the RPC's identity guard). The
/// generated full-row `ChatMessage` entity therefore cannot deserialize these
/// rows: requiring `tenant_id` made every row fail parsing, the drops were
/// silent, and every reopened conversation rendered empty. This type mirrors
/// the RPC's actual projection and requires only what the transcript mapping
/// reads.
#[derive(Deserialize)]
pub(crate) struct BootstrapChatRow {
    pub(crate) id: Uuid,
    pub(crate) conversation_id: Uuid,
    pub(crate) sequence_no: i64,
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) tool_calls: Option<ChatMessageToolCalls>,
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
}

/// Projects one durable row into the transcript.
///
/// A tool round is durable in the conversation as an assistant turn carrying
/// `tool_calls` plus one `role="tool"` message per answer — the pairing a
/// provider validates, written by this app and by xfalcon-web alike. The two
/// halves are projected differently:
///
/// - the **assistant** turn keeps its calls — arguments included, capped by
///   [`ToolCallRef::with_payloads`] — and is kept even with no prose. A model
///   answering purely with tool calls emits no text, and that turn is the one
///   a reader most needs: without it, reopening a conversation shows an
///   answer with no trace of what produced it.
/// - the **answer** row contributes its existence (the requesting call's
///   [`ToolCallStatus`]) and its content, projected onto the call as a capped,
///   display-only `result` (stance narrowed 2026-08-05 — see
///   `xfalcon_contracts::chat::ChatToolCallView`). It is still never a
///   message of its own: a JSON blob is a detail panel, not a turn.
///
/// An unmappable row is dropped by the caller, never failed: one hostile or
/// unrecognized row must not make a conversation unreadable.
fn map_chat_message(
    message: &BootstrapChatRow,
    answered: &HashMap<&str, &BootstrapChatRow>,
) -> Result<Message, ChatError> {
    let sequence = u64::try_from(message.sequence_no)
        .map_err(|_| ChatError::Validation("invalid message sequence"))?;
    let role = match message.role.to_ascii_lowercase().as_str() {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "system" => MessageRole::System,
        "tool" => return Err(ChatError::Validation("tool answers are not projected")),
        _ => return Err(ChatError::Validation("unsupported message role")),
    };
    let tool_calls = if role == MessageRole::Assistant {
        requested_calls(message, answered)
    } else {
        Vec::new()
    };
    let content = if message.content.trim().is_empty() {
        Vec::new()
    } else {
        vec![MessageContent::try_text(message.content.clone())?]
    };
    Message::try_new(
        MessageId::from_uuid(message.id),
        ConversationId::from(message.conversation_id.to_string()),
        sequence,
        role,
        content,
        tool_calls,
    )
    .map(Message::into_completed)
}

/// The calls one assistant row advertises, in the order it advertised them.
///
/// Only the array form is understood; DA's column also admits an object, which
/// carries no order over its entries. A malformed entry is dropped rather than
/// failing the row — the turn's prose is still worth showing.
fn requested_calls(
    message: &BootstrapChatRow,
    answered: &HashMap<&str, &BootstrapChatRow>,
) -> Vec<ToolCallRef> {
    let Some(ChatMessageToolCalls::Array(calls)) = message.tool_calls.as_ref() else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|call| {
            let call_id = call.get("id")?.as_str()?;
            let function = call.get("function")?;
            let name = function.get("name")?.as_str()?;
            let answer = answered.get(call_id);
            let status = if answer.is_some() {
                ToolCallStatus::Answered
            } else {
                ToolCallStatus::Unanswered
            };
            let arguments = arguments_text(function.get("arguments"));
            let result = answer.map(|row| row.content.clone());
            // Summarize the WHOLE outcome, then cap the copy kept for the
            // Result panel — the order is the requirement, not a preference.
            // Capped text is cut mid-document and no longer parses, so a
            // summary built after the cap would vanish precisely on the calls
            // with the largest outcomes: the ones a reader most needs
            // summarized. An unanswered call gets none, because a label with no
            // outcome behind it is a claim about a call that never returned.
            let step = result
                .as_deref()
                .and_then(|answer_text| tool_step_summary(name, answer_text, arguments.as_deref()));
            Some(
                ToolCallRef::try_new(ToolCallId::try_new(call_id).ok()?, name, status)
                    .ok()?
                    .with_step(step)
                    .with_payloads(arguments, result),
            )
        })
        .collect()
}

/// Provider arguments are JSON *text*; a row holding an object instead is
/// re-serialized rather than dropped (same rule as `chat::history`). A row
/// with no arguments projects `None` — nothing is fabricated.
fn arguments_text(arguments: Option<&serde_json::Value>) -> Option<String> {
    match arguments {
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(value) => serde_json::to_string(value).ok(),
        None => None,
    }
}

fn parse_run_record(
    value: &serde_json::Value,
    conversation_id: &ConversationId,
) -> Result<RunRecord, ChatError> {
    let run_id = value
        .get("id")
        .and_then(|v| serde_json::from_value::<Uuid>(v.clone()).ok())
        .map(|id| RunId::from(id.to_string()))
        .ok_or(ChatError::Validation("run id is required"))?;
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("running");
    let state = match status {
        "draft" => RunState::Draft,
        "queued" => RunState::Queued,
        "running" | "streaming" => RunState::Running,
        "awaiting_approval" => RunState::AwaitingApproval,
        "completed" => RunState::Completed,
        "failed" | "error" => RunState::Failed,
        "cancelled" => RunState::Cancelled,
        "interrupted" => RunState::Interrupted,
        "recovering" => RunState::Recovering,
        _ => RunState::Running,
    };
    let sequence = value
        .get("sequence_no")
        .and_then(|v| v.as_u64())
        .or_else(|| value.get("sequence").and_then(|v| v.as_u64()))
        .unwrap_or(1);
    RunRecord::try_new(run_id, conversation_id.clone(), state, sequence.max(1))
}
