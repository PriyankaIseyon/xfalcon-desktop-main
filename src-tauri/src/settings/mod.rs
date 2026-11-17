//! Device-local appearance preferences.
//!
//! One JSON document at `<app_local_data>/settings.json`, read and written
//! whole. Preferences are not secrets and not account data — they describe
//! this installation's window, so they live beside the app's other local
//! state rather than in the keychain or in DA.
//!
//! The renderer cannot keep them itself: `src/security/frontend-boundary`
//! forbids `localStorage` in production frontend sources, and that boundary
//! has no preference exception. So the two commands below are the only way
//! the UI reads or writes them.
//!
//! # Reading never fails
//!
//! A missing, empty, truncated, or malformed file yields
//! [`AppearanceSettings::default`]. A corrupted preference file is a reason
//! to render the default palette, never a reason to refuse to render — and
//! the next write repairs it. The contract's `#[serde(default)]` fields
//! extend the same tolerance one level down, so a file written by a build
//! that knew fewer fields still reads.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use xfalcon_contracts::settings::AppearanceSettings;

const SETTINGS_FILE: &str = "settings.json";

/// Writes cannot be partial, so a crash mid-write cannot leave a half
/// document that the next read would silently reinterpret as defaults.
///
/// Rename-into-place is the same discipline the rest of this app's local
/// writes use. On Windows `fs::rename` onto an existing file fails, so the
/// destination is removed first — the window that opens is one where the
/// file is absent, which reads as defaults, not as garbage.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temporary, path)
}

/// Reads and writes the appearance document under one app-local root.
///
/// Deliberately not a cache: both operations touch the file. The surface is
/// Settings and app boot, so the read count is measured in ones per session
/// and an in-memory copy would only add a way for the two to disagree.
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    #[must_use]
    pub fn new(app_local_data: PathBuf) -> Self {
        Self {
            path: app_local_data.join(SETTINGS_FILE),
        }
    }

    #[must_use]
    pub fn appearance(&self) -> AppearanceSettings {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Persists `appearance`. A write that fails is reported: the UI has
    /// already applied the choice locally, and telling the user it was kept
    /// when it was not is the one outcome worse than the failure.
    pub fn set_appearance(&self, appearance: AppearanceSettings) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&appearance).map_err(std::io::Error::other)?;
        write_atomically(&self.path, &bytes)
    }
}

pub mod commands;

#[cfg(test)]
mod tests;
