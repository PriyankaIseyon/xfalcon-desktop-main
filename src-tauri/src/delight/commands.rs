//! The two commands the greeting and the message milestone use.
//!
//! Failures cross the boundary as a [`SettingsOperationError`] code, never an
//! `io::Error` string, which would put a filesystem path in front of the user.
//! The renderer treats both as optional: it carries on without them.

use tauri::Manager as _;
use xfalcon_contracts::settings::{SettingsErrorCode, SettingsOperationError};

use super::KeepsakeStore;

fn store(app: &tauri::AppHandle) -> Result<KeepsakeStore, SettingsOperationError> {
    app.path()
        .app_local_data_dir()
        .map(KeepsakeStore::new)
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))
}

/// `true` once per local calendar day, for the first caller to claim it. Runs
/// off the main thread: the file read and the write it may make are blocking.
#[tauri::command]
pub async fn delight_greeting_claim(
    app: tauri::AppHandle,
    day: String,
) -> Result<bool, SettingsOperationError> {
    let store = store(&app)?;
    tokio::task::spawn_blocking(move || store.claim_greeting(&day))
        .await
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))?
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))
}

/// Counts a message sent from this installation and returns the new total. Runs
/// off the main thread: the file read, fsync and rename are blocking.
#[tauri::command]
pub async fn delight_message_sent(app: tauri::AppHandle) -> Result<u64, SettingsOperationError> {
    let store = store(&app)?;
    tokio::task::spawn_blocking(move || store.count_message())
        .await
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))?
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))
}
