//! The two commands the appearance surface uses.
//!
//! Both cross the boundary as `AppearanceSettings` or a
//! [`SettingsOperationError`] code — never an `io::Error` string, which
//! would put a filesystem path in front of the user.

use tauri::Manager as _;
use xfalcon_contracts::settings::{AppearanceSettings, SettingsErrorCode, SettingsOperationError};

use super::SettingsStore;

fn store(app: &tauri::AppHandle) -> Result<SettingsStore, SettingsOperationError> {
    app.path()
        .app_local_data_dir()
        .map(SettingsStore::new)
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))
}

#[tauri::command]
pub fn settings_appearance_get(
    app: tauri::AppHandle,
) -> Result<AppearanceSettings, SettingsOperationError> {
    Ok(store(&app)?.appearance())
}

#[tauri::command]
pub fn settings_appearance_set(
    app: tauri::AppHandle,
    appearance: AppearanceSettings,
) -> Result<AppearanceSettings, SettingsOperationError> {
    store(&app)?
        .set_appearance(appearance)
        .map_err(|_| SettingsOperationError::new(SettingsErrorCode::StorageUnavailable))?;
    Ok(appearance)
}
