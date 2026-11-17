use serde_json::json;
use uuid::Uuid;
use xfalcon_contracts::chat::{ToolStepPathV1, ToolStepSummaryV1};
use xfalcon_contracts::{ConversationId, RunId, RunState};

use crate::chat::domain::{
    ArtifactId, AssistantDraft, AttachmentId, ChatBootstrap, ChatError, ChatProjectionEvent,
    ChatSnapshot, Conversation, ConversationSummary, Message, MessageContent, MessageId,
    MessageRole, MessageSequence, NewConversation, NewMessage, PersistedRunEvent, RunRecord,
    SyncCursor, ToolCallId, ToolCallRef, ToolCallStatus, WorkspaceChatBootstrap,
    MAX_MESSAGE_TEXT_BYTES, MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_NAME_BYTES,
};

#[test]
fn message_and_attachment_ids_are_stable_uuids_across_round_trips() {
    let message_id = MessageId::new();
    let attachment_id = AttachmentId::from_uuid(Uuid::nil());
    let artifact_id = ArtifactId::new();
    let tool_call_id = ToolCallId::try_new("call_abc123").expect("a provider call id");

    assert_ne!(message_id.as_uuid(), &Uuid::nil());
    assert_eq!(
        MessageId::from_uuid(*message_id.as_uuid()),
        message_id,
        "MessageId must round-trip through its UUID"
    );
    assert_eq!(attachment_id.as_uuid(), &Uuid::nil());

    let wire = serde_json::to_value(message_id).expect("serialize MessageId");
    let restored: MessageId = serde_json::from_value(wire).expect("deserialize MessageId");
    assert_eq!(restored, message_id);

    // A provider call id is an opaque string and crosses the wire as one: a
    // UUID here would mean inventing an id and losing the pairing the provider
    // validates against its own `tool_calls`.
    let tool_wire = serde_json::to_value(tool_call_id.clone()).expect("serialize ToolCallId");
    assert_eq!(tool_wire, json!("call_abc123"));
    let tool_restored: ToolCallId =
        serde_json::from_value(tool_wire).expect("deserialize ToolCallId");
    assert_eq!(tool_restored, tool_call_id);
    assert_eq!(ArtifactId::from_uuid(*artifact_id.as_uuid()), artifact_id);
}

#[test]
fn role_content_and_tool_call_combinations_are_validated() {
    let conversation = ConversationId::from("conv-1");

    let user = Message::try_new(
        MessageId::new(),
        conversation.clone(),
        1,
        MessageRole::User,
        vec![MessageContent::text("hello")],
        vec![],
    )
    .expect("user text message");
    assert_eq!(user.role(), MessageRole::User);

    let assistant = Message::try_new(
        MessageId::new(),
        conversation.clone(),
        2,
        MessageRole::Assistant,
        vec![MessageContent::text("answer")],
        vec![ToolCallRef::try_new(
            ToolCallId::try_new("call_a").expect("a provider call id"),
            "search",
            ToolCallStatus::Answered,
        )
        .expect("a named call")],
    )
    .expect("assistant message with tool calls");
    assert!(!assistant.tool_calls().is_empty());

    let tool = Message::try_new(
        MessageId::new(),
        conversation.clone(),
        3,
        MessageRole::Tool,
        vec![MessageContent::ToolResultRef {
            tool_call_id: ToolCallId::try_new("call_a").expect("a provider call id"),
        }],
        vec![],
    )
    .expect("tool result message");
    assert_eq!(tool.role(), MessageRole::Tool);

    assert_eq!(
        Message::try_new(
            MessageId::new(),
            conversation.clone(),
            4,
            MessageRole::User,
            vec![MessageContent::ToolResultRef {
                tool_call_id: ToolCallId::try_new("call_a").expect("a provider call id"),
            }],
            vec![],
        ),
        Err(ChatError::InvalidRoleContent)
    );
    assert_eq!(
        Message::try_new(
            MessageId::new(),
            conversation.clone(),
            5,
            MessageRole::User,
            vec![MessageContent::text("hi")],
            vec![ToolCallRef::try_new(
                ToolCallId::try_new("call_a").expect("a provider call id"),
                "search",
                ToolCallStatus::Answered,
            )
            .expect("a named call")],
        ),
        Err(ChatError::InvalidRoleContent)
    );
    assert_eq!(
        Message::try_new(
            MessageId::new(),
            conversation,
            6,
            MessageRole::Tool,
            vec![MessageContent::text("not a tool result")],
            vec![],
        ),
        Err(ChatError::InvalidRoleContent)
    );
}

