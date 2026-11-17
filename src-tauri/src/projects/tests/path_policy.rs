//! Adversarial containment tests shared across platforms: `..` traversal,
//! absolute-path injection, separator confusion, malformed/oversized names,
//! new-file parent resolution, renamed/replaced roots after trust, and
//! (where detectable on this build's OS) hard links and permissions.
//! Windows/macOS-only fixture forms (UNC, junctions, ADS, reserved device
//! names as real files, case-folding, Unicode normalization) live in
//! `platform_conformance.rs`.

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use crate::projects::{
    ContainmentError, PathPolicyError, ProjectId, RelativeProjectPath, TrustedRoot,
};

fn rejects(raw: &str, expected: PathPolicyError) {
    assert_eq!(
        RelativeProjectPath::new(raw).expect_err(raw),
        expected,
        "input: {raw:?}"
    );
}

#[test]
fn empty_path_is_rejected() {
    rejects("", PathPolicyError::Empty);
}

#[test]
fn parent_traversal_is_rejected_in_every_position() {
    for raw in [
        "..",
        "../etc/passwd",
        "a/../../etc/passwd",
        "a/b/..",
        "./..",
    ] {
        assert!(
            RelativeProjectPath::new(raw).is_err(),
            "traversal input accepted: {raw:?}"
        );
    }
    rejects("..", PathPolicyError::ParentTraversal);
    rejects("a/../../etc/passwd", PathPolicyError::ParentTraversal);
}

#[test]
fn bare_dot_component_is_rejected() {
    rejects(".", PathPolicyError::DotComponent);
    rejects("./a", PathPolicyError::DotComponent);
}

#[test]
fn absolute_unix_path_is_rejected() {
    rejects("/etc/passwd", PathPolicyError::Absolute);
}

#[test]
fn absolute_windows_drive_letter_path_is_rejected() {
    rejects("C:\\Windows\\System32", PathPolicyError::Absolute);
    rejects("C:/Windows", PathPolicyError::Absolute);
    rejects("C:", PathPolicyError::Absolute);
}

#[test]
fn unc_style_path_is_rejected() {
    rejects("\\\\server\\share\\file", PathPolicyError::Absolute);
}

#[test]
fn home_directory_shorthand_is_rejected() {
    rejects("~/secrets", PathPolicyError::Absolute);
}

#[test]
fn separator_confusion_is_rejected() {
    // Checked before `Path::components()` so the verdict is identical on
    // every target. Previously the check lived inside `validate_component`,
    // which runs *after* the split: on Windows `\` was consumed as a
    // separator, so this input reported `ParentTraversal` there and
    // `SeparatorConfusion` on Unix, and a benign `dir\file` was silently
    // accepted on Windows while Unix rejected it.
    rejects(
        "notes\\..\\..\\etc\\passwd",
        PathPolicyError::SeparatorConfusion,
    );
    rejects("dir\\file.txt", PathPolicyError::SeparatorConfusion);
}

/// The canonical constructor for a path produced by walking the filesystem:
/// it must yield one identical stored form regardless of the host's native
/// separator, or the same project tree indexes differently per platform.
#[test]
fn os_relative_paths_canonicalize_to_forward_slashes() {
    let from_components: std::path::PathBuf = ["readme_dir", "notes.txt"].iter().collect();

    let canonical =
        RelativeProjectPath::from_os_relative(&from_components).expect("valid relative path");

    assert_eq!(canonical.as_str(), "readme_dir/notes.txt");
}

#[test]
fn os_relative_rejects_traversal_and_absolute_forms() {
    assert_eq!(
        RelativeProjectPath::from_os_relative(Path::new("a/../b")).expect_err("traversal"),
        PathPolicyError::ParentTraversal
    );
    assert!(RelativeProjectPath::from_os_relative(Path::new("/etc/passwd")).is_err());
}

#[test]
fn nul_byte_is_rejected() {
    rejects("notes\0.txt", PathPolicyError::NulByte);
}

