//! Coverage for `super::super::search`: `search_files`'s four-tier filename
//! ranking and lexical tie-break, `search_text`'s case-sensitivity and
//! sensitive-file override gate, the `MAX_SEARCH_RESULTS`/
//! `MAX_SEARCH_FILES_SCANNED`/`MAX_MATCHES_PER_FILE` bounds, and
//! `validate_query`'s rejection of an empty or over-long query.
//!
//! Fixtures index a real temporary directory via `run_index_pass` (mirroring
//! `tests::index`'s own fixture shape) so `search_files`/`search_text` are
//! exercised against the same encrypted `IndexStore` a real caller uses,
//! rather than hand-crafting index rows.

use std::fs;
use std::sync::Arc;

use tempfile::{tempdir, TempDir};
use xfalcon_contracts::RunId;

use crate::persistence::{InMemorySecretStore, SecretStore};
use crate::projects::{
    evaluate, read_file, run_index_pass, search_files, search_text, CancellationSignal,
    FileSearchMode, IndexPolicy, IndexStore, LineRange, ProjectId, ReadFileOutcome,
    ReadFileRequest, RelativeProjectPath, SearchError, SearchMode, SecretClassifier,
    SensitiveDecision, SensitiveReadScope, TrustedRoot, MAX_MATCHES_PER_FILE,
    MAX_OFFICE_FILES_PER_SEARCH, MAX_SEARCH_FILES_SCANNED, MAX_SEARCH_QUERY_BYTES,
    MAX_SEARCH_RESULTS,
};
use crate::security::account_cleanup::ValidatedAccountId;

const ACCOUNT_ID: &str = "search-tests-account";
const PRODUCER_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.docx");

fn account() -> ValidatedAccountId {
    ValidatedAccountId::parse(ACCOUNT_ID).expect("valid account id")
}

fn secrets() -> Arc<InMemorySecretStore> {
    let secret_store = Arc::new(InMemorySecretStore::default());
    secret_store
        .put_key(ACCOUNT_ID, [0x99; 32])
        .expect("account key");
    secret_store
}

fn store(app_root: &std::path::Path) -> IndexStore {
    IndexStore::open(
        app_root.join("index.sqlite3"),
        secrets() as Arc<dyn SecretStore>,
    )
    .expect("open index store")
}

fn trusted_root(root: &std::path::Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), root).expect("establish trusted root")
}

fn classifier() -> SecretClassifier {
    SecretClassifier::built_in()
}

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

fn run_id() -> RunId {
    RunId::from("run-1")
}

/// Writes every `(relative_path, content)` pair under a fresh project
/// directory, indexes it, and returns everything the caller must keep alive
/// for the duration of the test.
fn index_fixture(files: &[(&str, &[u8])]) -> (TempDir, TrustedRoot, TempDir, IndexStore) {
    let project = tempdir().expect("project root");
    for (relative_path, content) in files {
        let path = project.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent dir");
        }
        fs::write(path, content).expect("fixture file");
    }
    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    run_index_pass(
        &root,
        &index_store,
        &account(),
        &CancellationSignal::default(),
        &classifier(),
        &IndexPolicy::default(),
    )
    .expect("index pass succeeds");
    (project, root, app_root, index_store)
}