#[test]
fn message_and_run_event_sequences_are_monotonically_ordered() {
    let conversation = ConversationId::from("conv-seq");
    let mut messages = MessageSequence::new();

    let first = Message::try_new(
        MessageId::new(),
        conversation.clone(),
        1,
        MessageRole::User,
        vec![MessageContent::text("one")],
        vec![],
    )
    .expect("first");
    let second = Message::try_new(
        MessageId::new(),
        conversation.clone(),
        2,
        MessageRole::Assistant,
        vec![MessageContent::text("two")],
        vec![],
    )
    .expect("second");

    messages.append(first).expect("append first");
    messages.append(second).expect("append second");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages.head_sequence(), 2);

    let regressing = Message::try_new(
        MessageId::new(),
        conversation.clone(),
        2,
        MessageRole::User,
        vec![MessageContent::text("dup")],
        vec![],
    )
    .expect("regressing message");
    assert_eq!(
        messages.append(regressing),
        Err(ChatError::SequenceNotMonotonic)
    );

    let mut cursor = SyncCursor::new(0);
    cursor.advance_to(3).expect("advance event sequence");
    assert_eq!(cursor.head_sequence(), 3);
    assert_eq!(cursor.advance_to(3), Err(ChatError::SequenceNotMonotonic));
    assert_eq!(cursor.advance_to(2), Err(ChatError::SequenceNotMonotonic));

    let event =
        PersistedRunEvent::try_new(RunId::from("run-1"), conversation, 4, RunState::Running)
            .expect("persisted run event");
    assert_eq!(event.sequence(), 4);
}

#[test]
fn run_record_rejects_zero_sequence() {
    assert_eq!(
        RunRecord::try_new(
            RunId::from("run-0"),
            ConversationId::from("conv-run"),
            RunState::Draft,
            0,
        ),
        Err(ChatError::Validation(
            "run sequence must be greater than zero"
        ))
    );

    let run = RunRecord::try_new(
        RunId::from("run-1"),
        ConversationId::from("conv-run"),
        RunState::Queued,
        1,
    )
    .expect("valid run record");
    assert_eq!(run.sequence(), 1);
}

#[test]
fn completed_message_content_is_immutable() {
    let mut message = Message::try_new(
        MessageId::new(),
        ConversationId::from("conv-imm"),
        1,
        MessageRole::Assistant,
        vec![MessageContent::text("draftable")],
        vec![],
    )
    .expect("open message");
    assert!(!message.is_completed());

    message
        .replace_content(vec![MessageContent::text("revised")])
        .expect("open message can revise");
    message.complete();
    assert!(message.is_completed());
    assert_eq!(message.content(), &[MessageContent::text("revised")]);
    assert_eq!(
        message.replace_content(vec![MessageContent::text("tamper")]),
        Err(ChatError::MessageImmutable)
    );
    assert_eq!(
        message.content(),
        &[MessageContent::text("revised")],
        "completed content must stay unchanged after failed mutation"
    );
}

