//! The reader half of the step-summary pin (spec: chat-tool-call-detail).
//!
//! Every case here feeds `tool_step_summary` the ANSWER TEXT a run stores —
//! `crate::tools::dispatch::recorded_answers` — because that is the only input
//! production ever hands it. The sibling `dispatch` tests assert a live dispatch
//! still writes those shapes; together the two make a rename on either side fail
//! a test instead of silently emptying every step summary.

use serde_json::json;
use xfalcon_contracts::chat::TOOL_STEP_PATH_CAP;

use crate::tools::dispatch::{recorded_answers as recorded, tool_step_summary};

const LIST_DIRECTORY: &str = "project_list_directory";
const READ_FILE: &str = "project_read_file";
const SEARCH_FILES: &str = "project_search_files";
const SEARCH_TEXT: &str = "project_search_text";
const PROPOSE_PATCH: &str = "project_propose_patch";

/// One listing answer with `count` entries, none sensitive.
fn listing_of(count: usize) -> String {
    let entries: Vec<_> = (0..count)
        .map(|index| {
            json!({
                "name": format!("entry-{index}.ts"),
                "kind": "file",
                "size_bytes": 12,
                "is_sensitive": false,
            })
        })
        .collect();
    json!({
        "status": "completed",
        "result": { "entries": entries },
        "truncated": false,
        "content_refs": [],
    })
    .to_string()
}

// ---------------------------------------------------------------------
// The wrapper is unwrapped first
// ---------------------------------------------------------------------

/// The regression guard for the one defect here whose symptom is
/// indistinguishable from correct behaviour: a reader bound to the wrapper's top
/// level matches nothing on every call, and `None` is also its contracted answer
/// for a tool it does not recognize.
#[test]
fn keys_sitting_at_the_wrapper_top_level_yield_nothing() {
    let unwrapped =
        r#"{"entries":[{"name":"cards.tsx","kind":"file","size_bytes":1,"is_sensitive":false}]}"#;
    assert_eq!(tool_step_summary(LIST_DIRECTORY, unwrapped, None), None);

    let hoisted = r#"{"status":"completed","entries":[{"name":"cards.tsx","kind":"file","size_bytes":1,"is_sensitive":false}]}"#;
    assert_eq!(
        tool_step_summary(LIST_DIRECTORY, hoisted, None),
        None,
        "the tool's document lives under `result`, never beside `status`"
    );
}

#[test]
fn an_outcome_dropped_before_it_was_stored_reports_no_count_at_all() {
    assert_eq!(
        tool_step_summary(LIST_DIRECTORY, recorded::RESULT_DROPPED_FOR_SIZE, None),
        None,
        "\"Listed 0 entries\" for a listing whose entries were dropped for size \
         states the opposite of what happened"
    );
}

#[test]
fn an_incomplete_current_preview_never_reports_a_partial_count_as_exact() {
    let answer = json!({
        "status": "completed",
        "result": { "entries": [{
            "name": "first-only.ts",
            "kind": "file",
            "size_bytes": 12,
            "is_sensitive": false,
        }] },
        "preview_complete": false,
        "preview_strategy": "json_structure",
        "exact_byte_length": 131_072,
        "sha256": "fixture",
        "source_truncated": false,
        "content_refs": [],
    })
    .to_string();

    assert_eq!(tool_step_summary(LIST_DIRECTORY, &answer, None), None);
}

#[test]
fn an_answer_that_is_not_json_at_all_says_nothing() {
    assert_eq!(tool_step_summary(LIST_DIRECTORY, "not json", None), None);
    assert_eq!(tool_step_summary(LIST_DIRECTORY, "", None), None);
}

#[test]
fn a_status_this_app_never_writes_says_nothing() {
    let unknown = r#"{"status":"weird","result":{"entries":[]}}"#;
    assert_eq!(tool_step_summary(LIST_DIRECTORY, unknown, None), None);
}

// ---------------------------------------------------------------------
// Only tools this app executed
// ---------------------------------------------------------------------

#[test]
fn an_mcp_call_carries_no_step_however_readable_its_answer_looks() {
    // Same wrapper, same keys, a server's authorship. A chip here would put a
    // server's claim about the filesystem into this app's own voice.
    assert_eq!(
        tool_step_summary("mcp_tenant_ida_query", recorded::LIST_DIRECTORY, None),
        None
    );
    assert_eq!(
        tool_step_summary("mcp_tenant_ida_query", recorded::DENIED, None),
        None,
        "every tool is refused through the same wrapper; a name this app cannot \
         place is a row it cannot describe"
    );
}

