//! Tests for the `ToolInvocation` state machine and its journal, mirroring
//! `agent/run.rs`'s rigor: explicit transition table, terminal immutability,
//! and journal-before-mutation ordering. Also proves the "stable invocation
//! IDs make retries idempotent" property the plan calls for.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use xfalcon_contracts::tools::{
    InvocationId, ToolId, ToolInvocationEvent, ToolInvocationStatus, ToolSource,
};
use xfalcon_contracts::RunId;

use crate::tools::invocation::{
    ContentRef, InMemoryInvocationJournal, InvocationError, InvocationJournal,
    InvocationJournalError, ToolInvocation, ToolResult, ToolResultError, ToolResultPreviewStrategy,
    MAX_EXACT_TOOL_RESULT_BYTES,
};
use crate::tools::policy::{Actor, PolicyDenial};
use crate::tools::ToolResultBudget;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn open_invocation(
    journal: &mut InMemoryInvocationJournal,
) -> (ToolInvocation, InvocationId, RunId, ToolId) {
    let invocation_id = InvocationId::from("invocation-1");
    let run_id = RunId::from("run-1");
    let tool_id = ToolId::from("read_tool");
    let actor = Actor::new("user-1").expect("valid actor");
    let (invocation, outcome) = ToolInvocation::open(
        invocation_id.clone(),
        run_id.clone(),
        tool_id.clone(),
        json!({}),
        actor,
        deadline(),
        journal,
    )
    .expect("open should succeed");
    assert_eq!(outcome.status, ToolInvocationStatus::Requested);
    assert_eq!(outcome.sequence, 1);
    (invocation, invocation_id, run_id, tool_id)
}

#[test]
fn open_journals_requested_as_the_first_sequence() {
    let mut journal = InMemoryInvocationJournal::new();
    let (invocation, invocation_id, run_id, tool_id) = open_invocation(&mut journal);

    assert_eq!(invocation.status(), ToolInvocationStatus::Requested);
    assert_eq!(invocation.sequence(), 1);
    let entries = journal.entries_for(&invocation_id);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].event,
        ToolInvocationEvent::requested(invocation_id, run_id, tool_id)
    );
}

#[test]
fn open_rejects_oversized_arguments() {
    let mut journal = InMemoryInvocationJournal::new();
    let oversized = json!({ "data": "x".repeat(crate::tools::invocation::MAX_ARGUMENTS_BYTES) });
    let result = ToolInvocation::open(
        InvocationId::from("invocation-1"),
        RunId::from("run-1"),
        ToolId::from("read_tool"),
        oversized,
        Actor::new("user-1").expect("valid actor"),
        deadline(),
        &mut journal,
    );
    assert!(matches!(result, Err(InvocationError::ArgumentsTooLarge(_))));
    assert!(journal.entries().is_empty());
}

#[test]
fn full_lifecycle_reaches_completed_through_approval_and_running() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    invocation
        .require_approval(&mut journal)
        .expect("requires approval");
    assert_eq!(invocation.status(), ToolInvocationStatus::AwaitingApproval);

    invocation.approve(&mut journal).expect("approve");
    assert_eq!(invocation.status(), ToolInvocationStatus::Approved);

    invocation.start(&mut journal).expect("start");
    assert_eq!(invocation.status(), ToolInvocationStatus::Running);

    invocation
        .progress("halfway done", &mut journal)
        .expect("progress");
    assert_eq!(invocation.status(), ToolInvocationStatus::Running);

    invocation.complete(false, &mut journal).expect("complete");
    assert_eq!(invocation.status(), ToolInvocationStatus::Completed);
}

#[test]
fn full_lifecycle_reaches_failed_through_approval_and_running() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    invocation
        .require_approval(&mut journal)
        .expect("requires approval");
    invocation.approve(&mut journal).expect("approve");
    invocation.start(&mut journal).expect("start");
    assert_eq!(invocation.status(), ToolInvocationStatus::Running);

    invocation
        .fail("provider unavailable", true, &mut journal)
        .expect("fail");
    assert_eq!(invocation.status(), ToolInvocationStatus::Failed);

    // Failed is terminal: no further transition is accepted.
    let result = invocation.complete(false, &mut journal);
    assert!(matches!(
        result,
        Err(InvocationError::TerminalImmutable(
            ToolInvocationStatus::Failed
        ))
    ));
}

