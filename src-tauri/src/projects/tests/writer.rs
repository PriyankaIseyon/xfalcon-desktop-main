//! Behavioral tests for the atomic writer (`crate::projects::writer`).
//!
//! `ApprovalProof::new` is `pub(crate)`, reachable directly from these
//! in-crate tests without needing a full `crate::approvals` harness --
//! exactly mirroring how `crate::tools::policy::ApprovalRequest::approve`
//! and `crate::projects::sensitive::SensitiveOverrideRequest::grant` are
//! exercised directly by their own module's tests elsewhere in this
//! codebase.

use std::fs;

use tempfile::tempdir;

use crate::projects::{
    execute, ApprovalProof, ContainmentError, EditOperation, MutationPlan, PatchError, ProjectId,
    RelativeProjectPath, TrustedRoot, WriterError,
};

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

fn root(dir: &std::path::Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), dir).expect("establish trusted root")
}

const NOT_EXPIRED: i64 = 0;

fn approve(plan: &MutationPlan, second_confirmation_given: bool) -> ApprovalProof {
    ApprovalProof::new(plan.plan_id(), second_confirmation_given)
}

// -- Happy paths --------------------------------------------------------------

#[test]
fn create_writes_exactly_the_proposed_bytes() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let content = b"created content\n".to_vec();
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: content.clone(),
        }],
    )
    .expect("propose");

    let outcome = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert_eq!(outcome.operations_completed, 1);
    assert_eq!(outcome.operations_total, 1);
    assert_eq!(fs::read(dir.path().join("new.txt")).expect("read"), content);
}

#[test]
fn modify_replaces_content_exactly_and_leaves_no_temp_file_behind() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"original").expect("seed");
    let trusted_root = root(dir.path());
    let new_content = b"replaced content".to_vec();
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: new_content.clone(),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert_eq!(
        fs::read(dir.path().join("doc.txt")).expect("read"),
        new_content
    );
    assert_no_temp_files(dir.path());
}

#[test]
fn delete_removes_the_file() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("gone.txt"), b"bye").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("gone.txt"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED).expect("execute");

    assert!(!dir.path().join("gone.txt").exists());
}

#[test]
fn rename_moves_content_to_the_new_path_and_removes_the_old_one() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("old.txt"), b"content").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("old.txt"),
            to: relative("new.txt"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert!(!dir.path().join("old.txt").exists());
    assert_eq!(
        fs::read(dir.path().join("new.txt")).expect("read"),
        b"content"
    );
}

#[test]
fn rename_onto_an_existing_file_with_second_confirmation_replaces_it() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("old.txt"), b"source").expect("seed source");
    fs::write(dir.path().join("existing.txt"), b"clobbered").expect("seed destination");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("old.txt"),
            to: relative("existing.txt"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED).expect("execute");

    assert!(!dir.path().join("old.txt").exists());
    assert_eq!(
        fs::read(dir.path().join("existing.txt")).expect("read"),
        b"source"
    );
}

#[test]
fn a_multi_operation_plan_applies_every_operation() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("modify-me.txt"), b"old").expect("seed");
    fs::write(dir.path().join("delete-me.txt"), b"gone soon").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("created.txt"),
                content: b"fresh".to_vec(),
            },
            EditOperation::Modify {
                path: relative("modify-me.txt"),
                content: b"new".to_vec(),
            },
            EditOperation::Delete {
                path: relative("delete-me.txt"),
            },
        ],
    )
    .expect("propose");

    let outcome = execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED).expect("execute");

    assert_eq!(outcome.operations_completed, 3);
    assert_eq!(fs::read(dir.path().join("created.txt")).unwrap(), b"fresh");
    assert_eq!(fs::read(dir.path().join("modify-me.txt")).unwrap(), b"new");
    assert!(!dir.path().join("delete-me.txt").exists());
}

// -- CRLF/LF and final-newline byte-exact preservation through a real write --

#[test]
fn a_real_write_preserves_crlf_bytes_exactly() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let crlf = b"one\r\ntwo\r\nthree\r\n".to_vec();
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("crlf.txt"),
            content: crlf.clone(),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert_eq!(fs::read(dir.path().join("crlf.txt")).unwrap(), crlf);
}

#[test]
fn a_real_write_preserves_a_missing_final_newline() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let no_trailing_newline = b"no newline at the end".to_vec();
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("no-newline.txt"),
            content: no_trailing_newline.clone(),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    let written = fs::read(dir.path().join("no-newline.txt")).unwrap();
    assert_eq!(written, no_trailing_newline);
    assert!(!written.ends_with(b"\n"));
}

// -- Stale base hash (execute-time) -------------------------------------------

#[test]
fn a_file_changed_since_propose_is_rejected_and_the_out_of_band_write_survives() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"v1").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: b"v2-from-the-plan".to_vec(),
        }],
    )
    .expect("propose");

    // The file changes out from under the plan between propose and execute.
    fs::write(dir.path().join("doc.txt"), b"changed-out-of-band").expect("race");

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a stale base hash must reject the whole plan");
    assert!(matches!(
        failure.error,
        WriterError::StaleBaseHash(path) if path == "doc.txt"
    ));
    assert_eq!(failure.operations_completed, 0);
    // The whole plan is rejected before any mutation: the out-of-band write
    // survives untouched, never silently overwritten.
    assert_eq!(
        fs::read(dir.path().join("doc.txt")).unwrap(),
        b"changed-out-of-band"
    );
    assert_no_temp_files(dir.path());
}