#[test]
fn reserved_windows_device_names_are_rejected_case_insensitively_and_with_extension() {
    for raw in [
        "CON", "con", "Con.txt", "NUL", "nul.log", "COM1", "com3.txt", "LPT9", "lpt2.dat",
    ] {
        assert_eq!(
            RelativeProjectPath::new(raw).expect_err(raw),
            PathPolicyError::ReservedDeviceName,
            "input: {raw:?}"
        );
    }
    // A name that merely *starts* with a reserved prefix is fine.
    assert!(RelativeProjectPath::new("CONFIG.txt").is_ok());
    assert!(RelativeProjectPath::new("console.log").is_ok());
}

#[test]
fn alternate_data_stream_marker_is_rejected() {
    rejects("notes.txt:hidden", PathPolicyError::AlternateDataStream);
    rejects("notes.txt:$DATA", PathPolicyError::AlternateDataStream);
}

#[test]
fn trailing_dot_or_space_is_rejected() {
    rejects("notes.txt.", PathPolicyError::TrailingDotOrSpace);
    rejects("notes.txt ", PathPolicyError::TrailingDotOrSpace);
}

#[test]
fn path_length_boundary_is_enforced() {
    // Built from multiple components on purpose: a single 4096-byte component
    // trips the 255-byte `MAX_COMPONENT_BYTES` ceiling long before the total
    // length is ever considered, so a one-component fixture could never
    // exercise this boundary. 17 components of 240 bytes plus 16 separators
    // is exactly 4096.
    let component = "a".repeat(240);
    let at_limit = vec![component.as_str(); 17].join("/");
    assert_eq!(at_limit.len(), 4096);
    assert!(RelativeProjectPath::new(&at_limit).is_ok());

    let over_limit = format!("{at_limit}a");
    assert_eq!(over_limit.len(), 4097);
    rejects(&over_limit, PathPolicyError::TooLong(4096));
}

#[test]
fn component_length_boundary_is_enforced() {
    let at_limit = "a".repeat(255);
    assert!(RelativeProjectPath::new(&at_limit).is_ok());
    let over_limit = "a".repeat(256);
    rejects(&over_limit, PathPolicyError::ComponentTooLong(255));
}

#[test]
fn component_count_boundary_is_enforced() {
    let at_limit = vec!["a"; 256].join("/");
    assert!(RelativeProjectPath::new(&at_limit).is_ok());
    let over_limit = vec!["a"; 257].join("/");
    rejects(&over_limit, PathPolicyError::TooManyComponents(256));
}

#[test]
fn well_formed_relative_paths_are_accepted() {
    for raw in [
        "notes.txt",
        "src/lib.rs",
        "a/b/c/d.txt",
        "data analysis.csv",
        "résumé.docx",
    ] {
        assert!(RelativeProjectPath::new(raw).is_ok(), "input: {raw:?}");
    }
}

fn trusted_root(root: &Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), root).expect("establish trusted root")
}

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

#[test]
fn resolve_existing_finds_a_real_file_in_root() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("notes.txt"), b"hello").expect("fixture file");
    let root = trusted_root(directory.path());

    let resolved = root
        .resolve_existing(relative("notes.txt"))
        .expect("existing file resolves");
    assert_eq!(resolved.relative().as_str(), "notes.txt");
    assert_eq!(resolved.project_id(), root.project_id());
}

#[test]
fn resolve_existing_rejects_a_missing_path() {
    let directory = tempdir().expect("fixture root");
    let root = trusted_root(directory.path());

    let error = root
        .resolve_existing(relative("does-not-exist.txt"))
        .expect_err("missing path must be rejected");
    assert!(matches!(error, ContainmentError::NotFound));
}

#[test]
fn resolve_new_allows_a_not_yet_existing_file_with_an_in_root_parent() {
    let directory = tempdir().expect("fixture root");
    fs::create_dir(directory.path().join("subdir")).expect("fixture subdir");
    let root = trusted_root(directory.path());

    let resolved = root
        .resolve_new(relative("subdir/new-file.txt"))
        .expect("new file under an existing in-root parent resolves");
    assert_eq!(resolved.relative().as_str(), "subdir/new-file.txt");
}

