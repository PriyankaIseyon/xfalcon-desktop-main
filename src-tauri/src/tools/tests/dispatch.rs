//! Live internal tool dispatch, against a real trusted project on disk.

use std::fs;
use std::str::FromStr;
use std::sync::Arc;

use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::tools::{InvocationId, ToolId};
use xfalcon_contracts::RunId;

use crate::agent::tool_loop::{ToolCallOutcome, ToolExecutionError, ToolExecutorPort};
use crate::agent::tool_session::wire_tool_name;
use crate::agent::turn_driver::render_tool_outcome;
use crate::approvals::ApprovalsService;
use crate::persistence::InMemorySecretStore;
use crate::projects::{ProjectId, ProjectsService, SecretClassifier};
use crate::security::account_cleanup::{CurrentAccountScope, ValidatedAccountId};
use crate::tools::dispatch::{
    recorded_answers as recorded, tool_step_summary, ProjectToolContext, ProjectToolDispatcher,
};
use crate::tools::invocation::ToolResult;

const ACCOUNT: &str = "dispatch-account";
const PRODUCER_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.docx");

struct Fixture {
    dispatcher: ProjectToolDispatcher,
    scope: Arc<CurrentAccountScope>,
    /// Held so the temporary directories outlive the dispatcher.
    _app_root: TempDir,
    _project_root: TempDir,
}