#[test]
fn an_internal_tool_with_no_typed_outcome_carries_no_step() {
    assert_eq!(
        tool_step_summary("__internal__get_skill", recorded::READ_FILE, None),
        None
    );
}

// ---------------------------------------------------------------------
// Answered rows with no outcome behind them
// ---------------------------------------------------------------------

#[test]
fn a_denied_call_is_labelled_as_denied_and_names_no_path() {
    let step = tool_step_summary(PROPOSE_PATCH, recorded::DENIED, None)
        .expect("a denied row is answered and must say so");
    assert_eq!(step.label, "Blocked — this call was not permitted");
    assert!(step.paths.is_empty());
    assert_eq!(step.path_total, 0);
}

#[test]
fn a_failures_own_message_appears_nowhere_in_the_label() {
    let step = tool_step_summary(SEARCH_FILES, recorded::FAILED, None).expect("failed row");
    assert_eq!(step.label, "Failed — the tool did not finish");
    for fragment in ["index", "unavailable", "project"] {
        assert!(
            !step.label.contains(fragment),
            "the wrapper's own error string must not reach the label: {}",
            step.label
        );
    }
}

#[test]
fn the_remaining_unfinished_statuses_are_named_from_fixed_vocabulary() {
    for (status, expected) in [
        (
            "invalid_arguments",
            "Rejected — the arguments did not match this tool",
        ),
        ("cancelled", "Cancelled"),
        ("awaiting_approval", "Waiting for a person to approve this"),
    ] {
        let answer = json!({ "status": status }).to_string();
        let step = tool_step_summary(READ_FILE, &answer, None)
            .unwrap_or_else(|| panic!("{status} is an answered row"));
        assert_eq!(step.label, expected);
        assert!(step.paths.is_empty());
    }
}

// ---------------------------------------------------------------------
// Directory listings
// ---------------------------------------------------------------------