#[test]
fn illegal_transition_is_rejected() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    // Requested -> Running is not in the allowed-transition table; approval
    // and start must happen first.
    let result = invocation.start(&mut journal);
    assert!(matches!(
        result,
        Err(InvocationError::IllegalTransition {
            from: ToolInvocationStatus::Requested,
            to: ToolInvocationStatus::Running,
        })
    ));
}

#[test]
fn terminal_status_rejects_further_mutation() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    invocation
        .deny(&PolicyDenial::PrincipalNotAuthenticated, &mut journal)
        .expect("deny");
    assert_eq!(invocation.status(), ToolInvocationStatus::Denied);

    let result = invocation.approve(&mut journal);
    assert!(matches!(
        result,
        Err(InvocationError::TerminalImmutable(
            ToolInvocationStatus::Denied
        ))
    ));
}

#[test]
fn cancel_is_idempotent_once_already_cancelled() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    invocation
        .require_approval(&mut journal)
        .expect("requires approval");
    invocation.cancel(&mut journal).expect("first cancel");
    assert_eq!(invocation.status(), ToolInvocationStatus::Cancelled);
    let sequence_after_first_cancel = invocation.sequence();

    // A second cancel on an already-cancelled invocation must not error and
    // must not advance the sequence (no new journal entry).
    let outcome = invocation
        .cancel(&mut journal)
        .expect("second cancel is a no-op");
    assert_eq!(outcome.status, ToolInvocationStatus::Cancelled);
    assert_eq!(invocation.sequence(), sequence_after_first_cancel);
}

#[test]
fn progress_is_rejected_outside_running_status() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    let result = invocation.progress("too early", &mut journal);
    assert!(matches!(
        result,
        Err(InvocationError::ProgressWhileNotRunning)
    ));
}

#[test]
fn deny_rejects_an_oversized_reason_code() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);

    // Every real `PolicyDenial::code()` is a short, fixed constant, so this
    // uses the test-only raw-reason-code hatch to prove the bound itself is
    // still enforced (defense in depth), mirroring how `ArgumentsTooLarge`
    // is exercised directly against `open`.
    let oversized_code = "x".repeat(129);
    let result = invocation.deny_with_raw_reason_code(oversized_code, &mut journal);

    assert!(matches!(
        result,
        Err(InvocationError::ReasonCodeTooLarge(_))
    ));
    assert_eq!(invocation.status(), ToolInvocationStatus::Requested);
    assert_eq!(invocation.sequence(), 1);
}

#[test]
fn progress_rejects_an_oversized_summary() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);
    invocation.approve(&mut journal).expect("approve");
    invocation.start(&mut journal).expect("start");
    let sequence_before = invocation.sequence();

    let oversized_summary = "x".repeat(2_049);
    let result = invocation.progress(oversized_summary, &mut journal);

    assert!(matches!(
        result,
        Err(InvocationError::ProgressSummaryTooLarge(_))
    ));
    assert_eq!(invocation.status(), ToolInvocationStatus::Running);
    assert_eq!(invocation.sequence(), sequence_before);
}

#[test]
fn fail_rejects_an_oversized_message() {
    let mut journal = InMemoryInvocationJournal::new();
    let (mut invocation, ..) = open_invocation(&mut journal);
    invocation.approve(&mut journal).expect("approve");
    invocation.start(&mut journal).expect("start");
    let sequence_before = invocation.sequence();

    let oversized_message = "x".repeat(2_049);
    let result = invocation.fail(oversized_message, true, &mut journal);

    assert!(matches!(
        result,
        Err(InvocationError::FailureMessageTooLarge(_))
    ));
    assert_eq!(invocation.status(), ToolInvocationStatus::Running);
    assert_eq!(invocation.sequence(), sequence_before);
}

#[test]
fn debug_output_never_reveals_raw_arguments() {
    let mut journal = InMemoryInvocationJournal::new();
    let invocation_id = InvocationId::from("invocation-1");
    let run_id = RunId::from("run-1");
    let tool_id = ToolId::from("read_tool");
    let actor = Actor::new("user-1").expect("valid actor");
    let (invocation, _) = ToolInvocation::open(
        invocation_id,
        run_id,
        tool_id,
        json!({ "secret": "TOP_SECRET_MARKER" }),
        actor,
        deadline(),
        &mut journal,
    )
    .expect("open should succeed");

    let debug_output = format!("{invocation:?}");
    assert!(!debug_output.contains("TOP_SECRET_MARKER"));
    assert!(debug_output.contains("arguments_bytes"));
}

