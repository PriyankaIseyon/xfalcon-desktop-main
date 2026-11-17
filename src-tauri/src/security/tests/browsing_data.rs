use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

use tempfile::tempdir;

use crate::security::browsing_data::{
    acknowledge_webview_storage_migration, is_exact_main_page, is_exact_storage_migration_ack,
    marker_status, spawn_acknowledgement_timeout, MigrationMarkerStatus, StartupMigration,
    StorageOrigin, DEVELOPMENT_MIGRATION_MARKER, MAIN_PAGE, MIGRATION_PAGE,
    PACKAGED_MIGRATION_MARKER,
};

#[test]
fn acknowledged_migration_writes_a_valid_idempotent_marker() {
    let directory = tempdir().expect("app data");
    assert_eq!(
        marker_status(directory.path(), StorageOrigin::Packaged).expect("initial status"),
        MigrationMarkerStatus::Required
    );

    acknowledge_webview_storage_migration(directory.path(), StorageOrigin::Packaged)
        .expect("acknowledge");
    acknowledge_webview_storage_migration(directory.path(), StorageOrigin::Packaged)
        .expect("idempotent acknowledge");

    assert_eq!(
        marker_status(directory.path(), StorageOrigin::Packaged).expect("complete status"),
        MigrationMarkerStatus::Complete
    );
    assert!(directory.path().join(PACKAGED_MIGRATION_MARKER).is_file());
}

#[test]
fn marker_persistence_failure_is_reported() {
    let directory = tempdir().expect("app data");
    std::fs::write(directory.path().join("migrations"), b"not a directory")
        .expect("block marker directory");

    assert!(
        acknowledge_webview_storage_migration(directory.path(), StorageOrigin::Packaged).is_err()
    );
    assert!(!directory.path().join(PACKAGED_MIGRATION_MARKER).exists());
}

#[test]
fn acknowledgement_requires_the_exact_main_origin_path_and_query() {
    #[cfg(windows)]
    let packaged_ack = "http://tauri.localhost/index.html?xfalcon-storage-migration=v1";
    #[cfg(not(windows))]
    let packaged_ack = "tauri://localhost/index.html?xfalcon-storage-migration=v1";
    assert!(is_exact_storage_migration_ack(
        &packaged_ack.parse().expect("packaged acknowledgement"),
        StorageOrigin::Packaged
    ));
    assert!(is_exact_storage_migration_ack(
        &"http://localhost:1420/index.html?xfalcon-storage-migration=v1"
            .parse()
            .expect("development acknowledgement"),
        StorageOrigin::Development
    ));
    assert!(is_exact_storage_migration_ack(
        &"http://127.0.0.1:1420/index.html?xfalcon-storage-migration=v1"
            .parse()
            .expect("loopback development acknowledgement"),
        StorageOrigin::Development
    ));
    assert!(!is_exact_storage_migration_ack(
        &packaged_ack.parse().expect("packaged acknowledgement"),
        StorageOrigin::Development
    ));

    for url in [
        "tauri://foreign/index.html?xfalcon-storage-migration=v1",
        "https://tauri.localhost/index.html?xfalcon-storage-migration=v1",
        "tauri://localhost/storage-migration.html?xfalcon-storage-migration=v1",
        "tauri://localhost/index.html",
        "tauri://localhost/index.html?xfalcon-storage-migration=v1&extra=1",
        "tauri://localhost/index.html?extra=1&xfalcon-storage-migration=v1",
        "tauri://localhost/index.html?xfalcon-storage-migration=v2",
        "tauri://localhost/index.html?xfalcon-storage-migration=v1#fragment",
        "http://localhost:1420/index.html?xfalcon-storage-migration=v1",
        "http://127.0.0.1:1420/index.html?xfalcon-storage-migration=v1",
    ] {
        assert!(!is_exact_storage_migration_ack(
            &url.parse().expect("rejected acknowledgement"),
            StorageOrigin::Packaged
        ));
    }
}