#[test]
fn office_search_uses_shared_extraction_for_indexed_and_live_files_and_matches_read_lines() {
    let (_project, root, _app, index_store) = index_fixture(&[("brief.docx", PRODUCER_DOCX)]);
    let indexed = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "Unicode café",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("indexed Office search");
    let matched = indexed.matches.first().expect("indexed DOCX match");
    assert!(
        matched.line_number > 1,
        "the extraction limitations header precedes document text"
    );

    let path = relative("brief.docx");
    let read = read_file(
        &root,
        &classifier(),
        SensitiveReadScope::new(*root.project_id(), path.clone(), run_id()),
        ReadFileRequest {
            relative_path: path,
            line_range: Some(LineRange::new(matched.line_number, matched.line_number).unwrap()),
            sensitive_override: None,
            page_bytes: 4096,
        },
    )
    .expect("read the matched Office line");
    let ReadFileOutcome::Text(line) = read else {
        panic!("expected extracted text line");
    };
    assert_eq!(line.first_line, matched.line_number);
    assert!(line.content.contains(&matched.line_preview));

    let header_only_indexed = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "OCR-scanned",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("indexed search excludes generated header");
    assert!(header_only_indexed.matches.is_empty());

    let live_project = tempdir().expect("unindexed project");
    fs::write(live_project.path().join("live.docx"), PRODUCER_DOCX).expect("producer DOCX");
    let live_root = trusted_root(live_project.path());
    let app = tempdir().expect("index app");
    let empty_store = store(app.path());
    let live = search_text(
        &live_root,
        &empty_store,
        &classifier(),
        &account(),
        *live_root.project_id(),
        "Referenced DOCX header",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("live Office search before indexing");
    assert_eq!(live.matches[0].relative_path.as_str(), "live.docx");

    let header_only_live = search_text(
        &live_root,
        &empty_store,
        &classifier(),
        &account(),
        *live_root.project_id(),
        "OCR-scanned",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("live search excludes generated header");
    assert!(header_only_live.matches.is_empty());
}

#[test]
fn no_text_office_files_are_current_nonmatches_for_indexed_and_live_search() {
    let blank_legacy_ppt = crate::documents::synthetic_blank_legacy_ppt_fixture();
    let files: [(&str, &[u8]); 7] = [
        (
            "blank.doc",
            include_bytes!("../../../tests/fixtures/office/legacy-poi/empty.doc"),
        ),
        (
            "blank.docx",
            include_bytes!("../../../tests/fixtures/office/no-text.docx"),
        ),
        ("blank.ppt", &blank_legacy_ppt),
        (
            "blank.pptx",
            include_bytes!("../../../tests/fixtures/office/no-text.pptx"),
        ),
        (
            "image.docx",
            include_bytes!("../../../tests/fixtures/office/image-only.docx"),
        ),
        (
            "image.pptx",
            include_bytes!("../../../tests/fixtures/office/image-only.pptx"),
        ),
        ("corrupt.docx", b"not an OOXML package"),
    ];
    let (_project, root, _app, index_store) = index_fixture(&files);
    let indexed = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle-not-present",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("indexed search");
    assert!(indexed.matches.is_empty());
    assert_eq!(indexed.files_scanned, files.len());
    assert_eq!(indexed.files_stale, 1, "only the corrupt package is stale");

    let live_project = tempdir().expect("live project");
    for (relative_path, content) in files {
        fs::write(live_project.path().join(relative_path), content).expect("live fixture");
    }
    let live_root = trusted_root(live_project.path());
    let app_root = tempdir().expect("live index app");
    let empty_store = store(app_root.path());
    let live = search_text(
        &live_root,
        &empty_store,
        &classifier(),
        &account(),
        *live_root.project_id(),
        "needle-not-present",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("live search");
    assert!(live.matches.is_empty());
    assert_eq!(live.files_scanned, files.len());
    assert_eq!(live.files_stale, 1, "only the corrupt package is stale");
}

#[test]
fn office_search_applies_sensitivity_failure_and_per_search_file_bounds() {
    let files: Vec<(String, &[u8])> = (0..=MAX_OFFICE_FILES_PER_SEARCH)
        .map(|index| (format!("office-{index:02}.docx"), PRODUCER_DOCX))
        .collect();
    let fixture: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, bytes)| (path.as_str(), *bytes))
        .chain([(".env.docx", PRODUCER_DOCX), ("broken.docx", b"invalid")])
        .collect();
    let (_project, root, _app, index_store) = index_fixture(&fixture);
    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "Producer DOCX fixture",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("bounded Office search");
    assert!(
        outcome.truncated,
        "the per-search Office file cap must be visible"
    );
    assert_eq!(outcome.matches.len(), MAX_OFFICE_FILES_PER_SEARCH - 1);
    assert_eq!(
        outcome.files_stale, 1,
        "the malformed Office file is counted stale"
    );
    assert!(outcome
        .matches
        .iter()
        .all(|matched| matched.relative_path.as_str() != ".env.docx"));
}

// ── `search_files`: tier ranking and lexical tie-break ──────────────────

#[test]
fn search_files_ranks_exact_prefix_substring_and_path_tiers_in_order() {
    let (_project, root, _app, index_store) = index_fixture(&[
        ("readme", b"exact"),
        ("readme.txt", b"prefix"),
        ("old_readme.log", b"substring"),
        ("readme_dir/notes.txt", b"path-substring"),
        ("unrelated.txt", b"no match"),
    ]);

    let outcome = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "readme",
        FileSearchMode::Substring,
    )
    .expect("search succeeds");
    let paths: Vec<&str> = outcome
        .results
        .iter()
        .map(|result| result.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "readme",
            "readme.txt",
            "old_readme.log",
            "readme_dir/notes.txt",
        ]
    );
    assert!(!outcome.truncated);
}