#[test]
fn journal_replay_of_the_same_invocation_and_sequence_is_idempotent() {
    let mut journal = InMemoryInvocationJournal::new();
    let invocation_id = InvocationId::from("invocation-1");
    let run_id = RunId::from("run-1");
    let tool_id = ToolId::from("read_tool");
    let event = ToolInvocationEvent::requested(invocation_id.clone(), run_id, tool_id);

    journal
        .record(&invocation_id, 1, event.clone())
        .expect("first record succeeds");
    journal
        .record(&invocation_id, 1, event.clone())
        .expect("replaying the identical event is a no-op, proving retries are safe");

    assert_eq!(journal.entries_for(&invocation_id).len(), 1);
}

#[test]
fn journal_rejects_a_genuine_sequence_conflict() {
    let mut journal = InMemoryInvocationJournal::new();
    let invocation_id = InvocationId::from("invocation-1");
    let run_id = RunId::from("run-1");
    let tool_id = ToolId::from("read_tool");
    let first_event =
        ToolInvocationEvent::requested(invocation_id.clone(), run_id, tool_id.clone());
    let conflicting_event = ToolInvocationEvent::cancelled(invocation_id.clone());

    journal
        .record(&invocation_id, 1, first_event)
        .expect("first record succeeds");
    let result = journal.record(&invocation_id, 1, conflicting_event);

    assert!(matches!(
        result,
        Err(InvocationJournalError::SequenceConflict { sequence: 1, .. })
    ));
    assert_eq!(journal.entries_for(&invocation_id).len(), 1);
}

#[test]
fn tool_result_keeps_structured_output_above_the_old_64_kib_limit() {
    let oversized = json!({ "data": "x".repeat(128 * 1024) });
    let result = ToolResult::new(Vec::new(), Some(oversized), ToolSource::Internal)
        .expect("128 KiB remains exact");

    assert!(result.structured().is_some());
    assert!(!result.truncated());
    assert!(!result.model_preview(ToolResultBudget::floor()).complete());
    assert_eq!(
        result.model_preview(ToolResultBudget::floor()).strategy(),
        ToolResultPreviewStrategy::JsonStructure
    );
    assert!(
        serde_json::to_vec(result.model_preview(ToolResultBudget::floor()).value())
            .expect("preview serializes")
            .len()
            <= ToolResultBudget::floor().result_bytes()
    );
}

#[test]
fn model_preview_elides_results_above_the_durable_context_limit() {
    let oversized = json!({ "data": "x".repeat(ToolResultBudget::FLOOR_BYTES) });
    let result = ToolResult::new(Vec::new(), Some(oversized), ToolSource::Internal)
        .expect("exact result remains available");
    let preview = result.model_preview(ToolResultBudget::floor());

    assert!(!preview.complete());
    assert!(
        serde_json::to_vec(preview.value())
            .expect("preview serializes")
            .len()
            <= ToolResultBudget::floor().result_bytes()
    );
}

#[test]
fn tool_result_rejects_retaining_exact_bytes_above_eight_mib() {
    let oversized = json!({ "data": "x".repeat(MAX_EXACT_TOOL_RESULT_BYTES) });
    let error = ToolResult::new(Vec::new(), Some(oversized), ToolSource::Internal)
        .expect_err("values above 8 MiB fail loudly");

    assert!(matches!(
        error,
        ToolResultError::StructuredResultTooLarge {
            actual_bytes,
            max_bytes: MAX_EXACT_TOOL_RESULT_BYTES,
        } if actual_bytes > MAX_EXACT_TOOL_RESULT_BYTES
    ));
}

#[test]
fn tool_result_accepts_exactly_eight_mib() {
    // JSON string serialization adds the two quote bytes.
    let boundary = Value::String("x".repeat(MAX_EXACT_TOOL_RESULT_BYTES - 2));
    let result = ToolResult::new(Vec::new(), Some(boundary), ToolSource::Internal)
        .expect("the exact ceiling is inclusive");

    assert_eq!(
        serde_json::to_vec(result.structured().expect("structured value"))
            .expect("value serializes")
            .len(),
        MAX_EXACT_TOOL_RESULT_BYTES
    );
    assert!(!result.truncated());
}

