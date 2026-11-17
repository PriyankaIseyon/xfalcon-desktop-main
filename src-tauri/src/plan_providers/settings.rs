//! The device-wide settings for the plan providers.
//!
//! Not per account: a plan runs on this device, and the switch records that
//! the person accepted what leaves the device for it. A missing, damaged or
//! newer file reads as everything off, so the safe state is also the default.
//! An unreadable effort is the exception: it falls back to `high` alone, so one
//! bad value never turns Claude off.

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use xfalcon_contracts::plan_providers::ClaudeEffort;

pub(crate) const PLAN_SETTINGS_FILE: &str = "plan-providers.json";
/// The name a temporary sibling of the settings file starts with. A crashed
/// write leaves one behind, and the account clear's device sweep removes it.
pub(crate) const TEMPORARY_PREFIX: &str = ".plan-providers-";
const SETTINGS_VERSION: u16 = 1;
const MAX_SETTINGS_BYTES: u64 = 4 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PlanSettings {
    pub(crate) claude_enabled: bool,
    pub(crate) claude_effort: ClaudeEffort,
}

#[derive(Debug, thiserror::Error, Clone, Copy, Eq, PartialEq)]
pub(crate) enum PlanSettingsError {
    #[error("the plan settings could not be saved")]
    Unavailable,
}

#[derive(Deserialize, Serialize)]
struct SettingsDocument {
    version: u16,
    claude_enabled: bool,
    #[serde(default, deserialize_with = "lenient_effort")]
    claude_effort: ClaudeEffort,
}

pub(crate) struct PlanSettingsStore {
    path: PathBuf,
}

impl PlanSettingsStore {
    #[must_use]
    pub(crate) fn new(app_local_data: &Path) -> Self {
        Self {
            path: app_local_data.join(PLAN_SETTINGS_FILE),
        }
    }

    #[must_use]
    pub(crate) fn load(&self) -> PlanSettings {
        read_document(&self.path).unwrap_or_default()
    }

    pub(crate) fn set_claude_enabled(&self, enabled: bool) -> Result<(), PlanSettingsError> {
        let mut settings = self.load();
        settings.claude_enabled = enabled;
        write_document(&self.path, settings).map_err(|_| PlanSettingsError::Unavailable)
    }

    pub(crate) fn set_claude_effort(&self, effort: ClaudeEffort) -> Result<(), PlanSettingsError> {
        let mut settings = self.load();
        settings.claude_effort = effort;
        write_document(&self.path, settings).map_err(|_| PlanSettingsError::Unavailable)
    }
}

fn read_document(path: &Path) -> Option<PlanSettings> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).ok()? > MAX_SETTINGS_BYTES {
        return None;
    }
    let document: SettingsDocument = serde_json::from_slice(&bytes).ok()?;
    (document.version == SETTINGS_VERSION).then_some(PlanSettings {
        claude_enabled: document.claude_enabled,
        claude_effort: document.claude_effort,
    })
}

/// A value this build cannot read is the default effort. Failing here would fail the
/// whole document, and a document that does not read turns Claude off.
fn lenient_effort<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ClaudeEffort, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<ClaudeEffort>(value).unwrap_or_default())
}

