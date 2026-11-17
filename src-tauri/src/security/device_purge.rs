//! Headless full purge for installers, uninstall hooks and manual uninstall:
//! `xFalcon --purge-local-data`.
//!
//! Unlike the account clear that Reset xFalcon runs first, which removes one
//! account's material and the loose device items beside it, this removes every
//! local trace of the app: the app's data, configuration, cache, logs and WebView storage on each
//! platform, and both credential-store services.
//! It starts no window and writes no log file. The fixed target set is computed
//! by [`purge_targets`], which refuses anything that is not an absolute path
//! ending in one of the app's own names.
//!
//! The in-app full reset reuses this purge through a detached helper, started
//! with `--after-parent-exit` (see [`spawn_reset_helper`]).

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Stdio},
    sync::{mpsc, Mutex, PoisonError},
    thread,
    time::Duration,
};

const APP_ID: &str = "ai.xfalcon.desktop";
const PROVIDER_KEYS_SERVICE: &str = "ai.xfalcon.desktop.provider-keys";
const PROBE_USER: &str = "purge-probe";

/// Final path components a purge may delete. Anything else is refused.
const ALLOWED_NAMES: [&str; 3] = [
    APP_ID,
    "ai.xfalcon.desktop.plist",
    "ai.xfalcon.desktop.savedState",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetOs {
    Windows,
    MacOs,
    Linux,
}

/// The environment a purge resolves its fixed targets from. A `None` variable
/// skips only the targets that need it. [`PurgeEnv::from_process`] reads a
/// relative value as unset; [`purge_targets`] refuses one it is given.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PurgeEnv {
    pub home: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
}

