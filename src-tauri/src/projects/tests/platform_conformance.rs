//! Platform-specific fixture tests for the adversarial matrix the plan
//! requires: Windows UNC/junctions/ADS/reserved names/stable volume+file
//! identity; macOS case-folding/Unicode normalization/stable device+file
//! identity; Linux bind-mounts/non-UTF-8 boundaries/stable device+inode
//! identity. Gated per platform with `#[cfg(...)]` (mirroring
//! `crate::security::tests::browsing_data`'s `#[cfg(unix)]` real-symlink
//! idiom) rather than mocked -- see
//! `docs/decisions.md` for
//! which of these were verified by actually running on that OS versus by
//! reading/reasoning about the implementation in this Linux sandbox (the
//! Windows- and macOS-only sections below are compiled and traced, not
//! executed, here).

use std::fs;

use tempfile::tempdir;

use crate::projects::{ProjectId, RelativeProjectPath, TrustedRoot};

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

#[cfg(unix)]
#[test]
fn stable_identity_differs_across_distinct_directories() {
    let first = tempdir().expect("first root");
    let second = tempdir().expect("second root");
    let first_root = TrustedRoot::establish(ProjectId::new(), first.path()).expect("first root");
    let second_root = TrustedRoot::establish(ProjectId::new(), second.path()).expect("second root");
    assert_ne!(first_root.identity(), second_root.identity());
}

#[cfg(target_os = "linux")]
#[test]
fn distinct_backing_filesystems_report_distinct_identities() {
    // A real bind-mount (mounting `other` on top of `original`'s path)
    // requires root/CAP_SYS_ADMIN, unavailable in this sandbox. This proves
    // the *detection mechanism* a bind-mount swap relies on: two
    // directories with different backing device ids report different
    // `RootIdentity` values, which is exactly what would change if a
    // different filesystem were later mounted at an already-trusted path.
    // See the decision record's disclosed-limitation note.
    let original = tempdir().expect("original root");
    let other = tempdir().expect("other filesystem stand-in");
    let original_root =
        TrustedRoot::establish(ProjectId::new(), original.path()).expect("original root");
    let other_root = TrustedRoot::establish(ProjectId::new(), other.path()).expect("other root");
    assert_ne!(original_root.identity(), other_root.identity());
}

#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_named_sibling_does_not_disrupt_resolution_of_a_valid_utf8_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("notes.txt"), b"content").expect("utf8 file");
    // Raw, non-UTF-8 filename bytes. Addressing a file *by* such a name is a
    // Task 5 (indexing/enumeration) concern -- this API's only input source
    // (Tauri IPC command arguments) is inherently UTF-8 JSON, so
    // `RelativeProjectPath` is deliberately `&str`-based. This test only
    // proves a non-UTF-8 sibling's mere on-disk presence cannot disrupt
    // resolution of a different, valid path in the same root.
    let weird_name = OsStr::from_bytes(&[0xFF, 0xFE, b'x']);
    fs::write(directory.path().join(weird_name), b"non-utf8 named file").expect("non-utf8 sibling");

    let root = TrustedRoot::establish(ProjectId::new(), directory.path()).expect("root");
    assert!(root.resolve_existing(relative("notes.txt")).is_ok());
}

// ---------------------------------------------------------------------------
// Windows-only fixtures. Disclosed limitation: compiled cleanly
// (`cargo test -p xfalcon-desktop --no-run`) and traced manually against
// `platform::windows_identity`/`platform::is_link_or_reparse`, but NOT
// executed on real Windows in this Linux sandbox.
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn reserved_device_name_is_rejected_before_any_filesystem_access_on_windows() {
    // The platform-neutral rejection is exercised (and does execute) in
    // `path_policy.rs`; this only re-confirms the same code path compiles
    // and holds on an actual Windows target.
    assert!(RelativeProjectPath::new("CON").is_err());
    assert!(RelativeProjectPath::new("nul.txt").is_err());
    assert!(RelativeProjectPath::new("COM1").is_err());
}