#[test]
fn assistant_draft_commits_to_a_completed_message_without_mutating_prior_commits() {
    let conversation = ConversationId::from("conv-draft");
    assert_eq!(
        AssistantDraft::new(conversation.clone(), 0).map(|_| ()),
        Err(ChatError::Validation(
            "draft sequence must be greater than zero"
        ))
    );

    let mut draft = AssistantDraft::new(conversation.clone(), 7).expect("draft");
    draft.append_text("Hello").expect("append draft text");
    draft
        .append_text(", world")
        .expect("append more draft text");
    assert_eq!(draft.text(), "Hello, world");
    assert!(!draft.is_committed());

    let committed = draft.commit().expect("commit draft");
    assert!(committed.is_completed());
    assert_eq!(committed.sequence(), 7);
    assert_eq!(committed.role(), MessageRole::Assistant);
    assert_eq!(committed.content(), &[MessageContent::text("Hello, world")]);

    let mut second = AssistantDraft::new(conversation, 8).expect("second draft");
    second.append_text("next").expect("second draft text");
    let second_committed = second.commit().expect("commit second");
    assert_eq!(
        committed.content(),
        &[MessageContent::text("Hello, world")],
        "prior committed message must remain independent of later drafts"
    );
    assert_eq!(second_committed.content(), &[MessageContent::text("next")]);
}

#[test]
fn new_message_carries_tool_calls_and_rejects_invalid_role_content_combinations() {
    let conversation = ConversationId::from("conv-new-message");
    let tool_calls = vec![ToolCallRef::try_new(
        ToolCallId::try_new("call_a").expect("a provider call id"),
        "search",
        ToolCallStatus::Answered,
    )
    .expect("a named call")];

    let assistant = NewMessage::try_new(
        conversation.clone(),
        MessageRole::Assistant,
        vec![MessageContent::text("calling tools")],
        tool_calls.clone(),
    )
    .expect("assistant new message with tool calls");
    assert_eq!(assistant.role(), MessageRole::Assistant);
    assert_eq!(assistant.tool_calls(), tool_calls.as_slice());
    assert_eq!(
        assistant.content(),
        &[MessageContent::text("calling tools")]
    );

    let user = NewMessage::try_new(
        conversation.clone(),
        MessageRole::User,
        vec![MessageContent::text("hello")],
        vec![],
    )
    .expect("user new message");
    assert!(user.tool_calls().is_empty());

    assert_eq!(
        NewMessage::try_new(
            conversation.clone(),
            MessageRole::User,
            vec![MessageContent::ToolResultRef {
                tool_call_id: ToolCallId::try_new("call_a").expect("a provider call id"),
            }],
            vec![],
        ),
        Err(ChatError::InvalidRoleContent)
    );
    assert_eq!(
        NewMessage::try_new(
            conversation.clone(),
            MessageRole::User,
            vec![MessageContent::text("hi")],
            tool_calls.clone(),
        ),
        Err(ChatError::InvalidRoleContent)
    );
    assert_eq!(
        NewMessage::try_new(
            conversation.clone(),
            MessageRole::Tool,
            vec![MessageContent::text("not a tool result")],
            vec![],
        ),
        Err(ChatError::InvalidRoleContent)
    );
    assert_eq!(
        NewMessage::try_new(
            conversation,
            MessageRole::Assistant,
            vec![MessageContent::text("x".repeat(MAX_MESSAGE_TEXT_BYTES + 1))],
            vec![],
        ),
        Err(ChatError::TextTooLarge)
    );
}

#[test]
fn a_message_holds_any_answer_a_run_may_produce() {
    // A 300 KiB answer: over the old 128,000-byte bound, within the run's
    // output budget — it must store, not fail after the run finished.
    let conversation = ConversationId::from("conversation-long-answer");
    let answer = "long answer ".repeat(300 * 1024 / 12);
    assert!(answer.len() > 128_000);
    assert!(NewMessage::try_new(
        conversation,
        MessageRole::Assistant,
        vec![MessageContent::text(answer)],
        vec![],
    )
    .is_ok());
    assert_eq!(MAX_MESSAGE_TEXT_BYTES, 512 * 1024);
}