async fn fixture() -> Fixture {
    let app_root = TempDir::new().expect("app root");
    let project_root = TempDir::new().expect("project root");
    std::fs::write(project_root.path().join("README.md"), b"hello world\n").expect("seed file");
    std::fs::create_dir(project_root.path().join("src")).expect("seed dir");

    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native(ACCOUNT).expect("bind account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key(ACCOUNT, [0x11; 32]).expect("account key");
    let projects = Arc::new(ProjectsService::new(
        app_root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        app_root.path().to_path_buf(),
        scope.clone(),
        secrets,
    ));

    let view = projects
        .create(
            project_root.path().to_str().expect("utf-8 project path"),
            Some("Fixture".to_owned()),
        )
        .await
        .expect("trust the project");
    let project_id = ProjectId::from_str(view.id.as_str()).expect("project id");

    let dispatcher = ProjectToolDispatcher::new(ProjectToolContext {
        projects,
        approvals,
        classifier: Arc::new(SecretClassifier::built_in()),
        account: ValidatedAccountId::parse(ACCOUNT).expect("account"),
        project_id,
        run_id: RunId::from("run-dispatch"),
        budget: crate::tools::ToolResultBudget::floor(),
    });

    Fixture {
        dispatcher,
        scope,
        _app_root: app_root,
        _project_root: project_root,
    }
}

async fn call(
    fixture: &Fixture,
    tool: &str,
    arguments: Value,
) -> Result<ToolResult, ToolExecutionError> {
    fixture
        .dispatcher
        .execute(&ToolId::from(tool), &arguments, CancellationToken::new())
        .await
}

fn structured(result: &ToolResult) -> Value {
    result
        .structured()
        .cloned()
        .expect("an internal tool always returns a structured result")
}

// ---------------------------------------------------------------------
// The run's project is not negotiable by the model
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_model_supplied_project_id_cannot_change_the_run_bound_authority() {
    // The published schema rejects this legacy/extra field. Dispatch also
    // treats it as inert: authority comes only from ProjectToolContext.
    let fixture = fixture().await;
    let other = ProjectId::new();

    let result = call(
        &fixture,
        "project.list_directory",
        json!({"project_id": other.to_string()}),
    )
    .await;

    let entries = structured(&result.expect("the inert field cannot retarget the call"));
    let names: Vec<_> = entries["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(names.contains(&"README.md"));
}

#[tokio::test]
async fn a_call_needs_no_model_visible_project_id() {
    let fixture = fixture().await;

    let result = call(&fixture, "project.list_directory", json!({})).await;

    assert!(
        result.is_ok(),
        "the run context already carries authority: {result:?}"
    );
}

// ---------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------

#[tokio::test]
async fn list_directory_returns_the_project_root_children() {
    let fixture = fixture().await;

    let result = call(&fixture, "project.list_directory", json!({}))
        .await
        .expect("list succeeds");

    let entries = structured(&result);
    let names: Vec<&str> = entries["entries"]
        .as_array()
        .expect("entries is an array")
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(names.contains(&"README.md"), "{names:?}");
    assert!(names.contains(&"src"), "{names:?}");
}

#[tokio::test]
async fn a_large_directory_pages_in_name_order_with_its_total_and_next_offset() {
    let fixture = fixture().await;
    let big = fixture._project_root.path().join("big");
    std::fs::create_dir(&big).expect("big dir");
    // Written in reverse, so filesystem order is not the order the pages use.
    for index in (0..1_500).rev() {
        std::fs::write(big.join(format!("f{index:04}.txt")), b"x").expect("seed");
    }
    let budget = crate::tools::ToolResultBudget::floor();

    let mut seen = Vec::new();
    let mut offset = 0;
    let mut pages = 0;
    loop {
        let result = call(
            &fixture,
            "project.list_directory",
            json!({ "path": "big", "offset": offset }),
        )
        .await
        .expect("list");
        let page = structured(&result);
        assert_eq!(page["total"], 1_500);
        assert_eq!(page["offset"], offset);
        // Every page arrives whole: it fits the budget it was sized to.
        assert!(result.model_preview(budget).complete());
        let names = page["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["name"].as_str().expect("name").to_owned())
            .collect::<Vec<_>>();
        assert!(!names.is_empty());
        seen.extend(names);
        pages += 1;
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert_eq!(next as usize, seen.len());
                assert!(page["detail"]
                    .as_str()
                    .expect("detail")
                    .contains(&next.to_string()));
                offset = next;
            }
            None => break,
        }
    }
    assert!(pages > 1, "1,500 entries do not fit one floor-budget page");
    let expected = (0..1_500)
        .map(|index| format!("f{index:04}.txt"))
        .collect::<Vec<_>>();
    assert_eq!(
        seen, expected,
        "pages continue one stable order, with nothing skipped"
    );
}

#[tokio::test]
async fn a_listing_limit_bounds_the_page_and_a_past_the_end_offset_is_refused() {
    let fixture = fixture().await;
    let limited = structured(
        &call(&fixture, "project.list_directory", json!({ "limit": 1 }))
            .await
            .expect("limited list"),
    );
    assert_eq!(limited["entries"].as_array().unwrap().len(), 1);
    assert_eq!(limited["total"], 2);
    assert_eq!(limited["next_offset"], 1);

    let error = call(&fixture, "project.list_directory", json!({ "offset": 9 }))
        .await
        .expect_err("an offset past the last entry");
    assert!(format!("{error:?}").contains("past the last entry"));
    let error = call(&fixture, "project.list_directory", json!({ "limit": 0 }))
        .await
        .expect_err("a zero limit");
    assert!(format!("{error:?}").contains("at least 1"));
}

#[tokio::test]
async fn pages_of_long_lines_stay_within_the_budget_and_cover_the_whole_file() {
    let fixture = fixture().await;
    // Each line escapes to about 20 KB — two of them would overflow a
    // floor-budget page, so a page that checked its budget after appending
    // (the old overshoot) would carry two.
    let line = "\"q\"\t".repeat(2_800);
    let text = format!("{line}\n").repeat(40);
    std::fs::write(fixture._project_root.path().join("wide.txt"), &text).expect("seed");
    let budget = crate::tools::ToolResultBudget::floor();

    let mut start = 1u64;
    let last_seen = loop {
        let result = call(
            &fixture,
            "project.read_file",
            json!({ "path": "wide.txt", "start_line": start }),
        )
        .await
        .expect("read");
        assert!(
            result.model_preview(budget).complete(),
            "a page is never cut on the way to the model"
        );
        let page = structured(&result);
        assert_eq!(page["first_line"], start);
        assert_eq!(page["truncated"], false, "no line was cut");
        let last_line = page["last_line"].as_u64().expect("last line");
        match page["next_line"].as_u64() {
            Some(next) => {
                assert_eq!(
                    next,
                    last_line + 1,
                    "the next page starts after the last line shown"
                );
                start = next;
            }
            None => break last_line,
        }
    };
    assert_eq!(last_seen, 40, "every line was reached");
}

#[tokio::test]
async fn a_line_too_long_for_a_whole_page_is_cut_marked_and_skipped_past() {
    let fixture = fixture().await;
    let line = "\"q\"\t".repeat(4_700);
    std::fs::write(
        fixture._project_root.path().join("huge-line.txt"),
        format!("{line}\nnext\n"),
    )
    .expect("seed");
    let budget = crate::tools::ToolResultBudget::floor();

    let result = call(
        &fixture,
        "project.read_file",
        json!({ "path": "huge-line.txt" }),
    )
    .await
    .expect("read");
    assert!(result.model_preview(budget).complete());
    let page = structured(&result);
    assert_eq!(page["truncated"], true);
    assert_eq!(page["last_line"], 1);
    assert_eq!(
        page["next_line"], 2,
        "the next page starts after the cut line"
    );
    assert!(page["detail"]
        .as_str()
        .expect("detail")
        .contains("were cut"));
    assert!(line.starts_with(page["content"].as_str().unwrap().trim_end_matches('\n')));
}

#[tokio::test]
async fn read_file_returns_text_content() {
    let fixture = fixture().await;

    let result = call(&fixture, "project.read_file", json!({"path": "README.md"}))
        .await
        .expect("read succeeds");

    let body = structured(&result);
    assert_eq!(body["kind"], "text");
    assert_eq!(body["content"], "hello world\n");
}

#[tokio::test]
async fn project_tools_search_and_page_the_same_extracted_office_text() {
    let fixture = fixture().await;
    fs::write(
        fixture._project_root.path().join("brief.docx"),
        PRODUCER_DOCX,
    )
    .expect("producer DOCX fixture");

    let search = call(
        &fixture,
        "project.search_text",
        json!({"query": "Referenced DOCX header"}),
    )
    .await
    .expect("Office search succeeds");
    let search_body = structured(&search);
    let matches = search_body["matches"].as_array().expect("matches");
    let matched = matches
        .iter()
        .find(|matched| matched["path"] == "brief.docx")
        .expect("the producer DOCX was searched");
    let line = matched["line"].as_u64().expect("line number");
    let read = call(
        &fixture,
        "project.read_file",
        json!({"path": "brief.docx", "start_line": line, "end_line": line}),
    )
    .await
    .expect("Office line read succeeds");
    let body = structured(&read);
    assert_eq!(body["kind"], "text");
    assert_eq!(body["first_line"], line);
    assert!(body["content"]
        .as_str()
        .unwrap()
        .contains("Referenced DOCX header"));
}

#[tokio::test]
async fn read_file_never_returns_binary_bytes_after_the_text_sniff() {
    let fixture = fixture().await;
    let content = format!(
        "{}binary\0payload\nmore plain text\n",
        "plain text\n".repeat(1_000)
    );
    std::fs::write(fixture._project_root.path().join("mixed.dat"), &content)
        .expect("binary bytes lie beyond the 8,000-byte sniff");

    for start in [1, 1_001, 1_002] {
        let result = call(
            &fixture,
            "project.read_file",
            json!({ "path": "mixed.dat", "start_line": start }),
        )
        .await
        .expect("binary reads return metadata");
        let body = structured(&result);
        assert_eq!(body["kind"], "metadata_only");
        assert_eq!(body["content_kind"], "binary");
        assert_eq!(body["reason"], "binary");
        assert!(body.get("content").is_none(), "no partial raw bytes leak");
        assert!(body["sha256"].as_str().is_some());
    }
}

#[tokio::test]
async fn a_traversal_path_never_escapes_the_project() {
    let fixture = fixture().await;

    let result = call(
        &fixture,
        "project.read_file",
        json!({"path": "../../etc/passwd"}),
    )
    .await;

    assert!(
        matches!(result, Err(ToolExecutionError::Failed(_))),
        "containment is enforced by TrustedRoot, and dispatch must not \
         weaken it: {result:?}"
    );
}

#[tokio::test]
async fn a_start_line_alone_reads_one_page_and_an_end_line_alone_is_refused() {
    let fixture = fixture().await;

    let page = call(
        &fixture,
        "project.read_file",
        json!({
            "path": "README.md",
            "start_line": 1,
        }),
    )
    .await
    .expect("a start line alone reads one page from there");
    assert_eq!(
        page.structured()
            .expect("a read returns a structured document")["kind"],
        json!("text")
    );

    let result = call(
        &fixture,
        "project.read_file",
        json!({
            "path": "README.md",
            "end_line": 4,
        }),
    )
    .await;
    assert!(
        matches!(result, Err(ToolExecutionError::Failed(_))),
        "an end line without a start is ambiguous rather than harmless: {result:?}"
    );
}

// ---------------------------------------------------------------------
// Writing project files
// ---------------------------------------------------------------------

#[tokio::test]
async fn propose_patch_writes_the_file_within_the_same_tool_call() {
    let fixture = fixture().await;
    let target = fixture._project_root.path().join("NEW.md");

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({
            "operations": [{
                "kind": "create",
                "path": "NEW.md",
                "content": "hi",
            }],
        }),
    )
    .await
    .expect("propose succeeds");

    let plan = structured(&result);
    assert!(plan["plan_id"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(plan["total_bytes"], 2);
    assert!(
        plan.get("unified_diff").is_none(),
        "the model wrote the content; echoing it back only costs tokens"
    );
    assert_eq!(
        plan["operations"][0],
        json!({ "index": 0, "kind": "create", "path": "NEW.md", "bytes": 2, "applied": true })
    );
    // Trusting the project was the consent, so the tool call that plans the
    // change is also the one that lands it -- and the model is told so, or it
    // reports a change as pending that already happened.
    assert_eq!(plan["applied"], true);
    assert_eq!(fs::read(&target).expect("written file"), b"hi");
}

#[tokio::test]
async fn a_patch_proposal_keeps_the_run_bound_account_after_ambient_account_switches() {
    let fixture = fixture().await;
    fixture
        .scope
        .bind_native("another-account")
        .expect("switch ambient account");

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({
            "operations": [{
                "kind": "create",
                "path": "FROZEN.md",
                "content": "run-bound",
            }],
        }),
    )
    .await;

    assert!(
        result.is_ok(),
        "proposal authority must stay with the run account: {result:?}"
    );
}

#[tokio::test]
async fn the_model_cannot_replace_whole_a_file_larger_than_it_can_replace_blind() {
    let fixture = fixture().await;
    let existing = "x".repeat(crate::projects::MAX_FILE_READ_BYTES as usize + 1);
    std::fs::write(fixture._project_root.path().join("big.txt"), &existing).expect("seed");

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({ "operations": [{ "kind": "modify", "path": "big.txt", "content": "tiny" }] }),
    )
    .await
    .expect_err("a blind whole-file replace is refused");
    let ToolExecutionError::Recoverable(message) = error else {
        panic!("the model can fix this by using anchored edits: {error:?}");
    };
    assert!(message.contains("big.txt") && message.contains("`replace` or `insert`"));
    assert_eq!(
        std::fs::read_to_string(fixture._project_root.path().join("big.txt")).unwrap(),
        existing,
        "nothing was written"
    );
}

