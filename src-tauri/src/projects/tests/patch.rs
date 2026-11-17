use std::fs;

use tempfile::tempdir;

use crate::projects::{
    hash_bytes, EditOperation, MutationPlan, MutationRiskTier, PatchError, ProjectId,
    RelativeProjectPath, TextEdit, TrustedRoot,
};

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

fn root(dir: &std::path::Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), dir).expect("establish trusted root")
}

#[test]
fn create_proposes_a_plan_with_no_base_hash_and_a_created_diff() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("new.txt"),
            content: b"hello\n".to_vec(),
        }],
    )
    .expect("propose create");

    assert!(plan.base_hashes().is_empty());
    assert_eq!(
        plan.proposed_hashes().get("new.txt").copied(),
        Some(hash_bytes(b"hello\n"))
    );
    assert!(plan.unified_diff().contains("new.txt"));
    assert_eq!(plan.risk(), MutationRiskTier::Standard);
}

#[test]
fn create_targeting_an_existing_path_is_rejected() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("existing.txt"), b"content").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("existing.txt"),
            content: b"new".to_vec(),
        }],
    )
    .expect_err("create over an existing path must fail");
    assert!(matches!(error, PatchError::CreateTargetExists(path) if path == "existing.txt"));
}

#[test]
fn modify_captures_the_current_content_as_the_base_hash() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"original").expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: b"changed".to_vec(),
        }],
    )
    .expect("propose modify");

    assert_eq!(
        plan.base_hashes().get("doc.txt").copied(),
        Some(hash_bytes(b"original"))
    );
    assert_eq!(
        plan.proposed_hashes().get("doc.txt").copied(),
        Some(hash_bytes(b"changed"))
    );
    assert!(plan.unified_diff().contains("-original"));
    assert!(plan.unified_diff().contains("+changed"));
}

#[test]
fn modify_targeting_a_missing_path_is_rejected() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("missing.txt"),
            content: b"x".to_vec(),
        }],
    )
    .expect_err("modify of a missing path must fail");
    assert!(matches!(error, PatchError::ModifyTargetMissing(path) if path == "missing.txt"));
}

#[test]
fn delete_captures_the_current_content_as_the_base_hash_and_diffs_it_as_removed() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("gone.txt"), b"bye").expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("gone.txt"),
        }],
    )
    .expect("propose delete");

    assert_eq!(
        plan.base_hashes().get("gone.txt").copied(),
        Some(hash_bytes(b"bye"))
    );
    assert!(plan.proposed_hashes().is_empty());
    assert_eq!(plan.risk(), MutationRiskTier::SecondConfirmationRequired);
}

#[test]
fn delete_targeting_a_missing_path_is_rejected() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("missing.txt"),
        }],
    )
    .expect_err("delete of a missing path must fail");
    assert!(matches!(error, PatchError::DeleteTargetMissing(path) if path == "missing.txt"));
}

#[test]
fn rename_onto_a_fresh_path_does_not_require_second_confirmation() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("old.txt"), b"content").expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("old.txt"),
            to: relative("new.txt"),
        }],
    )
    .expect("propose rename");

    assert_eq!(plan.risk(), MutationRiskTier::Standard);
    assert_eq!(plan.total_paths(), 2);
    assert_eq!(
        plan.proposed_hashes().get("new.txt").copied(),
        Some(hash_bytes(b"content"))
    );
}

#[test]
fn rename_onto_an_existing_path_requires_second_confirmation_and_captures_both_base_hashes() {
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
    .expect("propose rename-existing");

    assert_eq!(plan.risk(), MutationRiskTier::SecondConfirmationRequired);
    assert_eq!(
        plan.base_hashes().get("old.txt").copied(),
        Some(hash_bytes(b"source"))
    );
    assert_eq!(
        plan.base_hashes().get("existing.txt").copied(),
        Some(hash_bytes(b"clobbered"))
    );
}

#[test]
fn rename_source_missing_is_rejected() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("missing.txt"),
            to: relative("new.txt"),
        }],
    )
    .expect_err("rename of a missing source must fail");
    assert!(matches!(error, PatchError::RenameSourceMissing(path) if path == "missing.txt"));
}

