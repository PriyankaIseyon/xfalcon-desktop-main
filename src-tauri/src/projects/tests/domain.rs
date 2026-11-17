//! Coverage for `super::super::domain`'s containment API: the
//! `MAX_DIRECTORY_ENTRIES` boundary and the `MismatchedRoot` defense-in-depth
//! check when a `ResolvedProjectPath` produced by one `TrustedRoot` is
//! presented to a *different* `TrustedRoot`.

use std::fs;

use tempfile::tempdir;

use crate::projects::{ContainmentError, ProjectId, TrustedRoot, MAX_DIRECTORY_ENTRIES};

fn trusted_root(root: &std::path::Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), root).expect("establish trusted root")
}

// ── `MAX_DIRECTORY_ENTRIES` boundary ─────────────────────────────────────

#[test]
fn directory_entry_count_at_the_limit_succeeds_and_one_over_is_rejected() {
    const LIMIT: usize = 64;
    let directory = tempdir().expect("fixture root");
    for index in 0..LIMIT {
        fs::write(directory.path().join(format!("f{index:05}.txt")), b"x").expect("fixture file");
    }
    let root = trusted_root(directory.path());

    assert_eq!(
        root.list_root_entries_bounded(LIMIT)
            .expect("exactly at the limit must succeed")
            .len(),
        LIMIT
    );

    fs::write(directory.path().join(format!("f{LIMIT:05}.txt")), b"x")
        .expect("one more fixture file");

    let error = root
        .list_root_entries_bounded(LIMIT)
        .expect_err("one over the limit must be rejected");
    assert!(matches!(
        error,
        ContainmentError::TooManyEntries(limit) if limit == LIMIT
    ));
}

#[test]
fn a_folder_past_the_old_ten_thousand_entry_bound_lists() {
    assert_eq!(MAX_DIRECTORY_ENTRIES, 100_000);
    let directory = tempdir().expect("fixture root");
    for index in 0..12_000 {
        fs::write(directory.path().join(format!("part-{index:05}.csv")), b"").expect("fixture");
    }
    let root = trusted_root(directory.path());
    assert_eq!(root.list_root_entries().expect("lists").len(), 12_000);
}

// ── `MismatchedRoot` defense in depth ─────────────────────────────────────

#[test]
fn a_resolved_path_from_a_different_trusted_root_is_rejected_by_every_accessor() {
    let directory_a = tempdir().expect("fixture root a");
    fs::write(directory_a.path().join("shared.txt"), b"a").expect("fixture file a");
    let root_a = trusted_root(directory_a.path());

    let directory_b = tempdir().expect("fixture root b");
    fs::write(directory_b.path().join("shared.txt"), b"b").expect("fixture file b");
    let root_b = trusted_root(directory_b.path());

    let resolved_from_a = root_a
        .resolve_existing_str("shared.txt")
        .expect("resolves against its own root");

    assert!(matches!(
        root_b.open_file(&resolved_from_a),
        Err(ContainmentError::MismatchedRoot)
    ));
    assert!(matches!(
        root_b.file_metadata(&resolved_from_a),
        Err(ContainmentError::MismatchedRoot)
    ));
    assert!(matches!(
        root_b.list_entries(&resolved_from_a),
        Err(ContainmentError::MismatchedRoot)
    ));

    // Sanity check: the same resolved path against its own root is fine,
    // proving the rejections above are about the mismatched root, not some
    // other fixture mistake.
    assert!(root_a.open_file(&resolved_from_a).is_ok());
}