#[tokio::test]
async fn propose_patch_rejects_content_that_is_not_text() {
    let fixture = fixture().await;

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({
            "operations": [{
                "kind": "create",
                "path": "NEW.md",
                "content": 42,
            }],
        }),
    )
    .await;

    assert!(matches!(result, Err(ToolExecutionError::Recoverable(_))));
}

// ---------------------------------------------------------------------
// What a run stores, and what the reader reads back
// ---------------------------------------------------------------------

/// The answer text a run would store for one live dispatch: the same wrapper
/// `TurnDriver::fold_outcomes` writes into the pairing `role="tool"` row.
fn stored_answer(result: ToolResult) -> String {
    render_tool_outcome(
        &ToolCallOutcome::Completed {
            invocation_id: InvocationId::from("invocation-step"),
            result,
        },
        crate::tools::ToolResultBudget::floor(),
    )
}

fn sorted_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// The writer half of the step-summary pin (spec: chat-tool-call-detail, "the
/// outcome shape is pinned").
///
/// The recorded fixture is what `crate::tools::tests::step_summary` reads. If a
/// dispatch arm renames a key, every step summary goes silently empty — the one
/// failure mode this data cannot report — so the live shape is compared against
/// the recording key for key, on both sides of the wrapper.
#[tokio::test]
async fn a_live_dispatch_still_writes_the_shapes_the_step_reader_expects() {
    let fixture = fixture().await;
    let cases = [
        (
            "project.list_directory",
            json!({}),
            recorded::LIST_DIRECTORY,
            Some("entries"),
        ),
        (
            "project.read_file",
            json!({"path": "README.md"}),
            recorded::READ_FILE,
            None,
        ),
        (
            "project.search_files",
            json!({"query": "README"}),
            recorded::SEARCH_FILES,
            None,
        ),
        (
            "project.search_text",
            json!({"query": "hello"}),
            recorded::SEARCH_TEXT,
            None,
        ),
        (
            "project.propose_patch",
            json!({"operations": [{
                "kind": "create",
                "path": "NEW.md",
                "content": "hi",
            }]}),
            recorded::PROPOSE_PATCH,
            Some("operations"),
        ),
    ];

    for (tool, arguments, recording, rows_key) in cases {
        let live: Value = serde_json::from_str(&stored_answer(
            call(&fixture, tool, arguments).await.expect("dispatch"),
        ))
        .expect("the stored answer is JSON");
        let recorded: Value = serde_json::from_str(recording).expect("the recording is JSON");

        assert_eq!(live["media_type"], "application/json");
        assert_eq!(live["serialization_format"], "json");
        assert_eq!(live["serialization_version"], 1);
        assert!(live.get("retrieval").is_some());
        let mut legacy_shape = live.clone();
        let wrapper = legacy_shape.as_object_mut().expect("wrapper object");
        for key in [
            "media_type",
            "serialization_format",
            "serialization_version",
            "retrieval",
        ] {
            wrapper.remove(key);
        }
        assert_eq!(
            sorted_keys(&legacy_shape),
            sorted_keys(&recorded),
            "{tool}: the wrapper's own shape moved"
        );
        assert_eq!(
            sorted_keys(&live["result"]),
            sorted_keys(&recorded["result"]),
            "{tool}: the outcome document's shape moved"
        );
        if let Some(key) = rows_key {
            assert_eq!(
                sorted_keys(&live["result"][key][0]),
                sorted_keys(&recorded["result"][key][0]),
                "{tool}: the shape of one {key} row moved"
            );
        }
    }
}