#[test]
fn search_files_tie_break_is_lexical_path_within_the_same_tier() {
    let (_project, root, _app, index_store) = index_fixture(&[
        ("b_readme.log", b"x"),
        ("a_readme.log", b"x"),
        ("c_readme.log", b"x"),
    ]);

    let outcome = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "readme",
        FileSearchMode::Substring,
    )
    .expect("search succeeds");
    let paths: Vec<&str> = outcome
        .results
        .iter()
        .map(|result| result.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["a_readme.log", "b_readme.log", "c_readme.log"]);
}

#[test]
fn search_files_result_cap_truncates_to_the_lexically_smallest_matches() {
    let files: Vec<(String, Vec<u8>)> = (0..=MAX_SEARCH_RESULTS)
        .map(|index| (format!("needle_{index:04}.txt"), b"x".to_vec()))
        .collect();
    let file_refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, content)| (name.as_str(), content.as_slice()))
        .collect();
    let (_project, root, _app, index_store) = index_fixture(&file_refs);

    let outcome = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        FileSearchMode::Substring,
    )
    .expect("search succeeds");
    assert_eq!(outcome.results.len(), MAX_SEARCH_RESULTS);
    assert!(outcome.truncated);
    assert_eq!(
        outcome
            .results
            .last()
            .expect("at least one result")
            .relative_path
            .as_str(),
        format!("needle_{:04}.txt", MAX_SEARCH_RESULTS - 1)
    );
}

#[test]
fn search_files_query_validation_rejects_empty_and_over_long_queries() {
    let app_root = tempdir().expect("app root");
    let project = tempdir().expect("project root");
    let root = trusted_root(project.path());
    let index_store = store(app_root.path());
    // Validation happens before anything is read, so this project id need not
    // match the root's -- the call never gets that far.
    let project_id = ProjectId::new();

    assert!(matches!(
        search_files(
            &root,
            &index_store,
            &classifier(),
            &account(),
            project_id,
            "",
            FileSearchMode::Substring,
        ),
        Err(SearchError::EmptyQuery)
    ));

    let over_long = "a".repeat(MAX_SEARCH_QUERY_BYTES + 1);
    assert!(matches!(
        search_files(
            &root,
            &index_store,
            &classifier(),
            &account(),
            project_id,
            &over_long,
            FileSearchMode::Substring,
        ),
        Err(SearchError::QueryTooLong(limit)) if limit == MAX_SEARCH_QUERY_BYTES
    ));
}

// ── `search_text`: case sensitivity, sensitive-file gate, bounds ────────

#[test]
fn search_text_case_sensitivity_toggle() {
    let (_project, root, _app, index_store) =
        index_fixture(&[("notes.txt", b"Needle in a haystack")]);
    let project_id = *root.project_id();

    let case_sensitive_miss = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        project_id,
        "needle",
        SearchMode::Literal,
        true,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    assert!(
        case_sensitive_miss.matches.is_empty(),
        "case-sensitive search must not match differing case"
    );

    let case_insensitive_hit = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        project_id,
        "needle",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    assert_eq!(case_insensitive_hit.matches.len(), 1);

    let case_sensitive_hit = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        project_id,
        "Needle",
        SearchMode::Literal,
        true,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    assert_eq!(case_sensitive_hit.matches.len(), 1);
}

#[test]
fn search_text_excludes_a_classified_sensitive_file_unless_a_matching_override_is_presented() {
    let (_project, root, _app, index_store) = index_fixture(&[
        ("credentials.json", b"SECRET=top-secret-value"),
        ("notes.txt", b"nothing secret here"),
    ]);
    let project_id = *root.project_id();

    let outcome_without_override = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        project_id,
        "secret",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    let paths_without_override: Vec<&str> = outcome_without_override
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert!(
        !paths_without_override.contains(&"credentials.json"),
        "a classified-sensitive file's content must never appear without a matching override; got {paths_without_override:?}"
    );
    assert!(
        paths_without_override.contains(&"notes.txt"),
        "filtering the sensitive file must not suppress ordinary matches; got {paths_without_override:?}"
    );

    let scope = SensitiveReadScope::new(project_id, relative("credentials.json"), run_id());
    let SensitiveDecision::RequiresOverride(request) = evaluate(&classifier(), scope, None) else {
        panic!("expected a pending override request for a classified-sensitive file");
    };
    let grant = request.grant();

    let outcome_with_override = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        project_id,
        "secret",
        SearchMode::Literal,
        false,
        &[grant],
        &run_id(),
    )
    .expect("search succeeds");
    let paths_with_override: Vec<&str> = outcome_with_override
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert!(
        paths_with_override.contains(&"credentials.json"),
        "a matching override must admit the sensitive file; got {paths_with_override:?}"
    );
}