#[test]
fn a_table_preview_keeps_every_leading_row_that_fits_and_counts_the_rest() {
    let rows = (0..10_000)
        .map(|index| json!({"session_id": index, "amount": index * 2}))
        .collect::<Vec<_>>();
    let value = Value::Array(rows);
    let exact = serde_json::to_vec(&value).expect("exact result serializes");
    let result = ToolResult::new(Vec::new(), Some(value), ToolSource::Internal)
        .expect("table remains below exact ceiling");

    let mut kept_before = 0;
    for budget in [
        ToolResultBudget::floor(),
        ToolResultBudget::for_context_tokens(1_000_000),
    ] {
        let preview = result.model_preview(budget);
        let value = preview.value();
        assert!(!preview.complete());
        assert_eq!(value["kind"], "table");
        assert_eq!(value["item_count"], 10_000);
        assert_eq!(value["columns"], json!(["amount", "session_id"]));
        let items = value["items"].as_array().expect("items");
        // Rows arrive in order and whole, and the count of the rest is exact.
        assert_eq!(items[0], json!({"session_id": 0, "amount": 0}));
        assert_eq!(items.last().unwrap()["session_id"], items.len() - 1);
        assert_eq!(value["omitted_items"], 10_000 - items.len());
        assert!(serde_json::to_vec(value).unwrap().len() <= budget.result_bytes());
        // A larger budget shows more of the table, not the same eight rows.
        assert!(items.len() > kept_before);
        kept_before = items.len();
        assert_eq!(preview.exact_byte_length(), exact.len());
        assert_eq!(preview.sha256().len(), 64);
    }
    assert!(
        kept_before > 1_000,
        "a 1M-token budget carries thousands of rows"
    );
}

#[test]
fn an_object_preview_keeps_small_fields_whole_and_cuts_the_large_one_to_fit() {
    let content = "line of text\n".repeat(20_000);
    let value = json!({
        "kind": "text",
        "path": "notes.txt",
        "first_line": 1,
        "content": content,
    });
    let result = ToolResult::new(Vec::new(), Some(value), ToolSource::Internal).expect("result");
    let budget = ToolResultBudget::floor();
    let preview = result.model_preview(budget);
    let fitted = preview.value();

    assert!(!preview.complete());
    assert_eq!(fitted["kind"], "text");
    assert_eq!(fitted["path"], "notes.txt");
    assert_eq!(fitted["first_line"], 1);
    let cut = &fitted["content"];
    assert_eq!(cut["kind"], "text");
    assert_eq!(cut["total_bytes"], content.len());
    let head = cut["head"].as_str().expect("head");
    let tail = cut["tail"].as_str().expect("tail");
    assert!(content.starts_with(head) && content.ends_with(tail));
    assert_eq!(
        cut["omitted_bytes"].as_u64().unwrap() as usize,
        content.len() - head.len() - tail.len()
    );
    // The head gets most of the room: the start says what the text is.
    assert!(head.len() > tail.len() * 2);
    assert!(serde_json::to_vec(fitted).unwrap().len() <= budget.result_bytes());
}

#[test]
fn an_object_of_more_fields_than_fit_keeps_those_that_do_and_counts_the_rest() {
    let fields = (0..5_000)
        .map(|index| (format!("field_{index:05}"), json!("v".repeat(40))))
        .collect::<serde_json::Map<_, _>>();
    let result = ToolResult::new(
        Vec::new(),
        Some(serde_json::Value::Object(fields)),
        ToolSource::Internal,
    )
    .expect("result");
    let budget = ToolResultBudget::floor();
    let preview = result.model_preview(budget);
    let fitted = preview.value();

    assert!(serde_json::to_vec(fitted).unwrap().len() <= budget.result_bytes());
    assert_eq!(fitted["kind"], "object");
    assert_eq!(fitted["field_count"], 5_000);
    let kept = fitted["fields"].as_object().expect("the fields that fit");
    assert!(kept.len() > 100, "a page of whole fields, not a bare note");
    assert!(kept.values().all(|value| value == &json!("v".repeat(40))));
    assert_eq!(fitted["omitted_fields"], 5_000 - kept.len());
}