#[test]
fn a_stale_base_hash_in_a_multi_operation_plan_rejects_every_operation_not_just_the_stale_one() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("fine.txt"), b"fine").expect("seed");
    fs::write(dir.path().join("stale.txt"), b"v1").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Modify {
                path: relative("fine.txt"),
                content: b"updated".to_vec(),
            },
            EditOperation::Modify {
                path: relative("stale.txt"),
                content: b"v2".to_vec(),
            },
        ],
    )
    .expect("propose");

    fs::write(dir.path().join("stale.txt"), b"raced").expect("race only the second file");

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("must reject the whole plan");
    assert!(matches!(failure.error, WriterError::StaleBaseHash(_)));
    assert_eq!(failure.operations_completed, 0);
    // Neither operation applied -- including the one whose own base hash was
    // still fresh.
    assert_eq!(fs::read(dir.path().join("fine.txt")).unwrap(), b"fine");
    assert_no_temp_files(dir.path());
}

// -- Approval token / expiry / second-confirmation preamble -------------------

#[test]
fn an_approval_for_a_different_plan_is_rejected() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"a".to_vec(),
        }],
    )
    .expect("propose");
    let other_plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("b.txt"),
            content: b"b".to_vec(),
        }],
    )
    .expect("propose a different plan");

    let mismatched_approval = ApprovalProof::new(other_plan.plan_id(), false);
    let failure = execute(dir.path(), &plan, &mismatched_approval, NOT_EXPIRED)
        .expect_err("an approval for a different plan id must be rejected");
    assert!(matches!(failure.error, WriterError::ApprovalTokenMismatch));
    assert!(!dir.path().join("a.txt").exists());
}

#[test]
fn an_expired_plan_is_rejected_even_with_a_matching_approval() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"a".to_vec(),
        }],
    )
    .expect("propose");

    let failure = execute(
        dir.path(),
        &plan,
        &approve(&plan, false),
        plan.expires_at_ms(),
    )
    .expect_err("an expired plan must be rejected");
    assert!(matches!(failure.error, WriterError::Expired));
    assert!(!dir.path().join("a.txt").exists());
}

#[test]
fn a_plan_requiring_second_confirmation_is_rejected_without_it() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("gone.txt"), b"bye").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("gone.txt"),
        }],
    )
    .expect("propose delete (requires second confirmation)");

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a delete plan without second confirmation must be rejected");
    assert!(matches!(
        failure.error,
        WriterError::SecondConfirmationRequired
    ));
    assert!(dir.path().join("gone.txt").exists());
}

// -- The non-confirmable containment denial: cannot be approved past ---------

#[cfg(unix)]
#[test]
fn a_symlink_escape_is_rejected_regardless_of_second_confirmation_being_true() {
    // The exact scenario the plan calls "structurally impossible to approve
    // past, at any confirmation level": a path that resolves through a
    // symlink escaping the trusted root. `second_confirmation_given: true`
    // (the most permissive confirmation state this API can express) is
    // presented, and it still fails identically to the `false` case below --
    // proving the containment check never consults that field.
    use std::os::unix::fs::symlink;

    let dir = tempdir().expect("root");
    let outside = tempdir().expect("outside target");
    fs::write(outside.path().join("secret.txt"), b"outside").expect("outside file");
    // A plain file occupies the name at propose time so `MutationPlan::propose`
    // itself succeeds (it does its own containment check too); the symlink
    // is swapped in afterward to simulate the TOCTOU window `execute`'s own
    // unconditional re-check exists to close.
    fs::write(dir.path().join("escape.txt"), b"placeholder").expect("placeholder");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("escape.txt"),
            content: b"attacker-controlled".to_vec(),
        }],
    )
    .expect("propose against the placeholder file");

    fs::remove_file(dir.path().join("escape.txt")).expect("remove placeholder");
    symlink(
        outside.path().join("secret.txt"),
        dir.path().join("escape.txt"),
    )
    .expect("plant an escaping symlink at the same path");

    for second_confirmation_given in [false, true] {
        let approval = ApprovalProof::new(plan.plan_id(), second_confirmation_given);
        let failure = execute(dir.path(), &plan, &approval, NOT_EXPIRED)
            .expect_err("a symlink escape must be rejected regardless of confirmation level");
        assert!(
            matches!(
                failure.error,
                WriterError::Containment(ContainmentError::LinkOrReparse)
            ),
            "second_confirmation_given={second_confirmation_given}: {:?}",
            failure.error
        );
    }
    assert_eq!(
        fs::read(outside.path().join("secret.txt")).unwrap(),
        b"outside",
        "the outside file must never be touched"
    );
}