#[test]
fn attachment_and_artifact_content_use_references_not_embedded_bytes() {
    let attachment = MessageContent::AttachmentRef {
        attachment_id: AttachmentId::new(),
        file_name: "sheet.csv".to_owned(),
        mime_type: "text/csv".to_owned(),
    };
    let artifact = MessageContent::ArtifactRef {
        artifact_id: ArtifactId::new(),
        title: "Revenue".to_owned(),
        kind: "table".to_owned(),
    };

    let attachment_wire = serde_json::to_value(&attachment).expect("serialize attachment ref");
    let artifact_wire = serde_json::to_value(&artifact).expect("serialize artifact ref");

    assert!(attachment_wire.get("bytes").is_none());
    assert!(attachment_wire.get("data").is_none());
    assert!(attachment_wire.get("content").is_none());
    assert!(artifact_wire.get("bytes").is_none());
    assert!(artifact_wire.get("data").is_none());
    assert!(artifact_wire.get("content").is_none());
    assert!(attachment_wire.get("attachment_id").is_some());
    assert!(artifact_wire.get("artifact_id").is_some());

    let message = Message::try_new(
        MessageId::new(),
        ConversationId::from("conv-refs"),
        1,
        MessageRole::User,
        vec![MessageContent::text("see file"), attachment],
        vec![],
    )
    .expect("user message with attachment ref");
    assert!(message
        .content()
        .iter()
        .any(|part| matches!(part, MessageContent::AttachmentRef { .. })));
}

#[test]
fn text_and_tool_metadata_reject_oversized_payloads_at_deserialization_boundaries() {
    let oversized_text = "x".repeat(MAX_MESSAGE_TEXT_BYTES + 1);
    let text_wire = json!({
        "type": "text",
        "text": oversized_text,
    });
    let text_error = serde_json::from_value::<MessageContent>(text_wire)
        .expect_err("oversized text must fail deserialize");
    assert!(
        text_error.to_string().contains("maximum")
            || text_error.to_string().contains("too large")
            || text_error.to_string().contains("MAX_MESSAGE_TEXT"),
        "unexpected error: {text_error}"
    );

    assert_eq!(
        MessageContent::try_text("x".repeat(MAX_MESSAGE_TEXT_BYTES + 1)),
        Err(ChatError::TextTooLarge)
    );
    // The bound is inclusive. Asserted because the composer refuses exactly
    // one byte more than this (`chat::service`'s `MAX_PROMPT_LEN` is this
    // constant), so a `>=` here would refuse a prompt the composer accepted.
    assert!(MessageContent::try_text("x".repeat(MAX_MESSAGE_TEXT_BYTES)).is_ok());

    // A call id and a tool name both arrive as server- or provider-supplied text
    // and land in the transcript, so both are bounded at the deserialize
    // boundary rather than trusted.
    let oversized_id = "c".repeat(MAX_TOOL_CALL_ID_BYTES + 1);
    let tool_wire = json!({
        "id": oversized_id,
        "name": "search",
        "status": "answered",
    });
    let tool_error = serde_json::from_value::<ToolCallRef>(tool_wire)
        .expect_err("an oversized call id must fail deserialize");
    assert!(
        tool_error.to_string().contains("too long"),
        "unexpected error: {tool_error}"
    );
    assert_eq!(
        ToolCallId::try_new(""),
        Err(ChatError::Validation("tool call id is required"))
    );
    assert_eq!(
        ToolCallRef::try_new(
            ToolCallId::try_new("call_a").expect("a provider call id"),
            "n".repeat(MAX_TOOL_NAME_BYTES + 1),
            ToolCallStatus::Answered,
        ),
        Err(ChatError::Validation("tool call name is too long"))
    );
}