#[test]
fn resolve_new_accepts_a_parent_directory_that_does_not_exist_yet() {
    let directory = tempdir().expect("fixture root");
    let root = trusted_root(directory.path());

    // Writing a new file into a new directory is an ordinary create; the
    // directories themselves are only materialized by the writer.
    let resolved = root
        .resolve_new(relative("missing-subdir/new-file.txt"))
        .expect("a new file under a not-yet-existing parent is a creatable location");
    assert_eq!(resolved.relative().as_str(), "missing-subdir/new-file.txt");
    assert!(!directory.path().join("missing-subdir").exists());
}

#[cfg(unix)]
#[test]
fn resolve_new_still_rejects_a_link_on_the_part_of_the_path_that_exists() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    let outside = tempdir().expect("outside target");
    symlink(outside.path(), directory.path().join("link")).expect("escaping symlink");
    let root = trusted_root(directory.path());

    // Allowing a missing tail must not weaken the check on the components that
    // do exist: the link is still where the walk stops.
    let error = root
        .resolve_new(relative("link/deeper/new-file.txt"))
        .expect_err("a link on the existing prefix must still fail closed");
    assert!(matches!(error, ContainmentError::LinkOrReparse));
}

#[test]
fn resolve_new_permits_an_already_occupied_plain_file_name() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("existing.txt"), b"old").expect("fixture file");
    let root = trusted_root(directory.path());

    // `resolve_new` on an already-occupied *plain* file is allowed --
    // overwrite-or-not is a Task 6 (patch/writer) policy decision, not this
    // API's job; it only guarantees the name is safe to act on.
    let resolved = root
        .resolve_new(relative("existing.txt"))
        .expect("occupied plain file still resolves for `resolve_new`");
    assert_eq!(resolved.relative().as_str(), "existing.txt");
}

#[test]
fn root_identity_survives_a_pure_rename_of_an_unrelated_sibling() {
    // Sanity check that identity comparison is about the *root itself*, not
    // a hash of directory listing contents.
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("a.txt"), b"a").expect("fixture file");
    let root = trusted_root(directory.path());
    fs::rename(
        directory.path().join("a.txt"),
        directory.path().join("b.txt"),
    )
    .expect("sibling rename");

    let resolved = root.resolve_existing(relative("b.txt"));
    assert!(resolved.is_ok());
}

/// Windows and macOS only, deliberately.
///
/// The guarantee under test is that a root deleted and recreated at the same
/// path fails identity revalidation. Windows gets that from
/// `(volume_serial_number, file_index)` and macOS from
/// `(dev, ino, birth_time)` on APFS, whose inode numbers are not immediately
/// reused.
///
/// Linux cannot promise it. The inode IS reused straight after `rmdir` —
/// reproduced locally on ext4, same inode both times — so the whole
/// distinction rests on `statx`'s `STATX_BTIME`. `super::super::platform`
/// already refuses to build an identity at all when the filesystem does not
/// report birth time, but where it is reported with coarse granularity a
/// delete-and-recreate inside one tick is indistinguishable. That is a real
/// residual, recorded in `docs/threat-model.md` TM-007 and
/// `docs/compliance.md` CM-013, not something to assert here and have fail on
/// the `unix-rust` job.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn reopen_rejects_a_root_deleted_and_recreated_at_the_same_path() {
    let directory = tempdir().expect("fixture root");
    let established = trusted_root(directory.path());
    let (project_id, canonical_path, identity) = (
        *established.project_id(),
        established.canonical_path().to_path_buf(),
        established.identity(),
    );
    drop(established);

    fs::remove_dir(&canonical_path).expect("remove original root");
    fs::create_dir(&canonical_path).expect("recreate root at the same path");

    let error = TrustedRoot::reopen(project_id, canonical_path, identity)
        .expect_err("a recreated root must fail identity revalidation");
    assert!(matches!(error, ContainmentError::RootIdentityChanged));
}

#[test]
fn reopen_succeeds_for_an_untouched_root_with_the_recorded_identity() {
    let directory = tempdir().expect("fixture root");
    let established = trusted_root(directory.path());
    let (project_id, canonical_path, identity) = (
        *established.project_id(),
        established.canonical_path().to_path_buf(),
        established.identity(),
    );
    drop(established);

    let reopened = TrustedRoot::reopen(project_id, canonical_path, identity);
    assert!(reopened.is_ok());
}