impl PurgeEnv {
    pub fn from_process() -> Self {
        Self {
            home: env_path("HOME").or_else(|| env_path("USERPROFILE")),
            local_app_data: env_path("LOCALAPPDATA"),
            app_data: env_path("APPDATA"),
            xdg_data_home: env_path("XDG_DATA_HOME"),
            xdg_config_home: env_path("XDG_CONFIG_HOME"),
            xdg_cache_home: env_path("XDG_CACHE_HOME"),
        }
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .and_then(|value| absolute_path(PathBuf::from(value)))
}

/// Keeps `value` only when it is absolute. A relative value is ignored, not
/// refused: the XDG base directory spec says so, and it leaves the other targets
/// to be purged.
fn absolute_path(value: PathBuf) -> Option<PathBuf> {
    value.is_absolute().then_some(value)
}

#[derive(Debug, Eq, PartialEq)]
pub struct PurgeTargets {
    pub paths: Vec<PathBuf>,
    pub keychain_services: Vec<&'static str>,
    pub unresolved_roots: Vec<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PurgeError {
    #[error("a purge target is not an absolute path")]
    RelativeTarget,
    #[error("a purge target does not name an xFalcon location")]
    UnexpectedTarget,
    #[error("the credential store could not be cleared: {0}")]
    Keychain(String),
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct PurgeReport {
    pub deleted_paths: usize,
    pub deleted_credentials: usize,
    pub residue: Vec<PathBuf>,
    pub keychain_error: Option<String>,
    pub unresolved_roots: Vec<&'static str>,
}

impl PurgeReport {
    pub fn is_clean(&self) -> bool {
        self.residue.is_empty() && self.keychain_error.is_none() && self.unresolved_roots.is_empty()
    }
}

/// Resolves the fixed purge targets for `os` from `env`.
///
/// Every path is checked before any is returned, so one bad value refuses the
/// whole purge and nothing is deleted.
pub fn purge_targets(os: TargetOs, env: &PurgeEnv) -> Result<PurgeTargets, PurgeError> {
    let mut candidates = Vec::new();
    let mut unresolved_roots = Vec::new();
    match os {
        TargetOs::Windows => {
            if env.local_app_data.is_none() {
                unresolved_roots.push("LOCALAPPDATA");
            }
            if env.app_data.is_none() {
                unresolved_roots.push("APPDATA");
            }
            push_under(&mut candidates, os, &env.local_app_data, &[APP_ID]);
            push_under(&mut candidates, os, &env.app_data, &[APP_ID]);
        }
        TargetOs::MacOs => {
            let home = &env.home;
            if home.is_none() {
                unresolved_roots.push("HOME");
            }
            push_under(
                &mut candidates,
                os,
                home,
                &["Library", "Application Support", APP_ID],
            );
            push_under(&mut candidates, os, home, &["Library", "Caches", APP_ID]);
            push_under(&mut candidates, os, home, &["Library", "WebKit", APP_ID]);
            push_under(
                &mut candidates,
                os,
                home,
                &["Library", "HTTPStorages", APP_ID],
            );
            push_under(&mut candidates, os, home, &["Library", "Logs", APP_ID]);
            push_under(
                &mut candidates,
                os,
                home,
                &["Library", "Preferences", "ai.xfalcon.desktop.plist"],
            );
            push_under(
                &mut candidates,
                os,
                home,
                &[
                    "Library",
                    "Saved Application State",
                    "ai.xfalcon.desktop.savedState",
                ],
            );
        }
        TargetOs::Linux => {
            let home_local = env
                .home
                .as_ref()
                .map(|home| join_for(os, home, &[".local", "share"]));
            let home_config = env
                .home
                .as_ref()
                .map(|home| join_for(os, home, &[".config"]));
            let home_cache = env
                .home
                .as_ref()
                .map(|home| join_for(os, home, &[".cache"]));
            let data_root = env.xdg_data_home.clone().or(home_local);
            let config_root = env.xdg_config_home.clone().or(home_config);
            let cache_root = env.xdg_cache_home.clone().or(home_cache);
            if data_root.is_none() {
                unresolved_roots.push("XDG_DATA_HOME or HOME");
            }
            if config_root.is_none() {
                unresolved_roots.push("XDG_CONFIG_HOME or HOME");
            }
            if cache_root.is_none() {
                unresolved_roots.push("XDG_CACHE_HOME or HOME");
            }
            push_under(&mut candidates, os, &data_root, &[APP_ID]);
            push_under(&mut candidates, os, &config_root, &[APP_ID]);
            push_under(&mut candidates, os, &cache_root, &[APP_ID]);
        }
    }
    let paths = candidates
        .into_iter()
        .map(|path| checked_target(os, path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PurgeTargets {
        paths,
        keychain_services: vec![APP_ID, PROVIDER_KEYS_SERVICE],
        unresolved_roots,
    })
}

/// Appends `root` joined with `parts` when `root` is set.
fn push_under(out: &mut Vec<PathBuf>, os: TargetOs, root: &Option<PathBuf>, parts: &[&str]) {
    if let Some(root) = root {
        out.push(join_for(os, root, parts));
    }
}

/// Joins `parts` onto `root` with the separator of the TARGET `os`, as a string.
///
/// This deliberately does not use `PathBuf::join`: that uses the HOST's
/// separator, so a macOS or Linux target computed on a Windows host would get
/// `\` inside its paths. Building the string here makes the result identical
/// on every host.
fn join_for(os: TargetOs, root: &Path, parts: &[&str]) -> PathBuf {
    let (separator, is_separator): (&str, fn(u8) -> bool) = match os {
        TargetOs::Windows => ("\\", |byte| byte == b'\\' || byte == b'/'),
        TargetOs::MacOs | TargetOs::Linux => ("/", |byte| byte == b'/'),
    };
    let mut joined = root.as_os_str().to_os_string();
    for part in parts {
        let already_separated = joined
            .as_encoded_bytes()
            .last()
            .is_some_and(|byte| is_separator(*byte));
        if !already_separated {
            joined.push(separator);
        }
        joined.push(part);
    }
    PathBuf::from(joined)
}

/// Accepts `path` only if it is absolute for `os`, has no `..` segment, and its
/// final segment is one of the app's own names.
fn checked_target(os: TargetOs, path: PathBuf) -> Result<PathBuf, PurgeError> {
    let text = path.to_string_lossy().into_owned();
    let absolute = match os {
        TargetOs::Windows => is_windows_absolute(&text),
        TargetOs::MacOs | TargetOs::Linux => text.starts_with('/'),
    };
    if !absolute {
        return Err(PurgeError::RelativeTarget);
    }
    let segments: Vec<&str> = match os {
        TargetOs::Windows => text.split(['\\', '/']).collect(),
        TargetOs::MacOs | TargetOs::Linux => text.split('/').collect(),
    };
    if segments.contains(&"..") {
        return Err(PurgeError::UnexpectedTarget);
    }
    let last = segments
        .iter()
        .rev()
        .find(|segment| !segment.is_empty())
        .copied()
        .unwrap_or_default();
    if !ALLOWED_NAMES.contains(&last) {
        return Err(PurgeError::UnexpectedTarget);
    }
    Ok(path)
}

fn is_windows_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc = text.starts_with("\\\\");
    drive || unc
}

/// Deletes one checked target. Links are unlinked and never followed; a
/// missing path is success. Returns whether something was removed.
fn remove_target(path: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        // A Windows directory symlink or junction needs `remove_dir`; neither
        // call follows the link.
        return match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(_) => fs::remove_dir(path).map(|()| true),
        };
    }
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(true)
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn still_present(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

/// Runs the purge: credential-store entries first, then every fixed path. It
/// never stops at a failure; anything still present afterwards is listed in
/// `residue`, and a credential-store failure is recorded in `keychain_error`.
/// `delete_keychain` is injected so tests can avoid the real store.
pub fn run_purge(
    os: TargetOs,
    env: &PurgeEnv,
    delete_keychain: &dyn Fn(&[&str]) -> Result<usize, PurgeError>,
) -> Result<PurgeReport, PurgeError> {
    let targets = purge_targets(os, env)?;
    let mut report = PurgeReport {
        unresolved_roots: targets.unresolved_roots.clone(),
        ..PurgeReport::default()
    };
    match delete_keychain(&targets.keychain_services) {
        Ok(count) => report.deleted_credentials = count,
        Err(error) => report.keychain_error = Some(error.to_string()),
    }
    for path in &targets.paths {
        // A failed removal is judged by what is still there, not by its error.
        if let Ok(true) = remove_target(path) {
            report.deleted_paths += 1;
        }
        if still_present(path) {
            report.residue.push(path.clone());
        }
    }
    Ok(report)
}

/// Deletes every credential-store entry whose service is exactly one of
/// `services`, from the platform's default store. Returns an error unless every
/// service was searched and every matching entry was deleted: a store that
/// cannot be searched or read never reads as clear.
pub fn purge_keychain(services: &[&str]) -> Result<usize, PurgeError> {
    // Installs the platform's default store on first use. A failure here is
    // an error only when no store is installed afterwards, because then the
    // search has nothing to search and nothing can be verified.
    let install_error = keyring::Entry::new(APP_ID, PROBE_USER)
        .err()
        .filter(|_| keyring_core::get_default_store().is_none())
        .map(|error| PurgeError::Keychain(error.to_string()));
    let (deleted, search_error) = delete_matching_credentials(services);
    match install_error.or(search_error) {
        Some(error) => Err(error),
        None => Ok(deleted),
    }
}

/// Deletes the matching entries of every service, and returns how many were
/// deleted with the first error met. A failed service does not stop the next
/// one, so each service is searched whatever happened before.
fn delete_matching_credentials(services: &[&str]) -> (usize, Option<PurgeError>) {
    let mut deleted = 0;
    let mut first_error: Option<PurgeError> = None;
    for service in services {
        let entries = match search_service(service) {
            Ok(entries) => entries,
            Err(error) => {
                first_error.get_or_insert_with(|| search_failure(&error));
                continue;
            }
        };
        for entry in entries {
            // A search can be a superset on some stores, so only an exact match
            // is deleted. An entry whose service cannot be read cannot be
            // verified, so it is an error, not a skip.
            let Some((entry_service, _)) = entry.get_specifiers() else {
                first_error.get_or_insert_with(|| {
                    PurgeError::Keychain("a credential's service could not be read".to_owned())
                });
                continue;
            };
            if entry_service != *service {
                continue;
            }
            match entry.delete_credential() {
                Ok(()) => deleted += 1,
                Err(keyring_core::Error::NoEntry) => {}
                Err(error) => {
                    first_error.get_or_insert_with(|| PurgeError::Keychain(error.to_string()));
                }
            }
        }
    }
    (deleted, first_error)
}

/// What a failed search means. With no store, or a store that does not provide
/// search, nothing can be verified, so the purge must not report the store clear.
/// `NotSupportedByStore` is the trait's default `search`: "this store does not
/// provide search capabilities".
fn search_failure(error: &keyring_core::Error) -> PurgeError {
    match error {
        keyring_core::Error::NoDefaultStore => {
            PurgeError::Keychain("no credential store is available".to_owned())
        }
        keyring_core::Error::NotSupportedByStore(_) => {
            PurgeError::Keychain("the credential store cannot search for entries".to_owned())
        }
        other => PurgeError::Keychain(other.to_string()),
    }
}

/// Searches the default store for `service`. Windows can only filter by target
/// name, so it uses a pattern; the other stores take the exact `service`
/// attribute. The test mock store takes `service` too, so tests run the
/// non-Windows branch.
fn search_service(service: &str) -> keyring_core::Result<Vec<keyring_core::Entry>> {
    let pattern = windows_target_pattern(service);
    let spec: HashMap<&str, &str> = if cfg!(windows) {
        HashMap::from([("pattern", pattern.as_str())])
    } else {
        HashMap::from([("service", service)])
    };
    keyring_core::Entry::search(&spec)
}

/// Matches the Windows Credential Manager target name `keyring:<user>@<service>`
/// that `windows-native-keyring-store` writes with its default delimiters.
/// Its `enumerate_credentials` anchors `^keyring:(.*)@(.*)$`, so the service is
/// the part after the last `@`, and this pattern anchors that service exactly.
fn windows_target_pattern(service: &str) -> String {
    let mut escaped = String::new();
    for character in service.chars() {
        if !(character.is_ascii_alphanumeric() || character == '-' || character == '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    format!("^keyring:.*@{escaped}$")
}

fn current_os() -> TargetOs {
    if cfg!(windows) {
        TargetOs::Windows
    } else if cfg!(target_os = "macos") {
        TargetOs::MacOs
    } else {
        TargetOs::Linux
    }
}

/// Maps a purge outcome to the process exit code: 0 clean, 2 something remains,
/// 1 the purge was refused before deleting anything.
fn exit_code(outcome: &Result<PurgeReport, PurgeError>) -> i32 {
    match outcome {
        Ok(report) if report.is_clean() => 0,
        Ok(_) => 2,
        Err(_) => 1,
    }
}

/// Flags the in-app reset passes to its helper, and the uninstallers pass for
/// retries. They are honored only together with `--purge-local-data`.
pub const AFTER_PARENT_FLAG: &str = "--after-parent-exit";
pub const RELAUNCH_FLAG: &str = "--relaunch";
pub const RETRY_FLAG: &str = "--retry-until-clean";
const PURGE_FLAG: &str = "--purge-local-data";
/// How long the helper waits for its parent to exit before giving up.
const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Purge runs made before the helper gives up on residue, and the pause
/// between runs. Anti-virus or a slow handle release can briefly hold a file.
const RETRY_ATTEMPTS: u32 = 20;
const RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Default, Eq, PartialEq)]
struct PurgeFlags {
    after_parent: bool,
    relaunch: bool,
    retry: bool,
}

/// Reads the flags from the arguments that follow `argv[0]`. Arguments are
/// compared as OS strings, so a non-UTF-8 argument matches no flag and never
/// panics.
fn parse_flags(args: &[OsString]) -> PurgeFlags {
    let has = |flag: &str| args.iter().any(|argument| argument == flag);
    PurgeFlags {
        after_parent: has(AFTER_PARENT_FLAG),
        relaunch: has(RELAUNCH_FLAG),
        retry: has(RETRY_FLAG),
    }
}

/// Reads and discards `reader` on a thread until it reports end of file or a
/// read error, and reports whether that happened within `timeout`. The parent
/// holds the write end of the helper's stdin for its whole life, so EOF means
/// the parent has exited. A read error counts as EOF: the pipe is gone.
fn wait_for_eof<R: io::Read + Send + 'static>(mut reader: R, timeout: Duration) -> bool {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 64];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = sender.send(());
    });
    receiver.recv_timeout(timeout).is_ok()
}