/// Windows and macOS only, for the same reason as
/// `super::path_policy::reopen_rejects_a_root_deleted_and_recreated_at_the_same_path`
/// — see that test's comment. Linux reuses the inode immediately, so the
/// distinction rests entirely on birth-time granularity, and the residual is
/// recorded in `docs/threat-model.md` TM-007 rather than asserted here.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn a_root_deleted_and_recreated_since_propose_is_rejected_regardless_of_second_confirmation() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"a".to_vec(),
        }],
    )
    .expect("propose");

    // Release the live directory handle before deleting. `TrustedRoot` holds
    // an open `cap_std::fs::Dir`, and Windows refuses to remove a directory
    // any process still has open (ERROR_SHARING_VIOLATION) — so without this
    // the fixture failed while setting itself up, never reaching the
    // behaviour under test. Dropping it also models the scenario accurately:
    // the root is deleted and recreated during the propose -> approve gap,
    // when the app is not holding it open. `plan` owns the project id and
    // root identity captured at propose time, so it stays valid.
    drop(trusted_root);
    fs::remove_dir(dir.path()).expect("delete the trusted root");
    fs::create_dir(dir.path()).expect("recreate at the same path (different identity)");

    for second_confirmation_given in [false, true] {
        let approval = ApprovalProof::new(plan.plan_id(), second_confirmation_given);
        let failure = execute(dir.path(), &plan, &approval, NOT_EXPIRED)
            .expect_err("a changed root identity must be rejected regardless of confirmation");
        assert!(matches!(
            failure.error,
            WriterError::Containment(ContainmentError::RootIdentityChanged)
        ));
    }
}

// -- Temp-file cleanup on every failure path ----------------------------------

#[cfg(unix)]
#[test]
fn a_permission_denied_parent_directory_fails_cleanly_with_no_temp_file_left_behind() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("root");
    let subdir = dir.path().join("locked");
    fs::create_dir(&subdir).expect("seed subdirectory");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("locked/new.txt"),
            content: b"content".to_vec(),
        }],
    )
    .expect("propose");

    fs::set_permissions(&subdir, fs::Permissions::from_mode(0o000)).expect("lock down");
    let result = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED);
    fs::set_permissions(&subdir, fs::Permissions::from_mode(0o755)).expect("restore");

    let failure = result.expect_err("a permission-denied parent directory must fail cleanly");
    assert!(
        matches!(
            failure.error,
            WriterError::Containment(ContainmentError::Io(_))
        ),
        "expected a permission-denied containment I/O error, got {:?}",
        failure.error
    );
    assert_no_temp_files(&subdir);
}

#[cfg(unix)]
#[test]
fn a_later_operations_staging_failure_cleans_up_an_earlier_operations_already_staged_temp_file() {
    // Two operations: the first targets a normal, writable location (its
    // temp file is successfully created, written, and flushed -- genuinely
    // staged); the second targets a permission-denied subdirectory, so
    // *its* staging fails. `execute` stages every operation before applying
    // any of them, so this proves the first operation's already-created
    // temp file is cleaned up by the staging-phase failure path
    // (`cleanup(&staged)` in `execute`), not just the "nothing was ever
    // created" case the dedicated single-operation test above covers, and
    // that neither operation's destination file was ever actually written.
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("root");
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).expect("seed subdirectory");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("first.txt"),
                content: b"would have been staged first".to_vec(),
            },
            EditOperation::Create {
                path: relative("locked/second.txt"),
                content: b"never reachable".to_vec(),
            },
        ],
    )
    .expect("propose");

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock down");
    let result = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");

    let failure = result.expect_err("a permission-denied second operation must fail cleanly");
    assert!(
        matches!(
            failure.error,
            WriterError::Containment(ContainmentError::Io(_))
        ),
        "expected a permission-denied containment I/O error, got {:?}",
        failure.error
    );
    assert!(
        !dir.path().join("first.txt").exists(),
        "the first operation must not have been applied either -- staging failed before any apply"
    );
    assert_no_temp_files(dir.path());
}

// -- Cross-volume rename classification (unit-level; see the module doc
// comment on why a genuine cross-mount rename is not exercised end-to-end
// in this sandbox) --------------------------------------------------------

#[test]
fn create_target_that_now_exists_is_rejected_without_touching_it() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"planned".to_vec(),
        }],
    )
    .expect("propose");

    // Something now occupies the path since propose time.
    fs::write(dir.path().join("new.txt"), b"already here").expect("race");

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("create over a now-existing path must be rejected");
    assert!(matches!(failure.error, WriterError::CreateTargetNowExists(path) if path == "new.txt"));
    assert_eq!(
        fs::read(dir.path().join("new.txt")).unwrap(),
        b"already here"
    );
}

#[test]
fn a_create_target_planted_after_staging_but_before_its_own_apply_is_rejected_not_clobbered() {
    // The staging-time existence check above (`create_target_that_now_exists_is_rejected...`)
    // only proves a file already present *before* `execute` is even called
    // is caught. This proves the narrower, harder case: a file planted in
    // the window between this operation's own staging (which already
    // passed) and its own apply -- the exact TOCTOU window a large,
    // multi-operation plan can leave open for a later operation (Task 6
    // review finding on `writer.rs`'s `execute`/`apply`). Uses the
    // `#[cfg(test)]`-only hook `execute` runs at exactly that point.
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"planned".to_vec(),
        }],
    )
    .expect("propose");

    let dir_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(
            dir_path.join("new.txt"),
            b"planted-during-the-staging-to-apply-window",
        )
        .expect("plant a racing file between staging and apply");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect_err(
        "a file planted between staging and apply must still be rejected, never silently clobbered",
    );
    assert!(matches!(failure.error, WriterError::CreateTargetNowExists(path) if path == "new.txt"));
    assert_eq!(
        fs::read(dir.path().join("new.txt")).unwrap(),
        b"planted-during-the-staging-to-apply-window",
        "the out-of-band file must survive untouched, never overwritten by the stale temp file"
    );
    assert_no_temp_files(dir.path());
}