#[test]
fn completed_marker_accepts_only_the_unadorned_main_page() {
    #[cfg(windows)]
    let packaged_main = "http://tauri.localhost/index.html";
    #[cfg(not(windows))]
    let packaged_main = "tauri://localhost/index.html";
    assert!(is_exact_main_page(
        &packaged_main.parse().expect("packaged main"),
        StorageOrigin::Packaged
    ));
    assert!(!is_exact_main_page(
        &packaged_main.parse().expect("packaged main"),
        StorageOrigin::Development
    ));
    assert!(is_exact_main_page(
        &"http://localhost:1420/index.html"
            .parse()
            .expect("development main"),
        StorageOrigin::Development
    ));
    assert!(is_exact_main_page(
        &"http://127.0.0.1:1420/"
            .parse()
            .expect("loopback development root"),
        StorageOrigin::Development
    ));
    assert!(!is_exact_main_page(
        &"http://localhost:1420/index.html"
            .parse()
            .expect("development main"),
        StorageOrigin::Packaged
    ));
    // The app root is the same document, and is what a window built from
    // `WebviewUrl::App("index.html")` actually reports: a real packaged run logs
    // `http://tauri.localhost/`. Rejecting it left the window hidden until the
    // startup deadline killed the app.
    #[cfg(windows)]
    let packaged_root = "http://tauri.localhost/";
    #[cfg(not(windows))]
    let packaged_root = "tauri://localhost/";
    assert!(is_exact_main_page(
        &packaged_root.parse().expect("packaged root"),
        StorageOrigin::Packaged
    ));
    // The same root without the trailing slash. This is the spelling macOS and
    // Linux actually deliver, and rejecting it killed every packaged macOS launch
    // after the first one. See `the_platform_app_base_url_is_a_recognized_entry_document`.
    #[cfg(windows)]
    let packaged_bare_root = "http://tauri.localhost";
    #[cfg(not(windows))]
    let packaged_bare_root = "tauri://localhost";
    assert!(is_exact_main_page(
        &packaged_bare_root.parse().expect("packaged bare root"),
        StorageOrigin::Packaged
    ));

    for url in [
        "tauri://localhost/index.html?query=1",
        "tauri://localhost/index.html#fragment",
        "tauri://foreign/index.html",
    ] {
        assert!(!is_exact_main_page(
            &url.parse().expect("rejected main"),
            StorageOrigin::Packaged
        ));
    }
}

#[test]
fn initial_page_skips_the_migration_only_after_a_valid_marker() {
    let directory = tempdir().expect("app data");
    let required = StartupMigration::new(directory.path().to_path_buf(), StorageOrigin::Packaged)
        .expect("required migration");
    assert_eq!(required.initial_page(), MIGRATION_PAGE);

    acknowledge_webview_storage_migration(directory.path(), StorageOrigin::Packaged)
        .expect("marker");
    let complete = StartupMigration::new(directory.path().to_path_buf(), StorageOrigin::Packaged)
        .expect("complete migration");
    assert_eq!(complete.initial_page(), MAIN_PAGE);
}

#[test]
fn development_marker_never_suppresses_packaged_origin_migration() {
    let directory = tempdir().expect("app data");
    acknowledge_webview_storage_migration(directory.path(), StorageOrigin::Development)
        .expect("development marker");

    assert!(directory
        .path()
        .join(DEVELOPMENT_MIGRATION_MARKER)
        .is_file());
    assert!(!directory.path().join(PACKAGED_MIGRATION_MARKER).exists());
    assert_eq!(
        marker_status(directory.path(), StorageOrigin::Packaged).expect("packaged status"),
        MigrationMarkerStatus::Required
    );
    assert_eq!(
        StartupMigration::new(directory.path().to_path_buf(), StorageOrigin::Packaged)
            .expect("packaged migration")
            .initial_page(),
        MIGRATION_PAGE
    );
}

