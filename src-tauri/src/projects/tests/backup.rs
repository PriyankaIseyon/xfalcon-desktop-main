use std::sync::Arc;

use tempfile::tempdir;

use crate::persistence::InMemorySecretStore;
use crate::projects::{ProjectId, RelativeProjectPath, ShadowBackupStore};
use crate::security::account_cleanup::ValidatedAccountId;

const ACCOUNT: &str = "backup-account";

fn store(root: &std::path::Path) -> (ShadowBackupStore, ValidatedAccountId) {
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT, [0x42; 32])
        .expect("store account key");
    let account = ValidatedAccountId::parse(ACCOUNT).expect("account");
    let store = ShadowBackupStore::open(root.join("shadow_backups.sqlite3"), secrets)
        .expect("backup store");
    (store, account)
}

#[test]
fn a_revision_round_trips_exact_binary_bytes() {
    let directory = tempdir().expect("tempdir");
    let (store, account) = store(directory.path());
    let project_id = ProjectId::new();
    let path = RelativeProjectPath::new("résumé/data.bin").expect("path");
    let bytes = b"\0\xff\x01exact\r\nbytes";

    let revision = store
        .capture(&account, project_id, &path, bytes, 1_783_000_000_000)
        .expect("capture");
    let listed = store
        .list(&account, project_id, &path)
        .expect("list revisions");
    let loaded = store
        .load(&account, revision.backup_id)
        .expect("load revision");

    assert_eq!(listed, vec![revision]);
    assert_eq!(loaded.project_id, project_id);
    assert_eq!(loaded.relative_path, path);
    assert_eq!(loaded.content, bytes);
}

#[test]
fn retention_keeps_only_the_newest_fifty_revisions_per_file() {
    let directory = tempdir().expect("tempdir");
    let (store, account) = store(directory.path());
    let project_id = ProjectId::new();
    let path = RelativeProjectPath::new("notes.txt").expect("path");

    for sequence in 0..55 {
        store
            .capture(
                &account,
                project_id,
                &path,
                format!("revision {sequence}").as_bytes(),
                sequence,
            )
            .expect("capture");
    }

    let revisions = store
        .list(&account, project_id, &path)
        .expect("list revisions");
    assert_eq!(revisions.len(), 50);
    assert_eq!(revisions.first().expect("newest").created_at_ms, 54);
    assert_eq!(revisions.last().expect("oldest retained").created_at_ms, 5);
}

#[test]
fn the_database_contains_neither_file_content_nor_relative_path() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("shadow_backups.sqlite3");
    let (store, account) = store(directory.path());
    let path = RelativeProjectPath::new("private/customer-list.txt").expect("path");
    store
        .capture(
            &account,
            ProjectId::new(),
            &path,
            b"CONFIDENTIAL-CUSTOMER-CONTENT-7391",
            1,
        )
        .expect("capture");
    drop(store);

    let raw = std::fs::read(database).expect("database bytes");
    assert!(!raw
        .windows(b"CONFIDENTIAL-CUSTOMER-CONTENT-7391".len())
        .any(|window| window == b"CONFIDENTIAL-CUSTOMER-CONTENT-7391"));
    assert!(!raw
        .windows(path.as_str().len())
        .any(|window| window == path.as_str().as_bytes()));
}

#[test]
fn another_account_cannot_list_or_open_a_revision() {
    let directory = tempdir().expect("tempdir");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT, [0x42; 32])
        .expect("store account key");
    secrets
        .put_key("other-account", [0x43; 32])
        .expect("store other key");
    let store = ShadowBackupStore::open(directory.path().join("shadow_backups.sqlite3"), secrets)
        .expect("backup store");
    let account = ValidatedAccountId::parse(ACCOUNT).expect("account");
    let other = ValidatedAccountId::parse("other-account").expect("other account");
    let project_id = ProjectId::new();
    let path = RelativeProjectPath::new("notes.txt").expect("path");
    let revision = store
        .capture(&account, project_id, &path, b"private", 1)
        .expect("capture");

    assert!(store
        .list(&other, project_id, &path)
        .expect("other list")
        .is_empty());
    assert!(store.load(&other, revision.backup_id).is_err());
}

#[test]
fn account_retention_removes_the_oldest_unprotected_revision() {
    let directory = tempdir().expect("tempdir");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT, [0x42; 32])
        .expect("store account key");
    let account = ValidatedAccountId::parse(ACCOUNT).expect("account");
    let store = ShadowBackupStore::open(directory.path().join("shadow_backups.sqlite3"), secrets)
        .expect("backup store")
        .with_retention_limits(50, 24);
    let project_id = ProjectId::new();
    let first_path = RelativeProjectPath::new("first.txt").expect("path");
    let second_path = RelativeProjectPath::new("second.txt").expect("path");

    let first = store
        .capture(&account, project_id, &first_path, b"first revision", 1)
        .expect("first capture");
    let second = store
        .capture(&account, project_id, &second_path, b"second revision", 2)
        .expect("second capture");

    assert!(store.load(&account, first.backup_id).is_err());
    assert_eq!(
        store
            .load(&account, second.backup_id)
            .expect("newest revision")
            .content,
        b"second revision"
    );
}