#[test]
fn apply_time_modify_rejects_content_changed_after_staging() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"original").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: b"planned".to_vec(),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("doc.txt"), b"newer edit").expect("race");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a modify must revalidate its base immediately before replacement");
    assert!(matches!(failure.error, WriterError::StaleBaseHash(path) if path == "doc.txt"));
    assert_eq!(failure.operations_completed, 0);
    assert_eq!(fs::read(dir.path().join("doc.txt")).unwrap(), b"newer edit");
    assert_no_temp_files(dir.path());
}

#[test]
fn apply_time_stale_base_reports_the_successful_prefix_and_cleans_the_tail() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"original").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("first.txt"),
                content: b"first".to_vec(),
            },
            EditOperation::Modify {
                path: relative("doc.txt"),
                content: b"planned".to_vec(),
            },
        ],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("doc.txt"), b"newer edit").expect("race");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("the stale second operation must stop after the successful prefix");
    assert!(matches!(failure.error, WriterError::StaleBaseHash(path) if path == "doc.txt"));
    assert_eq!(failure.operations_completed, 1);
    assert_eq!(failure.operations_total, 2);
    assert_eq!(fs::read(dir.path().join("first.txt")).unwrap(), b"first");
    assert_eq!(fs::read(dir.path().join("doc.txt")).unwrap(), b"newer edit");
    assert_no_temp_files(dir.path());
}

#[test]
fn apply_time_delete_rejects_content_changed_after_staging() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"original").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("doc.txt"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("doc.txt"), b"newer edit").expect("race");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED)
        .expect_err("a delete must revalidate its base immediately before removal");
    assert!(matches!(failure.error, WriterError::StaleBaseHash(path) if path == "doc.txt"));
    assert_eq!(failure.operations_completed, 0);
    assert_eq!(fs::read(dir.path().join("doc.txt")).unwrap(), b"newer edit");
}

#[test]
fn apply_time_rename_rejects_a_changed_source() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("source.txt"), b"original").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("source.txt"),
            to: relative("destination.txt"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("source.txt"), b"newer edit").expect("race");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a rename must revalidate its source immediately before moving it");
    assert!(matches!(failure.error, WriterError::StaleBaseHash(path) if path == "source.txt"));
    assert_eq!(
        fs::read(dir.path().join("source.txt")).unwrap(),
        b"newer edit"
    );
    assert!(!dir.path().join("destination.txt").exists());
}

#[test]
fn apply_time_rename_rejects_a_changed_existing_destination() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("source.txt"), b"source").expect("seed source");
    fs::write(dir.path().join("destination.txt"), b"original destination")
        .expect("seed destination");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("source.txt"),
            to: relative("destination.txt"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("destination.txt"), b"newer destination").expect("race");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED)
        .expect_err("a rename must revalidate an existing destination before replacing it");
    assert!(matches!(failure.error, WriterError::StaleBaseHash(path) if path == "destination.txt"));
    assert_eq!(fs::read(dir.path().join("source.txt")).unwrap(), b"source");
    assert_eq!(
        fs::read(dir.path().join("destination.txt")).unwrap(),
        b"newer destination"
    );
}

#[test]
fn apply_time_rename_refuses_a_destination_that_appeared_after_staging() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("source.txt"), b"source").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("source.txt"),
            to: relative("destination.txt"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("destination.txt"), b"new arrival").expect("race");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a rename must not clobber a destination that appeared after staging");
    assert!(matches!(
        failure.error,
        WriterError::RenameTargetNowExists(path) if path == "destination.txt"
    ));
    assert_eq!(failure.operations_completed, 0);
    assert_eq!(fs::read(dir.path().join("source.txt")).unwrap(), b"source");
    assert_eq!(
        fs::read(dir.path().join("destination.txt")).unwrap(),
        b"new arrival"
    );
}

// -- Flush failures: staged-content sync and post-mutation parent-fsync
// (Task 6 plan Step 1's "file and parent-directory flush failures" test
// category; see writer.rs's `test_hooks` module doc comment for why these
// use a narrow, cfg(test)-only fault-injection seam rather than a real OS
// failure, which cannot be reproduced deterministically in this sandbox for
// either flush call specifically) -----------------------------------------

#[test]
fn a_staged_content_flush_failure_cleans_up_its_temp_file_and_rejects_the_whole_plan() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"content".to_vec(),
        }],
    )
    .expect("propose");

    let guard = crate::projects::writer::test_hooks::ForceStageContentSyncFailure::enable();
    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a staged-content flush failure must fail the plan, not be swallowed");
    drop(guard);

    assert!(matches!(failure.error, WriterError::StageWrite(_, _)));
    assert_eq!(failure.operations_completed, 0);
    assert!(!failure.durability_unconfirmed);
    assert!(!dir.path().join("new.txt").exists());
    assert_no_temp_files(dir.path());
}

