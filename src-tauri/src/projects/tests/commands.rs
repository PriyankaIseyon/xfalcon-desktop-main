//! Coverage for `super::super::commands`: the wire projection of a directory
//! listing and a file read, and every way both refuse.
//!
//! Fixtures build a real [`ProjectsService`] over a temporary app root with a
//! bound account, then trust a real temporary project directory through
//! `ProjectsService::create` — the same harness shape `super::super::trust`'s
//! own service tests use, and the only one that can exercise "an unknown
//! project id is refused" honestly (a bare `TrustedRoot` fixture has no
//! project registry to be absent from). The `#[tauri::command]` wrappers
//! themselves are two-line delegations to the functions under test here; what
//! they add is `AppRuntimeState` lookup and `to_string()` on the error, which
//! needs a running Tauri app to exercise and carries no policy.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::tempdir;
use xfalcon_contracts::projects::{
    ProjectEntryKind, ProjectFileMetadataReason, ProjectFileStat, ProjectSensitiveClass,
};

use crate::persistence::InMemorySecretStore;
use crate::projects::commands::{
    list_project_directory, read_project_document, read_project_file_bytes, stat_project_file,
    ProjectFileError,
};
use crate::projects::viewer::ViewerError;
use crate::projects::{
    ContainmentError, PathPolicyError, ProjectsService, ProjectsServiceError, ReadError,
};
use crate::security::account_cleanup::CurrentAccountScope;

const ACCOUNT_ID: &str = "account-1";
const PRODUCER_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.docx");

fn service(app_root: &Path) -> ProjectsService {
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT_ID, [0x33; 32])
        .expect("account key");
    let accounts = Arc::new(CurrentAccountScope::default());
    accounts.bind_native(ACCOUNT_ID).expect("bind account");
    ProjectsService::new(app_root.to_path_buf(), accounts, secrets)
}

/// Trusts `project_root` for the bound account and returns its id as the wire
/// string both commands take.
async fn trusted_project(service: &ProjectsService, project_root: &Path) -> String {
    service
        .create(&project_root.to_string_lossy(), None)
        .await
        .expect("trust the fixture project")
        .id
        .as_str()
        .to_owned()
}

const ESCAPING_SYMLINK_NAME: &str = "escape.txt";

/// Plants a symlink named [`ESCAPING_SYMLINK_NAME`] inside `project_root`
/// pointing at `target` outside it, and reports whether it exists.
///
/// Kept platform-split rather than `#[cfg(unix)]`-gating the two tests that
/// use it: both behaviours (a symlink is *listed* as a symlink; a symlink is
/// *refused* on read) are containment claims that have to hold on the target
/// the exit gate actually runs on, and a Unix-only test proves nothing about
/// the Windows build.
#[cfg(unix)]
fn plant_escaping_symlink(project_root: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(target, project_root.join(ESCAPING_SYMLINK_NAME))
        .expect("escaping symlink");
    true
}

/// Windows counterpart, using `symlink_file` as the reparse-point stand-in and
/// borrowing `super::platform_conformance`'s skip contract verbatim: creating
/// one needs `SeCreateSymbolicLinkPrivilege` (Developer Mode or an elevated
/// process), and a *security* test that fails for an environment reason trains
/// people to ignore a red suite. So it skips loudly when the privilege is
/// genuinely absent, refuses to skip under
/// `XFALCON_REQUIRE_PRIVILEGED_TESTS=1`, and treats every other error as a
/// hard failure.
#[cfg(windows)]
fn plant_escaping_symlink(project_root: &Path, target: &Path) -> bool {
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
    match std::os::windows::fs::symlink_file(target, project_root.join(ESCAPING_SYMLINK_NAME)) {
        Ok(()) => true,
        Err(error) => {
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_PRIVILEGE_NOT_HELD),
                "creating the fixture reparse point failed for an unexpected reason: {error}"
            );
            assert!(
                std::env::var("XFALCON_REQUIRE_PRIVILEGED_TESTS").as_deref() != Ok("1"),
                "XFALCON_REQUIRE_PRIVILEGED_TESTS=1 but SeCreateSymbolicLinkPrivilege is not \
                 held — enable Developer Mode or run elevated; this containment check must not \
                 be skipped in a conformance run"
            );
            eprintln!(
                "SKIPPED a symlink fixture: SeCreateSymbolicLinkPrivilege not held (enable \
                 Windows Developer Mode to run it)"
            );
            false
        }
    }
}

