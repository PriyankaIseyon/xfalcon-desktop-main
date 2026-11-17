//! The Windows uninstaller must run the app's own purge on a real uninstall,
//! and never as a step of an upgrade: the NSIS hook is wired through
//! `tauri.conf.json` and must name a file that defines the pre-uninstall macro
//! and runs the purge behind its guards. Running a newer setup over an older
//! install runs the old uninstaller without `/UPDATE`, in place (`$EXEDIR` equals
//! `$INSTDIR`), whereas a real uninstall runs from a temp copy; an unguarded
//! purge would wipe the data on a routine upgrade.

use std::path::Path;

use serde_json::Value;

#[test]
fn nsis_uninstall_hook_runs_purge_and_skips_updates() {
    let config: Value = serde_json::from_str(include_str!("../../../tauri.conf.json"))
        .expect("Tauri config must parse");
    let hooks = config["bundle"]["windows"]["nsis"]["installerHooks"]
        .as_str()
        .expect("NSIS installerHooks must be configured");

    // Tauri resolves this path relative to the directory holding tauri.conf.json.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(hooks);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("installerHooks {} must exist: {error}", path.display()));

    let macro_start = text
        .find("!macro NSIS_HOOK_PREUNINSTALL")
        .expect("the hook must define the pre-uninstall macro");
    let body = &text[macro_start..];
    // Comments explain the guards and may name the flag; only code counts.
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(';'))
        .collect();
    let position = |needle: &str| {
        lines
            .iter()
            .position(|line| *line == needle)
            .unwrap_or_else(|| panic!("the hook must contain the line {needle:?}"))
    };

    let not_update = position("${If} $UpdateMode <> 1");
    let real_uninstall = position("${If} $EXEDIR != $INSTDIR");
    let box_ticked = position("${OrIf} $DeleteAppDataCheckboxState = 1");
    let purge = lines
        .iter()
        .position(|line| line.contains("--purge-local-data"))
        .expect("the uninstall hook must run the app's purge mode");
    let end = position("!macroend");

    assert_eq!(
        real_uninstall,
        not_update + 1,
        "the real-uninstall test must be the first condition inside the update guard"
    );
    assert_eq!(
        box_ticked,
        real_uninstall + 1,
        "the checkbox is the only other way in, and it must share the same block"
    );
    assert!(
        box_ticked < purge && purge < end,
        "the purge must sit inside the guarded block"
    );
    // The purge must not be reachable before the guard closes any earlier block.
    assert!(
        !lines[..not_update]
            .iter()
            .any(|line| line.contains("nsExec")),
        "nothing may run the purge before the guards"
    );
}

#[test]
fn nsis_uninstall_hook_retries_the_purge_until_clean() {
    let config: Value = serde_json::from_str(include_str!("../../../tauri.conf.json"))
        .expect("Tauri config must parse");
    let hooks = config["bundle"]["windows"]["nsis"]["installerHooks"]
        .as_str()
        .expect("NSIS installerHooks must be configured");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(hooks);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("installerHooks {} must exist: {error}", path.display()));

    let purge_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.starts_with(';') && line.contains("--purge-local-data"))
        .expect("the uninstall hook must run the app's purge mode");
    assert!(
        purge_line.contains("--retry-until-clean"),
        "the uninstall purge must retry while files remain: {purge_line}"
    );
}

#[test]
fn macos_uninstall_script_retries_the_purge_until_clean() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/uninstall-macos.sh");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must exist: {error}", path.display()));

    let purge_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.starts_with('#') && line.contains("--purge-local-data"))
        .expect("the uninstall script must run the app's purge mode");
    assert!(
        purge_line.contains("--retry-until-clean"),
        "the uninstall script's purge must retry while files remain: {purge_line}"
    );
}