#[test]
fn a_parent_directory_fsync_failure_after_a_successful_mutation_is_counted_not_lost() {
    // Table-driven across every operation kind `apply` handles: a
    // post-mutation parent-directory fsync failure must be surfaced (never
    // swallowed) while still correctly counting the mutation -- which
    // already, durably landed on disk -- as completed, never miscounted as
    // "0 of N operations completed" (Task 6 review finding, Critical).
    enum Scenario {
        Create,
        Modify,
        Delete,
        Rename,
    }

    for scenario in [
        Scenario::Create,
        Scenario::Modify,
        Scenario::Delete,
        Scenario::Rename,
    ] {
        let dir = tempdir().expect("root");
        let mut operations = match scenario {
            Scenario::Create => vec![EditOperation::Create {
                path: relative("new.txt"),
                content: b"content".to_vec(),
            }],
            Scenario::Modify => {
                fs::write(dir.path().join("doc.txt"), b"old").expect("seed");
                vec![EditOperation::Modify {
                    path: relative("doc.txt"),
                    content: b"new".to_vec(),
                }]
            }
            Scenario::Delete => {
                fs::write(dir.path().join("gone.txt"), b"bye").expect("seed");
                vec![EditOperation::Delete {
                    path: relative("gone.txt"),
                }]
            }
            Scenario::Rename => {
                fs::write(dir.path().join("old.txt"), b"payload").expect("seed");
                vec![EditOperation::Rename {
                    from: relative("old.txt"),
                    to: relative("new.txt"),
                }]
            }
        };
        fs::write(dir.path().join("untouched.txt"), b"original tail").expect("seed tail");
        operations.extend([
            EditOperation::Create {
                path: relative("unapplied.txt"),
                content: b"unused staged content".to_vec(),
            },
            EditOperation::Create {
                path: relative("staged-only/child/unapplied.txt"),
                content: b"unused nested content".to_vec(),
            },
            EditOperation::Modify {
                path: relative("untouched.txt"),
                content: b"must not land".to_vec(),
            },
        ]);

        let trusted_root = root(dir.path());
        let plan = MutationPlan::propose(&trusted_root, operations).expect("propose");
        let needs_confirmation = plan.requires_second_confirmation();

        let guard = crate::projects::writer::test_hooks::ForceFsyncParentFailure::enable();
        let failure = execute(
            dir.path(),
            &plan,
            &approve(&plan, needs_confirmation),
            NOT_EXPIRED,
        )
        .expect_err("a parent-directory fsync failure must still be surfaced as a failure");
        drop(guard);

        assert!(
            failure.durability_unconfirmed,
            "a post-mutation durability failure must be distinguished from the mutation never happening"
        );
        assert_eq!(
            failure.operations_completed, 1,
            "the operation that actually landed on disk must be counted, not reported as 0 of N"
        );
        assert_eq!(failure.operations_total, 4);

        match scenario {
            Scenario::Create => {
                assert_eq!(fs::read(dir.path().join("new.txt")).unwrap(), b"content")
            }
            Scenario::Modify => {
                assert_eq!(fs::read(dir.path().join("doc.txt")).unwrap(), b"new")
            }
            Scenario::Delete => assert!(!dir.path().join("gone.txt").exists()),
            Scenario::Rename => {
                assert!(!dir.path().join("old.txt").exists());
                assert_eq!(fs::read(dir.path().join("new.txt")).unwrap(), b"payload");
            }
        }
        assert!(!dir.path().join("unapplied.txt").exists());
        assert!(!dir.path().join("staged-only").exists());
        assert_eq!(
            fs::read(dir.path().join("untouched.txt")).expect("unchanged tail"),
            b"original tail"
        );
        assert_no_temp_files(dir.path());
    }
}

fn assert_no_temp_files(dir: &std::path::Path) {
    let entries = fs::read_dir(dir).expect("read directory");
    for entry in entries {
        let entry = entry.expect("directory entry");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        assert!(
            !(name.starts_with(".xfalcon-mut-") && name.ends_with(".tmp")),
            "orphaned temp file left behind: {name}"
        );
    }
}

// -- Parent directories -------------------------------------------------------

#[test]
fn create_makes_the_missing_directories_on_the_way_to_a_new_file() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("src/feature/new/mod.rs"),
            content: b"pub fn go() {}\n".to_vec(),
        }],
    )
    .expect("propose");

    let outcome = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert_eq!(outcome.operations_completed, 1);
    assert_eq!(
        fs::read(dir.path().join("src/feature/new/mod.rs")).expect("read"),
        b"pub fn go() {}\n"
    );
}

#[test]
fn a_rename_into_a_directory_that_does_not_exist_yet_creates_it() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("loose.txt"), b"content").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("loose.txt"),
            to: relative("archive/2026/loose.txt"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert_eq!(
        fs::read(dir.path().join("archive/2026/loose.txt")).expect("read"),
        b"content"
    );
    assert!(!dir.path().join("loose.txt").exists());
}

#[test]
fn directories_created_for_a_plan_that_then_fails_are_removed_again() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("stale.txt"), b"v1").expect("seed");
    let trusted_root = root(dir.path());
    // The second operation's base hash goes stale before execute, so the whole
    // plan is rejected -- after the first operation has already created its
    // directories.
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("fresh/dir/new.txt"),
                content: b"created".to_vec(),
            },
            EditOperation::Modify {
                path: relative("stale.txt"),
                content: b"v2".to_vec(),
            },
        ],
    )
    .expect("propose");
    fs::write(dir.path().join("stale.txt"), b"changed out of band").expect("out-of-band write");

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a stale base hash rejects the plan");
    assert_eq!(failure.operations_completed, 0);

    // Nothing of the abandoned plan is left behind, directories included.
    assert!(!dir.path().join("fresh/dir/new.txt").exists());
    assert!(
        !dir.path().join("fresh").exists(),
        "a rolled-back plan leaves no empty directories behind"
    );
}

