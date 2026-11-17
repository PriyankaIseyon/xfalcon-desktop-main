use indexmap::IndexMap;
use serde_json::json;
use uuid::Uuid;
use xfalcon_contracts::ConversationId;

use crate::chat::da_repository::{
    ensure_expected_tenant, map_conversation_bootstrap, DaChatRepository,
};
use crate::chat::domain::{ChatError, ToolCallRef, ToolCallStatus};
use crate::clients::da::chat_api::DaChatSession;
use crate::clients::da::generated::ConversationBootstrap;
use crate::tools::dispatch::recorded_answers as recorded;

fn conversation_map(
    conversation_id: Uuid,
    tenant_id: Option<&str>,
) -> IndexMap<String, serde_json::Value> {
    let mut map = IndexMap::new();
    map.insert("id".to_owned(), json!(conversation_id));
    map.insert("title".to_owned(), json!("Tenant check"));
    if let Some(tenant_id) = tenant_id {
        map.insert("tenant_id".to_owned(), json!(tenant_id));
    }
    map
}

fn bootstrap_with(conversation_id: Uuid, tenant_id: Option<&str>) -> ConversationBootstrap {
    ConversationBootstrap {
        conversation: conversation_map(conversation_id, tenant_id),
        chats: Some(vec![]),
        tool_calls: Some(vec![]),
        artifacts: Some(vec![]),
        attached_server_ids: Some(vec![]),
        latest_run: None,
        model_capacity: None,
    }
}

#[test]
fn with_expected_tenant_stores_tenant_binding() {
    let client = crate::clients::da::mapping::build_hardened_client().expect("client");
    let session = DaChatSession::with_client("https://tenant.example.com", "bearer-token", client)
        .expect("session");
    let repo = DaChatRepository::new(session).with_expected_tenant("tenant-a");
    assert_eq!(repo.expected_tenant_id(), Some("tenant-a"));
}

#[test]
fn ensure_expected_tenant_rejects_mismatch_and_absence() {
    assert_eq!(
        ensure_expected_tenant(Some("tenant-a"), Some("tenant-b")),
        Err(ChatError::Validation("account mismatch"))
    );
    assert_eq!(
        ensure_expected_tenant(Some("tenant-a"), None),
        Err(ChatError::Validation("account mismatch"))
    );
    assert_eq!(
        ensure_expected_tenant(Some("tenant-a"), Some("tenant-a")),
        Ok(())
    );
    assert_eq!(ensure_expected_tenant(None, None), Ok(()));
    assert_eq!(ensure_expected_tenant(None, Some("anything")), Ok(()));
}

#[test]
fn bootstrap_rejects_mismatched_tenant_id() {
    let id = Uuid::new_v4();
    let fallback = ConversationId::from(id.to_string());
    let error = map_conversation_bootstrap(
        bootstrap_with(id, Some("other-tenant")),
        &fallback,
        Some("expected-tenant"),
    )
    .expect_err("mismatched tenant");
    assert_eq!(error, ChatError::Validation("account mismatch"));
}

#[test]
fn bootstrap_rejects_missing_tenant_id_when_expected() {
    let id = Uuid::new_v4();
    let fallback = ConversationId::from(id.to_string());
    let error =
        map_conversation_bootstrap(bootstrap_with(id, None), &fallback, Some("expected-tenant"))
            .expect_err("missing tenant");
    assert_eq!(error, ChatError::Validation("account mismatch"));
}

#[test]
fn bootstrap_rejects_mismatched_conversation_id() {
    let requested = Uuid::new_v4();
    let payload_id = Uuid::new_v4();
    assert_ne!(requested, payload_id);
    let fallback = ConversationId::from(requested.to_string());
    let error = map_conversation_bootstrap(
        bootstrap_with(payload_id, Some("tenant-a")),
        &fallback,
        Some("tenant-a"),
    )
    .expect_err("mismatched conversation id");
    assert_eq!(error, ChatError::Validation("conversation id mismatch"));
}

#[test]
fn bootstrap_accepts_matching_tenant_and_conversation_id() {
    let id = Uuid::new_v4();
    let fallback = ConversationId::from(id.to_string());
    let bootstrap = map_conversation_bootstrap(
        bootstrap_with(id, Some("expected-tenant")),
        &fallback,
        Some("expected-tenant"),
    )
    .expect("matching bootstrap");
    assert_eq!(bootstrap.conversation().id(), &fallback);
    assert_eq!(bootstrap.conversation().title(), "Tenant check");
}