#[test]
fn a_nested_result_fits_at_every_level_and_keeps_its_field_order() {
    let rows = (0..8_000)
        .map(|index| json!({"id": index, "note": format!("row {index} \u{1}\"\u{e9}\"")}))
        .collect::<Vec<_>>();
    let log = "entry\n".repeat(60_000);
    let value = json!({
        "status": "ok",
        "rows": rows,
        "log": log,
        "count": 8_000,
    });
    let result =
        ToolResult::new(Vec::new(), Some(value.clone()), ToolSource::Internal).expect("result");

    for budget in [
        ToolResultBudget::floor(),
        ToolResultBudget::for_context_tokens(1_000_000),
    ] {
        let preview = result.model_preview(budget);
        let fitted = preview.value();
        assert!(serde_json::to_vec(fitted).unwrap().len() <= budget.result_bytes());
        assert_eq!(
            fitted
                .as_object()
                .expect("still an object")
                .keys()
                .collect::<Vec<_>>(),
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            "fields keep the result's own order"
        );
        assert_eq!(fitted["status"], "ok");
        assert_eq!(fitted["count"], 8_000);

        let table = &fitted["rows"];
        assert_eq!(table["kind"], "table");
        let items = table["items"].as_array().expect("rows");
        assert!(!items.is_empty());
        assert_eq!(items[..], rows[..items.len()], "leading rows, whole");
        assert_eq!(table["omitted_items"], 8_000 - items.len());

        let text = &fitted["log"];
        let head = text["head"].as_str().expect("head");
        let tail = text["tail"].as_str().expect("tail");
        assert!(log.starts_with(head) && log.ends_with(tail));
        assert_eq!(
            text["omitted_bytes"].as_u64().unwrap() as usize,
            log.len() - head.len() - tail.len()
        );
    }
}

#[test]
fn a_text_preview_fills_its_budget_and_counts_what_it_drops_exactly() {
    // Every character escapes differently: two-byte, six-byte control,
    // escaped quote, escaped newline, three-byte.
    let text = "\u{e9}\u{1}\"\n\u{6f22}".repeat(20_000);
    let result =
        ToolResult::new(Vec::new(), Some(json!(text)), ToolSource::Internal).expect("result");
    let budget = ToolResultBudget::floor();
    let preview = result.model_preview(budget);
    let fitted = preview.value();

    assert_eq!(preview.strategy(), ToolResultPreviewStrategy::TextEdges);
    let size = serde_json::to_vec(fitted).unwrap().len();
    assert!(size <= budget.result_bytes());
    assert!(size > budget.result_bytes() - 64, "the budget is filled");
    let head = fitted["head"].as_str().expect("head");
    let tail = fitted["tail"].as_str().expect("tail");
    assert!(text.starts_with(head) && text.ends_with(tail));
    assert!(head.len() > tail.len() * 2);
    assert_eq!(
        fitted["omitted_bytes"].as_u64().unwrap() as usize,
        text.len() - head.len() - tail.len()
    );
    assert_eq!(fitted["total_bytes"], text.len());
}

#[test]
fn a_result_that_fits_the_budget_is_complete() {
    let value = json!({"rows": (0..100).collect::<Vec<_>>()});
    let result =
        ToolResult::new(Vec::new(), Some(value.clone()), ToolSource::Internal).expect("result");
    let preview = result.model_preview(ToolResultBudget::floor());
    assert!(preview.complete());
    assert_eq!(preview.value(), &value);
    assert_eq!(preview.strategy(), ToolResultPreviewStrategy::Complete);
}

#[test]
fn tool_result_caps_content_ref_count_and_marks_truncated() {
    let refs: Vec<ContentRef> = (0..100)
        .map(|index| ContentRef::new(format!("ref-{index}"), 128))
        .collect();
    let result = ToolResult::new(refs, None, ToolSource::Mcp).expect("no structured value");

    assert!(result.truncated());
    assert_eq!(result.content_refs().len(), 64);
}

#[test]
fn tool_result_debug_output_never_reveals_structured_content() {
    let structured = json!({ "secret": "TOP_SECRET_MARKER" });
    let result = ToolResult::new(Vec::new(), Some(structured), ToolSource::Internal)
        .expect("small structured result");

    let debug_output = format!("{result:?}");
    assert!(!debug_output.contains("TOP_SECRET_MARKER"));
    assert!(debug_output.contains("structured_present"));
}