#[test]
fn a_created_directory_that_is_no_longer_empty_survives_a_rollback() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    // The first operation fails at *apply* time (its target is planted in the
    // staging-to-apply window), which rolls back the second operation -- after
    // staging already created that operation's directory.
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("planted.txt"),
                content: b"ours".to_vec(),
            },
            EditOperation::Create {
                path: relative("shared/new.txt"),
                content: b"created".to_vec(),
            },
        ],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("planted.txt"), b"someone else").expect("planted file");
        // Another writer puts a file into the directory this plan created.
        fs::write(root_path.join("shared/someone-elses.txt"), b"not ours").expect("foreign write");
    });

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a create whose target appeared must be rejected");

    assert!(
        dir.path().join("shared/someone-elses.txt").exists(),
        "cleanup must never delete another writer's file"
    );
    assert!(!dir.path().join("shared/new.txt").exists());
}

// -- Permissions --------------------------------------------------------------

/// A replace-by-rename lands a *different* file on disk than the one it
/// replaced, so without an explicit copy the target's own permissions are
/// silently rewritten to the temp file's.
#[cfg(unix)]
#[test]
fn modifying_an_executable_file_keeps_it_executable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("root");
    let script = dir.path().join("build.sh");
    fs::write(&script, b"#!/bin/sh\necho old\n").expect("seed");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("seed mode");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("build.sh"),
            content: b"#!/bin/sh\necho new\n".to_vec(),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    let mode = fs::metadata(&script).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "an edited script must stay executable");
}

#[cfg(unix)]
#[test]
fn modifying_a_private_file_does_not_widen_who_can_read_it() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("root");
    let secret = dir.path().join("notes.txt");
    fs::write(&secret, b"private").expect("seed");
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).expect("seed mode");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("notes.txt"),
            content: b"still private".to_vec(),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    let mode = fs::metadata(&secret).expect("stat").permissions().mode();
    assert_eq!(
        mode & 0o077,
        0,
        "editing a file must not make it readable by anyone new"
    );
    assert_eq!(mode & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn a_created_file_takes_the_platform_default_rather_than_inheriting_anything() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("root");
    // A sibling with an unusual mode must not influence the new file: there is
    // no prior file at the create target to inherit from.
    let sibling = dir.path().join("existing.txt");
    fs::write(&sibling, b"x").expect("seed");
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o777)).expect("seed mode");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("fresh.txt"),
            content: b"new".to_vec(),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    let mode = fs::metadata(dir.path().join("fresh.txt"))
        .expect("stat")
        .permissions()
        .mode();
    assert_ne!(
        mode & 0o777,
        0o777,
        "a create must not inherit a sibling's mode"
    );
}

#[cfg(unix)]
#[test]
fn a_rename_onto_an_existing_file_keeps_the_source_files_own_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("root");
    let source = dir.path().join("new-build.sh");
    fs::write(&source, b"#!/bin/sh\n").expect("seed source");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).expect("seed mode");
    fs::write(dir.path().join("build.sh"), b"old").expect("seed destination");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("new-build.sh"),
            to: relative("build.sh"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED).expect("execute");

    // A rename moves the inode, so the moved file keeps its own mode with no
    // copying involved -- this pins that the permission work did not disturb it.
    let mode = fs::metadata(dir.path().join("build.sh"))
        .expect("stat")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[cfg(windows)]
#[test]
fn modifying_a_read_only_file_fails_with_the_original_intact_and_no_temp_left() {
    let dir = tempdir().expect("root");
    let path = dir.path().join("locked.txt");
    fs::write(&path, b"original").expect("seed");
    let mut permissions = fs::metadata(&path).expect("stat").permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).expect("seed read-only");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("locked.txt"),
            content: b"replaced".to_vec(),
        }],
    )
    .expect("propose");

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a read-only destination cannot be replaced");
    assert_eq!(failure.operations_completed, 0);

    assert_eq!(
        fs::read(&path).expect("read back"),
        b"original",
        "the original file must be intact"
    );
    // The staged file must never be marked read-only, or cleanup could not
    // remove it.
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .expect("list root")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".xfalcon-mut-")
        })
        .collect();
    assert!(leftovers.is_empty(), "no orphaned temp file may survive");
}

/// Platform-independent half of the permission fix: whether there is anything
/// to inherit at all. The mode assertions above are Unix-only, so this pins the
/// branch decision on every platform.
#[test]
fn only_a_replaced_file_has_permissions_to_inherit() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("existing.txt"), b"x").expect("seed");
    let trusted_root = root(dir.path());

    let existing = trusted_root
        .write_target(
            &trusted_root
                .resolve_existing(relative("existing.txt"))
                .expect("resolve existing"),
        )
        .expect("write target");
    assert!(
        existing.existing_permissions().is_some(),
        "a replace has the target's own permissions to carry across"
    );

    let fresh = trusted_root
        .write_target(
            &trusted_root
                .resolve_new(relative("fresh.txt"))
                .expect("resolve new"),
        )
        .expect("write target");
    assert!(
        fresh.existing_permissions().is_none(),
        "a create has no prior file, so it takes the platform default"
    );
}

// -- Directories --------------------------------------------------------------