#[test]
fn domain_aggregates_expose_bootstrap_snapshot_and_projection_shapes() {
    let conversation_id = ConversationId::from(Uuid::new_v4().to_string());
    let conversation = Conversation::new(conversation_id.clone(), "Planning");
    let summary = ConversationSummary::from_conversation(&conversation);
    assert_eq!(summary.id(), &conversation_id);
    assert_eq!(summary.title(), "Planning");

    let message = Message::try_new(
        MessageId::new(),
        conversation_id.clone(),
        1,
        MessageRole::User,
        vec![MessageContent::text("go")],
        vec![],
    )
    .expect("message")
    .into_completed();

    let run = RunRecord::try_new(
        RunId::from("run-42"),
        conversation_id.clone(),
        RunState::Queued,
        1,
    )
    .expect("run record");
    let cursor = SyncCursor::new(1);
    let bootstrap = ChatBootstrap::new(
        conversation.clone(),
        vec![message.clone()],
        vec![run.clone()],
        cursor.clone(),
    );
    assert_eq!(bootstrap.conversation().id(), &conversation_id);
    assert_eq!(bootstrap.messages().len(), 1);
    assert_eq!(bootstrap.runs()[0].state(), &RunState::Queued);

    let workspace = WorkspaceChatBootstrap::new(vec![summary], SyncCursor::new(0));
    assert_eq!(workspace.conversations().len(), 1);

    let draft = AssistantDraft::new(conversation_id.clone(), 2).expect("draft");
    let snapshot = ChatSnapshot::new(conversation, vec![message], Some(draft), vec![run], cursor);
    assert!(snapshot.draft().is_some());

    let _new_conversation = NewConversation {
        title: Some("Planning".to_owned()),
    };
    let _new_message = NewMessage::try_new(
        conversation_id.clone(),
        MessageRole::User,
        vec![MessageContent::text("go")],
        vec![],
    )
    .expect("new message");
    let _event = ChatProjectionEvent::ReplaceSnapshot(snapshot);
    let _persisted =
        PersistedRunEvent::try_new(RunId::from("run-42"), conversation_id, 2, RunState::Running)
            .expect("run event");
}

/// The sealed-snapshot rule, stated as the loss it prevents: `ToolCallRef` is
/// sealed into `EncryptedChatProjection`, so a field added without a default
/// would make every conversation sealed by an earlier build unreadable on the
/// first launch after this change — an offline copy destroyed by a schema
/// addition.
#[test]
fn a_tool_call_sealed_before_steps_existed_still_opens() {
    let sealed = json!({
        "id": "call_a",
        "name": "project_list_directory",
        "status": "answered",
        "arguments": "{}",
        "arguments_truncated": false,
        "result": "{\"status\":\"completed\"}",
        "result_truncated": false,
    });

    let call: ToolCallRef =
        serde_json::from_value(sealed).expect("a snapshot with no step field must still open");
    assert_eq!(call.step, None);
    assert_eq!(
        call.arguments.as_deref(),
        Some("{}"),
        "nothing else is lost"
    );
}

#[test]
fn a_sealed_step_summary_round_trips_and_survives_a_re_seal() {
    let summary = ToolStepSummaryV1::new(
        "Listed 400 entries",
        vec![ToolStepPathV1 {
            path: ".env".to_owned(),
            is_sensitive: true,
        }],
        400,
    );
    let capped = "x".repeat(xfalcon_contracts::chat::TOOL_PAYLOAD_PROJECTION_CAP_BYTES + 64);
    let call = ToolCallRef::try_new(
        ToolCallId::try_new("call_a").expect("a provider call id"),
        "project_list_directory",
        ToolCallStatus::Answered,
    )
    .expect("call")
    .with_step(Some(summary.clone()))
    .with_payloads(None, Some(capped));
    assert!(call.result_truncated);

    let reopened: ToolCallRef = serde_json::from_str(
        &serde_json::to_string(&call).expect("a call serializes into a snapshot"),
    )
    .expect("a sealed call reopens");

    // Re-capping already-capped text cannot reach the summary, so a re-sealed
    // call keeps it whole — the two truncations stay independent facts.
    assert_eq!(reopened.step, Some(summary));
    assert!(reopened.result_truncated);
    assert_eq!(
        reopened.result.as_deref().map(str::len),
        Some(xfalcon_contracts::chat::TOOL_PAYLOAD_PROJECTION_CAP_BYTES)
    );
}