#[test]
fn rename_to_the_same_path_is_rejected() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("same.txt"), b"content").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("same.txt"),
            to: relative("same.txt"),
        }],
    )
    .expect_err("rename to itself must fail");
    assert!(matches!(error, PatchError::RenameSamePath(path) if path == "same.txt"));
}

// -- Stale base hash --------------------------------------------------------

#[test]
fn a_stale_base_hash_check_reflects_disk_state_at_propose_time_only() {
    // `propose` itself only ever reads disk once, at propose time -- the
    // *execute*-time re-check of this same base hash (writer.rs's own test
    // suite) is what actually rejects a file changed since. This proves the
    // base hash proposal captured is the value the file had *before* an
    // out-of-band change, not a value that magically tracks later edits.
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"v1").expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: b"v2".to_vec(),
        }],
    )
    .expect("propose modify");
    let captured_base = plan.base_hashes().get("doc.txt").copied();

    fs::write(dir.path().join("doc.txt"), b"changed-out-of-band").expect("out-of-band change");

    assert_eq!(captured_base, Some(hash_bytes(b"v1")));
    assert_ne!(captured_base, Some(hash_bytes(b"changed-out-of-band")));
}

// -- CRLF/LF preservation and final newline handling -------------------------

#[test]
fn content_bytes_are_never_normalized_crlf_is_preserved_verbatim() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let crlf_content = b"line one\r\nline two\r\n".to_vec();

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("crlf.txt"),
            content: crlf_content.clone(),
        }],
    )
    .expect("propose create");

    assert_eq!(
        plan.proposed_hashes().get("crlf.txt").copied(),
        Some(hash_bytes(&crlf_content))
    );
}

#[test]
fn content_without_a_final_newline_is_preserved_verbatim() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"no trailing newline").expect("seed file");
    let trusted_root = root(dir.path());
    let new_content = b"still no trailing newline".to_vec();

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: new_content.clone(),
        }],
    )
    .expect("propose modify");

    assert_eq!(
        plan.proposed_hashes().get("doc.txt").copied(),
        Some(hash_bytes(&new_content))
    );
    assert!(plan.unified_diff().contains("No newline at end of file"));
}

// -- UTF encodings / binary content classification --------------------------

#[test]
fn valid_utf8_text_content_produces_a_line_diff() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("utf8.txt"),
            content: "caf\u{00e9} \u{1f600}\n".as_bytes().to_vec(),
        }],
    )
    .expect("propose create");

    assert_eq!(plan.risk(), MutationRiskTier::Standard);
    assert!(plan.unified_diff().contains("utf8.txt"));
}

#[test]
fn non_utf8_content_is_classified_binary_and_requires_second_confirmation() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    // Invalid UTF-8: a lone continuation byte.
    let non_utf8 = vec![0x66, 0x6f, 0x80, 0x6f];

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("weird.bin"),
            content: non_utf8.clone(),
        }],
    )
    .expect("propose create");

    assert_eq!(plan.risk(), MutationRiskTier::SecondConfirmationRequired);
    assert!(plan.unified_diff().contains("Binary file created"));
    assert_eq!(
        plan.proposed_hashes().get("weird.bin").copied(),
        Some(hash_bytes(&non_utf8))
    );
}

#[test]
fn binary_content_with_nul_bytes_requires_second_confirmation_and_never_corrupts_the_diff() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let binary_content = vec![0u8, 1, 2, 3, 0, 255];

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("data.bin"),
            content: binary_content,
        }],
    )
    .expect("propose create");

    assert_eq!(plan.risk(), MutationRiskTier::SecondConfirmationRequired);
    assert_eq!(
        plan.unified_diff(),
        "Binary file created: data.bin (6 bytes)\n"
    );
}