// ── Directory listing ─────────────────────────────────────────────────────

#[tokio::test]
async fn listing_with_no_path_projects_the_trusted_roots_children() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("notes.txt"), b"notes").expect("fixture file");
    fs::write(project_root.path().join(".env"), b"SECRET=1").expect("fixture file");
    fs::create_dir(project_root.path().join("subdir")).expect("fixture subdir");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let listing =
        list_project_directory(&service, &project_id, None).expect("listing the root succeeds");

    assert_eq!(listing.project_id.as_str(), project_id);
    assert!(
        listing.path.is_none(),
        "the root's listing must report no relative path, not an empty string"
    );
    let notes = listing
        .entries
        .iter()
        .find(|entry| entry.name == "notes.txt")
        .expect("notes.txt listed");
    assert_eq!(notes.kind, ProjectEntryKind::File);
    assert_eq!(notes.size_bytes, 5);
    assert!(!notes.is_sensitive);

    let env = listing
        .entries
        .iter()
        .find(|entry| entry.name == ".env")
        .expect(".env listed");
    assert!(
        env.is_sensitive,
        "a sensitive child must be flagged in the listing so the UI can mark it"
    );

    let subdir = listing
        .entries
        .iter()
        .find(|entry| entry.name == "subdir")
        .expect("subdir listed");
    assert_eq!(subdir.kind, ProjectEntryKind::Directory);
}

#[tokio::test]
async fn listing_a_named_subdirectory_echoes_the_path_it_listed() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::create_dir(project_root.path().join("subdir")).expect("fixture subdir");
    fs::write(project_root.path().join("subdir/inner.txt"), b"inner").expect("fixture file");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let listing = list_project_directory(&service, &project_id, Some("subdir"))
        .expect("listing a subdirectory succeeds");

    assert_eq!(listing.path.as_deref(), Some("subdir"));
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "inner.txt");
}

#[tokio::test]
async fn listing_an_empty_path_lists_the_root_exactly_as_the_agent_path_does() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("notes.txt"), b"notes").expect("fixture file");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let listing = list_project_directory(&service, &project_id, Some(""))
        .expect("an empty path is the root, not an error");

    assert!(listing.path.is_none());
    assert_eq!(listing.entries.len(), 1);
}

#[tokio::test]
async fn listing_refuses_a_parent_traversal_path() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let error = list_project_directory(&service, &project_id, Some("../outside"))
        .expect_err("`..` must be refused, never listed");

    assert!(matches!(
        error,
        ProjectFileError::Path(PathPolicyError::ParentTraversal)
    ));
}

#[tokio::test]
async fn listing_a_missing_directory_fails_instead_of_returning_no_entries() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let error = list_project_directory(&service, &project_id, Some("nope"))
        .expect_err("a missing directory must fail loudly");

    assert!(matches!(
        error,
        ProjectFileError::Read(ReadError::Containment(ContainmentError::NotFound))
    ));
}

#[tokio::test]
async fn listing_reports_a_symlink_child_as_a_symlink_rather_than_following_or_dropping_it() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let outside = tempdir().expect("directory outside the root");
    fs::write(outside.path().join("secret.txt"), b"outside").expect("fixture file");
    if !plant_escaping_symlink(project_root.path(), &outside.path().join("secret.txt")) {
        return;
    }
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let listing = list_project_directory(&service, &project_id, None).expect("listing succeeds");

    let escape = listing
        .entries
        .iter()
        .find(|entry| entry.name == ESCAPING_SYMLINK_NAME)
        .expect("a symlink child must be listed, not silently dropped");
    assert_eq!(
        escape.kind,
        ProjectEntryKind::Symlink,
        "the entry must be reported as what it is -- neither resolved to its target's kind nor \
         omitted from the listing"
    );
}