#[test]
fn search_text_skips_binary_content_never_scanning_it() {
    let mut content = vec![0u8, 1, 2, 0, 3];
    content.extend_from_slice(b"needle");
    let (_project, root, _app, index_store) = index_fixture(&[("binary.bin", content.as_slice())]);

    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    assert!(outcome.matches.is_empty());
    assert_eq!(
        outcome.files_scanned, 0,
        "a binary-classified candidate must never count toward files_scanned"
    );
}

#[test]
fn search_text_per_file_match_cap_is_enforced() {
    let lines: Vec<u8> = (0..MAX_MATCHES_PER_FILE + 5)
        .flat_map(|_| b"needle\n".to_vec())
        .collect();
    let (_project, root, _app, index_store) = index_fixture(&[("many_hits.txt", lines.as_slice())]);

    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    assert_eq!(outcome.matches.len(), MAX_MATCHES_PER_FILE);
    assert!(
        !outcome.truncated,
        "hitting only the per-file cap must not set the overall result truncation flag"
    );
}

#[test]
fn search_text_files_scanned_cap_truncates_before_reaching_every_candidate() {
    let mut files: Vec<(String, Vec<u8>)> = (0..MAX_SEARCH_FILES_SCANNED)
        .map(|index| (format!("file_{index:05}.txt"), b"no-match-here".to_vec()))
        .collect();
    // Sorts lexically after every `file_*` candidate above, so it is never
    // reached once the scan cap stops the walk -- proves the cap actually
    // bounds *which* candidates get scanned, not just how many.
    files.push(("zzz_target.txt".to_owned(), b"needle".to_vec()));
    let file_refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, content)| (name.as_str(), content.as_slice()))
        .collect();
    let (_project, root, _app, index_store) = index_fixture(&file_refs);

    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        SearchMode::Literal,
        false,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    assert!(
        outcome.matches.is_empty(),
        "the one matching file sorts lexically after the scan cap and must never be reached"
    );
    assert!(outcome.truncated);
    assert_eq!(outcome.files_scanned, MAX_SEARCH_FILES_SCANNED);
}

#[test]
fn search_text_query_validation_rejects_empty_and_over_long_queries() {
    let project = tempdir().expect("project root");
    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());

    assert!(matches!(
        search_text(
            &root,
            &index_store,
            &classifier(),
            &account(),
            *root.project_id(),
            "",
            SearchMode::Literal,
            false,
            &[],
            &run_id(),
        ),
        Err(SearchError::EmptyQuery)
    ));

    let over_long = "a".repeat(MAX_SEARCH_QUERY_BYTES + 1);
    assert!(matches!(
        search_text(
            &root,
            &index_store,
            &classifier(),
            &account(),
            *root.project_id(),
            &over_long,
            SearchMode::Literal,
            false,
            &[],
            &run_id(),
        ),
        Err(SearchError::QueryTooLong(limit)) if limit == MAX_SEARCH_QUERY_BYTES
    ));
}

// ── Regex, glob, and files the index has not reached ─────────────────────

#[test]
fn the_same_query_means_different_things_in_literal_and_regex_mode() {
    let (_project, root, _app, index_store) =
        index_fixture(&[("dot.rs", b"a.b = 1\n"), ("wild.rs", b"axb = 1\n")]);

    let literal = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "a.b",
        SearchMode::Literal,
        true,
        &[],
        &run_id(),
    )
    .expect("a literal query searches");
    let paths: Vec<&str> = literal
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["dot.rs"],
        "in literal mode `.` is a dot, not any character"
    );

    let pattern = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "a.b",
        SearchMode::Regex,
        true,
        &[],
        &run_id(),
    )
    .expect("a regex query searches");
    let paths: Vec<&str> = pattern
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["dot.rs", "wild.rs"],
        "in regex mode `.` matches any character"
    );
}

#[test]
fn a_regex_query_matches_a_pattern_a_literal_search_cannot_express() {
    let (_project, root, _app, index_store) = index_fixture(&[
        ("a.rs", b"fn foo(x: u8) {}\n"),
        ("b.rs", b"fn foobar(x: u8) {}\n"),
    ]);

    // Word-boundary: `foo` as a whole identifier, not the prefix of `foobar`.
    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        r"\bfoo\b",
        SearchMode::Regex,
        true,
        &[],
        &run_id(),
    )
    .expect("a regex query searches");

    let paths: Vec<&str> = outcome
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["a.rs"]);
}