/// The join is the whole difference between naming the file the run touched and
/// naming a different one, so it is proved against a real listing of a real
/// subdirectory rather than a hand-written entry array.
#[tokio::test]
async fn a_live_listing_reads_back_as_a_step_naming_the_files_it_listed() {
    let fixture = fixture().await;
    std::fs::write(
        fixture._project_root.path().join("src/lib.rs"),
        b"fn main() {}\n",
    )
    .expect("seed a nested file");
    let arguments = json!({"path": "src"});

    let answer = stored_answer(
        call(&fixture, "project.list_directory", arguments.clone())
            .await
            .expect("list succeeds"),
    );

    // The wire alias is what a durable row carries; the reader must match the
    // name `wire_tool_name` actually produces, not the dotted id.
    let step = tool_step_summary(
        &wire_tool_name(&ToolId::from("project.list_directory")),
        &answer,
        Some(&arguments.to_string()),
    )
    .expect("a live listing summarizes");

    assert_eq!(step.label, "Listed 1 entry");
    assert_eq!(
        step.paths
            .iter()
            .map(|path| path.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/lib.rs"]
    );
}

/// The other four aliases, pinned the same way: the reader matches the name the
/// wire carries, and sanitization is not invertible.
#[tokio::test]
async fn every_internal_tool_the_reader_claims_is_reachable_by_its_wire_alias() {
    let fixture = fixture().await;
    for (tool, arguments) in [
        ("project.read_file", json!({"path": "README.md"})),
        ("project.search_files", json!({"query": "README"})),
        ("project.search_text", json!({"query": "hello"})),
        (
            "project.propose_patch",
            json!({"operations": [{"kind": "delete", "path": "README.md"}]}),
        ),
    ] {
        let answer = stored_answer(call(&fixture, tool, arguments).await.expect("dispatch"));
        assert!(
            tool_step_summary(&wire_tool_name(&ToolId::from(tool)), &answer, None).is_some(),
            "{tool} produced no step summary from its own live outcome"
        );
    }
}

// ---------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_already_cancelled_run_does_no_work() {
    let fixture = fixture().await;
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = fixture
        .dispatcher
        .execute(&ToolId::from("project.list_directory"), &json!({}), cancel)
        .await;

    assert!(matches!(result, Err(ToolExecutionError::Cancelled)));
}