#[test]
fn a_modify_whose_existing_content_is_binary_but_new_content_is_text_still_requires_second_confirmation(
) {
    // `diff_for_modify` falls back to the opaque "Binary files ... differ"
    // placeholder whenever *either* side is non-text -- so the risk-tier
    // classification must trigger on either side too, not only the
    // proposed (new) content, or a human reviewer could be shown only that
    // opaque placeholder for a change that was never actually flagged as
    // reduced-reviewability (Task 6 review finding).
    let dir = tempdir().expect("root");
    // Invalid UTF-8: a lone continuation byte -- the *existing* content is
    // binary, but the *proposed* new content below is plain valid text.
    let existing_binary = vec![0x66, 0x6f, 0x80, 0x6f];
    fs::write(dir.path().join("was-binary.txt"), &existing_binary).expect("seed binary content");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("was-binary.txt"),
            content: b"now plain text".to_vec(),
        }],
    )
    .expect("propose modify");

    assert_eq!(plan.risk(), MutationRiskTier::SecondConfirmationRequired);
    assert!(plan.unified_diff().contains("Binary files"));
}

#[test]
fn modifying_a_real_office_container_remains_an_explicit_binary_change() {
    let dir = tempdir().expect("root");
    let source = include_bytes!("../../../tests/fixtures/office/producer.docx");
    fs::write(dir.path().join("brief.docx"), source).expect("Office source");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("brief.docx"),
            content: b"replacement text".to_vec(),
        }],
    )
    .expect("binary Office change plan");

    assert_eq!(plan.risk(), MutationRiskTier::SecondConfirmationRequired);
    assert!(plan.unified_diff().contains("Binary files"));
    assert_eq!(fs::read(dir.path().join("brief.docx")).unwrap(), source);
}

// -- Directory replacement ---------------------------------------------------

#[test]
fn modify_targeting_a_directory_is_rejected() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("subdir")).expect("seed directory");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("subdir"),
            content: b"x".to_vec(),
        }],
    )
    .expect_err("modify of a directory must fail");
    assert!(matches!(error, PatchError::TargetIsDirectory(_)));
}

#[test]
fn delete_targeting_a_directory_is_rejected() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("subdir")).expect("seed directory");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("subdir"),
        }],
    )
    .expect_err("delete of a directory must fail");
    assert!(matches!(error, PatchError::TargetIsDirectory(_)));
}

#[test]
fn rename_destination_that_is_a_directory_is_rejected() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("source.txt"), b"content").expect("seed source");
    fs::create_dir(dir.path().join("subdir")).expect("seed directory");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("source.txt"),
            to: relative("subdir"),
        }],
    )
    .expect_err("rename onto a directory must fail");
    assert!(matches!(error, PatchError::TargetIsDirectory(_)));
}

// -- Locked / permission-denied files (Unix-only: chmod 000) -----------------

#[cfg(unix)]
#[test]
fn modify_of_a_permission_denied_file_fails_cleanly_never_panics() {
    use std::os::unix::fs::PermissionsExt;

    use crate::projects::ContainmentError;

    let dir = tempdir().expect("root");
    let file_path = dir.path().join("locked.txt");
    fs::write(&file_path, b"secret").expect("seed file");
    fs::set_permissions(&file_path, fs::Permissions::from_mode(0o000)).expect("lock down");
    let trusted_root = root(dir.path());

    let result = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("locked.txt"),
            content: b"new".to_vec(),
        }],
    );

    // Restore permissions so the tempdir can clean itself up.
    fs::set_permissions(&file_path, fs::Permissions::from_mode(0o644)).expect("restore");

    let error = result.expect_err("a permission-denied read must fail cleanly, never panic");
    assert!(
        matches!(error, PatchError::Containment(ContainmentError::Io(_))),
        "expected a permission-denied containment I/O error, got {error:?}"
    );
}

// -- Plan-level bounds and policy --------------------------------------------

#[test]
fn an_empty_plan_is_rejected() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let error =
        MutationPlan::propose(&trusted_root, vec![]).expect_err("an empty plan must be rejected");
    assert!(matches!(error, PatchError::Empty));
}