// ── File view: stat and ranges ─────────────────────────────────────────────

#[tokio::test]
async fn a_text_file_stats_as_text_and_its_bytes_read_back_exactly() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let content = b"line one\nline two\nline three";
    fs::write(project_root.path().join("notes.txt"), content).expect("fixture file");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let ProjectFileStat::Text {
        path, size_bytes, ..
    } = stat_project_file(&service, &project_id, "notes.txt").expect("stat succeeds")
    else {
        panic!("expected a text stat");
    };
    assert_eq!(path, "notes.txt");
    assert_eq!(size_bytes, content.len() as u64);

    let bytes = read_project_file_bytes(&service, &project_id, "notes.txt", 5, 3).expect("range");
    assert_eq!(bytes, b"one");
}

#[tokio::test]
async fn project_document_read_returns_bounded_text_and_refuses_sensitive_or_invalid_office() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("brief.docx"), PRODUCER_DOCX).expect("DOCX fixture");
    fs::write(project_root.path().join("broken.pptx"), b"invalid").expect("bad fixture");
    fs::write(project_root.path().join(".env.docx"), PRODUCER_DOCX).expect("sensitive fixture");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let document =
        read_project_document(&service, &project_id, "brief.docx").expect("document read");
    assert_eq!(document.path, "brief.docx");
    assert_eq!(document.size_bytes, PRODUCER_DOCX.len() as u64);
    assert!(document.modified_ms.is_some());
    assert!(document.text.contains("Producer DOCX fixture"));
    assert!(matches!(
        read_project_document(&service, &project_id, ".env.docx"),
        Err(ProjectFileError::Viewer(ViewerError::Sensitive))
    ));
    assert!(matches!(
        read_project_document(&service, &project_id, "broken.pptx"),
        Err(ProjectFileError::Viewer(ViewerError::Office(_)))
    ));
    let outside_project = read_project_document(&service, &project_id, "../brief.docx");
    assert!(matches!(outside_project, Err(ProjectFileError::Path(_))));
}

#[tokio::test]
async fn a_sensitive_file_is_withheld_with_its_class_and_nothing_else() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join(".env"), b"SECRET=top-secret-value").expect("fixture file");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    // Matching the variant *exhaustively* is the assertion: the withheld arm
    // carries a class and a path and structurally cannot carry content, a
    // hash, a size, or a time, so a future field that leaked any of those
    // would fail to compile here rather than ship.
    let ProjectFileStat::SensitiveWithheld {
        version: _,
        path,
        sensitive_class,
    } = stat_project_file(&service, &project_id, ".env").expect("the call succeeds")
    else {
        panic!("a classified-sensitive file must never stat as readable");
    };
    assert_eq!(path, ".env");
    assert_eq!(sensitive_class, ProjectSensitiveClass::DotEnv);
    assert!(matches!(
        read_project_file_bytes(&service, &project_id, ".env", 0, 4),
        Err(ProjectFileError::Viewer(ViewerError::Sensitive))
    ));
}

#[tokio::test]
async fn a_binary_file_stats_as_not_text_with_its_size() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    fs::write(project_root.path().join("blob.bin"), [0u8, 1, 2, 0, 3, 4]).expect("fixture file");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    let ProjectFileStat::NotText {
        path,
        size_bytes,
        reason,
        ..
    } = stat_project_file(&service, &project_id, "blob.bin").expect("the call succeeds")
    else {
        panic!("a binary file must not stat as text");
    };
    assert_eq!(path, "blob.bin");
    assert_eq!(size_bytes, 6);
    assert_eq!(reason, ProjectFileMetadataReason::Binary);
}