#[test]
fn a_regex_that_does_not_compile_says_what_is_wrong_with_it() {
    let (_project, root, _app, index_store) = index_fixture(&[("a.rs", b"x\n")]);

    let error = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "foo(",
        SearchMode::Regex,
        true,
        &[],
        &run_id(),
    )
    .expect_err("an unbalanced group is not a pattern");

    let SearchError::InvalidRegex(message) = error else {
        panic!("expected an invalid-regex error");
    };
    // The crate's own message names the position and the problem, which is what
    // lets the caller correct the pattern rather than guess.
    assert!(message.contains("unclosed group"), "{message}");
}

#[test]
fn a_glob_matches_a_subtree_and_a_single_star_stays_within_one_segment() {
    let (_project, root, _app, index_store) = index_fixture(&[
        ("src/main.rs", b"x"),
        ("src/deep/nested.rs", b"x"),
        ("src/notes.txt", b"x"),
    ]);

    let subtree = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "src/**/*.rs",
        FileSearchMode::Glob,
    )
    .expect("a glob search succeeds");
    let paths: Vec<&str> = subtree
        .results
        .iter()
        .map(|result| result.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["src/deep/nested.rs", "src/main.rs"]);

    let one_segment = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "src/*.rs",
        FileSearchMode::Glob,
    )
    .expect("a glob search succeeds");
    let paths: Vec<&str> = one_segment
        .results
        .iter()
        .map(|result| result.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["src/main.rs"],
        "a single star must not cross a path separator"
    );
}

#[test]
fn a_glob_that_does_not_compile_says_what_is_wrong_with_it() {
    let (_project, root, _app, index_store) = index_fixture(&[("a.rs", b"x")]);

    let error = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "src/[unclosed",
        FileSearchMode::Glob,
    )
    .expect_err("an unclosed class is not a glob");

    assert!(matches!(error, SearchError::InvalidGlob(_)));
}

#[test]
fn a_file_the_index_has_not_reached_is_still_found_by_name_and_by_content() {
    let (project, root, _app, index_store) = index_fixture(&[("indexed.rs", b"old content\n")]);

    // Written after the index pass, so the index knows nothing about it. A
    // direct read of this file works, so a search that cannot see it would be
    // answering a different question than the one asked.
    fs::write(project.path().join("just-written.rs"), b"fn needle() {}\n").expect("new file");

    let by_name = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "just-written",
        FileSearchMode::Substring,
    )
    .expect("search succeeds");
    let paths: Vec<&str> = by_name
        .results
        .iter()
        .map(|result| result.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["just-written.rs"]);
    assert!(by_name.live_scanned > 0, "the live walk did the finding");

    let by_content = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        SearchMode::Literal,
        true,
        &[],
        &run_id(),
    )
    .expect("search succeeds");
    let paths: Vec<&str> = by_content
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["just-written.rs"]);
}

#[test]
fn a_sensitive_file_the_index_has_not_reached_is_still_withheld() {
    let (project, root, _app, index_store) = index_fixture(&[("indexed.rs", b"x")]);

    // The live walk finds it, but sensitivity is decided from the path before
    // anything is opened -- the same default-deny an indexed candidate carries.
    fs::write(project.path().join("credentials.json"), b"SECRET=needle\n").expect("sensitive file");

    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        SearchMode::Literal,
        true,
        &[],
        &run_id(),
    )
    .expect("search succeeds");

    assert!(
        outcome.matches.is_empty(),
        "a sensitive file must not leak content through the live walk"
    );
}

#[test]
fn the_live_walk_honors_the_same_exclusions_an_index_pass_does() {
    let (project, root, _app, index_store) = index_fixture(&[("indexed.rs", b"x")]);
    fs::write(project.path().join(".gitignore"), b"ignored/\n").expect("gitignore");
    // `require_git(true)` means .gitignore is only honored inside a real git
    // boundary, so make this one.
    fs::create_dir_all(project.path().join(".git")).expect("git dir");
    fs::create_dir_all(project.path().join("ignored")).expect("ignored dir");
    fs::write(project.path().join("ignored/hit.rs"), b"needle\n").expect("ignored file");
    fs::create_dir_all(project.path().join(".xfalcon")).expect("namespace dir");
    fs::write(project.path().join(".xfalcon/hit.rs"), b"needle\n").expect("namespace file");

    let outcome = search_text(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "needle",
        SearchMode::Literal,
        true,
        &[],
        &run_id(),
    )
    .expect("search succeeds");

    let paths: Vec<&str> = outcome
        .matches
        .iter()
        .map(|hit| hit.relative_path.as_str())
        .collect();
    assert!(
        paths.is_empty(),
        "the live walk must not surface what indexing skips: {paths:?}"
    );
}