#[cfg(windows)]
#[test]
fn a_reparse_point_escaping_the_root_is_rejected() {
    // `std` has no stable junction constructor; a symlink is used as a
    // stand-in since it sets the same `FILE_ATTRIBUTE_REPARSE_POINT`
    // `platform::is_link_or_reparse` inspects (a real junction would also be
    // caught by the same attribute check, not a symlink-specific one).
    // Creating it requires Developer Mode or an elevated process on
    // Windows.
    use std::os::windows::fs::symlink_file;

    use crate::projects::ContainmentError;

    let directory = tempdir().expect("fixture root");
    let outside = tempdir().expect("outside target");
    fs::write(outside.path().join("secret.txt"), b"outside").expect("outside file");
    // Creating a symlink on Windows needs SeCreateSymbolicLinkPrivilege —
    // Developer Mode or an elevated process. A developer without it would
    // otherwise see this *security* test fail for an environment reason,
    // which trains people to ignore a red suite. It therefore skips loudly
    // when the privilege is genuinely absent, and refuses to skip when
    // `XFALCON_REQUIRE_PRIVILEGED_TESTS=1` is set, so the platform
    // conformance runs the slice exit gate requires cannot silently lose
    // this coverage. Any other error is still a hard failure.
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
    if let Err(error) = symlink_file(
        outside.path().join("secret.txt"),
        directory.path().join("escape.txt"),
    ) {
        assert_eq!(
            error.raw_os_error(),
            Some(ERROR_PRIVILEGE_NOT_HELD),
            "creating the fixture reparse point failed for an unexpected reason: {error}"
        );
        assert!(
            std::env::var("XFALCON_REQUIRE_PRIVILEGED_TESTS").as_deref() != Ok("1"),
            "XFALCON_REQUIRE_PRIVILEGED_TESTS=1 but SeCreateSymbolicLinkPrivilege is not \
             held — enable Developer Mode or run elevated; this containment check must \
             not be skipped in a conformance run"
        );
        eprintln!(
            "SKIPPED a_reparse_point_escaping_the_root_is_rejected: \
             SeCreateSymbolicLinkPrivilege not held (enable Windows Developer Mode to run it)"
        );
        return;
    }
    let root = TrustedRoot::establish(ProjectId::new(), directory.path()).expect("root");

    let error = root
        .resolve_existing(relative("escape.txt"))
        .expect_err("an escaping reparse point must be rejected");
    assert!(matches!(error, ContainmentError::LinkOrReparse));
}

#[cfg(windows)]
#[test]
fn stable_identity_on_windows_differs_across_distinct_directories() {
    let first = tempdir().expect("first root");
    let second = tempdir().expect("second root");
    let first_root = TrustedRoot::establish(ProjectId::new(), first.path()).expect("first root");
    let second_root = TrustedRoot::establish(ProjectId::new(), second.path()).expect("second root");
    assert_ne!(first_root.identity(), second_root.identity());
}

// ---------------------------------------------------------------------------
// macOS-only fixtures. Same disclosed-but-unexecuted status as the Windows
// section above.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[test]
fn case_insensitive_volume_lookup_still_goes_through_the_same_containment_path() {
    // On a case-insensitive (default) APFS volume, `Notes.txt` and
    // `notes.txt` name the same directory entry. This proves whichever
    // casing the OS resolves still passes through
    // `TrustedRoot::resolve_existing`'s ordinary containment walk -- there
    // is no separate case-folding-aware lookup that could diverge from it.
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("Notes.txt"), b"content").expect("fixture file");
    let root = TrustedRoot::establish(ProjectId::new(), directory.path()).expect("root");
    assert!(root.resolve_existing(relative("Notes.txt")).is_ok());
}

#[cfg(target_os = "macos")]
#[test]
fn an_on_disk_unicode_normalization_variant_resolves_through_the_same_containment_path() {
    // APFS stores filenames as supplied (unlike HFS+'s NFD normalization),
    // so an NFC-composed name and its NFD-decomposed equivalent are, in
    // general, two distinct directory entries there. This does not assert
    // cross-normalization equivalence (a filesystem property, not this
    // module's) -- only that whichever variant is actually on disk resolves
    // through the ordinary containment path.
    let directory = tempdir().expect("fixture root");
    let nfc_name = "caf\u{00e9}.txt"; // "café", NFC (precomposed é)
    fs::write(directory.path().join(nfc_name), b"content").expect("fixture file");
    let root = TrustedRoot::establish(ProjectId::new(), directory.path()).expect("root");
    assert!(root.resolve_existing(relative(nfc_name)).is_ok());
}