/// One durable tool round as any client writes it: the assistant turn carrying
/// `tool_calls`, then the `role="tool"` message answering it.
///
/// The row shape mirrors `get_chat_bootstrap`'s ACTUAL projection — which has
/// NO `tenant_id` (the RPC is identity-scoped and does not re-project the
/// column). The fixture must never re-add it: a fixture richer than the wire
/// is how the all-rows-drop bug (empty reopened conversations) slipped past
/// this suite.
fn tool_round_chats(conversation_id: Uuid) -> Vec<serde_json::Value> {
    let message = |sequence: i64, role: &str, content: &str, extra: serde_json::Value| {
        let mut value = json!({
            "id": Uuid::new_v4(),
            "conversation_id": conversation_id,
            "user_id": null,
            "sequence_no": sequence,
            "role": role,
            "content": content,
            "token_input": null,
            "token_output": null,
            "metadata": null,
            "superseded_by_checkpoint_id": null,
            "created_at": "2026-08-04T12:00:00Z",
        });
        if let (Some(object), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            for (key, item) in extra {
                object.insert(key.clone(), item.clone());
            }
        }
        value
    };

    vec![
        message(1, "user", "How many rows?", json!({})),
        // Calls-only assistant turn: content is empty, the pairing is in
        // `tool_calls`.
        message(
            2,
            "assistant",
            "",
            json!({"tool_calls": [{
                "id": "call_a",
                "type": "function",
                "function": {"name": "ida_query", "arguments": "{}"}
            }]}),
        ),
        message(
            3,
            "tool",
            r#"{"status":"completed","result":{"rows":42}}"#,
            json!({"tool_call_id": "call_a", "name": "ida_query"}),
        ),
        message(4, "assistant", "42 rows.", json!({})),
    ]
}

/// Reopening a conversation must show what ran, not just what was said — and it
/// must do so for a round another client executed.
#[test]
fn a_conversation_carrying_a_tool_round_shows_the_round_it_ran() {
    let conversation_id = Uuid::new_v4();
    let mut bootstrap = bootstrap_with(conversation_id, Some("tenant"));
    bootstrap.chats = Some(tool_round_chats(conversation_id));

    let snapshot = map_conversation_bootstrap(
        bootstrap,
        &ConversationId::from(conversation_id.to_string()),
        Some("tenant"),
    )
    .expect("a tool round must not make a conversation unreadable");

    let messages = snapshot.messages();
    assert_eq!(
        messages.len(),
        3,
        "the question, the calls-only turn, and the answer — the `role=\"tool\"` \
         row contributes only its existence: {messages:#?}"
    );
    assert_eq!(text_of(&messages[0]), Some("How many rows?"));

    // The calls-only turn: no prose, and the round is the whole turn.
    assert_eq!(text_of(&messages[1]), None);
    let calls = messages[1].tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_str(), "call_a");
    assert_eq!(calls[0].name, "ida_query");
    assert_eq!(
        calls[0].status,
        ToolCallStatus::Answered,
        "the answering row is present, so the call is answered"
    );
    // The narrowed stance: the call carries its capped, display-only payloads.
    assert_eq!(calls[0].arguments.as_deref(), Some("{}"));
    assert_eq!(
        calls[0].result.as_deref(),
        Some(r#"{"status":"completed","result":{"rows":42}}"#),
        "the answer row's content is projected onto the call it answers"
    );
    assert!(!calls[0].arguments_truncated);
    assert!(!calls[0].result_truncated);

    assert_eq!(text_of(&messages[2]), Some("42 rows."));
}

/// The projection is bounded: a payload past the contract cap is cut on a char
/// boundary and flagged, never carried whole and never dropped.
#[test]
fn oversized_tool_payloads_are_truncated_and_flagged() {
    let conversation_id = Uuid::new_v4();
    let mut bootstrap = bootstrap_with(conversation_id, Some("tenant"));
    let cap = xfalcon_contracts::chat::TOOL_PAYLOAD_PROJECTION_CAP_BYTES;
    let huge = "x".repeat(cap + 100);
    let mut chats = tool_round_chats(conversation_id);
    chats[2]["content"] = serde_json::Value::String(huge);
    bootstrap.chats = Some(chats);

    let snapshot = map_conversation_bootstrap(
        bootstrap,
        &ConversationId::from(conversation_id.to_string()),
        Some("tenant"),
    )
    .expect("an oversized answer must not make a conversation unreadable");

    let calls = snapshot.messages()[1].tool_calls();
    assert_eq!(calls[0].status, ToolCallStatus::Answered);
    let result = calls[0].result.as_deref().expect("capped result present");
    assert_eq!(result.len(), cap);
    assert!(calls[0].result_truncated);
}