// ---------------------------------------------------------------------
// Anchored edits over the wire
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_replace_operation_edits_a_file_without_re_sending_it() {
    let fixture = fixture().await;

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "replace",
            "path": "README.md",
            "edits": [{"old_string": "hello world", "new_string": "goodbye world"}],
        }]}),
    )
    .await
    .expect("an anchored edit applies");

    let document = structured(&result);
    assert_eq!(document["applied"], json!(true));
    assert_eq!(document["operations"][0]["path"], "README.md");
    assert_eq!(
        fs::read_to_string(fixture._project_root.path().join("README.md")).expect("read back"),
        "goodbye world\n"
    );
}

#[tokio::test]
async fn an_unmatched_anchor_tells_the_caller_what_to_do_instead_of_failing_blankly() {
    let fixture = fixture().await;

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "replace",
            "path": "README.md",
            "edits": [{"old_string": "not in the file", "new_string": "x"}],
        }]}),
    )
    .await
    .expect_err("an unmatched anchor must not write");

    let ToolExecutionError::Recoverable(message) = error else {
        panic!("expected a failed tool call");
    };
    // This message is what the model reads and retries from, so the guidance
    // has to survive the trip out through every error wrapper.
    assert!(message.contains("edit 1"), "{message}");
    assert!(message.contains("must match the file exactly"), "{message}");
    assert!(message.contains("Read the file again"), "{message}");
}

#[tokio::test]
async fn an_ambiguous_anchor_reports_the_match_count_and_names_replace_all() {
    let fixture = fixture().await;
    fs::write(
        fixture._project_root.path().join("repeats.txt"),
        b"same\nsame\n",
    )
    .expect("seed file");

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "replace",
            "path": "repeats.txt",
            "edits": [{"old_string": "same", "new_string": "other"}],
        }]}),
    )
    .await
    .expect_err("an ambiguous anchor must not write");

    let ToolExecutionError::Recoverable(message) = error else {
        panic!("expected a failed tool call");
    };
    assert!(message.contains("matched 2 places"), "{message}");
    assert!(message.contains("replace_all"), "{message}");
}

#[tokio::test]
async fn replace_all_changes_every_occurrence_when_asked() {
    let fixture = fixture().await;
    fs::write(
        fixture._project_root.path().join("repeats.txt"),
        b"same\nsame\n",
    )
    .expect("seed file");

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "replace",
            "path": "repeats.txt",
            "edits": [{"old_string": "same", "new_string": "other", "replace_all": true}],
        }]}),
    )
    .await
    .expect("replace_all applies");

    assert_eq!(structured(&result)["applied"], json!(true));
    assert_eq!(
        fs::read(fixture._project_root.path().join("repeats.txt")).expect("read back"),
        b"other\nother\n"
    );
}

#[tokio::test]
async fn an_insert_operation_adds_lines_after_a_line_number() {
    let fixture = fixture().await;

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "insert",
            "path": "README.md",
            "insert_line": 0,
            "content": "# Title",
        }]}),
    )
    .await
    .expect("an insert applies");

    assert_eq!(structured(&result)["applied"], json!(true));
    assert_eq!(
        fs::read(fixture._project_root.path().join("README.md")).expect("read back"),
        b"# Title\nhello world\n"
    );
}

#[tokio::test]
async fn an_insert_without_a_line_is_refused_rather_than_defaulted_to_the_top() {
    let fixture = fixture().await;

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "insert",
            "path": "README.md",
            "content": "# Title",
        }]}),
    )
    .await
    .expect_err("a missing insert point must not be guessed");

    let ToolExecutionError::Recoverable(message) = error else {
        panic!("expected a failed tool call");
    };
    assert!(message.contains("insert_line"), "{message}");
}

#[tokio::test]
async fn creating_a_file_in_a_directory_that_does_not_exist_yet_writes_it() {
    let fixture = fixture().await;

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "create",
            "path": "src/feature/new/mod.rs",
            "content": "pub fn go() {}\n",
        }]}),
    )
    .await
    .expect("scaffolding into a new directory is an ordinary create");

    assert_eq!(structured(&result)["applied"], json!(true));
    assert_eq!(
        fs::read(fixture._project_root.path().join("src/feature/new/mod.rs")).expect("read back"),
        b"pub fn go() {}\n"
    );
}