#[test]
fn duplicate_path_targets_within_one_plan_are_rejected() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("a.txt"),
                content: b"1".to_vec(),
            },
            EditOperation::Create {
                path: relative("a.txt"),
                content: b"2".to_vec(),
            },
        ],
    )
    .expect_err("duplicate targets must be rejected");
    assert!(matches!(error, PatchError::DuplicatePathTarget(path) if path == "a.txt"));
}

#[test]
fn a_plan_with_more_than_ten_paths_requires_second_confirmation_and_exactly_ten_does_not() {
    use crate::projects::SECOND_CONFIRMATION_PATH_THRESHOLD;

    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());
    let at_limit: Vec<EditOperation> = (0..SECOND_CONFIRMATION_PATH_THRESHOLD)
        .map(|index| EditOperation::Create {
            path: relative(&format!("file-{index}.txt")),
            content: b"x".to_vec(),
        })
        .collect();
    let plan_at_limit =
        MutationPlan::propose(&trusted_root, at_limit).expect("exactly at the limit succeeds");
    assert_eq!(plan_at_limit.risk(), MutationRiskTier::Standard);

    let over_limit: Vec<EditOperation> = (0..=SECOND_CONFIRMATION_PATH_THRESHOLD)
        .map(|index| EditOperation::Create {
            path: relative(&format!("over-{index}.txt")),
            content: b"x".to_vec(),
        })
        .collect();
    let plan_over_limit = MutationPlan::propose(&trusted_root, over_limit)
        .expect("one over the limit still proposes");
    assert_eq!(
        plan_over_limit.risk(),
        MutationRiskTier::SecondConfirmationRequired
    );
}

#[test]
fn a_plan_exceeding_the_second_confirmation_byte_threshold_requires_it_and_at_the_threshold_does_not(
) {
    use crate::projects::SECOND_CONFIRMATION_BYTE_THRESHOLD;

    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let at_limit = vec![EditOperation::Create {
        path: relative("at-limit.bin"),
        content: vec![b'a'; SECOND_CONFIRMATION_BYTE_THRESHOLD as usize],
    }];
    let plan_at_limit =
        MutationPlan::propose(&trusted_root, at_limit).expect("exactly at the byte limit");
    assert_eq!(plan_at_limit.risk(), MutationRiskTier::Standard);

    let over_limit = vec![EditOperation::Create {
        path: relative("over-limit.bin"),
        content: vec![b'a'; SECOND_CONFIRMATION_BYTE_THRESHOLD as usize + 1],
    }];
    let plan_over_limit =
        MutationPlan::propose(&trusted_root, over_limit).expect("one byte over the limit");
    assert_eq!(
        plan_over_limit.risk(),
        MutationRiskTier::SecondConfirmationRequired
    );
}

#[test]
fn a_plan_with_more_than_the_max_operations_is_rejected_and_exactly_the_max_is_not() {
    use crate::projects::MAX_OPERATIONS_PER_PLAN;

    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let at_limit: Vec<EditOperation> = (0..MAX_OPERATIONS_PER_PLAN)
        .map(|index| EditOperation::Create {
            path: relative(&format!("file-{index}.txt")),
            content: b"x".to_vec(),
        })
        .collect();
    assert!(MutationPlan::propose(&trusted_root, at_limit).is_ok());

    let over_limit: Vec<EditOperation> = (0..=MAX_OPERATIONS_PER_PLAN)
        .map(|index| EditOperation::Create {
            path: relative(&format!("over-{index}.txt")),
            content: b"x".to_vec(),
        })
        .collect();
    let error = MutationPlan::propose(&trusted_root, over_limit)
        .expect_err("one over the operation-count limit must be rejected");
    assert!(matches!(
        error,
        PatchError::TooManyOperations(limit) if limit == MAX_OPERATIONS_PER_PLAN
    ));
}

// -- Plan id content-addressing ----------------------------------------------

#[test]
fn two_identical_proposals_against_the_same_root_produce_the_same_plan_id() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let first = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"content".to_vec(),
        }],
    )
    .expect("first proposal");
    let second = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"content".to_vec(),
        }],
    )
    .expect("second, identical proposal");

    assert_eq!(first.plan_id(), second.plan_id());
    assert!(first.digest_is_self_consistent());
}