/// A write can fail between the two halves of a round. A call with no answer is
/// shown as such rather than as a success — a reader cannot tell them apart
/// otherwise, and only one of them produced anything.
#[test]
fn a_call_whose_answer_never_landed_is_shown_as_unanswered() {
    let conversation_id = Uuid::new_v4();
    let mut bootstrap = bootstrap_with(conversation_id, Some("tenant"));
    let mut chats = tool_round_chats(conversation_id);
    chats.retain(|row| row["role"] != "tool");
    bootstrap.chats = Some(chats);

    let snapshot = map_conversation_bootstrap(
        bootstrap,
        &ConversationId::from(conversation_id.to_string()),
        Some("tenant"),
    )
    .expect("a half-written round must not make a conversation unreadable");

    let calls = snapshot.messages()[1].tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, ToolCallStatus::Unanswered);
}

// ---------------------------------------------------------------------
// The step summary rides the projection (spec: chat-tool-call-detail)
// ---------------------------------------------------------------------

/// The same round shape as [`tool_round_chats`], with the call's wire name,
/// argument text and stored answer supplied — the three inputs a step summary is
/// read from.
fn internal_round_chats(
    conversation_id: Uuid,
    name: &str,
    arguments: &str,
    answer: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut chats = tool_round_chats(conversation_id);
    chats[1]["tool_calls"] = json!([{
        "id": "call_a",
        "type": "function",
        "function": {"name": name, "arguments": arguments},
    }]);
    match answer {
        Some(answer) => chats[2]["content"] = json!(answer),
        None => {
            chats.remove(2);
        }
    }
    chats
}

fn projected_call(chats: Vec<serde_json::Value>, conversation_id: Uuid) -> ToolCallRef {
    let mut bootstrap = bootstrap_with(conversation_id, Some("tenant"));
    bootstrap.chats = Some(chats);
    let snapshot = map_conversation_bootstrap(
        bootstrap,
        &ConversationId::from(conversation_id.to_string()),
        Some("tenant"),
    )
    .expect("a tool round must not make a conversation unreadable");
    snapshot.messages()[1]
        .tool_calls()
        .first()
        .cloned()
        .expect("the assistant turn's call")
}

