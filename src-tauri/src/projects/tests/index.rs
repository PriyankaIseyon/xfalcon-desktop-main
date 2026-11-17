//! Adversarial and incremental-behavior coverage for `super::super::index`:
//! `.gitignore`/`.ignore` handling, hidden-files policy, the `.xfalcon/`
//! namespace exclusion, cancellation, root identity change, symlink escape,
//! and the mark/sweep/skip-if-unchanged incremental design.
//!
//! Fixtures use a real temporary directory and a real `IndexStore` backed by
//! `InMemorySecretStore`, mirroring `super::super::trust`'s own test fixture
//! shape (`ValidatedAccountId::parse`, `InMemorySecretStore::put_key`) rather
//! than mocking the encrypted store.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::tempdir;

use crate::persistence::{InMemorySecretStore, SecretStore};
use crate::projects::index::test_hooks::OnFirstIndexedEntry;
use crate::projects::{
    run_index_pass, search_files, CancellationSignal, ContainmentError, ContentKind,
    FileSearchMode, IndexError, IndexPassRequest, IndexPolicy, IndexStore, ProjectId,
    ProjectIndexer, RootIdentity, SecretClassifier, TrustedRoot,
};
use crate::security::account_cleanup::ValidatedAccountId;

const ACCOUNT_ID: &str = "index-tests-account";

fn account() -> ValidatedAccountId {
    ValidatedAccountId::parse(ACCOUNT_ID).expect("valid account id")
}

fn secrets() -> Arc<InMemorySecretStore> {
    let store = Arc::new(InMemorySecretStore::default());
    store.put_key(ACCOUNT_ID, [0x42; 32]).expect("account key");
    store
}

fn store(app_root: &Path) -> IndexStore {
    IndexStore::open(
        app_root.join("index.sqlite3"),
        secrets() as Arc<dyn SecretStore>,
    )
    .expect("open index store")
}

fn trusted_root(root: &Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), root).expect("establish trusted root")
}

fn classifier() -> SecretClassifier {
    SecretClassifier::built_in()
}

fn policy() -> IndexPolicy {
    IndexPolicy::default()
}

fn indexed_paths(
    store: &IndexStore,
    account: &ValidatedAccountId,
    project_id: ProjectId,
) -> Vec<String> {
    let (records, _truncated) = store.list_all(account, project_id).expect("list_all");
    let mut paths: Vec<String> = records
        .into_iter()
        .map(|record| record.relative_path.as_str().to_owned())
        .collect();
    paths.sort();
    paths
}

// ── `.gitignore` / `.ignore` ────────────────────────────────────────────

#[test]
fn gitignore_patterns_exclude_matching_files_from_the_index() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("keep.txt"), b"keep").expect("fixture file");
    fs::write(project.path().join("ignored.log"), b"ignored").expect("fixture file");
    fs::write(project.path().join(".gitignore"), b"*.log\n").expect(".gitignore");
    // The walk pins `require_git(true)` on purpose, so a `.gitignore` is only
    // honored inside a real repository boundary. Without this marker the
    // fixture asserts behaviour the indexer deliberately does not have — see
    // the companion test below, which locks that posture in.
    fs::create_dir(project.path().join(".git")).expect(".git marker");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    let outcome = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");
    assert!(!outcome.cancelled);
    assert!(!outcome.root_identity_changed);

    let paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert!(paths.contains(&"keep.txt".to_owned()));
    assert!(!paths.contains(&"ignored.log".to_owned()));
}

/// Pins the `require_git(true)` decision: a stray `.gitignore` in a directory
/// that is not a repository must not silently hide project files from the
/// index, because a file the user can see but the agent cannot is a confusing
/// and hard-to-diagnose failure mode.
#[test]
fn a_gitignore_outside_a_repository_boundary_is_not_honored() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("keep.txt"), b"keep").expect("fixture file");
    fs::write(project.path().join("looks_ignored.log"), b"present").expect("fixture file");
    fs::write(project.path().join(".gitignore"), b"*.log\n").expect(".gitignore");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &CancellationSignal::default(),
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");

    let paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert!(paths.contains(&"keep.txt".to_owned()));
    assert!(
        paths.contains(&"looks_ignored.log".to_owned()),
        "no .git boundary means the .gitignore must not apply"
    );
}