#[test]
fn a_directory_moves_with_everything_under_it() {
    let dir = tempdir().expect("root");
    fs::create_dir_all(dir.path().join("old/nested")).expect("seed tree");
    fs::write(dir.path().join("old/a.txt"), b"a").expect("seed file");
    fs::write(dir.path().join("old/nested/b.txt"), b"b").expect("seed nested file");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::RenameDirectory {
            from: relative("old"),
            to: relative("new"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert!(!dir.path().join("old").exists());
    assert_eq!(fs::read(dir.path().join("new/a.txt")).expect("read"), b"a");
    assert_eq!(
        fs::read(dir.path().join("new/nested/b.txt")).expect("read"),
        b"b"
    );
}

#[test]
fn a_directory_can_move_into_a_parent_that_does_not_exist_yet() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("loose")).expect("seed dir");
    fs::write(dir.path().join("loose/a.txt"), b"a").expect("seed file");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::RenameDirectory {
            from: relative("loose"),
            to: relative("archive/2026/loose"),
        }],
    )
    .expect("propose");

    execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect("execute");

    assert_eq!(
        fs::read(dir.path().join("archive/2026/loose/a.txt")).expect("read"),
        b"a"
    );
}

#[test]
fn renaming_a_directory_onto_an_existing_one_is_refused_rather_than_merged() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("source")).expect("seed source");
    fs::write(dir.path().join("source/a.txt"), b"a").expect("seed file");
    fs::create_dir(dir.path().join("destination")).expect("seed destination");
    fs::write(dir.path().join("destination/keep.txt"), b"keep").expect("seed kept file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::RenameDirectory {
            from: relative("source"),
            to: relative("destination"),
        }],
    )
    .expect_err("a directory rename must not absorb an existing tree");

    assert!(matches!(
        error,
        PatchError::RenameDirectoryDestinationExists(path) if path == "destination"
    ));
    assert_eq!(
        fs::read(dir.path().join("destination/keep.txt")).expect("read"),
        b"keep"
    );
}

#[test]
fn apply_time_directory_rename_refuses_a_destination_that_appeared_after_staging() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("source")).expect("seed source");
    fs::write(dir.path().join("source/keep.txt"), b"source").expect("seed source file");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::RenameDirectory {
            from: relative("source"),
            to: relative("destination"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::create_dir(root_path.join("destination")).expect("race destination");
        fs::write(root_path.join("destination/new.txt"), b"new arrival")
            .expect("seed race destination");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED).expect_err(
        "a directory rename must not replace a destination that appeared after staging",
    );
    assert!(matches!(
        failure.error,
        WriterError::RenameDirectoryDestinationExists(path) if path == "destination"
    ));
    assert_eq!(failure.operations_completed, 0);
    assert_eq!(
        fs::read(dir.path().join("source/keep.txt")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(dir.path().join("destination/new.txt")).unwrap(),
        b"new arrival"
    );
}

#[test]
fn apply_time_directory_rename_rejects_a_replaced_source_entry() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("source")).expect("seed source");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::RenameDirectory {
            from: relative("source"),
            to: relative("destination"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::remove_dir(root_path.join("source")).expect("remove staged source");
        fs::create_dir(root_path.join("source")).expect("replace source entry");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, false), NOT_EXPIRED)
        .expect_err("a directory rename must not move a replacement source entry");
    assert!(matches!(
        failure.error,
        WriterError::DirectoryEntryChanged(path) if path == "source"
    ));
    assert_eq!(failure.operations_completed, 0);
    assert!(dir.path().join("source").is_dir());
    assert!(!dir.path().join("destination").exists());
}

#[test]
fn an_empty_directory_is_removed() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("empty")).expect("seed dir");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::DeleteDirectory {
            path: relative("empty"),
        }],
    )
    .expect("propose");
    // A directory delete is as irreversible as a file delete, so it carries the
    // same confirmation requirement.
    assert!(plan.requires_second_confirmation());

    execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED).expect("execute");

    assert!(!dir.path().join("empty").exists());
}

#[test]
fn apply_time_directory_delete_rejects_a_replaced_entry() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("empty")).expect("seed dir");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::DeleteDirectory {
            path: relative("empty"),
        }],
    )
    .expect("propose");

    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::remove_dir(root_path.join("empty")).expect("remove staged directory");
        fs::create_dir(root_path.join("empty")).expect("replace directory entry");
    });

    let failure = execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED)
        .expect_err("a directory delete must not remove a replacement entry");
    assert!(matches!(
        failure.error,
        WriterError::DirectoryEntryChanged(path) if path == "empty"
    ));
    assert_eq!(failure.operations_completed, 0);
    assert!(dir.path().join("empty").is_dir());
}