/// Runs the purge again while it leaves residue or a credential-store error, up
/// to `attempts` runs in total, pausing with `sleep` between runs. Each run
/// repeats the credential-store step, which is idempotent. A refused purge
/// (`Err`) is returned at once: retrying cannot change it.
fn run_purge_until_clean(
    os: TargetOs,
    env: &PurgeEnv,
    delete_keychain: &dyn Fn(&[&str]) -> Result<usize, PurgeError>,
    attempts: u32,
    delay: Duration,
    sleep: &dyn Fn(Duration),
) -> Result<PurgeReport, PurgeError> {
    let mut outcome = run_purge(os, env, delete_keychain);
    for _ in 1..attempts {
        match &outcome {
            Ok(report) if !report.is_clean() => {}
            _ => break,
        }
        sleep(delay);
        outcome = run_purge(os, env, delete_keychain);
    }
    outcome
}

/// Arguments for the reset helper. It always waits for the parent and purges;
/// it relaunches the app only for an installed build, never a dev run.
fn helper_args(relaunch: bool) -> Vec<&'static str> {
    let mut args = vec![PURGE_FLAG, AFTER_PARENT_FLAG];
    if relaunch {
        args.push(RELAUNCH_FLAG);
    }
    args
}

/// The command that starts the app again. On macOS the executable lives in
/// `<bundle>.app/Contents/MacOS/`, and `open -n` starts the bundle the way
/// Finder does, instead of running the binary directly.
fn relaunch_command(exe: &Path) -> (PathBuf, Vec<OsString>) {
    let bundle = exe
        .parent()
        .filter(|dir| dir.file_name() == Some(OsStr::new("MacOS")))
        .and_then(Path::parent)
        .filter(|dir| dir.file_name() == Some(OsStr::new("Contents")))
        .and_then(Path::parent)
        .filter(|dir| {
            dir.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".app"))
        });
    match bundle {
        Some(bundle) => (
            PathBuf::from("open"),
            vec![OsString::from("-n"), bundle.as_os_str().to_os_string()],
        ),
        None => (exe.to_path_buf(), Vec::new()),
    }
}