#[tokio::test]
async fn a_file_too_long_to_return_whole_is_fully_readable_one_page_at_a_time() {
    let fixture = fixture().await;
    // Comfortably past `MAX_FILE_READ_BYTES`, so no single call can carry it.
    const LINES: usize = 6_000;
    let padding = "x".repeat(190);
    let lines: String = (1..=LINES)
        .map(|index| format!("line {index} {padding}\n"))
        .collect();
    fs::write(fixture._project_root.path().join("long.txt"), &lines).expect("seed file");

    let mut start_line = 1_u64;
    let mut pages = 0;
    let mut collected = String::new();
    loop {
        let arguments = if start_line == 1 {
            json!({"path": "long.txt"})
        } else {
            json!({"path": "long.txt", "start_line": start_line})
        };
        let page = structured(
            &call(&fixture, "project.read_file", arguments)
                .await
                .expect("every page reads"),
        );
        pages += 1;
        assert_eq!(page["kind"], json!("text"));
        assert_eq!(
            page["first_line"],
            json!(start_line),
            "a page starts where the previous one said to continue"
        );
        collected.push_str(page["content"].as_str().expect("page content"));

        match page["next_line"].as_u64() {
            Some(next) => {
                assert_eq!(
                    next,
                    page["last_line"].as_u64().expect("a last line") + 1,
                    "the cursor is the line after the page"
                );
                // The result says what to call next, so the bound is a page
                // rather than a wall.
                assert!(page["detail"]
                    .as_str()
                    .expect("a detail sentence")
                    .contains(&format!("start_line {next}")));
                start_line = next;
            }
            None => {
                assert_eq!(
                    page["last_line"],
                    json!(LINES as u64),
                    "the last page reaches the end of the file"
                );
                break;
            }
        }
    }

    assert!(pages > 1, "a file this long takes more than one read");
    assert_eq!(collected, lines, "paging reconstructs the file exactly");
}

#[tokio::test]
async fn a_maximal_page_still_reaches_the_caller_as_a_structured_result() {
    let fixture = fixture().await;
    // Worst case for JSON escaping: every byte of content needs two. A page
    // that cannot fit in a tool result would be dropped whole rather than
    // truncated, so the read bound has to leave room for this.
    let lines: String = std::iter::repeat_n("\"".repeat(200), 4_000)
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(fixture._project_root.path().join("quoted.txt"), &lines).expect("seed file");

    let result = call(&fixture, "project.read_file", json!({"path": "quoted.txt"}))
        .await
        .expect("a page reads");

    assert!(
        result.structured().is_some(),
        "a page must never be so large that the whole document is dropped"
    );
    let document = structured(&result);
    assert_eq!(document["kind"], json!("text"));
    assert!(document["next_line"].as_u64().is_some());
}

// ---------------------------------------------------------------------
// What the model is told when it writes
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_credential_file_cannot_be_written_renamed_or_deleted_by_the_model() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::write(root.join(".env"), b"TOKEN=abc\n").expect("seed");
    fs::create_dir(root.join(".git")).expect("git dir");

    for operation in [
        json!({"kind": "modify", "path": ".env", "content": "x"}),
        json!({"kind": "delete", "path": ".env"}),
        json!({"kind": "rename", "from": ".env", "to": "env.txt"}),
        json!({"kind": "create", "path": ".git/hooks.txt", "content": "x"}),
        json!({"kind": "create", "path": ".xfalcon/skills/evil.md", "content": "x"}),
        json!({"kind": "create", "path": "keys/server.pem", "content": "x"}),
    ] {
        let error = call(
            &fixture,
            "project.propose_patch",
            json!({ "operations": [operation.clone()] }),
        )
        .await
        .expect_err("a protected path is refused");
        let ToolExecutionError::Failed(message) = error else {
            panic!("a refusal is final, not something to retry: {operation}");
        };
        assert!(
            message.contains("cannot change it"),
            "{operation}: {message}"
        );
    }
    assert_eq!(
        fs::read_to_string(root.join(".env")).expect("untouched"),
        "TOKEN=abc\n"
    );
    assert!(!root.join("env.txt").exists());
}

#[tokio::test]
async fn an_env_template_is_an_ordinary_file_to_write() {
    let fixture = fixture().await;

    call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{"kind": "create", "path": ".env.example", "content": "TOKEN=\n"}]}),
    )
    .await
    .expect("a template holds placeholders, not secrets");

    assert!(fixture._project_root.path().join(".env.example").exists());
}

#[tokio::test]
async fn a_file_one_read_cannot_return_whole_cannot_be_replaced_blind() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    // Under every old limit, over what one 32 KiB read returns.
    fs::write(root.join("page.html"), "<p>line</p>\n".repeat(4_000)).expect("seed");

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{"kind": "modify", "path": "page.html", "content": "<p>x</p>"}]}),
    )
    .await
    .expect_err("the model has seen only the first page of this file");
    assert!(matches!(error, ToolExecutionError::Recoverable(_)));

    // A file one read does return whole is fair game.
    fs::write(root.join("small.html"), "<p>old</p>\n").expect("seed");
    call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{"kind": "modify", "path": "small.html", "content": "<p>new</p>\n"}]}),
    )
    .await
    .expect("the model could read all of it");
}

#[tokio::test]
async fn several_bad_operations_come_back_together_and_nothing_is_written() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::write(root.join("a.html"), "old").expect("seed");

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [
            {"kind": "modify", "path": "a.html", "content": "new"},
            {"kind": "modify", "path": "missing-1.html", "content": "x"},
            {"kind": "modify", "path": "missing-2.html", "content": "x"},
        ]}),
    )
    .await
    .expect_err("two bad operations");

    let ToolExecutionError::Recoverable(message) = error else {
        panic!("the model can fix this: {error:?}");
    };
    assert!(
        message.contains("missing-1.html") && message.contains("missing-2.html"),
        "{message}"
    );
    assert_eq!(fs::read_to_string(root.join("a.html")).unwrap(), "old");
}