/// Writes through a temporary sibling, so a crash leaves the old document or
/// the new one, never a partial file that would read as everything off.
fn write_document(path: &Path, settings: PlanSettings) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("plan settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(&SettingsDocument {
        version: SETTINGS_VERSION,
        claude_enabled: settings.claude_enabled,
        claude_effort: settings.claude_effort,
    })
    .map_err(io::Error::other)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(TEMPORARY_PREFIX)
        .tempfile_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(root: &Path, bytes: &[u8]) {
        fs::write(root.join(PLAN_SETTINGS_FILE), bytes).expect("seed settings");
    }

    #[test]
    fn a_missing_file_loads_everything_off() {
        let root = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            PlanSettingsStore::new(root.path()).load(),
            PlanSettings::default()
        );
    }

    #[test]
    fn a_corrupt_file_loads_everything_off() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path(), b"{\"version\":1,\"claude_enabled\":");

        assert_eq!(
            PlanSettingsStore::new(root.path()).load(),
            PlanSettings::default()
        );
    }

    #[test]
    fn an_oversized_file_loads_everything_off_even_when_it_parses() {
        let root = tempfile::tempdir().expect("tempdir");
        let padding = "x".repeat(5_000);
        seed(
            root.path(),
            format!("{{\"version\":1,\"claude_enabled\":true,\"padding\":\"{padding}\"}}")
                .as_bytes(),
        );

        assert_eq!(
            PlanSettingsStore::new(root.path()).load(),
            PlanSettings::default()
        );
    }

    #[test]
    fn an_unknown_version_loads_everything_off() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path(), b"{\"version\":2,\"claude_enabled\":true}");

        assert_eq!(
            PlanSettingsStore::new(root.path()).load(),
            PlanSettings::default()
        );
    }

    #[test]
    fn enabling_claude_persists_in_the_documented_shape() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = PlanSettingsStore::new(root.path());

        store.set_claude_enabled(true).expect("settings write");

        assert!(store.load().claude_enabled);
        let bytes = fs::read(root.path().join(PLAN_SETTINGS_FILE)).expect("settings bytes");
        let document: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(document["version"], 1);
        assert_eq!(document["claude_enabled"], true);
    }

    #[test]
    fn a_second_store_on_the_same_directory_sees_the_setting() {
        let root = tempfile::tempdir().expect("tempdir");
        PlanSettingsStore::new(root.path())
            .set_claude_enabled(true)
            .expect("settings write");

        assert!(PlanSettingsStore::new(root.path()).load().claude_enabled);
    }

    #[test]
    fn disabling_claude_after_enabling_loads_off() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = PlanSettingsStore::new(root.path());
        store.set_claude_enabled(true).expect("settings write");

        store.set_claude_enabled(false).expect("settings write");

        assert!(!store.load().claude_enabled);
    }

    #[test]
    fn a_missing_parent_directory_is_created() {
        let root = tempfile::tempdir().expect("tempdir");
        let nested = root.path().join("app-local");

        PlanSettingsStore::new(&nested)
            .set_claude_enabled(true)
            .expect("settings write");

        assert!(PlanSettingsStore::new(&nested).load().claude_enabled);
    }

    #[test]
    fn a_path_that_cannot_be_written_reports_unavailable() {
        let root = tempfile::tempdir().expect("tempdir");
        let blocker = root.path().join("blocker");
        fs::write(&blocker, b"not a directory").expect("seed blocker");

        assert_eq!(
            PlanSettingsStore::new(&blocker).set_claude_enabled(true),
            Err(PlanSettingsError::Unavailable)
        );
    }

    #[test]
    fn the_default_effort_is_high_and_claude_stays_off() {
        assert_eq!(PlanSettings::default().claude_effort, ClaudeEffort::High);
        assert!(!PlanSettings::default().claude_enabled);
    }

    #[test]
    fn each_effort_level_persists_and_loads_back() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = PlanSettingsStore::new(root.path());

        for effort in ClaudeEffort::ALL {
            store.set_claude_effort(effort).expect("settings write");

            assert_eq!(store.load().claude_effort, effort);
        }
    }

    #[test]
    fn an_effort_persists_as_its_flag_value_under_its_own_key() {
        let root = tempfile::tempdir().expect("tempdir");
        PlanSettingsStore::new(root.path())
            .set_claude_effort(ClaudeEffort::Max)
            .expect("settings write");

        let bytes = fs::read(root.path().join(PLAN_SETTINGS_FILE)).expect("settings bytes");
        let document: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(document["version"], 1);
        assert_eq!(document["claude_effort"], "max");
    }

    #[test]
    fn a_document_without_an_effort_reads_high() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path(), b"{\"version\":1,\"claude_enabled\":true}");

        assert_eq!(
            PlanSettingsStore::new(root.path()).load(),
            PlanSettings {
                claude_enabled: true,
                claude_effort: ClaudeEffort::High,
            }
        );
    }

    #[test]
    fn an_unreadable_effort_reads_high_and_never_turns_claude_off() {
        let root = tempfile::tempdir().expect("tempdir");
        for effort in ["\"ultracode\"", "7", "null"] {
            seed(
                root.path(),
                format!("{{\"version\":1,\"claude_enabled\":true,\"claude_effort\":{effort}}}")
                    .as_bytes(),
            );

            assert_eq!(
                PlanSettingsStore::new(root.path()).load(),
                PlanSettings {
                    claude_enabled: true,
                    claude_effort: ClaudeEffort::High,
                },
                "effort {effort} must fall back to high"
            );
        }
    }

    #[test]
    fn a_damaged_file_with_an_effort_still_loads_everything_off() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(
            root.path(),
            b"{\"version\":1,\"claude_enabled\":true,\"claude_effort\":\"max\"",
        );

        assert_eq!(
            PlanSettingsStore::new(root.path()).load(),
            PlanSettings::default()
        );
    }

    #[test]
    fn toggling_claude_keeps_the_stored_effort() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = PlanSettingsStore::new(root.path());
        store
            .set_claude_effort(ClaudeEffort::Max)
            .expect("settings write");

        store.set_claude_enabled(true).expect("settings write");

        assert_eq!(
            store.load(),
            PlanSettings {
                claude_enabled: true,
                claude_effort: ClaudeEffort::Max,
            }
        );
    }

    #[test]
    fn changing_the_effort_keeps_claude_on() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = PlanSettingsStore::new(root.path());
        store.set_claude_enabled(true).expect("settings write");

        store
            .set_claude_effort(ClaudeEffort::Low)
            .expect("settings write");

        assert_eq!(
            store.load(),
            PlanSettings {
                claude_enabled: true,
                claude_effort: ClaudeEffort::Low,
            }
        );
    }
}