/// Proves revalidation happens on *every* call, not only once at
/// establish/reopen time: the root is swapped out from under an already-live
/// `TrustedRoot` handle, and the very next `resolve_existing` call (no new
/// `TrustedRoot` constructed) must still catch it.
#[cfg(unix)]
#[test]
fn resolve_existing_revalidates_identity_on_every_call_not_only_at_open_time() {
    let directory = tempdir().expect("fixture root");
    let root = trusted_root(directory.path());

    fs::remove_dir(directory.path()).expect("remove trusted root out from under the live handle");
    fs::create_dir(directory.path()).expect("recreate at the same path with a new identity");
    fs::write(
        directory.path().join("notes.txt"),
        b"new content after swap",
    )
    .expect("fixture file in the replacement root");

    let error = root
        .resolve_existing(relative("notes.txt"))
        .expect_err("a root swapped out from under a live handle must be rejected");
    assert!(matches!(error, ContainmentError::RootIdentityChanged));
}

#[cfg(unix)]
#[test]
fn resolve_existing_rejects_a_symlink_escaping_the_root() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    let outside = tempdir().expect("outside target");
    fs::write(outside.path().join("secret.txt"), b"outside").expect("outside file");
    symlink(
        outside.path().join("secret.txt"),
        directory.path().join("escape.txt"),
    )
    .expect("escaping symlink");
    let root = trusted_root(directory.path());

    let error = root
        .resolve_existing(relative("escape.txt"))
        .expect_err("an escaping symlink must be rejected");
    assert!(matches!(error, ContainmentError::LinkOrReparse));
}

#[cfg(unix)]
#[test]
fn resolve_existing_rejects_a_symlink_even_when_its_target_stays_inside_the_root() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("real.txt"), b"inside").expect("fixture file");
    symlink(
        directory.path().join("real.txt"),
        directory.path().join("alias.txt"),
    )
    .expect("internal symlink");
    let root = trusted_root(directory.path());

    // Deliberately stricter than "reject only escaping links": this
    // containment API's contract is that a resolved path is a genuine file
    // or directory, never indirection, even when the indirection happens to
    // stay inside the root -- see the decision record.
    let error = root
        .resolve_existing(relative("alias.txt"))
        .expect_err("an internal symlink must still be rejected");
    assert!(matches!(error, ContainmentError::LinkOrReparse));
}

#[cfg(unix)]
#[test]
fn resolve_existing_rejects_a_mid_path_symlink_even_when_the_final_component_is_a_plain_file() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    let real_subdir = directory.path().join("real-subdir");
    fs::create_dir(&real_subdir).expect("real subdir");
    fs::write(real_subdir.join("file.txt"), b"content").expect("file inside real subdir");
    symlink(&real_subdir, directory.path().join("linked-subdir")).expect("mid-path symlink");
    let root = trusted_root(directory.path());

    let error = root
        .resolve_existing(relative("linked-subdir/file.txt"))
        .expect_err("a mid-path symlink component must be rejected");
    assert!(matches!(error, ContainmentError::LinkOrReparse));
}

#[cfg(unix)]
#[test]
fn hard_link_inside_the_root_is_treated_as_an_ordinary_file() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("original.txt"), b"shared content").expect("fixture file");
    fs::hard_link(
        directory.path().join("original.txt"),
        directory.path().join("linked.txt"),
    )
    .expect("hard link");
    let root = trusted_root(directory.path());

    // A hard link is a second directory entry for the same inode, not path
    // indirection -- opening it is exactly as safe as opening any other
    // in-root regular file, so it must resolve normally.
    assert!(root.resolve_existing(relative("linked.txt")).is_ok());
    assert!(root.resolve_existing(relative("original.txt")).is_ok());
}

#[test]
fn equivalent_spellings_are_stored_as_one_canonical_path() {
    for raw in ["a/b.html", "a//b.html", "a/./b.html", "a/b.html/"] {
        assert_eq!(
            RelativeProjectPath::new(raw).expect("valid").as_str(),
            "a/b.html",
            "{raw}"
        );
    }
}