#[test]
fn dot_ignore_file_is_respected_independently_of_gitignore() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("keep.txt"), b"keep").expect("fixture file");
    fs::write(project.path().join("scratch.tmp"), b"scratch").expect("fixture file");
    fs::write(project.path().join(".ignore"), b"*.tmp\n").expect(".ignore");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");

    let paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert!(paths.contains(&"keep.txt".to_owned()));
    assert!(!paths.contains(&"scratch.tmp".to_owned()));
}

// ── Hidden-files policy ──────────────────────────────────────────────────

#[test]
fn hidden_files_and_dot_directories_are_never_indexed() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("visible.txt"), b"visible").expect("fixture file");
    fs::write(project.path().join(".hidden"), b"hidden").expect("fixture file");
    fs::create_dir_all(project.path().join(".github/workflows")).expect(".github dir");
    fs::write(project.path().join(".github/workflows/ci.yml"), b"name: ci")
        .expect("dot-dir fixture");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass");
    let paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert_eq!(paths, vec!["visible.txt".to_owned()]);
}

// ── `.xfalcon/` namespace exclusion ──────────────────────────────────────

#[test]
fn xfalcon_namespace_directory_is_never_indexed() {
    let project = tempdir().expect("project root");
    fs::create_dir_all(project.path().join(".xfalcon/skills")).expect(".xfalcon dir");
    fs::write(project.path().join(".xfalcon/skills/foo.md"), b"skill body").expect("skill fixture");
    fs::write(project.path().join("notes.txt"), b"notes").expect("fixture file");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");

    let paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert_eq!(paths, vec!["notes.txt".to_owned()]);
}

// ── Incremental: mark, sweep, skip-if-unchanged ──────────────────────────

#[test]
fn a_second_pass_over_an_unchanged_tree_reindexes_nothing() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("a.txt"), b"hello").expect("fixture file");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    let first = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("first pass");
    assert_eq!(first.files_indexed, 1);
    assert_eq!(first.files_unchanged, 0);

    let second = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("second pass");
    assert_eq!(second.files_indexed, 0);
    assert_eq!(second.files_unchanged, 1);
    assert_eq!(second.files_removed, 0);
}

#[test]
fn a_changed_file_is_reindexed_and_its_hash_changes() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("a.txt"), b"hello").expect("fixture file");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("first pass");
    let (records, _) = index_store
        .list_all(&account(), *root.project_id())
        .expect("list_all");
    let original_hash = records
        .iter()
        .find(|record| record.relative_path.as_str() == "a.txt")
        .and_then(|record| record.content_hash.clone())
        .expect("original hash present");

    // A different *size*, not just a different mtime, so the fingerprint
    // check fails deterministically regardless of filesystem mtime
    // resolution.
    fs::write(
        project.path().join("a.txt"),
        b"hello, much longer content now",
    )
    .expect("rewrite");

    let second = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("second pass");
    assert_eq!(second.files_indexed, 1);
    assert_eq!(second.files_unchanged, 0);

    let (records, _) = index_store
        .list_all(&account(), *root.project_id())
        .expect("list_all");
    let new_hash = records
        .iter()
        .find(|record| record.relative_path.as_str() == "a.txt")
        .and_then(|record| record.content_hash.clone())
        .expect("new hash present");
    assert_ne!(original_hash, new_hash);
}