#[cfg(windows)]
#[test]
fn directory_mutations_reject_a_replacement_with_identical_creation_metadata() {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Storage::FileSystem::{
        SetFileTime, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
    };

    for rename in [false, true] {
        let dir = tempdir().expect("root");
        fs::create_dir(dir.path().join("source")).expect("seed directory");
        let metadata = fs::metadata(dir.path().join("source")).expect("source metadata");
        let creation_time = metadata.creation_time();
        let attributes = metadata.file_attributes();
        let operation = if rename {
            EditOperation::RenameDirectory {
                from: relative("source"),
                to: relative("destination"),
            }
        } else {
            EditOperation::DeleteDirectory {
                path: relative("source"),
            }
        };
        let trusted_root = root(dir.path());
        let plan = MutationPlan::propose(&trusted_root, vec![operation]).expect("propose");
        let root_path = dir.path().to_path_buf();
        crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
            fs::rename(root_path.join("source"), root_path.join("parked-original"))
                .expect("retain the original directory entry");
            fs::create_dir(root_path.join("source")).expect("plant replacement");
            let replacement = fs::OpenOptions::new()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(root_path.join("source"))
                .expect("open replacement directory for timestamp change");
            let created = FILETIME {
                dwLowDateTime: creation_time as u32,
                dwHighDateTime: (creation_time >> 32) as u32,
            };
            // SAFETY: this fixture owns the live directory handle. FILETIME
            // has the Win32 layout; null pointers preserve the other times.
            let changed = unsafe {
                SetFileTime(
                    replacement.as_raw_handle(),
                    &created,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            assert_ne!(changed, 0, "{}", std::io::Error::last_os_error());
            drop(replacement);
            let replacement_metadata =
                fs::metadata(root_path.join("source")).expect("replacement metadata");
            assert_eq!(replacement_metadata.creation_time(), creation_time);
            assert_eq!(replacement_metadata.file_attributes(), attributes);
        });

        let failure = execute(
            dir.path(),
            &plan,
            &approve(&plan, plan.requires_second_confirmation()),
            NOT_EXPIRED,
        )
        .expect_err("matching creation metadata does not identify the original entry");
        assert!(matches!(
            failure.error,
            WriterError::DirectoryEntryChanged(path) if path == "source"
        ));
        assert_eq!(failure.operations_completed, 0);
        assert!(dir.path().join("source").is_dir());
        assert!(dir.path().join("parked-original").is_dir());
        assert!(!dir.path().join("destination").exists());
    }
}

#[test]
fn a_non_empty_directory_is_left_intact_and_says_its_contents_must_go_first() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("full")).expect("seed dir");
    fs::write(dir.path().join("full/a.txt"), b"a").expect("seed file");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::DeleteDirectory {
            path: relative("full"),
        }],
    )
    .expect("propose");

    let failure = execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED)
        .expect_err("a populated directory must not be removed");
    assert_eq!(failure.operations_completed, 0);
    assert!(
        failure.to_string().contains("delete its contents first"),
        "{failure}"
    );
    assert_eq!(fs::read(dir.path().join("full/a.txt")).expect("read"), b"a");
}

#[test]
fn a_file_appearing_after_the_plan_was_made_stops_the_directory_from_being_removed() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("empty-for-now")).expect("seed dir");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::DeleteDirectory {
            path: relative("empty-for-now"),
        }],
    )
    .expect("propose");

    // Emptiness is enforced by the removal itself, not captured at propose
    // time, so a file that appears in this window is protected rather than
    // destroyed.
    let root_path = dir.path().to_path_buf();
    crate::projects::writer::test_hooks::set_after_staging_before_apply_hook(move || {
        fs::write(root_path.join("empty-for-now/appeared.txt"), b"mine").expect("late file");
    });

    execute(dir.path(), &plan, &approve(&plan, true), NOT_EXPIRED)
        .expect_err("a directory that gained a file must not be removed");

    assert_eq!(
        fs::read(dir.path().join("empty-for-now/appeared.txt")).expect("read"),
        b"mine"
    );
}

#[test]
fn deleting_a_directory_and_its_files_in_one_plan_is_refused_with_the_sequence_that_works() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("doomed")).expect("seed dir");
    fs::write(dir.path().join("doomed/a.txt"), b"a").expect("seed file");
    let trusted_root = root(dir.path());

    // Staging holds an open handle to `doomed` for the whole apply phase, and a
    // directory cannot be removed while a handle to it is open. Refused up
    // front rather than failing halfway through.
    let error = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Delete {
                path: relative("doomed/a.txt"),
            },
            EditOperation::DeleteDirectory {
                path: relative("doomed"),
            },
        ],
    )
    .expect_err("a directory operation cannot share a plan with an operation inside it");

    assert!(matches!(
        error,
        PatchError::DirectoryOperationWithContainedPath(ref directory, ref path)
            if directory == "doomed" && path == "doomed/a.txt"
    ));
    assert!(error.to_string().contains("second call"), "{error}");
}

#[test]
fn deleting_the_files_then_the_directory_in_two_plans_removes_the_folder() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("doomed")).expect("seed dir");
    fs::write(dir.path().join("doomed/a.txt"), b"a").expect("seed file");
    fs::write(dir.path().join("doomed/b.txt"), b"b").expect("seed file");
    let trusted_root = root(dir.path());

    let files = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Delete {
                path: relative("doomed/a.txt"),
            },
            EditOperation::Delete {
                path: relative("doomed/b.txt"),
            },
        ],
    )
    .expect("propose the file deletes");
    execute(dir.path(), &files, &approve(&files, true), NOT_EXPIRED).expect("execute the deletes");

    let folder = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::DeleteDirectory {
            path: relative("doomed"),
        }],
    )
    .expect("propose the directory delete");
    execute(dir.path(), &folder, &approve(&folder, true), NOT_EXPIRED)
        .expect("execute the directory delete");

    assert!(!dir.path().join("doomed").exists());
}

#[test]
fn a_directory_operation_naming_a_file_says_which_operation_to_use() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("a.txt"), b"a").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::DeleteDirectory {
            path: relative("a.txt"),
        }],
    )
    .expect_err("a file is not a directory");

    assert!(matches!(
        error,
        PatchError::TargetIsNotDirectory(ref path) if path == "a.txt"
    ));
    assert!(error.to_string().contains("file operation"));
}