#[test]
fn two_proposals_with_different_content_produce_different_plan_ids() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let first = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"one".to_vec(),
        }],
    )
    .expect("first proposal");
    let second = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"two".to_vec(),
        }],
    )
    .expect("second proposal");

    assert_ne!(first.plan_id(), second.plan_id());
}

#[test]
fn a_plan_is_not_expired_before_its_ttl_and_is_expired_at_or_after_it() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("a.txt"),
            content: b"content".to_vec(),
        }],
    )
    .expect("propose");

    assert!(!plan.is_expired(plan.expires_at_ms() - 1));
    assert!(plan.is_expired(plan.expires_at_ms()));
    assert!(plan.is_expired(plan.expires_at_ms() + 1));
}

// -- Content size bounds ------------------------------------------------------

#[test]
fn content_exceeding_the_editable_bound_is_rejected_and_exactly_at_the_bound_is_not() {
    use crate::projects::MAX_EDITABLE_FILE_BYTES;

    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let at_limit = vec![EditOperation::Create {
        path: relative("at-limit.bin"),
        content: vec![b'a'; MAX_EDITABLE_FILE_BYTES as usize],
    }];
    assert!(MutationPlan::propose(&trusted_root, at_limit).is_ok());

    let over_limit = vec![EditOperation::Create {
        path: relative("over-limit.bin"),
        content: vec![b'a'; MAX_EDITABLE_FILE_BYTES as usize + 1],
    }];
    let error = MutationPlan::propose(&trusted_root, over_limit)
        .expect_err("content over the editable bound must be rejected");
    assert!(matches!(
        error,
        PatchError::ContentTooLarge(path, limit)
            if path == "over-limit.bin" && limit == MAX_EDITABLE_FILE_BYTES
    ));
}

// -- Anchored edits ---------------------------------------------------------

#[test]
fn replace_resolves_anchors_against_current_content_and_plans_a_whole_file_modify() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("lib.rs"), b"fn one() {}\nfn two() {}\n").expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Replace {
            path: relative("lib.rs"),
            edits: vec![TextEdit {
                old_string: "fn two()".to_owned(),
                new_string: "fn three()".to_owned(),
                replace_all: false,
            }],
        }],
    )
    .expect("propose an anchored edit");

    // The plan carries final content, never the anchor: what the reviewer sees
    // is what the writer writes.
    assert_eq!(plan.operations().len(), 1);
    assert_eq!(plan.operations()[0].kind(), "modify");
    assert_eq!(
        plan.base_hashes().get("lib.rs").copied(),
        Some(hash_bytes(b"fn one() {}\nfn two() {}\n"))
    );
    assert_eq!(
        plan.proposed_hashes().get("lib.rs").copied(),
        Some(hash_bytes(b"fn one() {}\nfn three() {}\n"))
    );
    // Reviewable exactly like a whole-file change.
    assert!(plan.unified_diff().contains("-fn two() {}"));
    assert!(plan.unified_diff().contains("+fn three() {}"));
}

#[test]
fn an_anchor_that_does_not_match_fails_the_proposal_with_actionable_detail() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("lib.rs"), b"fn one() {}\n").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Replace {
            path: relative("lib.rs"),
            edits: vec![TextEdit {
                old_string: "fn missing()".to_owned(),
                new_string: "fn other()".to_owned(),
                replace_all: false,
            }],
        }],
    )
    .expect_err("an unmatched anchor must not write");

    assert!(matches!(
        error,
        PatchError::Edit(crate::projects::EditError::AnchorNotFound(_, 1, _))
    ));
    // The edit error reaches the caller verbatim rather than being flattened
    // into a generic failure -- this is what the model reads and retries from.
    assert!(error.to_string().contains("must match the file exactly"));
}