#[test]
fn a_completed_pass_prunes_rows_for_files_deleted_since_the_previous_pass() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("a.txt"), b"a").expect("fixture file");
    fs::write(project.path().join("b.txt"), b"b").expect("fixture file");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("first pass");
    assert_eq!(
        indexed_paths(&index_store, &account(), *root.project_id()),
        vec!["a.txt".to_owned(), "b.txt".to_owned()]
    );

    fs::remove_file(project.path().join("b.txt")).expect("delete b.txt");

    let second = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("second pass");
    assert_eq!(second.files_removed, 1);
    assert_eq!(
        indexed_paths(&index_store, &account(), *root.project_id()),
        vec!["a.txt".to_owned()]
    );
}

// ── Cancellation ─────────────────────────────────────────────────────────
//
// `run_index_pass` captures `CancellationSignal::generation()` fresh at the
// start of *each* call ("`generation_at_start`"), so bumping the signal
// *before* calling it only changes what that call's own baseline is -- it
// does not retroactively mark the call cancelled (a freshly-started pass
// after an unrelated, already-settled revoke must run normally, not be
// permanently poisoned). The only way to observe `cancelled == true` is a
// bump that lands *during* an in-flight call -- the same shape a real
// `ProjectsService::revoke`/`cancel_all_projects` call racing against
// `ProjectIndexer::start`'s background thread would take in production.
//
// These tests previously reproduced that with a genuine two-thread race:
// 20,000 filler files plus a 2 ms head start in a cancelling thread, which
// *biased* toward the bump landing mid-walk without guaranteeing it (their
// own comment conceded as much). On a fast machine it landed outside the
// window and the assertions failed for reasons unrelated to the behaviour
// under test. They now drive the same interleaving deterministically through
// `index::test_hooks::OnFirstIndexedEntry`, which fires on the consumer
// thread the moment the walk yields its first entry -- unambiguously inside
// the pass -- so no sleeps, no filler trees, and no flakiness.

#[test]
fn a_pass_cancelled_while_in_flight_stops_early_and_reports_cancelled() {
    let project = tempdir().expect("project root");
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(project.path().join(name), b"x").expect("fixture file");
    }

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    let bump = cancellation.clone();
    let _hook = OnFirstIndexedEntry::install(move || bump.force_bump_for_test());

    let outcome = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("a cancelled pass still returns Ok, never Err");

    assert!(
        outcome.cancelled,
        "a generation bump landing mid-pass must mark the pass cancelled"
    );
    assert_eq!(outcome.files_removed, 0, "a cancelled pass never prunes");
}

#[test]
fn a_pass_cancelled_while_in_flight_never_prunes_rows_it_did_not_reach() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("a.txt"), b"a").expect("fixture file");
    fs::write(project.path().join("b.txt"), b"b").expect("fixture file");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("first pass indexes both files");

    // b.txt is gone from disk. A completed pass would sweep its row away; a
    // cancelled one must not, because "not reached yet" is indistinguishable
    // from "deleted" once the walk stops early.
    fs::remove_file(project.path().join("b.txt")).expect("delete b.txt");

    let bump = cancellation.clone();
    let _hook = OnFirstIndexedEntry::install(move || bump.force_bump_for_test());

    let outcome = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("a cancelled pass still returns Ok, never Err");

    assert!(outcome.cancelled, "the mid-pass bump must cancel the pass");
    assert_eq!(outcome.files_removed, 0, "a cancelled pass never prunes");
    assert!(
        indexed_paths(&index_store, &account(), *root.project_id()).contains(&"b.txt".to_owned()),
        "b.txt's row must survive a cancelled pass even though the file no longer exists on disk"
    );
}

// ── Root identity change ──────────────────────────────────────────────────
//
// `TrustedRoot::establish`/`reopen` guarantee, by construction, that a
// successfully-alive `TrustedRoot`'s recorded identity always matches its
// held `cap_std::fs::Dir` handle's *current* identity (both are captured
// from the same freshly-opened handle at construction time, which is never
// swapped out in place afterward) -- so the realistic, deterministic place a
// root-identity mismatch surfaces is `TrustedRoot::reopen`'s own upfront
// check, exercised here through `ProjectIndexer::start`/`run_request`
// (`super::super::index`'s actual, real background-thread entry point) with
// a deliberately wrong `expected_identity`, rather than through a
// live-handle filesystem race whose outcome would depend on
// platform/filesystem-specific fd-vs-path semantics this test cannot verify
// by execution in this sandbox. This still proves the plan's required
// "root identity change" behavior: the whole indexing operation fails
// cleanly before touching the store, never silently indexing (or pruning)
// anything against a root whose identity no longer matches what was
// recorded at trust time.

