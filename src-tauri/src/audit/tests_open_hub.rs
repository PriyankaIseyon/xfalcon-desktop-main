//! Startup behaviour of [`super::open_hub`]: the log opens when it can, and
//! a log that cannot be opened disables auditing without failing startup.

use std::fs;
use std::sync::Arc;

use tempfile::tempdir;
use zeroize::Zeroizing;

use super::open_hub;
use crate::domain::error::{AppError, AppResult};
use crate::persistence::{InMemorySecretStore, SecretStore};

/// A key store that is unavailable, as on a locked or missing OS keychain.
struct UnavailableSecrets;

impl SecretStore for UnavailableSecrets {
    fn load_or_create_active_key(&self, _: &str) -> AppResult<(u32, Zeroizing<[u8; 32]>)> {
        Err(AppError::PersistenceUnavailable)
    }
    fn active_key_version(&self, _: &str) -> AppResult<u32> {
        Err(AppError::PersistenceUnavailable)
    }
    fn account_key(&self, _: &str, _: u32) -> AppResult<Zeroizing<[u8; 32]>> {
        Err(AppError::PersistenceUnavailable)
    }
    fn delete_account_keys(&self, _: &str) -> AppResult<()> {
        Err(AppError::PersistenceUnavailable)
    }
}

#[test]
fn the_hub_opens_the_log_under_the_app_data_folder() {
    let root = tempdir().expect("temp dir");

    let hub = open_hub(root.path(), Arc::new(InMemorySecretStore::default()));

    assert!(hub.is_some());
    assert!(root.path().join("audit").is_dir());
}

#[test]
fn a_blocked_log_folder_disables_auditing_without_failing() {
    let root = tempdir().expect("temp dir");
    // The app data folder itself is a regular file, so the log folder can
    // never be created beneath it.
    let blocked = root.path().join("app-data");
    fs::write(&blocked, b"not a folder").expect("write blocker");

    let hub = open_hub(&blocked, Arc::new(InMemorySecretStore::default()));

    assert!(hub.is_none());
}

#[test]
fn an_unavailable_key_store_disables_auditing_without_failing() {
    let root = tempdir().expect("temp dir");

    let hub = open_hub(root.path(), Arc::new(UnavailableSecrets));

    assert!(hub.is_none());
}