#[tokio::test]
async fn a_retry_after_a_partial_write_skips_the_files_that_already_landed() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::write(root.join("done.html"), "<p>new</p>").expect("already landed");
    fs::write(root.join("todo.html"), "<p>old</p>").expect("not yet");

    let result = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [
            {"kind": "modify", "path": "done.html", "content": "<p>new</p>"},
            {"kind": "modify", "path": "todo.html", "content": "<p>new</p>"},
        ]}),
    )
    .await
    .expect("the resent plan applies what is left");

    let document = structured(&result);
    assert_eq!(document["unchanged"], json!(["done.html"]));
    assert_eq!(document["operations"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["operations"][0]["path"], "todo.html");
    assert_eq!(
        fs::read_to_string(root.join("todo.html")).unwrap(),
        "<p>new</p>"
    );
}

#[test]
fn a_partly_applied_plan_names_what_landed_and_what_did_not() {
    use crate::projects::{
        EditOperation, MutationPlan, ProjectId, RelativeProjectPath, TrustedRoot, WriterError,
        WriterExecutionFailure,
    };
    let dir = tempfile::tempdir().expect("root");
    let root = TrustedRoot::establish(ProjectId::new(), dir.path()).expect("root");
    let create = |path: &str| EditOperation::Create {
        path: RelativeProjectPath::new(path).expect("path"),
        content: b"x".to_vec(),
    };
    let plan = MutationPlan::propose(
        &root,
        vec![create("a.html"), create("b.html"), create("c.html")],
    )
    .expect("plan");

    let error = crate::tools::dispatch::apply_failure(
        &plan,
        crate::approvals::ApprovalsServiceError::Execution(WriterExecutionFailure {
            error: WriterError::Locked(std::io::Error::other("held by another program")),
            operations_completed: 1,
            operations_total: 3,
            durability_unconfirmed: false,
        }),
    );

    let ToolExecutionError::Recoverable(message) = error else {
        panic!("the model can carry on from this: {error:?}");
    };
    assert!(
        message.contains("Already applied and on disk: a.html (create)"),
        "{message}"
    );
    assert!(
        message.contains("Not applied: b.html (create); c.html (create)"),
        "{message}"
    );
    assert!(message.contains("held by another program"), "{message}");
}

#[tokio::test]
async fn rewriting_a_windows_file_keeps_it_a_windows_file() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::write(root.join("page.html"), "<h1>Old</h1>\r\n<p>text</p>\r\n").expect("seed");

    // A whole-file rewrite: the model sends bare `\n`.
    call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "modify", "path": "page.html",
            "content": "<h1>New</h1>\n<p>text</p>\n<p>more</p>\n",
        }]}),
    )
    .await
    .expect("rewrite");
    assert_eq!(
        fs::read(root.join("page.html")).unwrap(),
        b"<h1>New</h1>\r\n<p>text</p>\r\n<p>more</p>\r\n"
    );

    // A one-line anchor with a multi-line replacement.
    call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{
            "kind": "replace", "path": "page.html",
            "edits": [{"old_string": "<p>more</p>", "new_string": "<p>one</p>\n<p>two</p>"}],
        }]}),
    )
    .await
    .expect("edit");
    assert_eq!(
        fs::read(root.join("page.html")).unwrap(),
        b"<h1>New</h1>\r\n<p>text</p>\r\n<p>one</p>\r\n<p>two</p>\r\n"
    );
}

#[tokio::test]
async fn a_unix_file_and_a_mixed_file_are_left_exactly_as_the_model_wrote_them() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::write(root.join("unix.html"), "a\nb\n").expect("seed");
    fs::write(root.join("mixed.html"), "a\r\nb\n").expect("seed");

    for name in ["unix.html", "mixed.html"] {
        call(
            &fixture,
            "project.propose_patch",
            json!({"operations": [{"kind": "modify", "path": name, "content": "x\ny\n"}]}),
        )
        .await
        .expect("rewrite");
        assert_eq!(fs::read(root.join(name)).unwrap(), b"x\ny\n", "{name}");
    }
}

// -- Search notices ---------------------------------------------------------

#[test]
fn a_complete_search_carries_no_notice() {
    assert_eq!(
        crate::tools::dispatch::search_detail(false, false, 3, "matches"),
        None
    );
}

#[test]
fn a_capped_search_says_the_list_is_not_complete_and_what_to_do() {
    let detail = crate::tools::dispatch::search_detail(true, false, 200, "matches")
        .expect("a capped search is explained");

    assert!(detail.contains("first 200 matches"), "{detail}");
    assert!(detail.contains("NOT complete"), "{detail}");
    assert!(detail.contains("Narrow the query"), "{detail}");
}

#[test]
fn a_search_over_a_partial_index_says_a_missing_result_proves_nothing() {
    let detail = crate::tools::dispatch::search_detail(false, true, 4, "files")
        .expect("a partial index is explained");

    assert!(
        detail.contains("more files than the search index"),
        "{detail}"
    );
    assert!(detail.contains("Do not treat a missing result"), "{detail}");
}