#[test]
fn insert_adds_lines_without_re_sending_the_file() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("mod.rs"), b"mod a;\nmod c;\n").expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Insert {
            path: relative("mod.rs"),
            insert_line: 1,
            content: "mod b;".to_owned(),
        }],
    )
    .expect("propose an insert");

    assert_eq!(
        plan.proposed_hashes().get("mod.rs").copied(),
        Some(hash_bytes(b"mod a;\nmod b;\nmod c;\n"))
    );
}

#[test]
fn an_anchored_edit_of_a_file_that_is_not_text_is_refused() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("logo.png"), [0u8, 1, 2, 3]).expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Replace {
            path: relative("logo.png"),
            edits: vec![TextEdit {
                old_string: "anything".to_owned(),
                new_string: "else".to_owned(),
                replace_all: false,
            }],
        }],
    )
    .expect_err("binary content has no anchors");

    assert!(matches!(error, PatchError::NotText(path) if path == "logo.png"));
}

#[test]
fn a_change_that_matches_what_is_already_on_disk_is_refused_rather_than_written() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("doc.txt"), b"same").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("doc.txt"),
            content: b"same".to_vec(),
        }],
    )
    .expect_err("writing identical content is a mistake, not a write");

    assert!(matches!(error, PatchError::NoChange(path) if path == "doc.txt"));
}

#[test]
fn a_person_may_replace_the_whole_content_of_a_large_file() {
    let dir = tempdir().expect("root");
    // Past the old 1 MiB bound: a person's editor holds the whole file, so a
    // whole-file save is not a blind write (spec: project-file-editing).
    let existing = "x".repeat(2 * 1024 * 1024);
    fs::write(dir.path().join("big.txt"), &existing).expect("seed file");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("big.txt"),
            content: b"tiny".to_vec(),
        }],
    );
    assert!(plan.is_ok(), "the plan layer does not refuse by file size");
}

#[test]
fn deleting_a_file_too_large_to_hold_in_memory_still_plans() {
    use crate::projects::MAX_EDITABLE_FILE_BYTES;

    let dir = tempdir().expect("root");
    // Sized, not written: a delete never needs the content, only the identity
    // of what it removes.
    let file = fs::File::create(dir.path().join("huge.bin")).expect("seed file");
    file.set_len(MAX_EDITABLE_FILE_BYTES + 1)
        .expect("a file past the editable bound");
    drop(file);
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Delete {
            path: relative("huge.bin"),
        }],
    )
    .expect("a delete needs no content bound");

    assert!(plan.base_hashes().contains_key("huge.bin"));
    assert!(plan.unified_diff().contains("huge.bin"));
}

#[test]
fn renaming_a_file_too_large_to_hold_in_memory_still_plans() {
    use crate::projects::MAX_EDITABLE_FILE_BYTES;

    let dir = tempdir().expect("root");
    let file = fs::File::create(dir.path().join("huge.bin")).expect("seed file");
    file.set_len(MAX_EDITABLE_FILE_BYTES + 1)
        .expect("a file past the editable bound");
    drop(file);
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("huge.bin"),
            to: relative("moved.bin"),
        }],
    )
    .expect("a rename moves bytes without reading them");

    // The source's identity is captured by streaming, and the destination
    // inherits it.
    let from_hash = plan.base_hashes().get("huge.bin").copied();
    assert!(from_hash.is_some());
    assert_eq!(plan.proposed_hashes().get("moved.bin").copied(), from_hash);
}

#[test]
fn creating_a_file_in_a_directory_that_does_not_exist_yet_plans_without_creating_it() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("src/feature/new/mod.rs"),
            content: b"pub fn go() {}\n".to_vec(),
        }],
    )
    .expect("scaffolding into a new directory is an ordinary create");

    assert_eq!(plan.operations().len(), 1);
    // Proposing is still read-only: the directories appear at execute time.
    assert!(!dir.path().join("src").exists());
}