#[test]
fn acknowledgement_does_not_suppress_timeout_until_native_show_succeeds() {
    let directory = tempdir().expect("app data");
    let migration = StartupMigration::new(directory.path().to_path_buf(), StorageOrigin::Packaged)
        .expect("required migration");
    let ready = migration.ready_flag();
    #[cfg(windows)]
    let acknowledgement = "http://tauri.localhost/index.html?xfalcon-storage-migration=v1"
        .parse()
        .expect("acknowledgement");
    #[cfg(not(windows))]
    let acknowledgement = "tauri://localhost/index.html?xfalcon-storage-migration=v1"
        .parse()
        .expect("acknowledgement");

    assert!(migration
        .handle_loaded_url(&acknowledgement)
        .expect("persist acknowledgement"));
    assert!(!ready.load(Ordering::Acquire));

    migration.mark_ready();
    assert!(ready.load(Ordering::Acquire));
}

#[test]
fn acknowledgement_timeout_fails_closed_only_while_not_ready() {
    let (timeout_tx, timeout_rx) = mpsc::sync_channel(1);
    spawn_acknowledgement_timeout(
        Arc::new(AtomicBool::new(false)),
        Duration::ZERO,
        move || timeout_tx.send(()).expect("timeout signal"),
    )
    .expect("timeout worker")
    .join()
    .expect("timeout join");
    timeout_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("pending migration must fail closed");

    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let ready = Arc::new(AtomicBool::new(true));
    spawn_acknowledgement_timeout(ready.clone(), Duration::ZERO, move || {
        ready_tx.send(()).expect("unexpected timeout")
    })
    .expect("ready worker")
    .join()
    .expect("ready join");
    assert!(ready.load(Ordering::Acquire));
    assert!(ready_rx.try_recv().is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_app_data_root_is_rejected_before_marker_access() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    let target = directory.path().join("target");
    let app_root = directory.path().join("app-root");
    std::fs::create_dir(&target).expect("target");
    symlink(&target, &app_root).expect("app root symlink");

    assert!(marker_status(&app_root, StorageOrigin::Packaged).is_err());
    assert!(acknowledge_webview_storage_migration(&app_root, StorageOrigin::Packaged).is_err());
    assert!(!target.join(PACKAGED_MIGRATION_MARKER).exists());
}

/// The URL a packaged window is actually built with must be one this module
/// recognizes as the entry document.
///
/// `WebviewUrl::App("index.html")` is not joined onto the base URL: tauri
/// special-cases that file name and loads the base verbatim (2.11.3
/// `manager/webview.rs:451`). The base comes from
/// `manager/mod.rs::tauri_protocol_url`, and its parsed path differs by platform
/// because `tauri` is not a special scheme while `http` is. Asserting the path
/// shape here is what makes that platform difference visible in CI instead of in
/// a 30-second silent exit on someone's Mac.
#[test]
fn the_platform_app_base_url_is_a_recognized_entry_document() {
    #[cfg(windows)]
    let base = "http://tauri.localhost";
    #[cfg(not(windows))]
    let base = "tauri://localhost";

    let url: tauri::Url = base.parse().expect("platform app base url");

    #[cfg(windows)]
    assert_eq!(url.path(), "/", "http normalizes the empty path");
    #[cfg(not(windows))]
    assert_eq!(
        url.path(),
        "",
        "tauri is not a special scheme, so the path stays empty"
    );

    assert!(
        is_exact_main_page(&url, StorageOrigin::Packaged),
        "the window's own initial URL must count as the main page, or it is never shown"
    );
}

/// Accepting the bare root must not accept it from anywhere else.
#[test]
fn the_bare_app_root_is_recognized_only_at_the_app_origin() {
    for foreign in [
        "tauri://foreign",
        "tauri://localhost:1420",
        "http://evil.localhost",
        "https://tauri.localhost",
    ] {
        let url: tauri::Url = foreign.parse().expect("foreign root");
        assert!(
            !is_exact_main_page(&url, StorageOrigin::Packaged),
            "{foreign} must not be treated as the entry document"
        );
        assert!(
            !is_exact_storage_migration_ack(&url, StorageOrigin::Packaged),
            "{foreign} must not be treated as the acknowledgement"
        );
    }
}