#[tokio::test]
async fn a_parent_traversal_or_absolute_path_is_refused_by_stat_and_by_range() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    assert!(matches!(
        stat_project_file(&service, &project_id, "../outside.txt").expect_err("`..` refused"),
        ProjectFileError::Path(PathPolicyError::ParentTraversal)
    ));
    assert!(matches!(
        read_project_file_bytes(&service, &project_id, "../outside.txt", 0, 1)
            .expect_err("`..` refused"),
        ProjectFileError::Path(PathPolicyError::ParentTraversal)
    ));
    for absolute in ["/etc/passwd", "C:/Windows/win.ini", "~/.ssh/id_rsa"] {
        assert!(matches!(
            stat_project_file(&service, &project_id, absolute)
                .expect_err("an absolute path must be refused whichever platform's form it takes"),
            ProjectFileError::Path(PathPolicyError::Absolute)
        ));
    }
}

#[tokio::test]
async fn a_symlink_that_escapes_the_trusted_root_is_refused_not_followed() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let outside = tempdir().expect("directory outside the root");
    fs::write(outside.path().join("secret.txt"), b"outside content").expect("fixture file");
    if !plant_escaping_symlink(project_root.path(), &outside.path().join("secret.txt")) {
        return;
    }
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    assert!(matches!(
        stat_project_file(&service, &project_id, ESCAPING_SYMLINK_NAME)
            .expect_err("a symlink out of the root must be refused"),
        ProjectFileError::Viewer(ViewerError::Containment(ContainmentError::LinkOrReparse))
    ));
    assert!(matches!(
        read_project_file_bytes(&service, &project_id, ESCAPING_SYMLINK_NAME, 0, 8)
            .expect_err("a symlink out of the root must be refused"),
        ProjectFileError::Viewer(ViewerError::Containment(ContainmentError::LinkOrReparse))
    ));
}

// ── Project resolution ────────────────────────────────────────────────────

#[tokio::test]
async fn an_unknown_project_id_is_refused_by_every_command() {
    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let service = service(app_root.path());
    // A well-formed id that was never trusted by this account — the shape a
    // stale UI reference or another account's project takes.
    let unknown = crate::projects::ProjectId::new().to_string();
    trusted_project(&service, project_root.path()).await;

    assert!(matches!(
        list_project_directory(&service, &unknown, None)
            .expect_err("an untrusted project must not be listable"),
        ProjectFileError::Project(ProjectsServiceError::NotFound)
    ));
    assert!(matches!(
        stat_project_file(&service, &unknown, "notes.txt")
            .expect_err("an untrusted project must not be readable"),
        ProjectFileError::Project(ProjectsServiceError::NotFound)
    ));
    assert!(matches!(
        read_project_file_bytes(&service, &unknown, "notes.txt", 0, 1)
            .expect_err("an untrusted project must not be readable"),
        ProjectFileError::Project(ProjectsServiceError::NotFound)
    ));
}

#[tokio::test]
async fn a_malformed_project_id_is_refused_before_any_filesystem_access() {
    let app_root = tempdir().expect("app root");
    let service = service(app_root.path());

    assert!(matches!(
        list_project_directory(&service, "not-a-uuid", None).expect_err("malformed id refused"),
        ProjectFileError::InvalidProjectId
    ));
    assert!(matches!(
        stat_project_file(&service, "not-a-uuid", "notes.txt").expect_err("malformed id refused"),
        ProjectFileError::InvalidProjectId
    ));
}

