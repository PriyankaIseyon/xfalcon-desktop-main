//! Platform-specific conformance tests for the atomic writer
//! (`crate::projects::writer`), mirroring
//! `tests::platform_conformance`'s "gated per platform, real fixtures, not
//! mocked" convention. See that module's doc comment and
//! `docs/decisions.md` for
//! which sections are verified by actually running on that OS versus
//! compiled-and-traced only in this Linux-hosted-but-Windows-targeting
//! sandbox.

use std::fs;

use tempfile::tempdir;

use crate::projects::{
    execute, ApprovalProof, EditOperation, MutationPlan, ProjectId, TrustedRoot,
};

fn relative(raw: &str) -> crate::projects::RelativeProjectPath {
    crate::projects::RelativeProjectPath::new(raw).expect("valid relative path")
}

fn root(dir: &std::path::Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), dir).expect("establish trusted root")
}

// ---------------------------------------------------------------------------
// Unix: the real `fsync(2)`-on-a-directory-fd primitive
// (`super::platform::fsync_dir_after_replace`, via `rustix::fs::fsync`).
// Executed for real in this sandbox (Unix is also this sandbox's build
// host for non-Windows-gated code -- unlike the `#[cfg(windows)]` section
// below, which is compiled against the real Windows target here but cannot
// be executed in this Linux-hosted sandbox).
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn fsyncing_a_real_directory_after_a_completed_rename_succeeds() {
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

    // `execute` itself calls `WriteTarget::fsync_parent` ->
    // `platform::fsync_dir_after_replace` -> `rustix::fs::fsync` after the
    // rename lands; a successful `execute` return is therefore already
    // proof this real syscall path compiled and ran without error on this
    // Unix host -- see the module doc comment on why this is the one
    // section of this file actually *executed*, not merely compiled.
    let approval = ApprovalProof::new(plan.plan_id(), false);
    execute(dir.path(), &plan, &approval, 0)
        .expect("execute, including the parent-directory fsync");
    assert!(dir.path().join("new.txt").exists());
}

#[cfg(unix)]
#[test]
fn fsyncing_the_parent_directory_after_a_create_and_after_a_delete_both_succeed() {
    // Exercises `fsync_dir_after_replace` from the two other `apply` call
    // sites (`Staged::Replace`'s create path and `Staged::Delete`), not just
    // the rename path the previous test already covers -- all three are the
    // same underlying `rustix::fs::fsync` call, but this proves each of
    // `execute`'s three call sites into it actually reaches a real,
    // still-open, still-valid directory descriptor at the point it calls it.
    let dir = tempdir().expect("root");
    fs::write(dir.path().join("gone.txt"), b"bye").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![
            EditOperation::Create {
                path: relative("created.txt"),
                content: b"fresh".to_vec(),
            },
            EditOperation::Delete {
                path: relative("gone.txt"),
            },
        ],
    )
    .expect("propose");

    let approval = ApprovalProof::new(plan.plan_id(), true);
    execute(dir.path(), &plan, &approval, 0)
        .expect("execute, including fsync after both the create and the delete");
    assert!(dir.path().join("created.txt").exists());
    assert!(!dir.path().join("gone.txt").exists());
}

// ---------------------------------------------------------------------------
// Windows-only fixtures. Disclosed limitation, matching
// `tests::platform_conformance`'s own Windows section: compiled cleanly
// against the real `x86_64-pc-windows-msvc` target this sandbox's toolchain
// targets, and traced manually, but NOT executed on real Windows here.
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn a_file_opened_exclusively_without_share_delete_causes_a_clean_sharing_violation_not_a_corrupt_write(
) {
    // Windows' rename/replace (unlike Unix's `rename(2)`, which never cares
    // whether another process has the file open) requires the ability to
    // open the destination for delete access; a handle held open with
    // `dwShareMode = 0` (no sharing at all, not even `FILE_SHARE_DELETE`)
    // makes that fail with `ERROR_SHARING_VIOLATION`. This proves `execute`
    // surfaces that as a clean, typed error -- never a partial/corrupt
    // write, and the original content survives untouched -- exactly the
    // "locked ... files ... must fail cleanly" and "sharing-safe atomic
    // replace" requirements.
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    };

    let dir = tempdir().expect("root");
    let target_path = dir.path().join("locked.txt");
    fs::write(&target_path, b"original").expect("seed");
    let trusted_root = root(dir.path());
    let plan = MutationPlan::propose(
        &trusted_root,
        vec![EditOperation::Modify {
            path: relative("locked.txt"),
            content: b"attempted replacement".to_vec(),
        }],
    )
    .expect("propose");

    let wide_path: Vec<u16> = OsStr::new(&target_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide_path` is a valid, NUL-terminated UTF-16 string live for
    // the duration of this call; every other argument is a plain value with
    // no aliasing/lifetime requirement of its own. `dwShareMode = 0` is the
    // deliberate point of this test -- no sharing granted at all.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(
        handle, INVALID_HANDLE_VALUE,
        "failed to open the fixture file exclusively"
    );

    let approval = ApprovalProof::new(plan.plan_id(), false);
    let result = execute(dir.path(), &plan, &approval, 0);

    // SAFETY: `handle` is the valid handle returned by `CreateFileW` above,
    // not yet closed.
    unsafe {
        CloseHandle(handle);
    }

    let failure = result.expect_err(
        "a rename over an exclusively-opened destination must fail, never silently succeed with a torn write",
    );
    // `Locked`, not `Containment`: another process holding the file is an
    // ordinary, retryable condition, and reporting it as a containment
    // failure would make a file lock indistinguishable from a path escaping
    // the trusted root in the audit trail. Not `Replace` either — the
    // exclusive handle is taken before `execute` runs, so the violation
    // surfaces during pre-flight resolution, long before any atomic replace
    // is attempted.
    assert!(
        matches!(failure.error, crate::projects::WriterError::Locked(_)),
        "expected the sharing violation to surface as WriterError::Locked, got {:?}",
        failure.error
    );
    assert_eq!(
        fs::read(&target_path).expect("read back the original file"),
        b"original",
        "the original content must survive a sharing-violation failure untouched"
    );

    // Once the exclusive handle is closed, the identical plan (a fresh
    // approval for the same plan id) succeeds normally.
    let approval = ApprovalProof::new(plan.plan_id(), false);
    execute(dir.path(), &plan, &approval, 0).expect("execute succeeds once the handle is closed");
    assert_eq!(
        fs::read(&target_path).expect("read back"),
        b"attempted replacement"
    );
}

#[cfg(windows)]
#[test]
fn a_reserved_device_name_destination_is_rejected_before_any_windows_api_call() {
    // Mirrors `tests::platform_conformance`'s identical Windows reserved-name
    // assertion, re-confirmed specifically through the mutation-plan
    // proposal path (`RelativeProjectPath::new` is the actual chokepoint
    // both paths share).
    assert!(relative_checked("CON").is_none());
    assert!(relative_checked("nul.txt").is_none());
}

#[cfg(windows)]
fn relative_checked(raw: &str) -> Option<crate::projects::RelativeProjectPath> {
    crate::projects::RelativeProjectPath::new(raw).ok()
}
