use tempfile::TempDir;
use xfalcon_contracts::settings::{AppearanceSettings, ReadingSize, ThemePreference};

use super::SettingsStore;

fn store() -> (TempDir, SettingsStore) {
    let root = TempDir::new().expect("temp root");
    let store = SettingsStore::new(root.path().to_path_buf());
    (root, store)
}

#[test]
fn an_untouched_install_reads_light_small_and_hidden_reasoning() {
    let (_root, store) = store();
    assert_eq!(store.appearance(), AppearanceSettings::default());
}

#[test]
fn a_written_preference_reads_back() {
    let (_root, store) = store();
    let chosen = AppearanceSettings::new(ThemePreference::Light, ReadingSize::Large, false, true);
    store.set_appearance(chosen).expect("write");
    assert_eq!(store.appearance(), chosen);
}

#[test]
fn a_file_without_the_reasoning_preference_uses_the_hidden_default() {
    let (root, store) = store();
    std::fs::write(
        root.path().join("settings.json"),
        br#"{"version":1,"theme":"light","reading_size":"large"}"#,
    )
    .expect("seed");
    let read = store.appearance();
    assert_eq!(read.theme, ThemePreference::Light);
    assert_eq!(read.reading_size, ReadingSize::Large);
    assert!(!read.show_reasoning);
}

#[test]
fn hiding_reasoning_reads_back_as_hidden() {
    let (_root, store) = store();
    let hidden = AppearanceSettings::new(ThemePreference::Dark, ReadingSize::Small, false, true);
    store.set_appearance(hidden).expect("write");
    assert!(!store.appearance().show_reasoning);
}

#[test]
fn older_settings_enable_follow_ups_without_changing_other_preferences() {
    let (root, store) = store();
    std::fs::write(
        root.path().join("settings.json"),
        br#"{"version":1,"theme":"dark","reading_size":"large","show_reasoning":true}"#,
    )
    .expect("seed older settings");
    assert_eq!(
        store.appearance(),
        AppearanceSettings::new(ThemePreference::Dark, ReadingSize::Large, true, true)
    );
}

#[test]
fn follow_up_choices_survive_reopening_the_store() {
    let (root, store) = store();
    for enabled in [false, true] {
        let chosen =
            AppearanceSettings::new(ThemePreference::Dark, ReadingSize::Large, true, enabled);
        store.set_appearance(chosen).expect("save choice");
        let reopened = SettingsStore::new(root.path().to_path_buf());
        assert_eq!(reopened.appearance(), chosen);
    }
}

#[test]
fn a_corrupt_file_reads_as_defaults_rather_than_failing() {
    let (root, store) = store();
    std::fs::write(root.path().join("settings.json"), b"{ not json").expect("seed");
    assert_eq!(store.appearance(), AppearanceSettings::default());
}

#[test]
fn a_rewrite_replaces_rather_than_appends() {
    // The Windows rename path removes the destination first; this is the test
    // that a second write leaves one valid document rather than two
    // concatenated ones.
    let (_root, store) = store();
    store
        .set_appearance(AppearanceSettings::new(
            ThemePreference::Light,
            ReadingSize::Medium,
            false,
            true,
        ))
        .expect("first write");
    let second = AppearanceSettings::new(ThemePreference::Dark, ReadingSize::Large, true, true);
    store.set_appearance(second).expect("second write");
    assert_eq!(store.appearance(), second);
}

#[test]
fn no_temporary_file_is_left_behind() {
    let (root, store) = store();
    store
        .set_appearance(AppearanceSettings::default())
        .expect("write");
    let leftovers: Vec<_> = std::fs::read_dir(root.path())
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "left {leftovers:?}");
}