#[test]
fn every_bad_operation_is_reported_in_one_answer() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("ok.html"), b"<p>old</p>").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Modify {
                path: relative("missing-one.html"),
                content: b"x".to_vec(),
            },
            EditOperation::Modify {
                path: relative("ok.html"),
                content: b"<p>new</p>".to_vec(),
            },
            EditOperation::Modify {
                path: relative("missing-two.html"),
                content: b"x".to_vec(),
            },
        ],
    )
    .expect_err("two bad operations");

    let PatchError::Rejected(issues) = &error else {
        panic!("expected every issue at once, got {error:?}");
    };
    assert_eq!(issues.len(), 2);
    assert_eq!((issues[0].index, issues[1].index), (0, 2));
    let message = error.to_string();
    assert!(message.contains("missing-one.html"), "{message}");
    assert!(message.contains("missing-two.html"), "{message}");
    assert!(
        message.contains("nothing was written"),
        "the author must learn that the valid operation did not run either: {message}"
    );
}

#[test]
fn a_modification_that_changes_nothing_is_dropped_when_other_work_remains() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("done.html"), b"<p>same</p>").expect("seed file");
    fs::write(dir.path().join("todo.html"), b"<p>old</p>").expect("seed file");
    let trusted_root = root(dir.path());

    let (plan, unchanged) = MutationPlan::propose_reporting(
        &trusted_root,
        vec![
            EditOperation::Modify {
                path: relative("done.html"),
                content: b"<p>same</p>".to_vec(),
            },
            EditOperation::Modify {
                path: relative("todo.html"),
                content: b"<p>new</p>".to_vec(),
            },
        ],
    )
    .expect("a retry after a partial failure must not be refused for the files that landed");

    assert_eq!(unchanged, vec!["done.html".to_owned()]);
    assert_eq!(plan.operations().len(), 1);
    assert!(plan.proposed_hashes().contains_key("todo.html"));
}

#[test]
fn a_plan_where_every_file_is_already_up_to_date_is_refused_once() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("a.html"), b"a").expect("seed file");
    fs::write(dir.path().join("b.html"), b"b").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Modify {
                path: relative("a.html"),
                content: b"a".to_vec(),
            },
            EditOperation::Modify {
                path: relative("b.html"),
                content: b"b".to_vec(),
            },
        ],
    )
    .expect_err("nothing to write");

    assert!(matches!(error, PatchError::AllUnchanged(paths) if paths == "a.html, b.html"));
}

#[test]
fn two_spellings_of_one_path_are_one_target() {
    let dir = tempdir().expect("root");
    fs::create_dir(dir.path().join("sub")).expect("dir");
    fs::write(dir.path().join("sub/inner.html"), b"old").expect("seed file");
    let trusted_root = root(dir.path());

    for alias in ["sub//inner.html", "sub/./inner.html", "sub/inner.html/"] {
        let error = MutationPlan::propose(
            &trusted_root,
            vec![
                EditOperation::Modify {
                    path: relative("sub/inner.html"),
                    content: b"first".to_vec(),
                },
                EditOperation::Modify {
                    path: relative(alias),
                    content: b"second".to_vec(),
                },
            ],
        )
        .expect_err("aliases of one file are one target");
        assert!(
            matches!(&error, PatchError::DuplicatePathTarget(path) if path == "sub/inner.html"),
            "{alias}: {error:?}"
        );
    }
}

#[test]
fn a_create_with_a_placeholder_comment_is_refused() {
    let dir = tempdir().expect("root");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose_reporting(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("page.html"),
            content: b"<body>\n<!-- ... rest of the page unchanged ... -->\n</body>\n".to_vec(),
        }],
    )
    .expect_err("a placeholder would be written as the page");

    assert!(matches!(
        error,
        PatchError::Edit(crate::projects::EditError::PlaceholderContent(_, 2))
    ));
    assert!(!dir.path().join("page.html").exists());
}

#[test]
fn a_whole_file_rewrite_with_a_placeholder_comment_is_refused_and_leaves_the_file() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("a.js"), b"one();\ntwo();\nthree();\n").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose_reporting(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("a.js"),
            content: b"one();\n// ... rest of the file unchanged\n".to_vec(),
        }],
    )
    .expect_err("the rest of the file would be deleted");

    assert!(matches!(
        error,
        PatchError::Edit(crate::projects::EditError::PlaceholderContent(_, 2))
    ));
    assert_eq!(
        fs::read(dir.path().join("a.js")).expect("file"),
        b"one();\ntwo();\nthree();\n"
    );
}