/// Starts the app again after a reset. Best effort: a failed start does not
/// change the purge outcome, and the user can open the app by hand.
fn relaunch(exe: &Path) {
    let (program, args) = relaunch_command(exe);
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS: the new app must not share the helper's console.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(DETACHED_PROCESS);
    }
    let _ = command.spawn();
}

/// Entry point for `--purge-local-data`. Prints counts only, never the paths of
/// user data, and returns the exit code.
///
/// Without flags this is one purge, as the installers expect. `--retry-until-clean`
/// (the uninstallers) purges until clean, without waiting. `--after-parent-exit`
/// (the in-app reset's helper) first waits for the parent to exit, then purges
/// until clean; `--relaunch` starts the app again afterwards, whatever the purge
/// found.
pub fn purge_exit_code() -> i32 {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let flags = parse_flags(&args);
    let env = PurgeEnv::from_process();
    let os = current_os();
    if flags.after_parent && !wait_for_eof(io::stdin(), PARENT_EXIT_TIMEOUT) {
        eprintln!("xFalcon reset: the app did not exit in time. Nothing was deleted.");
        return 3;
    }
    let outcome = if flags.after_parent || flags.retry {
        run_purge_until_clean(
            os,
            &env,
            &purge_keychain,
            RETRY_ATTEMPTS,
            RETRY_DELAY,
            &thread::sleep,
        )
    } else {
        run_purge(os, &env, &purge_keychain)
    };
    match &outcome {
        Ok(report) => eprintln!(
            "xFalcon purge: removed {} path(s) and {} credential(s); {} path(s) remain; credential store {}.",
            report.deleted_paths,
            report.deleted_credentials,
            report.residue.len(),
            if report.keychain_error.is_some() {
                "could not be cleared"
            } else {
                "clear"
            },
        ),
        Err(_) => eprintln!("xFalcon purge refused: a target was unsafe. Nothing was deleted."),
    }
    if flags.relaunch {
        if let Ok(exe) = std::env::current_exe() {
            relaunch(&exe);
        }
    }
    exit_code(&outcome)
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ResetStartError {
    #[error("a reset is already under way")]
    AlreadyStarted,
    #[error("the reset helper could not be started")]
    Spawn,
}

/// The helper's stdin. The helper purges only when this pipe reaches end of
/// file, which happens when the parent exits, so the write end must stay open
/// for the life of the process.
static HELPER_PIPE: Mutex<Option<ChildStdin>> = Mutex::new(None);

/// Starts the detached purge helper for an in-app reset. Call it before
/// anything is erased: the helper waits for this process to exit, and the app
/// cannot delete files it still holds open.
pub fn spawn_reset_helper(relaunch: bool) -> Result<(), ResetStartError> {
    let mut pipe = HELPER_PIPE.lock().unwrap_or_else(PoisonError::into_inner);
    if pipe.is_some() {
        return Err(ResetStartError::AlreadyStarted);
    }
    let exe = std::env::current_exe().map_err(|_| ResetStartError::Spawn)?;
    let mut command = Command::new(exe);
    command
        .args(helper_args(relaunch))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = command.spawn().map_err(|_| ResetStartError::Spawn)?;
    let Some(stdin) = child.stdin.take() else {
        // Piped above, so this is not expected. Stop the helper rather than let
        // it see end of file and purge while this process is still running.
        let _ = child.kill();
        return Err(ResetStartError::Spawn);
    };
    // The `Child` is dropped without waiting; the helper outlives this call.
    *pipe = Some(stdin);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        path::Path,
    };

    use super::*;

    fn no_keychain(_: &[&str]) -> Result<usize, PurgeError> {
        Ok(0)
    }

    /// The target OS of the host running the tests.
    fn host_os() -> TargetOs {
        if cfg!(windows) {
            TargetOs::Windows
        } else if cfg!(target_os = "macos") {
            TargetOs::MacOs
        } else {
            TargetOs::Linux
        }
    }

    /// An env for the host's own OS, rooted entirely under `root`, so every
    /// deletion a test performs stays inside its temporary tree.
    fn host_env(root: &Path) -> PurgeEnv {
        match host_os() {
            TargetOs::Windows => PurgeEnv {
                local_app_data: Some(root.join("local")),
                app_data: Some(root.join("roaming")),
                ..PurgeEnv::default()
            },
            TargetOs::MacOs | TargetOs::Linux => PurgeEnv {
                home: Some(root.join("home")),
                ..PurgeEnv::default()
            },
        }
    }

    fn windows_env() -> PurgeEnv {
        PurgeEnv {
            home: None,
            local_app_data: Some(PathBuf::from(r"C:\Users\me\AppData\Local")),
            app_data: Some(PathBuf::from(r"C:\Users\me\AppData\Roaming")),
            xdg_data_home: None,
            xdg_config_home: None,
            xdg_cache_home: None,
        }
    }

    fn macos_env() -> PurgeEnv {
        PurgeEnv {
            home: Some(PathBuf::from("/Users/me")),
            local_app_data: None,
            app_data: None,
            xdg_data_home: None,
            xdg_config_home: None,
            xdg_cache_home: None,
        }
    }

    fn linux_env() -> PurgeEnv {
        PurgeEnv {
            home: Some(PathBuf::from("/home/me")),
            local_app_data: None,
            app_data: None,
            xdg_data_home: None,
            xdg_config_home: None,
            xdg_cache_home: None,
        }
    }

    #[test]
    fn windows_targets_are_the_fixed_set_in_order() {
        let targets = purge_targets(TargetOs::Windows, &windows_env()).expect("targets");

        assert_eq!(
            targets.paths,
            vec![
                PathBuf::from(r"C:\Users\me\AppData\Local\ai.xfalcon.desktop"),
                PathBuf::from(r"C:\Users\me\AppData\Roaming\ai.xfalcon.desktop"),
            ]
        );
        assert_eq!(
            targets.keychain_services,
            vec!["ai.xfalcon.desktop", "ai.xfalcon.desktop.provider-keys"]
        );
    }

    #[test]
    fn macos_targets_are_the_fixed_set_in_order() {
        let targets = purge_targets(TargetOs::MacOs, &macos_env()).expect("targets");

        assert_eq!(
            targets.paths,
            vec![
                PathBuf::from("/Users/me/Library/Application Support/ai.xfalcon.desktop"),
                PathBuf::from("/Users/me/Library/Caches/ai.xfalcon.desktop"),
                PathBuf::from("/Users/me/Library/WebKit/ai.xfalcon.desktop"),
                PathBuf::from("/Users/me/Library/HTTPStorages/ai.xfalcon.desktop"),
                PathBuf::from("/Users/me/Library/Logs/ai.xfalcon.desktop"),
                PathBuf::from("/Users/me/Library/Preferences/ai.xfalcon.desktop.plist"),
                PathBuf::from(
                    "/Users/me/Library/Saved Application State/ai.xfalcon.desktop.savedState",
                ),
            ]
        );
    }

    #[test]
    fn linux_targets_use_xdg_defaults_and_overrides() {
        let defaults = purge_targets(TargetOs::Linux, &linux_env()).expect("defaults");
        assert_eq!(
            defaults.paths,
            vec![
                PathBuf::from("/home/me/.local/share/ai.xfalcon.desktop"),
                PathBuf::from("/home/me/.config/ai.xfalcon.desktop"),
                PathBuf::from("/home/me/.cache/ai.xfalcon.desktop"),
            ]
        );

        let overridden = PurgeEnv {
            xdg_data_home: Some(PathBuf::from("/data")),
            xdg_config_home: Some(PathBuf::from("/conf")),
            xdg_cache_home: Some(PathBuf::from("/cache")),
            ..linux_env()
        };
        let targets = purge_targets(TargetOs::Linux, &overridden).expect("overrides");
        assert_eq!(
            targets.paths,
            vec![
                PathBuf::from("/data/ai.xfalcon.desktop"),
                PathBuf::from("/conf/ai.xfalcon.desktop"),
                PathBuf::from("/cache/ai.xfalcon.desktop"),
            ]
        );
    }

    #[test]
    fn a_missing_variable_skips_only_the_targets_that_need_it() {
        let windows = PurgeEnv {
            local_app_data: None,
            ..windows_env()
        };
        assert_eq!(
            purge_targets(TargetOs::Windows, &windows)
                .expect("targets")
                .paths,
            vec![PathBuf::from(
                r"C:\Users\me\AppData\Roaming\ai.xfalcon.desktop"
            ),]
        );
        assert_eq!(
            purge_targets(TargetOs::Windows, &windows)
                .expect("targets")
                .unresolved_roots,
            vec!["LOCALAPPDATA"]
        );

        let macos = PurgeEnv {
            home: None,
            ..macos_env()
        };
        assert_eq!(
            purge_targets(TargetOs::MacOs, &macos)
                .expect("targets")
                .paths,
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            purge_targets(TargetOs::MacOs, &macos)
                .expect("targets")
                .unresolved_roots,
            vec!["HOME"]
        );

        let linux = PurgeEnv {
            home: None,
            ..linux_env()
        };
        assert_eq!(
            purge_targets(TargetOs::Linux, &linux)
                .expect("targets")
                .paths,
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            purge_targets(TargetOs::Linux, &linux)
                .expect("targets")
                .unresolved_roots,
            vec![
                "XDG_DATA_HOME or HOME",
                "XDG_CONFIG_HOME or HOME",
                "XDG_CACHE_HOME or HOME"
            ]
        );
    }

    #[test]
    fn a_drive_root_yields_a_correctly_named_target() {
        let env = PurgeEnv {
            local_app_data: Some(PathBuf::from(r"C:\")),
            app_data: None,
            ..windows_env()
        };
        let targets = purge_targets(TargetOs::Windows, &env).expect("drive root is accepted");

        assert_eq!(targets.paths[0], PathBuf::from(r"C:\ai.xfalcon.desktop"));
    }

    #[test]
    fn a_relative_variable_refuses_the_whole_purge() {
        let windows = PurgeEnv {
            local_app_data: Some(PathBuf::from(r"AppData\Local")),
            ..windows_env()
        };
        assert_eq!(
            purge_targets(TargetOs::Windows, &windows),
            Err(PurgeError::RelativeTarget)
        );

        let linux = PurgeEnv {
            home: Some(PathBuf::from("relative/home")),
            ..linux_env()
        };
        assert_eq!(
            purge_targets(TargetOs::Linux, &linux),
            Err(PurgeError::RelativeTarget)
        );
    }

    #[test]
    fn a_path_not_ending_in_an_app_name_or_containing_parent_segments_is_refused() {
        assert_eq!(
            checked_target(TargetOs::Linux, PathBuf::from("/home/me/.local/share")),
            Err(PurgeError::UnexpectedTarget)
        );
        assert_eq!(
            checked_target(TargetOs::Linux, PathBuf::from("/x/../ai.xfalcon.desktop")),
            Err(PurgeError::UnexpectedTarget)
        );
        assert_eq!(
            checked_target(TargetOs::Windows, PathBuf::from(r"C:\Users\me")),
            Err(PurgeError::UnexpectedTarget)
        );
        assert!(checked_target(TargetOs::Linux, PathBuf::from("/x/ai.xfalcon.desktop")).is_ok());
    }

    #[test]
    fn windows_pattern_anchors_the_exact_service() {
        assert_eq!(
            windows_target_pattern("ai.xfalcon.desktop"),
            r"^keyring:.*@ai\.xfalcon\.desktop$"
        );
        let pattern = regex::Regex::new(&windows_target_pattern("ai.xfalcon.desktop"))
            .expect("valid pattern");
        assert!(pattern.is_match("keyring:acct-1:key-v1@ai.xfalcon.desktop"));
        assert!(!pattern.is_match("keyring:acct-1:key-v1@ai.xfalcon.desktop.provider-keys"));
        assert!(!pattern.is_match("keyring:acct-1@other.ai.xfalcon.desktop"));
    }

    #[test]
    fn exit_codes_follow_the_documented_mapping() {
        let clean = PurgeReport::default();
        assert_eq!(exit_code(&Ok(clean)), 0);

        let residue = PurgeReport {
            residue: vec![PathBuf::from("/x/ai.xfalcon.desktop")],
            ..PurgeReport::default()
        };
        assert_eq!(exit_code(&Ok(residue)), 2);

        let keychain = PurgeReport {
            keychain_error: Some("store locked".to_owned()),
            ..PurgeReport::default()
        };
        assert_eq!(exit_code(&Ok(keychain)), 2);

        assert_eq!(
            exit_code(&Err(PurgeError::RelativeTarget)),
            1,
            "a refused purge is an environment error"
        );
    }

    #[test]
    fn keychain_is_purged_before_paths_and_its_error_does_not_stop_the_paths() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let target = purge_targets(host_os(), &env).expect("targets").paths[0].clone();
        fs::create_dir_all(&target).expect("data target");
        fs::write(target.join("state"), b"x").expect("state file");
        let order = RefCell::new(Vec::new());
        let failing_keychain = |services: &[&str]| -> Result<usize, PurgeError> {
            order
                .borrow_mut()
                .push(format!("keychain:{}", services.len()));
            assert!(target.exists(), "paths must not be deleted before keychain");
            Err(PurgeError::Keychain("store locked".to_owned()))
        };

        let report = run_purge(host_os(), &env, &failing_keychain).expect("purge runs");

        assert_eq!(order.into_inner(), vec!["keychain:2".to_owned()]);
        assert_eq!(
            report.keychain_error.as_deref(),
            Some("the credential store could not be cleared: store locked")
        );
        assert_eq!(report.deleted_paths, 1);
        assert!(!target.exists());
        assert!(!report.is_clean());
    }

    #[test]
    fn run_purge_deletes_the_fixed_targets_and_leaves_neighbours_alone() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let targets = purge_targets(host_os(), &env).expect("targets").paths;
        let legacy_snapshots = directory.path().join("tmp/xfalcon-upload-snapshots");
        fs::create_dir_all(&legacy_snapshots).expect("legacy temp directory");
        fs::write(legacy_snapshots.join("attachment"), b"unowned").expect("legacy entry");
        assert!(!targets.contains(&legacy_snapshots));
        for target in &targets {
            fs::create_dir_all(target).expect("target directory");
            fs::write(target.join("entry"), b"x").expect("entry");
        }
        let neighbour = targets[0]
            .parent()
            .expect("target has a parent")
            .join("ai.xfalcon.desktop-other");
        fs::create_dir_all(&neighbour).expect("neighbour");
        let services_seen = RefCell::new(Vec::new());
        let keychain = |services: &[&str]| -> Result<usize, PurgeError> {
            services_seen
                .borrow_mut()
                .extend(services.iter().map(|s| (*s).to_owned()));
            Ok(2)
        };

        let report = run_purge(host_os(), &env, &keychain).expect("purge runs");

        assert_eq!(report.deleted_paths, targets.len());
        assert_eq!(report.deleted_credentials, 2);
        assert!(report.is_clean(), "residue: {:?}", report.residue);
        assert_eq!(
            services_seen.into_inner(),
            vec![APP_ID.to_owned(), PROVIDER_KEYS_SERVICE.to_owned()]
        );
        for target in &targets {
            assert!(!target.exists(), "{target:?} must be purged");
        }
        assert!(legacy_snapshots.join("attachment").exists());
        assert!(neighbour.exists(), "a sibling name is never a target");
    }

    #[test]
    fn missing_required_roots_are_reported_and_prevent_clean_success() {
        let env = match host_os() {
            TargetOs::Windows => PurgeEnv::default(),
            TargetOs::MacOs | TargetOs::Linux => PurgeEnv {
                home: None,
                xdg_data_home: None,
                xdg_config_home: None,
                xdg_cache_home: None,
                ..PurgeEnv::default()
            },
        };

        let report = run_purge(host_os(), &env, &no_keychain).expect("purge runs");

        assert!(!report.is_clean());
        assert!(!report.unresolved_roots.is_empty());
        assert_eq!(exit_code(&Ok(report)), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_target_is_unlinked_and_its_destination_survives() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary tree");
        let root = directory.path();
        let env = host_env(root);
        let targets = purge_targets(host_os(), &env).expect("targets").paths;
        let outside = root.join("outside");
        fs::create_dir_all(&outside).expect("outside");
        fs::write(outside.join("keep"), b"not ours").expect("outside file");
        fs::create_dir_all(targets[0].parent().expect("parent")).expect("link parent");
        symlink(&outside, &targets[0]).expect("link target");

        let report = run_purge(host_os(), &env, &no_keychain).expect("purge runs");

        assert!(report.is_clean(), "residue: {:?}", report.residue);
        assert!(
            fs::symlink_metadata(&targets[0]).is_err(),
            "the link itself must be removed"
        );
        assert!(
            outside.join("keep").exists(),
            "the link's destination must never be followed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_target_does_not_stop_the_others_and_is_reported_as_residue() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let targets = purge_targets(host_os(), &env).expect("targets").paths;
        for target in &targets {
            fs::create_dir_all(target).expect("target directory");
            fs::write(target.join("entry"), b"x").expect("entry");
        }
        // The stuck target holds a read-only folder, so its contents cannot be
        // unlinked and the removal fails. Every other target must still go.
        let stuck = targets[1].clone();
        let locked = stuck.join("locked");
        fs::create_dir_all(&locked).expect("locked folder");
        fs::write(locked.join("entry"), b"x").expect("locked entry");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("read-only folder");

        let result = run_purge(host_os(), &env, &no_keychain);
        // Restored before any assertion so the temporary tree can be removed.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore folder");
        let report = result.expect("purge runs");

        for (index, target) in targets.iter().enumerate() {
            if index != 1 {
                assert!(
                    !target.exists(),
                    "later targets are still purged: {target:?}"
                );
            }
        }
        assert_eq!(report.residue, vec![stuck.clone()]);
        assert!(!report.is_clean());
        assert_eq!(exit_code(&Ok(report)), 2);
    }

    /// Makes the first purge leave `target` behind, and returns how to release it.
    #[cfg(windows)]
    fn block_target(target: &Path) -> Box<dyn FnOnce()> {
        use std::os::windows::fs::OpenOptionsExt;
        // std opens files shareable for delete by default, so a plain handle
        // would not stop the removal. Share mode 0 refuses every other open
        // and delete until the handle is dropped.
        fs::create_dir_all(target).expect("target directory");
        let handle = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(target.join("entry"))
            .expect("held entry");
        Box::new(move || drop(handle))
    }

    #[cfg(unix)]
    fn block_target(target: &Path) -> Box<dyn FnOnce()> {
        // A regular file where the target's parent folder should be makes every
        // lookup of the target fail with an error other than "not found".
        let parent = target.parent().expect("target has a parent").to_path_buf();
        fs::create_dir_all(parent.parent().expect("grandparent")).expect("ancestors");
        fs::write(&parent, b"x").expect("blocking file");
        Box::new(move || fs::remove_file(&parent).expect("release"))
    }

    #[test]
    fn parse_flags_reads_only_the_helper_flags() {
        let args = |list: &[&str]| list.iter().map(|s| OsString::from(*s)).collect::<Vec<_>>();

        assert_eq!(parse_flags(&args(&[])), PurgeFlags::default());
        assert_eq!(
            parse_flags(&args(&["--after-parent-exit"])),
            PurgeFlags {
                after_parent: true,
                ..PurgeFlags::default()
            }
        );
        assert_eq!(
            parse_flags(&args(&["--after-parent-exit", "--relaunch"])),
            PurgeFlags {
                after_parent: true,
                relaunch: true,
                retry: false,
            }
        );
        assert_eq!(
            parse_flags(&args(&["--unrelated", "--relaunch"])),
            PurgeFlags {
                relaunch: true,
                ..PurgeFlags::default()
            }
        );
        assert_eq!(
            parse_flags(&args(&["--after-parent-exit-now"])),
            PurgeFlags::default()
        );
    }

    #[test]
    fn parse_flags_reads_the_retry_flag_on_its_own() {
        let args = |list: &[&str]| list.iter().map(|s| OsString::from(*s)).collect::<Vec<_>>();

        assert_eq!(
            parse_flags(&args(&["--retry-until-clean"])),
            PurgeFlags {
                retry: true,
                ..PurgeFlags::default()
            }
        );
        assert_eq!(
            parse_flags(&args(&["--retry-until-clean-now"])),
            PurgeFlags::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn parse_flags_does_not_panic_on_a_non_utf8_argument() {
        use std::os::unix::ffi::OsStringExt;

        let args = [
            OsString::from_vec(vec![0xff, 0xfe]),
            OsString::from(RETRY_FLAG),
        ];

        assert_eq!(
            parse_flags(&args),
            PurgeFlags {
                retry: true,
                ..PurgeFlags::default()
            }
        );
    }

    #[test]
    fn absolute_path_keeps_only_absolute_values() {
        // An absolute path on the host running the test: `/var/cache` has no drive
        // letter and is not absolute on Windows.
        let absolute = std::env::temp_dir();
        assert!(absolute.is_absolute());
        assert_eq!(absolute_path(absolute.clone()), Some(absolute));
        assert_eq!(absolute_path(PathBuf::from("cache")), None);
        assert_eq!(absolute_path(PathBuf::from("./cache")), None);
    }

    #[test]
    fn a_search_failure_never_reads_as_a_clear_store() {
        assert_eq!(
            search_failure(&keyring_core::Error::NoDefaultStore),
            PurgeError::Keychain("no credential store is available".to_owned())
        );
        assert_eq!(
            search_failure(&keyring_core::Error::NotSupportedByStore("x".to_owned())),
            PurgeError::Keychain("the credential store cannot search for entries".to_owned())
        );
        let other = keyring_core::Error::Invalid("user".to_owned(), "bad".to_owned());
        assert_eq!(
            search_failure(&other),
            PurgeError::Keychain(other.to_string())
        );
    }

    #[test]
    fn wait_for_eof_reports_end_of_file_at_once() {
        assert!(wait_for_eof(io::empty(), Duration::from_secs(5)));
        assert!(wait_for_eof(
            io::Cursor::new(b"bytes before the end".to_vec()),
            Duration::from_secs(5)
        ));
    }

    /// A reader that blocks while its writer is alive, as a pipe does while the
    /// parent runs. It reports end of file once the writer is dropped.
    struct HeldOpen(mpsc::Receiver<()>);

    impl io::Read for HeldOpen {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    #[test]
    fn wait_for_eof_times_out_while_the_writer_is_alive() {
        let (writer, held) = mpsc::channel::<()>();

        assert!(!wait_for_eof(HeldOpen(held), Duration::from_millis(50)));
        drop(writer);
    }

    #[test]
    fn run_purge_until_clean_stops_after_the_first_clean_run() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let sleeps = Cell::new(0);
        let sleep = |_: Duration| sleeps.set(sleeps.get() + 1);

        let report =
            run_purge_until_clean(host_os(), &env, &no_keychain, 5, Duration::ZERO, &sleep)
                .expect("purge runs");

        assert!(report.is_clean(), "residue: {:?}", report.residue);
        assert_eq!(sleeps.get(), 0);
    }

    #[test]
    fn run_purge_until_clean_retries_once_the_blocked_target_is_released() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let target = purge_targets(host_os(), &env).expect("targets").paths[0].clone();
        let release = RefCell::new(Some(block_target(&target)));
        let sleeps = Cell::new(0);
        let sleep = |_: Duration| {
            sleeps.set(sleeps.get() + 1);
            let next = release.borrow_mut().take();
            if let Some(release) = next {
                release();
            }
        };

        let report =
            run_purge_until_clean(host_os(), &env, &no_keychain, 5, Duration::ZERO, &sleep)
                .expect("purge runs");

        assert_eq!(sleeps.get(), 1);
        assert!(report.is_clean(), "residue: {:?}", report.residue);
        assert!(!target.exists(), "{target:?} must be purged on the retry");
    }

    #[test]
    fn run_purge_until_clean_retries_a_credential_error() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let calls = Cell::new(0);
        let keychain = |_: &[&str]| -> Result<usize, PurgeError> {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Err(PurgeError::Keychain("store locked".to_owned()))
            } else {
                Ok(2)
            }
        };
        let sleeps = Cell::new(0);
        let sleep = |_: Duration| sleeps.set(sleeps.get() + 1);

        let report = run_purge_until_clean(host_os(), &env, &keychain, 5, Duration::ZERO, &sleep)
            .expect("purge runs");

        assert_eq!(calls.get(), 2, "the credential step runs again");
        assert_eq!(sleeps.get(), 1);
        assert!(report.is_clean(), "residue: {:?}", report.residue);
        assert_eq!(report.deleted_credentials, 2);
    }

    #[test]
    fn run_purge_until_clean_gives_up_after_the_attempt_limit() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let env = host_env(directory.path());
        let target = purge_targets(host_os(), &env).expect("targets").paths[0].clone();
        // Never released: the residue outlasts every attempt.
        let _held = block_target(&target);
        let sleeps = Cell::new(0);
        let sleep = |_: Duration| sleeps.set(sleeps.get() + 1);

        let report =
            run_purge_until_clean(host_os(), &env, &no_keychain, 4, Duration::ZERO, &sleep)
                .expect("purge runs");

        assert_eq!(sleeps.get(), 3, "four runs make three pauses");
        assert_eq!(report.residue, vec![target]);
    }

    #[test]
    fn run_purge_until_clean_returns_a_refusal_without_retrying() {
        let directory = tempfile::tempdir().expect("temporary tree");
        let mut env = host_env(directory.path());
        match host_os() {
            TargetOs::Windows => env.local_app_data = Some(PathBuf::from(r"relative\local")),
            TargetOs::MacOs | TargetOs::Linux => env.home = Some(PathBuf::from("relative/home")),
        }
        let sleeps = Cell::new(0);
        let sleep = |_: Duration| sleeps.set(sleeps.get() + 1);

        let result =
            run_purge_until_clean(host_os(), &env, &no_keychain, 5, Duration::ZERO, &sleep);

        assert_eq!(result, Err(PurgeError::RelativeTarget));
        assert_eq!(sleeps.get(), 0);
    }

    #[test]
    fn relaunch_opens_a_macos_bundle_and_runs_other_executables_directly() {
        let bundle = Path::new("/Applications/xFalcon.app/Contents/MacOS/xfalcon");
        assert_eq!(
            relaunch_command(bundle),
            (
                PathBuf::from("open"),
                vec![
                    OsString::from("-n"),
                    OsString::from("/Applications/xFalcon.app"),
                ]
            )
        );

        let plain = Path::new("/opt/xfalcon/xfalcon");
        assert_eq!(relaunch_command(plain), (plain.to_path_buf(), Vec::new()));

        let outside_macos_dir = Path::new("/Users/me/Downloads/xFalcon.app/xfalcon");
        assert_eq!(
            relaunch_command(outside_macos_dir),
            (outside_macos_dir.to_path_buf(), Vec::new())
        );
    }

    #[test]
    fn helper_args_wait_for_the_parent_and_relaunch_only_when_asked() {
        assert_eq!(
            helper_args(false),
            vec!["--purge-local-data", "--after-parent-exit"]
        );
        assert_eq!(
            helper_args(true),
            vec!["--purge-local-data", "--after-parent-exit", "--relaunch"]
        );
    }

    #[test]
    fn reset_start_errors_name_no_path_or_os_detail() {
        for error in [ResetStartError::AlreadyStarted, ResetStartError::Spawn] {
            let message = error.to_string();
            assert!(
                !message.contains('/') && !message.contains('\\'),
                "{message}"
            );
        }
    }

    /// Exercises the real keychain search and delete path against the in-memory
    /// mock store. The mock matches `service` by substring and ignores
    /// `pattern`, so this test uses the non-Windows spec on every platform; the
    /// exact-service filter in `delete_matching_credentials` keeps the result
    /// correct either way.
    #[test]
    fn keychain_purge_deletes_only_the_two_services_entries() {
        keyring_core::set_default_store(keyring_core::mock::Store::new().expect("mock store"));
        let entries = [
            (APP_ID, "acct-1:key-v1"),
            (APP_ID, "acct-1:active"),
            (PROVIDER_KEYS_SERVICE, "acct-1:key-v1"),
            ("other.app", "acct-1:key-v1"),
        ];
        for (service, user) in entries {
            keyring_core::Entry::new(service, user)
                .expect("entry")
                .set_password("secret")
                .expect("set password");
        }

        let (deleted, error) = delete_matching_credentials(&[APP_ID, PROVIDER_KEYS_SERVICE]);

        assert_eq!(error, None);
        assert_eq!(deleted, 3);
        for (service, user) in [
            (APP_ID, "acct-1:key-v1"),
            (APP_ID, "acct-1:active"),
            (PROVIDER_KEYS_SERVICE, "acct-1:key-v1"),
        ] {
            assert!(matches!(
                keyring_core::Entry::new(service, user)
                    .expect("entry")
                    .get_password(),
                Err(keyring_core::Error::NoEntry)
            ));
        }
        assert_eq!(
            keyring_core::Entry::new("other.app", "acct-1:key-v1")
                .expect("entry")
                .get_password()
                .expect("unrelated service survives"),
            "secret"
        );
        keyring_core::unset_default_store();
    }
}