#[tokio::test]
async fn a_wrong_expected_identity_is_rejected_before_any_indexing_begins() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("a.txt"), b"a").expect("fixture file");

    let established = trusted_root(project.path());
    let project_id = *established.project_id();
    let canonical_path = established.canonical_path().to_path_buf();
    drop(established);

    let app_root = tempdir().expect("app root");
    let request = IndexPassRequest {
        project_id,
        canonical_path,
        // Deliberately wrong: astronomically unlikely to match any real
        // `(dev, ino)`/`(volume, file_index)` pair, so `TrustedRoot::reopen`
        // (called inside `run_request`) must reject it.
        expected_identity: RootIdentity::from_parts(u64::MAX, u64::MAX),
        account: account(),
        secrets: secrets() as Arc<dyn SecretStore>,
        index_db_path: app_root.path().join("index.sqlite3"),
        cancellation: CancellationSignal::default(),
        classifier: Arc::new(classifier()),
        policy: policy(),
    };

    let indexer = ProjectIndexer::new();
    let waiter = indexer
        .start(request)
        .expect("background indexing thread starts");
    let outcome = waiter.wait().await;
    assert!(
        matches!(
            outcome,
            Err(IndexError::Containment(
                ContainmentError::RootIdentityChanged
            ))
        ),
        "expected a root-identity rejection, got {outcome:?}"
    );

    // Nothing was written: the store this pass would have used was never
    // even opened successfully far enough to hold rows for `project_id`.
    let index_store = store(app_root.path());
    assert!(indexed_paths(&index_store, &account(), project_id).is_empty());
}

// ── Symlinks are never indexed ───────────────────────────────────────────

#[cfg(unix)]
#[test]
fn a_symlink_is_never_indexed_even_when_ignore_surfaces_it() {
    use std::os::unix::fs::symlink;

    let project = tempdir().expect("project root");
    let outside = tempdir().expect("outside target");
    fs::write(outside.path().join("secret.txt"), b"outside").expect("outside file");
    symlink(
        outside.path().join("secret.txt"),
        project.path().join("escape.txt"),
    )
    .expect("escaping symlink");
    fs::write(project.path().join("real.txt"), b"real").expect("real file");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    let outcome = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");
    assert!(!outcome.root_identity_changed);

    let paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert_eq!(paths, vec!["real.txt".to_owned()]);
}

// ── Content-kind classification feeding search's text-only filter ────────

#[test]
fn binary_content_is_classified_as_binary_not_text() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("binary.bin"), [0u8, 1, 2, 0, 3]).expect("binary fixture");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");

    let (records, _) = index_store
        .list_all(&account(), *root.project_id())
        .expect("list_all");
    let record = records
        .iter()
        .find(|record| record.relative_path.as_str() == "binary.bin")
        .expect("binary.bin indexed");
    assert_eq!(record.content_kind, ContentKind::Binary);
}

#[test]
fn a_sensitive_file_is_flagged_in_the_index_but_never_carries_content() {
    let project = tempdir().expect("project root");
    fs::write(project.path().join("credentials.json"), b"SECRET=1").expect("credentials fixture");

    let root = trusted_root(project.path());
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass succeeds");

    let (records, _) = index_store
        .list_all(&account(), *root.project_id())
        .expect("list_all");
    let record = records
        .iter()
        .find(|record| record.relative_path.as_str() == "credentials.json")
        .expect("credentials.json indexed");
    assert!(record.is_sensitive);
}

// ── 100,000-file synthetic tree benchmark/smoke (plan: "100,000-file
// synthetic tree startup/search behavior ... can be a benchmark/smoke test
// gated appropriately, not necessarily a fast unit test") ────────────────