#[test]
fn a_rewrite_keeps_a_placeholder_line_the_file_already_had() {
    let dir = tempdir().expect("root");
    fs::write(
        dir.path().join("notes.txt"),
        b"// ... rest of the list is generated\nitem\n",
    )
    .expect("seed file");
    let trusted_root = root(dir.path());

    MutationPlan::propose_reporting(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("notes.txt"),
            content: b"// ... rest of the list is generated\nitem\nitem2\n".to_vec(),
        }],
    )
    .expect("a line that was already there is not a placeholder");
}

#[test]
fn a_persons_own_plan_is_never_refused_for_a_placeholder_line() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("a.js"), b"one();\n").expect("seed file");
    let trusted_root = root(dir.path());

    // A save or a restore: whatever the person wrote is what gets written.
    MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("a.js"),
            content: b"one();\n// TODO ... rest unchanged\n".to_vec(),
        }],
    )
    .expect("a person's save is not reviewed for wording");
}

#[test]
fn an_insert_of_a_placeholder_comment_is_refused_for_a_model() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("a.js"), b"one();\n").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose_reporting(
        &trusted_root,
        vec![EditOperation::Insert {
            path: relative("a.js"),
            insert_line: 1,
            content: "// ... rest unchanged".to_owned(),
        }],
    )
    .expect_err("an insert is text too");

    assert!(matches!(
        error,
        PatchError::Edit(crate::projects::EditError::PlaceholderContent(_, 1))
    ));
}

#[test]
fn a_path_left_out_as_unchanged_is_not_counted_as_changed() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("a.js"), b"one();\n").expect("seed file");
    fs::write(dir.path().join("b.js"), b"two();\n").expect("seed file");
    let trusted_root = root(dir.path());

    let (plan, unchanged) = MutationPlan::propose_reporting(
        &trusted_root,
        vec![
            EditOperation::Modify {
                path: relative("a.js"),
                content: b"one();\n".to_vec(),
            },
            EditOperation::Modify {
                path: relative("b.js"),
                content: b"three();\n".to_vec(),
            },
        ],
    )
    .expect("one real change");

    assert_eq!(unchanged, vec!["a.js".to_owned()]);
    assert_eq!(plan.total_paths(), 1);
}

#[test]
fn a_create_sent_again_after_it_landed_is_unchanged_not_an_error() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("b.html"), b"<p>hi</p>\n").expect("seed file");
    fs::write(dir.path().join("c.html"), b"old\n").expect("seed file");
    let trusted_root = root(dir.path());

    let (plan, unchanged) = MutationPlan::propose_reporting(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("b.html"),
                content: b"<p>hi</p>\n".to_vec(),
            },
            EditOperation::Modify {
                path: relative("c.html"),
                content: b"new\n".to_vec(),
            },
        ],
    )
    .expect("the file that already landed is skipped");

    assert_eq!(unchanged, vec!["b.html".to_owned()]);
    assert_eq!(plan.proposed_hashes().len(), 1);
}

#[test]
fn a_create_over_a_different_existing_file_is_still_refused() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("b.html"), b"something else\n").expect("seed file");
    let trusted_root = root(dir.path());

    let error = MutationPlan::propose_reporting(
        &trusted_root,
        vec![EditOperation::Create {
            path: relative("b.html"),
            content: b"<p>hi</p>\n".to_vec(),
        }],
    )
    .expect_err("a different file is in the way");

    assert!(matches!(error, PatchError::CreateTargetExists(_)));
}

#[test]
fn a_rename_that_only_changes_case_is_one_operation_not_a_clash() {
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("page.html"), b"x\n").expect("seed file");
    let trusted_root = root(dir.path());

    let result = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Rename {
            from: relative("page.html"),
            to: relative("Page.html"),
        }],
    );

    assert!(
        !matches!(result, Err(PatchError::DuplicatePathTarget(_))),
        "{result:?}"
    );
}
