use serde_json::json;
use uuid::Uuid;

use crate::agent::context::ContextRole;
use crate::agent::turn_driver::UNANSWERED_CALL_ANSWER;
use crate::chat::history::context_from_snapshot;
use crate::clients::da::generated::{ChatMessageToolCalls, ContextMessage, ContextMessageRole};

fn row(sequence_no: i64, role: ContextMessageRole, content: &str) -> ContextMessage {
    ContextMessage {
        id: Uuid::from_u128(u128::try_from(sequence_no).unwrap_or(0) + 1),
        sequence_no,
        role,
        content: content.to_owned(),
        ..Default::default()
    }
}

fn assistant_calling(sequence_no: i64, call_ids: &[&str]) -> ContextMessage {
    let calls = call_ids
        .iter()
        .map(|id| {
            json!({"id": id, "type": "function", "function": {"name": "project.read_file", "arguments": "{}"}})
        })
        .collect();
    ContextMessage {
        tool_calls: Some(ChatMessageToolCalls::Array(calls)),
        ..row(sequence_no, ContextMessageRole::Assistant, "Reading.")
    }
}

fn answer(sequence_no: i64, call_id: &str) -> ContextMessage {
    ContextMessage {
        tool_call_id: Some(call_id.to_owned()),
        name: Some("project.read_file".to_owned()),
        ..row(
            sequence_no,
            ContextMessageRole::Tool,
            r#"{"status":"completed"}"#,
        )
    }
}

#[test]
fn a_round_with_every_call_answered_is_replayed_as_recorded() {
    let rows = [
        row(1, ContextMessageRole::User, "update the pages"),
        assistant_calling(2, &["a", "b"]),
        answer(3, "a"),
        answer(4, "b"),
        row(5, ContextMessageRole::Assistant, "Done."),
    ];
    let context = context_from_snapshot(&rows).expect("a closed round is valid");
    assert_eq!(context.len(), 5);
    assert_eq!(context[2].content, r#"{"status":"completed"}"#);
    assert!(context[2].durable_identity.is_some());
}

#[test]
fn a_round_cut_off_before_its_results_were_saved_no_longer_blocks_the_conversation() {
    let rows = [
        row(1, ContextMessageRole::User, "update the pages"),
        assistant_calling(2, &["a", "b"]),
        answer(3, "a"),
        row(4, ContextMessageRole::User, "retry"),
    ];
    let context = context_from_snapshot(&rows).expect("an open round is closed, not refused");
    assert_eq!(context.len(), 5);
    assert_eq!(context[2].content, r#"{"status":"completed"}"#);
    let closed = &context[3];
    assert_eq!(closed.role, ContextRole::Tool);
    assert_eq!(closed.content, UNANSWERED_CALL_ANSWER);
    assert_eq!(
        closed.responds_to.as_ref().map(|r| r.call_id.as_str()),
        Some("b")
    );
    assert!(
        closed.durable_identity.is_none(),
        "compaction must not count it"
    );
    assert_eq!(context[4].content, "retry");
}

#[test]
fn a_result_with_no_call_to_answer_is_left_out() {
    let rows = [
        row(1, ContextMessageRole::User, "hi"),
        answer(2, "nothing-asked-for-this"),
        row(3, ContextMessageRole::Assistant, "Hello."),
    ];
    let context = context_from_snapshot(&rows).expect("an orphan result is ignored");
    assert_eq!(context.len(), 2);
}