#[tokio::test]
async fn project_schema_summary_recursive_discovery_and_bounds() {
    use crate::projects::commands::get_project_schema_summary;
    use rusqlite::Connection;

    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    // 1. Nested CSV
    let nested_dir = project_root.path().join("data").join("nested");
    fs::create_dir_all(&nested_dir).expect("create nested dirs");
    fs::write(
        nested_dir.join("analytics.csv"),
        "session_id,duration,active\n101,45.2,true\n102,12.0,false\n",
    )
    .expect("write nested csv");

    // 2. Sensitive file (credentials.json) - valid JSON format that classifier flags; must be withheld
    fs::write(
        project_root.path().join("credentials.json"),
        r#"[{"api_key": "sk-12345", "service": "aws"}]"#,
    )
    .expect("write sensitive credentials.json");

    // 3. Oversized CSV (> 1 MiB) - must be skipped by size cap
    let large_dir = project_root.path().join("large");
    fs::create_dir_all(&large_dir).expect("large dir");
    let mut large_content = String::with_capacity(1_500_000);
    large_content.push_str("col1,col2\n");
    for _ in 0..60_000 {
        large_content.push_str("1234567890,abcdefghij\n");
    }
    fs::write(large_dir.join("too_big.csv"), large_content).expect("write large csv");

    // 4. SQLite DB
    let db_path = project_root.path().join("data").join("app.sqlite");
    {
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch(
            "CREATE TABLE metrics (id INTEGER PRIMARY KEY, metric_name TEXT, val REAL);
             INSERT INTO metrics (id, metric_name, val) VALUES (1, 'cpu', 82.5);",
        )
        .expect("seed sqlite");
    }

    let summary = get_project_schema_summary(&service, &project_id).expect("schema summary");
    assert_eq!(summary.project_id.as_str(), project_id.as_str());

    // Assert nested CSV found
    let analytics_table = summary
        .tables
        .iter()
        .find(|t| t.name == "analytics")
        .expect("nested analytics.csv table schema");
    assert_eq!(analytics_table.format, "csv");
    assert_eq!(analytics_table.columns.len(), 3);
    assert_eq!(analytics_table.columns[0].name, "session_id");
    assert_eq!(analytics_table.columns[0].inferred_type, "integer");

    // Assert SQLite table found
    let metrics_table = summary
        .tables
        .iter()
        .find(|t| t.name == "metrics")
        .expect("metrics sqlite table");
    assert_eq!(metrics_table.format, "sqlite");
    assert_eq!(metrics_table.columns.len(), 3);
    assert_eq!(metrics_table.columns[1].name, "metric_name");
    assert_eq!(metrics_table.columns[1].inferred_type, "text");

    // Assert oversized CSV skipped
    assert!(summary.tables.iter().all(|t| t.name != "too_big"));

    // Assert sensitive credentials.json not present (even though it's valid JSON)
    assert!(summary.tables.iter().all(|t| t.name != "credentials"));
}

#[tokio::test]
async fn project_schema_summary_refuses_escaping_symlink_sqlite() {
    use crate::projects::commands::get_project_schema_summary;
    use rusqlite::Connection;

    let app_root = tempdir().expect("app root");
    let project_root = tempdir().expect("project root");
    let outside_dir = tempdir().expect("outside root");
    let service = service(app_root.path());
    let project_id = trusted_project(&service, project_root.path()).await;

    // Create outside SQLite DB
    let outside_db = outside_dir.path().join("external.sqlite");
    {
        let conn = Connection::open(&outside_db).expect("open outside sqlite");
        conn.execute_batch(
            "CREATE TABLE secret_external (id INTEGER PRIMARY KEY, confidential TEXT);
             INSERT INTO secret_external (id, confidential) VALUES (1, 'leaked');",
        )
        .expect("seed outside sqlite");
    }

    if !plant_escaping_symlink(project_root.path(), &outside_db) {
        return;
    }

    let summary = get_project_schema_summary(&service, &project_id).expect("schema summary");
    // The escaping symlink must NOT be resolved or included in summary
    assert!(summary.tables.iter().all(|t| t.name != "secret_external"));
    assert!(summary.tables.iter().all(|t| t.name != "escape"));
}