#[test]
fn a_projected_listing_carries_a_step_naming_the_files_it_listed() {
    let conversation_id = Uuid::new_v4();
    let call = projected_call(
        internal_round_chats(
            conversation_id,
            "project_list_directory",
            recorded::LIST_DIRECTORY_ARGUMENTS,
            Some(recorded::LIST_DIRECTORY),
        ),
        conversation_id,
    );

    let step = call
        .step
        .expect("a listing this app executed carries a step");
    assert_eq!(step.label, "Listed 2 entries");
    assert_eq!(
        step.paths
            .iter()
            .map(|path| path.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/features/cards.tsx", "src/features/id_ed25519"]
    );
    assert!(step.paths[1].is_sensitive);
}

/// The ordering requirement, stated as the failure it prevents: the capped copy
/// is cut mid-document and no longer parses, so a summary derived from it would
/// vanish exactly on the calls with the largest outcomes.
#[test]
fn an_oversized_listing_keeps_a_complete_summary_while_its_result_is_cut() {
    let conversation_id = Uuid::new_v4();
    let entries: Vec<_> = (0..400)
        .map(|index| {
            json!({
                "name": format!("entry-{index}-{padding}.ts", padding = "p".repeat(120)),
                "kind": "file",
                "size_bytes": 12,
                "is_sensitive": false,
            })
        })
        .collect();
    let answer = json!({
        "status": "completed",
        "result": { "entries": entries },
        "truncated": false,
        "content_refs": [],
    })
    .to_string();
    assert!(
        answer.len() > xfalcon_contracts::chat::TOOL_PAYLOAD_PROJECTION_CAP_BYTES,
        "the case only means something if the projection cap actually bites"
    );

    let call = projected_call(
        internal_round_chats(
            conversation_id,
            "project_list_directory",
            recorded::LIST_DIRECTORY_ROOT_ARGUMENTS,
            Some(&answer),
        ),
        conversation_id,
    );

    assert!(call.result_truncated, "the Result panel's copy is bounded");
    let step = call.step.expect("the summary predates the cap");
    assert_eq!(step.label, "Listed 400 entries");
    assert_eq!(
        step.paths.len(),
        xfalcon_contracts::chat::TOOL_STEP_PATH_CAP
    );
    assert_eq!(
        step.path_total, 400,
        "twelve is this card's display budget, not what the run reached"
    );
}

#[test]
fn a_projected_mcp_call_carries_no_step() {
    let conversation_id = Uuid::new_v4();
    let call = projected_call(
        internal_round_chats(
            conversation_id,
            "mcp_tenant_ida_query",
            r#"{"sql":"select 1"}"#,
            Some(recorded::LIST_DIRECTORY),
        ),
        conversation_id,
    );

    assert_eq!(
        call.step, None,
        "a server's claim about the filesystem must not be spoken in this app's voice"
    );
    assert_eq!(
        call.arguments.as_deref(),
        Some(r#"{"sql":"select 1"}"#),
        "its payloads still project exactly as before"
    );
    assert_eq!(call.result.as_deref(), Some(recorded::LIST_DIRECTORY));
}

#[test]
fn an_unanswered_call_carries_no_step() {
    let conversation_id = Uuid::new_v4();
    let call = projected_call(
        internal_round_chats(
            conversation_id,
            "project_list_directory",
            recorded::LIST_DIRECTORY_ARGUMENTS,
            None,
        ),
        conversation_id,
    );

    assert_eq!(call.status, ToolCallStatus::Unanswered);
    assert_eq!(
        call.step, None,
        "a label with no outcome behind it is a claim about a call that never returned"
    );
}

#[test]
fn a_projected_denial_says_denied_rather_than_reading_like_a_success() {
    let conversation_id = Uuid::new_v4();
    let call = projected_call(
        internal_round_chats(
            conversation_id,
            "project_propose_patch",
            r#"{"operations":[]}"#,
            Some(recorded::DENIED),
        ),
        conversation_id,
    );

    // The answer row landed, so the trail marks the call answered either way.
    assert_eq!(call.status, ToolCallStatus::Answered);
    let step = call.step.expect("a denied row is answered and must say so");
    assert_eq!(step.label, "Blocked — this call was not permitted");
    assert!(step.paths.is_empty());
}

fn text_of(message: &crate::chat::domain::Message) -> Option<&str> {
    message.content().iter().find_map(|part| match part {
        crate::chat::domain::MessageContent::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

#[test]
fn terminal_da_runs_do_not_reappear_as_active_after_reconciliation() {
    use xfalcon_contracts::RunState;
    for (status, expected) in [
        ("queued", RunState::Queued),
        ("running", RunState::Running),
        ("streaming", RunState::Running),
        ("awaiting_approval", RunState::AwaitingApproval),
        ("completed", RunState::Completed),
        ("error", RunState::Failed),
        ("cancelled", RunState::Cancelled),
    ] {
        let conversation = Uuid::new_v4();
        let mut bootstrap = bootstrap_with(conversation, Some("tenant"));
        bootstrap.latest_run = Some(
            serde_json::from_value(json!({
                "id": Uuid::new_v4(), "status": status
            }))
            .unwrap(),
        );
        let snapshot = map_conversation_bootstrap(
            bootstrap,
            &ConversationId::from(conversation.to_string()),
            Some("tenant"),
        )
        .unwrap();
        assert_eq!(snapshot.runs()[0].state(), &expected, "{status}");
        if matches!(status, "completed" | "error" | "cancelled") {
            assert!(!crate::chat::wire::is_active_run(&snapshot.runs()[0]));
        }
    }
}

#[test]
fn a_user_turn_lists_the_files_it_was_sent_with_all_the_way_to_the_transcript() {
    use crate::chat::da_repository::with_attachments;
    use crate::chat::domain::ChatSnapshot;
    use crate::chat::sync::SyncAvailability;
    use crate::chat::wire::conversation_view_from;
    use crate::clients::da::generated::ChatAttachment;

    let conversation_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let reply_id = Uuid::new_v4();
    let mut bootstrap = bootstrap_with(conversation_id, None);
    bootstrap.chats = Some(vec![
        json!({"id": user_id, "conversation_id": conversation_id, "sequence_no": 1, "role": "user", "content": "Attached files."}),
        json!({"id": reply_id, "conversation_id": conversation_id, "sequence_no": 2, "role": "assistant", "content": "Done."}),
    ]);
    let mapped = map_conversation_bootstrap(
        bootstrap,
        &ConversationId::from(conversation_id.to_string()),
        None,
    )
    .expect("maps");
    let attachment = |name: &str, message_id: Option<Uuid>| ChatAttachment {
        id: Uuid::new_v4(),
        conversation_id,
        message_id,
        file_name: name.to_owned(),
        mime_type: "text/csv".to_owned(),
        ..Default::default()
    };
    let mapped = with_attachments(
        mapped,
        &[
            attachment("schema_consistent.csv", Some(user_id)),
            // Linked to the assistant's row, and to nothing: neither is a
            // file the person sent with a message.
            attachment("stray.csv", Some(reply_id)),
            attachment("unsent.csv", None),
        ],
    );

    let snapshot = ChatSnapshot::new(
        mapped.conversation().clone(),
        mapped.messages().to_vec(),
        None,
        mapped.runs().to_vec(),
        mapped.cursor().clone(),
    );
    let view = conversation_view_from(snapshot, SyncAvailability::Live);
    assert_eq!(view.messages[0].text, "Attached files.");
    assert_eq!(view.messages[0].attachments.len(), 1);
    assert_eq!(
        view.messages[0].attachments[0].file_name,
        "schema_consistent.csv"
    );
    assert!(view.messages[1].attachments.is_empty());
}