#[test]
fn an_entry_is_joined_onto_the_directory_that_held_it() {
    let step = tool_step_summary(
        LIST_DIRECTORY,
        recorded::LIST_DIRECTORY,
        Some(recorded::LIST_DIRECTORY_ARGUMENTS),
    )
    .expect("a listing summarizes");

    assert_eq!(step.label, "Listed 2 entries");
    assert_eq!(step.path_total, 2);
    assert_eq!(
        step.paths
            .iter()
            .map(|path| path.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/features/cards.tsx", "src/features/id_ed25519"],
        "a bare name would name a file at the project root the run never touched"
    );
    assert!(
        step.paths[1].is_sensitive,
        "the classifier's verdict travels with the path"
    );
    assert!(!step.paths[0].is_sensitive);
}

#[test]
fn a_paged_listing_says_how_large_the_directory_is() {
    let step = tool_step_summary(LIST_DIRECTORY, recorded::LIST_DIRECTORY_PAGED, None)
        .expect("a paged listing summarizes");
    assert_eq!(step.label, "Listed 2 of 5000 entries");
    assert_eq!(step.path_total, 2);
}

#[test]
fn a_listing_stored_before_paging_still_reads() {
    let step = tool_step_summary(
        LIST_DIRECTORY,
        recorded::LIST_DIRECTORY_UNPAGED,
        Some(recorded::LIST_DIRECTORY_ARGUMENTS),
    )
    .expect("the historical shape summarizes");
    assert_eq!(step.label, "Listed 2 entries");
    assert_eq!(step.path_total, 2);
}

#[test]
fn a_root_listing_needs_no_join() {
    for arguments in [
        None,
        Some(recorded::LIST_DIRECTORY_ROOT_ARGUMENTS),
        Some(r#"{"path":""}"#),
    ] {
        let step = tool_step_summary(LIST_DIRECTORY, recorded::LIST_DIRECTORY, arguments)
            .expect("a root listing summarizes");
        assert_eq!(
            step.paths
                .iter()
                .map(|path| path.path.as_str())
                .collect::<Vec<_>>(),
            vec!["cards.tsx", "id_ed25519"],
            "at the root the entry name already IS the relative path"
        );
    }
}

#[test]
fn an_unusable_directory_argument_costs_the_chips_not_the_step() {
    for arguments in [r#"{"path":"../../etc"}"#, r#"{"path":"/etc"}"#, "not json"] {
        let step = tool_step_summary(LIST_DIRECTORY, recorded::LIST_DIRECTORY, Some(arguments))
            .expect("the step still reports what the outcome held");
        assert_eq!(step.label, "Listed 2 entries");
        assert!(
            step.paths.is_empty(),
            "chips rooted at a guess — or bare names — are the same wrong answer"
        );
        assert_eq!(step.path_total, 2);
    }
}

#[test]
fn a_traversal_shaped_entry_is_dropped_while_the_total_stands() {
    let hostile = json!({
        "status": "completed",
        "result": { "entries": [
            { "name": "cards.tsx", "kind": "file", "size_bytes": 1, "is_sensitive": false },
            { "name": "../../etc/passwd", "kind": "file", "size_bytes": 1, "is_sensitive": false },
        ] },
    })
    .to_string();

    let step = tool_step_summary(LIST_DIRECTORY, &hostile, Some(r#"{"path":"src"}"#))
        .expect("one bad entry must not cost the step");
    assert_eq!(
        step.paths
            .iter()
            .map(|path| path.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/cards.tsx"]
    );
    assert_eq!(
        step.path_total, 2,
        "a dropped path lands in the shown-versus-held difference, not in the total"
    );
}

#[test]
fn sensitivity_defaults_to_marked_when_the_flag_cannot_be_read() {
    // Both writing arms always emit the flag, so an unreadable one means the
    // shape moved — and an unmarked withheld candidate is the one presentation
    // this data must never produce.
    let missing = r#"{"status":"completed","result":{"entries":[{"name":".env","kind":"file","size_bytes":1}]}}"#;
    let step = tool_step_summary(LIST_DIRECTORY, missing, None).expect("listing summarizes");
    assert!(step.paths[0].is_sensitive);
}

#[test]
fn a_listing_past_the_display_budget_shows_twelve_and_reports_four_hundred() {
    let step = tool_step_summary(LIST_DIRECTORY, &listing_of(400), None).expect("listing");
    assert_eq!(step.label, "Listed 400 entries");
    assert_eq!(step.paths.len(), TOOL_STEP_PATH_CAP);
    assert_eq!(step.path_total, 400);
    assert_eq!(
        step.paths.first().map(|path| path.path.as_str()),
        Some("entry-0.ts"),
        "the bound preserves outcome order rather than sampling"
    );
    assert_eq!(
        step.paths.last().map(|path| path.path.as_str()),
        Some("entry-11.ts")
    );
}

#[test]
fn a_listing_under_the_budget_reports_exactly_what_it_shows() {
    let step = tool_step_summary(LIST_DIRECTORY, &listing_of(3), None).expect("listing");
    assert_eq!(step.paths.len(), 3);
    assert_eq!(step.path_total, 3);
    assert_eq!(step.label, "Listed 3 entries");
}

#[test]
fn one_entry_reads_as_one_entry() {
    let step = tool_step_summary(LIST_DIRECTORY, &listing_of(1), None).expect("listing");
    assert_eq!(step.label, "Listed 1 entry");
}

// ---------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------

#[test]
fn a_read_names_the_one_file_it_read() {
    let step = tool_step_summary(READ_FILE, recorded::READ_FILE, None).expect("a read summarizes");
    assert_eq!(step.label, "Read");
    assert_eq!(step.path_total, 1);
    assert_eq!(step.paths[0].path, "README.md");
    assert!(!step.paths[0].is_sensitive);
}

#[test]
fn a_refused_sensitive_read_is_marked_and_says_a_person_must_approve_it() {
    let step = tool_step_summary(READ_FILE, recorded::READ_FILE_SENSITIVE, None)
        .expect("a refusal is an outcome worth naming");
    assert_eq!(
        step.label,
        "Blocked — a person must approve reading this file"
    );
    assert!(
        step.paths[0].is_sensitive,
        "a skipped candidate drawn as an ordinary hit is the one confusion this \
         data must never create"
    );
    assert_eq!(step.paths[0].path, ".env");
}

#[test]
fn a_partial_read_and_a_metadata_only_read_each_say_which() {
    let cut = r#"{"status":"completed","result":{"kind":"text","path":"big.log","content":"…","first_line":1,"last_line":900,"truncated":true}}"#;
    assert_eq!(
        tool_step_summary(READ_FILE, cut, None).expect("read").label,
        "Read · content cut short"
    );

    let metadata = r#"{"status":"completed","result":{"kind":"metadata_only","path":"logo.png","size_bytes":9001,"content_kind":"binary","sha256":null,"reason":"binary"}}"#;
    assert_eq!(
        tool_step_summary(READ_FILE, metadata, None)
            .expect("read")
            .label,
        "Read metadata only"
    );
}

#[test]
fn a_read_whose_path_fails_validation_loses_the_step_not_just_the_path() {
    assert_eq!(
        tool_step_summary(READ_FILE, recorded::READ_FILE_TRAVERSAL, None),
        None,
        "a bare \"Read\" naming nothing is a verb with no object"
    );
}

#[test]
fn a_read_kind_this_app_never_writes_says_nothing() {
    let unknown = r#"{"status":"completed","result":{"kind":"quantum","path":"README.md"}}"#;
    assert_eq!(tool_step_summary(READ_FILE, unknown, None), None);
}

// ---------------------------------------------------------------------
// Searches
// ---------------------------------------------------------------------

#[test]
fn a_file_search_names_its_hits_and_marks_the_sensitive_one() {
    let step = tool_step_summary(SEARCH_FILES, recorded::SEARCH_FILES, None).expect("search");
    assert_eq!(step.label, "Found 2 files");
    assert_eq!(step.path_total, 2);
    assert!(step.paths[1].is_sensitive);
}

#[test]
fn the_tools_own_caps_are_disclosed_separately_from_the_display_budget() {
    let step =
        tool_step_summary(SEARCH_FILES, recorded::SEARCH_FILES_CUT_SHORT, None).expect("search");
    assert_eq!(
        step.label, "Found 1 file · stopped early",
        "\"Found 1 file\" would read as \"there is 1\" when it means \"we stopped at 1\""
    );
    assert_eq!(step.paths.len(), 1);
    assert_eq!(step.path_total, 1);
}

#[test]
fn a_content_search_names_counts_and_never_the_query() {
    let step = tool_step_summary(
        SEARCH_TEXT,
        recorded::SEARCH_TEXT,
        Some(recorded::SEARCH_TEXT_ARGUMENTS),
    )
    .expect("search");

    assert_eq!(step.label, "Searched 128 files · 3 matches");
    for fragment in ["DROP", "users", "--"] {
        assert!(
            !step.label.contains(fragment),
            "a model must not be able to write this app's interface copy: {}",
            step.label
        );
    }
    assert_eq!(
        step.paths
            .iter()
            .map(|path| path.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/app.ts", "src/main.ts"],
        "twenty hits in one file is one file the run read"
    );
    assert_eq!(step.path_total, 2);
    assert!(
        step.paths.iter().all(|path| !path.is_sensitive),
        "content search skips a classified-sensitive candidate rather than matching in it"
    );
}

#[test]
fn a_content_search_that_matched_nothing_still_says_what_it_did() {
    let empty = r#"{"status":"completed","result":{"matches":[],"truncated":false,"index_truncated":false,"files_scanned":1,"files_stale":0}}"#;
    let step = tool_step_summary(SEARCH_TEXT, empty, None).expect("an empty search is an outcome");
    assert_eq!(step.label, "Searched 1 file · 0 matches");
    assert!(step.paths.is_empty());
    assert_eq!(step.path_total, 0);
}

// ---------------------------------------------------------------------
// Patch proposals
// ---------------------------------------------------------------------

#[test]
fn a_written_patch_names_every_file_it_touched() {
    let step = tool_step_summary(PROPOSE_PATCH, recorded::PROPOSE_PATCH, None).expect("plan");
    assert_eq!(step.label, "Changed 3 files");
    assert_eq!(step.path_total, 3);
    let paths: Vec<_> = step.paths.iter().map(|path| path.path.as_str()).collect();
    assert_eq!(paths, ["src/app.ts", "src/util.ts", "src/old.ts"]);
    assert!(step.paths.iter().all(|path| !path.is_sensitive));
}

#[test]
fn a_rename_is_named_by_where_the_file_went() {
    let answer = r#"{"status":"completed","result":{"applied":true,"total_paths":2,"operations":[{"index":0,"kind":"rename","from":"a.html","to":"docs/a.html","applied":true}]}}"#;
    let step = tool_step_summary(PROPOSE_PATCH, answer, None).expect("plan");
    assert_eq!(step.paths[0].path, "docs/a.html");
}

#[test]
fn a_patch_that_was_denied_says_it_changed_nothing() {
    let answer = r#"{"status":"completed","result":{"applied":false,"total_paths":1,"operations":[{"index":0,"kind":"modify","path":"a.html","bytes":3,"applied":false}]}}"#;
    let step = tool_step_summary(PROPOSE_PATCH, answer, None).expect("plan");
    assert_eq!(step.label, "Could not change 1 file");
}

/// Conversations stored before write results named their paths still open.
#[test]
fn an_answer_stored_before_paths_were_named_keeps_its_old_reading() {
    let step =
        tool_step_summary(PROPOSE_PATCH, recorded::LEGACY_PROPOSE_PATCH, None).expect("plan");
    assert_eq!(step.label, "Proposed changes to 3 files");
    assert_eq!(step.path_total, 3);
    assert!(step.paths.is_empty());
}