#[test]
fn a_search_that_is_capped_over_a_partial_index_says_both() {
    let detail = crate::tools::dispatch::search_detail(true, true, 200, "matches")
        .expect("both are explained");

    assert!(detail.contains("NOT complete"), "{detail}");
    assert!(detail.contains("search index"), "{detail}");
}

#[tokio::test]
async fn a_protected_directory_is_protected_however_deep_and_however_it_is_renamed() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::create_dir_all(root.join("vendor/lib/.git/hooks")).expect("nested repository");
    fs::create_dir(root.join(".ssh")).expect("ssh dir");
    fs::write(root.join(".ssh/config"), b"Host x\n").expect("seed");

    for operation in [
        json!({"kind": "create", "path": "vendor/lib/.git/hooks/pre-commit", "content": "x"}),
        json!({"kind": "rename_directory", "from": ".ssh", "to": "plain"}),
        json!({"kind": "delete_directory", "path": ".ssh"}),
    ] {
        let error = call(
            &fixture,
            "project.propose_patch",
            json!({ "operations": [operation.clone()] }),
        )
        .await
        .expect_err("a protected path is refused");
        let ToolExecutionError::Failed(message) = error else {
            panic!("a refusal is final: {operation}");
        };
        assert!(
            message.contains("cannot change it"),
            "{operation}: {message}"
        );
    }
    assert!(root.join(".ssh/config").exists());
    assert!(!root.join("plain").exists());
}

#[tokio::test]
async fn a_file_whose_escaped_size_passes_a_page_is_not_replaced_blind_even_when_its_raw_size_fits()
{
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    // About 24 KB raw, about 38 KB once every quote and newline is escaped: over
    // what one 32 KiB read carries, though under it in raw bytes.
    fs::write(root.join("data.json"), "\"a\":\"b\",\n".repeat(2_700)).expect("seed");

    let error = call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [{"kind": "modify", "path": "data.json", "content": "{}"}]}),
    )
    .await
    .expect_err("one read returns only the first page of this file");

    assert!(matches!(error, ToolExecutionError::Recoverable(_)));
}

#[tokio::test]
async fn an_insert_and_a_create_sent_again_are_reported_not_failed() {
    let fixture = fixture().await;
    let root = fixture._project_root.path();
    fs::write(root.join("a.html"), "<p>a</p>\n").expect("landed earlier");

    call(
        &fixture,
        "project.propose_patch",
        json!({"operations": [
            {"kind": "create", "path": "a.html", "content": "<p>a</p>\n"},
            {"kind": "create", "path": "b.html", "content": "<p>b</p>\n"},
        ]}),
    )
    .await
    .expect("the file that landed is skipped, the other is written");

    assert!(root.join("b.html").exists());
}

#[test]
fn a_flush_that_failed_part_way_does_not_claim_the_rest_landed() {
    use crate::projects::{
        EditOperation, MutationPlan, ProjectId, RelativeProjectPath, TrustedRoot, WriterError,
        WriterExecutionFailure,
    };
    let dir = tempfile::tempdir().expect("root");
    let root = TrustedRoot::establish(ProjectId::new(), dir.path()).expect("root");
    let plan = MutationPlan::propose(
        &root,
        vec![
            EditOperation::Create {
                path: RelativeProjectPath::new("a.html").expect("path"),
                content: b"a".to_vec(),
            },
            EditOperation::Create {
                path: RelativeProjectPath::new("b.html").expect("path"),
                content: b"b".to_vec(),
            },
        ],
    )
    .expect("plan");

    // The writer stops at the first flush it cannot confirm.
    let error = crate::tools::dispatch::apply_failure(
        &plan,
        crate::approvals::ApprovalsServiceError::Execution(WriterExecutionFailure {
            error: WriterError::Locked(std::io::Error::other("flush failed")),
            operations_completed: 1,
            operations_total: 2,
            durability_unconfirmed: true,
        }),
    );

    let ToolExecutionError::Recoverable(message) = error else {
        panic!("the second operation can still be sent: {error:?}");
    };
    assert!(
        !message.contains("Every operation was written"),
        "{message}"
    );
    assert!(message.contains("Not applied: b.html"), "{message}");
}

#[test]
fn a_write_whose_flush_could_not_be_confirmed_says_everything_landed() {
    use crate::projects::{
        EditOperation, MutationPlan, ProjectId, RelativeProjectPath, TrustedRoot, WriterError,
        WriterExecutionFailure,
    };
    let dir = tempfile::tempdir().expect("root");
    let root = TrustedRoot::establish(ProjectId::new(), dir.path()).expect("root");
    let plan = MutationPlan::propose(
        &root,
        vec![EditOperation::Create {
            path: RelativeProjectPath::new("a.html").expect("path"),
            content: b"x".to_vec(),
        }],
    )
    .expect("plan");

    let error = crate::tools::dispatch::apply_failure(
        &plan,
        crate::approvals::ApprovalsServiceError::Execution(WriterExecutionFailure {
            error: WriterError::Locked(std::io::Error::other("flush failed")),
            operations_completed: 1,
            operations_total: 1,
            durability_unconfirmed: true,
        }),
    );

    let ToolExecutionError::Failed(message) = error else {
        panic!("there is nothing left to retry: {error:?}");
    };
    assert!(message.contains("Every operation was written"), "{message}");
    assert!(message.contains("Do not resend"), "{message}");
}