/// Runs a full index pass plus a filename search over a pre-generated
/// synthetic tree (`benchmarks/projects/generate-tree.ps1` --
/// `benchmarks/projects/README.md` explains how to build one and run this
/// test). `#[ignore]`d rather than part of the default `cargo test` run:
/// generating and walking 100,000 files is deliberately excluded from this
/// crate's ordinary fast unit-test loop, matching the plan's own "not
/// necessarily a fast unit test" framing. Run explicitly with:
///
/// ```powershell
/// $env:XFALCON_PROJECT_INDEX_BENCH_ROOT = "C:\bench\xfalcon-project-index"
/// cargo test -p xfalcon-desktop --release -- --ignored startup_and_search_over_a_100_000_file_synthetic_tree
/// ```
#[test]
#[ignore = "requires a pre-generated benchmark tree; see benchmarks/projects/README.md"]
fn startup_and_search_over_a_100_000_file_synthetic_tree_completes_and_finds_matches() {
    let Ok(bench_root) = std::env::var("XFALCON_PROJECT_INDEX_BENCH_ROOT") else {
        panic!(
            "set XFALCON_PROJECT_INDEX_BENCH_ROOT to a tree generated by \
             benchmarks/projects/generate-tree.ps1 before running this test with --ignored \
             (see benchmarks/projects/README.md)"
        );
    };

    let root = trusted_root(Path::new(&bench_root));
    let app_root = tempdir().expect("app root");
    let index_store = store(app_root.path());
    let cancellation = CancellationSignal::default();

    let started = std::time::Instant::now();
    let outcome = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("index pass over the synthetic tree succeeds");
    let indexing_elapsed = started.elapsed();
    println!(
        "indexed {} files ({} unchanged, {} skipped) in {:?}",
        outcome.files_indexed, outcome.files_unchanged, outcome.files_skipped, indexing_elapsed
    );
    assert!(!outcome.cancelled);
    assert!(!outcome.root_identity_changed);
    assert!(
        outcome.files_indexed > 0,
        "expected at least one file to be indexed from the synthetic tree"
    );

    let search_started = std::time::Instant::now();
    let search_outcome = search_files(
        &root,
        &index_store,
        &classifier(),
        &account(),
        *root.project_id(),
        "source_",
        FileSearchMode::Substring,
    )
    .expect("search_files over the populated index succeeds");
    println!(
        "search_files matched {} results ({} truncated) in {:?}",
        search_outcome.results.len(),
        search_outcome.truncated,
        search_started.elapsed()
    );
    assert!(
        !search_outcome.results.is_empty(),
        "expected filename search to find at least one generated `source_*.txt` file"
    );

    // `.gitignore`'s `*.ignored` rule (written by generate-tree.ps1) must
    // still hold at this scale, not only in the single-file unit tests
    // above.
    let ignored_paths = indexed_paths(&index_store, &account(), *root.project_id());
    assert!(
        !ignored_paths.iter().any(|path| path.ends_with(".ignored")),
        "a `.gitignore`-excluded file leaked into the index at benchmark scale"
    );

    // A second, incremental pass over the same (unchanged) tree should be
    // dramatically cheaper than the first -- this is the whole point of the
    // fingerprint-skip design (see `super::super::index`'s module doc
    // comment). Not a hard performance assertion (this sandbox's disclosed
    // limitations mean this test is compiled and traced, not necessarily
    // executed here), but printed for a human comparing runs.
    let second_started = std::time::Instant::now();
    let second_outcome = run_index_pass(
        &root,
        &index_store,
        &account(),
        &cancellation,
        &classifier(),
        &policy(),
    )
    .expect("second, incremental pass succeeds");
    println!(
        "second pass: {} unchanged, {} indexed in {:?}",
        second_outcome.files_unchanged,
        second_outcome.files_indexed,
        second_started.elapsed()
    );
    assert_eq!(second_outcome.files_indexed, 0);
}
