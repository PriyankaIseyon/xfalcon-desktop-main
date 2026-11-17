//! Finds the user's installed `claude` program and checks that its version is
//! supported. Only real executables are candidates: a Windows batch file is
//! never returned, because `cmd.exe` would reinterpret the arguments given to it.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::process::{command, run_capped, ProcessError};

pub(crate) const MIN_CLAUDE_VERSION: ClaudeVersion = ClaudeVersion {
    major: 2,
    minor: 1,
    patch: 295,
};

/// Longer PATH entries are not install directories, so they are ignored.
const MAX_PATH_ENTRY_CHARS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct ClaudeVersion {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) patch: u32,
}

impl ClaudeVersion {
    /// Reads the first whitespace-separated token that is exactly `N.N.N`,
    /// optionally prefixed with `v`. A suffix such as `-beta` is refused rather
    /// than read as the release it precedes.
    pub(crate) fn parse_from_output(stdout: &str) -> Option<Self> {
        stdout.split_whitespace().find_map(|token| {
            let mut parts = token.strip_prefix('v').unwrap_or(token).split('.');
            let major = parse_number(parts.next()?)?;
            let minor = parse_number(parts.next()?)?;
            let patch = parse_number(parts.next()?)?;
            parts.next().is_none().then_some(Self {
                major,
                minor,
                patch,
            })
        })
    }
}

/// `str::parse` would also accept a leading `+`.
fn parse_number(part: &str) -> Option<u32> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

impl fmt::Display for ClaudeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub(crate) fn version_is_supported(version: ClaudeVersion) -> bool {
    version >= MIN_CLAUDE_VERSION
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostOs {
    Windows,
    MacOs,
    Linux,
}

impl HostOs {
    pub(crate) const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

pub(crate) trait LocateEnv {
    fn os(&self) -> HostOs;
    fn var(&self, name: &str) -> Option<OsString>;
    fn is_file(&self, path: &Path) -> bool;
    fn home_dir(&self) -> Option<PathBuf>;
}

pub(crate) struct RealLocateEnv;

impl LocateEnv for RealLocateEnv {
    fn os(&self) -> HostOs {
        HostOs::current()
    }

    fn var(&self, name: &str) -> Option<OsString> {
        // Windows resolves variable names case-insensitively in the OS itself.
        std::env::var_os(name)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file() && is_executable(path)
    }

    fn home_dir(&self) -> Option<PathBuf> {
        let name = if self.os() == HostOs::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        };
        self.var(name).map(PathBuf::from)
    }
}

/// On Unix only a file with an execute bit runs. A stale file without one must not
/// end the search, so it is skipped like a missing file.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

/// Windows runs a file by its name, so an existing file is enough there.
#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

/// Every place a `claude` program may live, in the order they are tried.
pub(crate) fn candidate_paths(env: &dyn LocateEnv) -> Vec<PathBuf> {
    let os = env.os();
    let mut candidates = Vec::new();
    for dir in path_dirs(env) {
        match os {
            HostOs::Windows => {
                candidates.push(dir.join("claude.exe"));
                // npm installs a `claude.cmd` shim, which is never spawned. The
                // executable it starts is used in its place.
                if env.is_file(&dir.join("claude.cmd")) {
                    candidates.push(npm_package_exe(&dir));
                }
            }
            HostOs::MacOs | HostOs::Linux => candidates.push(dir.join("claude")),
        }
    }

    let home = env
        .home_dir()
        .filter(|home| is_absolute_entry(os, &home.to_string_lossy()));
    match os {
        HostOs::Windows => {
            if let Some(home) = &home {
                candidates.push(join_all(home, &[".local", "bin", "claude.exe"]));
            }
            if let Some(appdata) = env_dir(env, "APPDATA") {
                candidates.push(npm_package_exe(&join_all(&appdata, &["npm"])));
            }
            if let Some(local_appdata) = env_dir(env, "LOCALAPPDATA") {
                candidates.push(join_all(
                    &local_appdata,
                    &["Programs", "claude", "claude.exe"],
                ));
            }
        }
        HostOs::MacOs | HostOs::Linux => {
            if let Some(home) = &home {
                candidates.push(join_all(home, &[".local", "bin", "claude"]));
                candidates.push(join_all(home, &[".claude", "local", "claude"]));
                // Directories where global npm, version managers and bun put `claude`.
                candidates.push(join_all(home, &["bin", "claude"]));
                candidates.push(join_all(home, &[".npm-global", "bin", "claude"]));
                candidates.push(join_all(home, &[".volta", "bin", "claude"]));
                candidates.push(join_all(home, &[".bun", "bin", "claude"]));
                candidates.push(join_all(home, &[".asdf", "shims", "claude"]));
            }
            candidates.push(PathBuf::from("/usr/local/bin/claude"));
            // MacPorts installs here.
            candidates.push(PathBuf::from("/opt/local/bin/claude"));
            if os == HostOs::MacOs {
                candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
            }
        }
    }
    candidates
}

pub(crate) fn locate_claude(env: &dyn LocateEnv) -> Option<PathBuf> {
    candidate_paths(env)
        .into_iter()
        .find(|candidate| env.is_file(candidate))
}

/// What the search for `claude` established. `Failed` is kept apart from `NotFound`:
/// a lookup that did not finish says nothing about whether the program is installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaudeLookup {
    Found(PathBuf),
    NotFound,
    /// Only the macOS login-shell lookup can fail.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Failed,
}

/// Brackets the PATH in the login shell's output, so the profile text printed before
/// and after it is ignored.
#[cfg(any(target_os = "macos", test))]
const PATH_BEGIN: &str = "__XF_PATH_BEGIN__";
#[cfg(any(target_os = "macos", test))]
const PATH_END: &str = "__XF_PATH_END__";

/// Asks the user's login shell for its PATH, because a Dock-launched app inherits
/// launchd's minimal PATH and cannot see the directories the shell adds for `claude`.
#[cfg(target_os = "macos")]
pub(crate) async fn login_shell_claude_path() -> ClaudeLookup {
    let shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|shell| shell.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
    let script = format!(r#"printf "{PATH_BEGIN}%s{PATH_END}" "$PATH""#);
    let mut lookup = command(&shell);
    // `-i` as well as `-l`: zsh reads `.zshrc` only in an interactive shell, and that
    // file is where nvm, volta and asdf set PATH. Stdin is null, so nothing can wait.
    lookup.args(["-l", "-i", "-c"]).arg(&script);
    let Ok(captured) = run_capped(lookup, None, Duration::from_secs(10), 64 * 1024).await else {
        return ClaudeLookup::Failed;
    };
    let stdout = String::from_utf8_lossy(&captured.stdout);
    claude_from_login_shell(&stdout, &RealLocateEnv)
}

// Linux and Windows do not ask a login shell: their PATH is already the user's own.
#[cfg(not(target_os = "macos"))]
pub(crate) async fn login_shell_claude_path() -> ClaudeLookup {
    ClaudeLookup::NotFound
}

pub(crate) async fn discover_claude() -> ClaudeLookup {
    if let Some(path) = locate_claude(&RealLocateEnv) {
        return ClaudeLookup::Found(path);
    }
    login_shell_claude_path().await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum LocateError {
    #[error("claude could not be started")]
    StartFailed,
    #[error("claude printed an unreadable version")]
    Unparseable,
    #[error("claude did not answer in time")]
    TimedOut,
}

pub(crate) async fn probe_version(path: &Path) -> Result<ClaudeVersion, LocateError> {
    if is_batch_script(path) {
        return Err(LocateError::StartFailed);
    }
    let mut probe = command(path);
    probe.arg("--version");
    // A program that exits non-zero still identifies itself, so its output is read
    // whatever its status.
    let captured = run_capped(probe, None, Duration::from_secs(10), 4096)
        .await
        .map_err(probe_error)?;
    let stdout = String::from_utf8_lossy(&captured.stdout);
    ClaudeVersion::parse_from_output(&stdout).ok_or(LocateError::Unparseable)
}

fn probe_error(error: ProcessError) -> LocateError {
    match error {
        ProcessError::StartFailed => LocateError::StartFailed,
        ProcessError::TimedOut => LocateError::TimedOut,
        ProcessError::OutputTooLarge | ProcessError::Io => LocateError::Unparseable,
    }
}

fn path_dirs(env: &dyn LocateEnv) -> Vec<PathBuf> {
    let Some(value) = env.var("PATH") else {
        return Vec::new();
    };
    // Lossy conversion alters only a non-UTF-8 entry, which then matches nothing.
    split_path(env.os(), &value.to_string_lossy())
}

/// Splits a PATH value with the separator of the OS being searched, not of the host,
/// so that a Windows PATH can be searched from any machine.
fn split_path(os: HostOs, value: &str) -> Vec<PathBuf> {
    let separator = if os == HostOs::Windows { ';' } else { ':' };
    value
        .split(separator)
        .map(|dir| unquoted_entry(os, dir))
        .filter(|dir| !dir.is_empty() && dir.chars().count() <= MAX_PATH_ENTRY_CHARS)
        .filter(|dir| is_absolute_entry(os, dir))
        .map(PathBuf::from)
        .collect()
}

/// Windows installers quote a PATH entry that holds a space, as in
/// `"C:\Program Files\nodejs"`. One pair of surrounding quotes is removed.
fn unquoted_entry(os: HostOs, dir: &str) -> &str {
    match (
        os,
        dir.strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"')),
    ) {
        (HostOs::Windows, Some(inner)) => inner,
        _ => dir,
    }
}

/// A relative entry would resolve against the working directory, so a stray
/// `claude` there could run in place of the installed one. The OS being searched
/// decides what is absolute, so a Windows PATH is judged the same on any host.
fn is_absolute_entry(os: HostOs, dir: &str) -> bool {
    match os {
        HostOs::Windows => {
            let drive_path = matches!(
                dir.as_bytes(),
                [drive, b':', b'\\' | b'/', ..] if drive.is_ascii_alphabetic()
            );
            drive_path || dir.starts_with(r"\\")
        }
        HostOs::MacOs | HostOs::Linux => dir.starts_with('/'),
    }
}

/// An empty or relative variable counts as unset: a relative base would resolve
/// against the working directory, where a stray `claude` must not be picked up.
fn env_dir(env: &dyn LocateEnv, name: &str) -> Option<PathBuf> {
    let os = env.os();
    env.var(name)
        .filter(|value| is_absolute_entry(os, &value.to_string_lossy()))
        .map(PathBuf::from)
}

/// The executable that npm's `claude.cmd` shim starts, given the directory that
/// holds the shim.
fn npm_package_exe(npm_dir: &Path) -> PathBuf {
    join_all(
        npm_dir,
        &[
            "node_modules",
            "@anthropic-ai",
            "claude-code",
            "bin",
            "claude.exe",
        ],
    )
}

fn join_all(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

/// Windows drops trailing dots and spaces from a file name, so `claude.cmd.`
/// still runs the batch file.
fn is_batch_script(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name
        .to_string_lossy()
        .trim_end_matches(['.', ' '])
        .to_ascii_lowercase();
    [".cmd", ".bat", ".ps1"]
        .into_iter()
        .any(|extension| name.ends_with(extension))
}

/// Searches the PATH that the login shell printed between the markers. Output
/// without both markers means the shell did not reach its last command, so the
/// lookup did not finish. A finished lookup with no `claude` in its PATH is
/// `NotFound`.
#[cfg(any(target_os = "macos", test))]
fn claude_from_login_shell(stdout: &str, env: &dyn LocateEnv) -> ClaudeLookup {
    let Some(start) = stdout.find(PATH_BEGIN) else {
        return ClaudeLookup::Failed;
    };
    let text = &stdout[start + PATH_BEGIN.len()..];
    let Some(end) = text.find(PATH_END) else {
        return ClaudeLookup::Failed;
    };
    // Only absolute entries are kept, as in the search of the process PATH.
    let found = split_path(env.os(), &text[..end])
        .into_iter()
        .map(|dir| dir.join("claude"))
        .find(|candidate| env.is_file(candidate));
    found.map_or(ClaudeLookup::NotFound, ClaudeLookup::Found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    struct FakeEnv {
        os: HostOs,
        vars: BTreeMap<String, OsString>,
        files: BTreeSet<PathBuf>,
        home: Option<PathBuf>,
    }

    impl FakeEnv {
        fn new(os: HostOs) -> Self {
            Self {
                os,
                vars: BTreeMap::new(),
                files: BTreeSet::new(),
                home: None,
            }
        }

        fn with_var(mut self, name: &str, value: &str) -> Self {
            self.vars.insert(name.to_owned(), OsString::from(value));
            self
        }

        fn with_home(mut self, home: &str) -> Self {
            self.home = Some(PathBuf::from(home));
            self
        }

        fn with_file(mut self, path: PathBuf) -> Self {
            self.files.insert(path);
            self
        }
    }

    impl LocateEnv for FakeEnv {
        fn os(&self) -> HostOs {
            self.os
        }

        fn var(&self, name: &str) -> Option<OsString> {
            self.vars
                .iter()
                .find(|(key, _)| match self.os {
                    HostOs::Windows => key.eq_ignore_ascii_case(name),
                    HostOs::MacOs | HostOs::Linux => key.as_str() == name,
                })
                .map(|(_, value)| value.clone())
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }

        fn home_dir(&self) -> Option<PathBuf> {
            self.home.clone()
        }
    }

    const ALL_OS: [HostOs; 3] = [HostOs::Windows, HostOs::MacOs, HostOs::Linux];

    fn sample_env(os: HostOs) -> FakeEnv {
        match os {
            HostOs::Windows => FakeEnv::new(os)
                .with_var("PATH", r"C:\tools;C:\npm")
                .with_var("APPDATA", r"C:\Users\me\AppData\Roaming")
                .with_var("LOCALAPPDATA", r"C:\Users\me\AppData\Local")
                .with_home(r"C:\Users\me"),
            HostOs::MacOs | HostOs::Linux => FakeEnv::new(os)
                .with_var("PATH", "/tools:/npm")
                .with_home("/home/me"),
        }
    }

    fn version(major: u32, minor: u32, patch: u32) -> ClaudeVersion {
        ClaudeVersion {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn parses_each_known_version_output_shape() {
        for output in [
            "2.1.296 (Claude Code)",
            "v2.1.296",
            "claude 2.1.296",
            "2.1.296\n",
        ] {
            assert_eq!(
                ClaudeVersion::parse_from_output(output),
                Some(version(2, 1, 296)),
                "{output:?}"
            );
        }
    }

    #[test]
    fn rejects_output_without_a_plain_three_part_version() {
        for output in [
            "",
            "abc",
            "2.1",
            "2.1.296.1",
            "2.1.296-beta",
            "1.2.x",
            "+2.1.296",
        ] {
            assert_eq!(ClaudeVersion::parse_from_output(output), None, "{output:?}");
        }
    }

    #[test]
    fn displays_as_a_dotted_triplet() {
        assert_eq!(version(2, 1, 296).to_string(), "2.1.296");
    }

    #[test]
    fn supports_the_minimum_version_and_later_only() {
        assert!(!version_is_supported(version(2, 1, 284)));
        assert!(version_is_supported(version(3, 0, 0)));
        assert!(version_is_supported(MIN_CLAUDE_VERSION));
    }

    #[test]
    fn two_one_two_ninety_four_is_outdated_and_two_one_two_ninety_five_is_accepted() {
        assert_eq!(MIN_CLAUDE_VERSION, version(2, 1, 295));
        assert!(!version_is_supported(version(2, 1, 294)));
        assert!(version_is_supported(version(2, 1, 295)));
    }

    #[test]
    fn linux_candidates_follow_path_then_fallbacks() {
        let env = FakeEnv::new(HostOs::Linux)
            .with_var("PATH", "/a:/b")
            .with_home("/home/me");
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new("/a"), &["claude"]),
                join_all(Path::new("/b"), &["claude"]),
                join_all(Path::new("/home/me"), &[".local", "bin", "claude"]),
                join_all(Path::new("/home/me"), &[".claude", "local", "claude"]),
                join_all(Path::new("/home/me"), &["bin", "claude"]),
                join_all(Path::new("/home/me"), &[".npm-global", "bin", "claude"]),
                join_all(Path::new("/home/me"), &[".volta", "bin", "claude"]),
                join_all(Path::new("/home/me"), &[".bun", "bin", "claude"]),
                join_all(Path::new("/home/me"), &[".asdf", "shims", "claude"]),
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude"),
            ]
        );
    }

    #[test]
    fn macos_candidates_add_homebrew_last() {
        let env = FakeEnv::new(HostOs::MacOs)
            .with_var("PATH", "/a:/b")
            .with_home("/Users/me");
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new("/a"), &["claude"]),
                join_all(Path::new("/b"), &["claude"]),
                join_all(Path::new("/Users/me"), &[".local", "bin", "claude"]),
                join_all(Path::new("/Users/me"), &[".claude", "local", "claude"]),
                join_all(Path::new("/Users/me"), &["bin", "claude"]),
                join_all(Path::new("/Users/me"), &[".npm-global", "bin", "claude"]),
                join_all(Path::new("/Users/me"), &[".volta", "bin", "claude"]),
                join_all(Path::new("/Users/me"), &[".bun", "bin", "claude"]),
                join_all(Path::new("/Users/me"), &[".asdf", "shims", "claude"]),
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude"),
                PathBuf::from("/opt/homebrew/bin/claude"),
            ]
        );
    }

    #[test]
    fn windows_candidates_follow_path_npm_shim_then_fallbacks() {
        let npm = Path::new(r"C:\npm");
        let env = sample_env(HostOs::Windows).with_file(join_all(npm, &["claude.cmd"]));
        let appdata = Path::new(r"C:\Users\me\AppData\Roaming");
        let local_appdata = Path::new(r"C:\Users\me\AppData\Local");
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new(r"C:\tools"), &["claude.exe"]),
                join_all(npm, &["claude.exe"]),
                npm_package_exe(npm),
                join_all(Path::new(r"C:\Users\me"), &[".local", "bin", "claude.exe"]),
                npm_package_exe(&join_all(appdata, &["npm"])),
                join_all(local_appdata, &["Programs", "claude", "claude.exe"]),
            ]
        );
    }

    #[test]
    fn windows_path_splits_on_semicolons_and_keeps_drive_letters() {
        let env = FakeEnv::new(HostOs::Windows).with_var("PATH", r"C:\tools;D:\bin;;");
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new(r"C:\tools"), &["claude.exe"]),
                join_all(Path::new(r"D:\bin"), &["claude.exe"]),
            ]
        );
    }

    #[test]
    fn unix_path_splits_on_colons_and_keeps_semicolons() {
        let env = FakeEnv::new(HostOs::Linux).with_var("PATH", "/a;b::/c:");
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new("/a;b"), &["claude"]),
                join_all(Path::new("/c"), &["claude"]),
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude"),
            ]
        );
    }

    #[test]
    fn overlong_path_entries_are_skipped_at_the_limit() {
        let kept = format!("/{}", "k".repeat(MAX_PATH_ENTRY_CHARS - 1));
        let dropped = format!("/{}", "d".repeat(MAX_PATH_ENTRY_CHARS));
        let env = FakeEnv::new(HostOs::Linux).with_var("PATH", &format!("{dropped}:{kept}"));
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new(&kept), &["claude"]),
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude"),
            ]
        );
    }

    #[test]
    fn relative_path_entries_are_skipped_on_each_os() {
        let windows =
            FakeEnv::new(HostOs::Windows).with_var("PATH", r"bin;.;..\x;./bin;C:tools;C:\tools");
        assert_eq!(path_dirs(&windows), vec![PathBuf::from(r"C:\tools")]);

        for os in [HostOs::MacOs, HostOs::Linux] {
            let env = FakeEnv::new(os)
                .with_var("PATH", "bin:.:..\\x:./bin:/tools")
                .with_file(PathBuf::from("bin/claude"));
            assert_eq!(path_dirs(&env), vec![PathBuf::from("/tools")], "{os:?}");
            assert_eq!(locate_claude(&env), None, "{os:?}");
        }
    }

    #[test]
    fn windows_drive_and_unc_path_entries_are_kept_on_any_host() {
        let env =
            FakeEnv::new(HostOs::Windows).with_var("PATH", r"C:\tools;d:/bin;\\server\share\bin");
        assert_eq!(
            path_dirs(&env),
            vec![
                PathBuf::from(r"C:\tools"),
                PathBuf::from("d:/bin"),
                PathBuf::from(r"\\server\share\bin"),
            ]
        );
    }

    #[test]
    fn unix_absolute_path_entries_are_kept() {
        for os in [HostOs::MacOs, HostOs::Linux] {
            let env = FakeEnv::new(os).with_var("PATH", "/usr/bin:/opt/tools");
            assert_eq!(
                path_dirs(&env),
                vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/tools")],
                "{os:?}"
            );
        }
    }

    #[test]
    fn empty_home_and_app_data_are_treated_as_unset() {
        let linux = FakeEnv::new(HostOs::Linux).with_home("");
        assert_eq!(
            candidate_paths(&linux),
            vec![
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude")
            ]
        );

        let windows = FakeEnv::new(HostOs::Windows)
            .with_home("")
            .with_var("APPDATA", "")
            .with_var("LOCALAPPDATA", "");
        assert_eq!(candidate_paths(&windows), Vec::<PathBuf>::new());
    }

    #[test]
    fn relative_home_and_app_data_directories_are_never_searched() {
        let linux = FakeEnv::new(HostOs::Linux)
            .with_home("relative/home")
            .with_file(join_all(
                Path::new("relative/home"),
                &[".local", "bin", "claude"],
            ))
            .with_file(join_all(
                Path::new("relative/home"),
                &[".claude", "local", "claude"],
            ));
        assert_eq!(
            candidate_paths(&linux),
            vec![
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude")
            ]
        );
        assert_eq!(locate_claude(&linux), None);

        let appdata = Path::new(r"relative\appdata");
        let local_appdata = Path::new(r"relative\localappdata");
        let windows = FakeEnv::new(HostOs::Windows)
            .with_home(r"relative\home")
            .with_var("APPDATA", r"relative\appdata")
            .with_var("LOCALAPPDATA", r"relative\localappdata")
            .with_file(join_all(
                Path::new(r"relative\home"),
                &[".local", "bin", "claude.exe"],
            ))
            .with_file(npm_package_exe(&join_all(appdata, &["npm"])))
            .with_file(join_all(
                local_appdata,
                &["Programs", "claude", "claude.exe"],
            ));
        assert_eq!(candidate_paths(&windows), Vec::<PathBuf>::new());
        assert_eq!(locate_claude(&windows), None);
    }

    #[test]
    fn path_entries_come_before_fallbacks_on_each_os() {
        let linux = FakeEnv::new(HostOs::Linux)
            .with_var("PATH", "/custom/bin")
            .with_home("/home/me")
            .with_file(join_all(Path::new("/custom/bin"), &["claude"]))
            .with_file(join_all(
                Path::new("/home/me"),
                &[".local", "bin", "claude"],
            ));
        assert_eq!(
            locate_claude(&linux),
            Some(join_all(Path::new("/custom/bin"), &["claude"]))
        );

        let windows = FakeEnv::new(HostOs::Windows)
            .with_var("PATH", r"C:\tools")
            .with_home(r"C:\Users\me")
            .with_file(join_all(Path::new(r"C:\tools"), &["claude.exe"]))
            .with_file(join_all(
                Path::new(r"C:\Users\me"),
                &[".local", "bin", "claude.exe"],
            ));
        assert_eq!(
            locate_claude(&windows),
            Some(join_all(Path::new(r"C:\tools"), &["claude.exe"]))
        );
    }

    #[test]
    fn fallbacks_are_used_when_path_has_no_program() {
        let env = FakeEnv::new(HostOs::Linux)
            .with_home("/home/me")
            .with_file(join_all(
                Path::new("/home/me"),
                &[".claude", "local", "claude"],
            ));
        assert_eq!(
            locate_claude(&env),
            Some(join_all(
                Path::new("/home/me"),
                &[".claude", "local", "claude"]
            ))
        );
    }

    #[test]
    fn npm_shim_resolves_to_its_package_executable_when_it_exists() {
        let npm = Path::new(r"C:\npm");
        let env = FakeEnv::new(HostOs::Windows)
            .with_var("PATH", r"C:\npm")
            .with_file(join_all(npm, &["claude.cmd"]))
            .with_file(npm_package_exe(npm));
        assert_eq!(locate_claude(&env), Some(npm_package_exe(npm)));
    }

    #[test]
    fn npm_shim_is_skipped_when_its_package_executable_is_missing() {
        let npm = Path::new(r"C:\npm");
        let env = FakeEnv::new(HostOs::Windows)
            .with_var("PATH", r"C:\npm")
            .with_file(join_all(npm, &["claude.cmd"]));
        assert_eq!(locate_claude(&env), None);
    }

    #[test]
    fn claude_exe_wins_over_the_npm_package() {
        let npm = Path::new(r"C:\npm");
        let env = FakeEnv::new(HostOs::Windows)
            .with_var("PATH", r"C:\npm")
            .with_file(join_all(npm, &["claude.cmd"]))
            .with_file(join_all(npm, &["claude.exe"]))
            .with_file(npm_package_exe(npm));
        assert_eq!(locate_claude(&env), Some(join_all(npm, &["claude.exe"])));
    }

    #[test]
    fn batch_scripts_are_never_located() {
        for os in ALL_OS {
            let mut env = sample_env(os);
            for dir in path_dirs(&env) {
                for name in ["claude.cmd", "claude.bat", "claude.ps1"] {
                    env = env.with_file(join_all(&dir, &[name]));
                }
            }
            assert_eq!(locate_claude(&env), None, "{os:?}");

            for candidate in candidate_paths(&env) {
                env = env.with_file(candidate);
            }
            let located = locate_claude(&env).expect("every candidate exists");
            assert!(!is_batch_script(&located), "{os:?}: {located:?}");
        }
    }

    #[test]
    fn batch_script_check_ignores_case_and_trailing_dots_and_spaces() {
        for name in [
            "claude.cmd",
            "CLAUDE.CMD",
            "run.Bat",
            "run.ps1",
            "claude.cmd.",
            "claude.cmd ",
        ] {
            assert!(is_batch_script(Path::new(name)), "{name}");
        }
        for name in [
            "claude",
            "claude.exe",
            "claude.cmd.exe",
            "cmd",
            "claude.com",
        ] {
            assert!(!is_batch_script(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn locate_returns_none_when_nothing_exists() {
        for os in ALL_OS {
            assert_eq!(locate_claude(&sample_env(os)), None, "{os:?}");
        }
    }

    /// The output a login shell prints for a PATH, with its markers.
    fn shell_output(before: &str, path: &str, after: &str) -> String {
        format!("{before}{PATH_BEGIN}{path}{PATH_END}{after}")
    }

    #[test]
    fn login_shell_path_is_searched_for_claude_in_its_own_entries() {
        // A fixed Unix path: a macOS PATH is split on ':', and a Windows temp path
        // holds a colon of its own.
        let entries = "/opt/xf-test-bin";
        let program = Path::new(entries).join("claude");
        let env = FakeEnv::new(HostOs::MacOs).with_file(program.clone());

        // Profile output before and after the markers is ignored.
        let stdout = shell_output("welcome, me\n", &format!("/usr/bin:{entries}"), "\nbye\n");
        assert_eq!(
            claude_from_login_shell(&stdout, &env),
            ClaudeLookup::Found(program.clone())
        );
        // A CRLF from the profile after the markers does not matter.
        let stdout = shell_output("", entries, "\r\n");
        assert_eq!(
            claude_from_login_shell(&stdout, &env),
            ClaudeLookup::Found(program)
        );
    }

    #[test]
    fn login_shell_output_without_both_markers_is_a_failed_lookup() {
        let env = FakeEnv::new(HostOs::MacOs).with_file(PathBuf::from("/bin/claude"));
        assert_eq!(
            claude_from_login_shell("/bin:/usr/bin\n", &env),
            ClaudeLookup::Failed
        );
        assert_eq!(
            claude_from_login_shell(&format!("{PATH_BEGIN}/bin"), &env),
            ClaudeLookup::Failed
        );
        assert_eq!(
            claude_from_login_shell(&format!("{PATH_END}/bin{PATH_BEGIN}"), &env),
            ClaudeLookup::Failed
        );
    }

    #[test]
    fn login_shell_path_skips_relative_and_overlong_entries() {
        let env = FakeEnv::new(HostOs::MacOs).with_file(PathBuf::from("bin/claude"));
        // A relative entry would search the working directory.
        let stdout = shell_output("", "bin:.", "");
        assert_eq!(
            claude_from_login_shell(&stdout, &env),
            ClaudeLookup::NotFound
        );

        let long = format!("/{}", "a".repeat(MAX_PATH_ENTRY_CHARS));
        let env = FakeEnv::new(HostOs::MacOs).with_file(join_all(Path::new(&long), &["claude"]));
        assert_eq!(
            claude_from_login_shell(&shell_output("", &long, ""), &env),
            ClaudeLookup::NotFound
        );
    }

    #[test]
    fn an_empty_login_shell_path_is_a_finished_lookup_that_found_nothing() {
        let env = FakeEnv::new(HostOs::MacOs).with_file(PathBuf::from("/bin/claude"));
        assert_eq!(
            claude_from_login_shell(&shell_output("", "", ""), &env),
            ClaudeLookup::NotFound
        );
    }

    #[cfg(unix)]
    fn write_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, "").expect("write");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    /// A file without an execute bit is not a program on Unix, so the search goes on
    /// past it to the next directory that holds a runnable `claude`.
    #[cfg(unix)]
    #[test]
    fn a_file_without_an_execute_bit_is_skipped_and_the_search_goes_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = dir.path().join("stale");
        let good = dir.path().join("good");
        std::fs::create_dir(&stale).expect("stale dir");
        std::fs::create_dir(&good).expect("good dir");
        std::fs::write(stale.join("claude"), "").expect("stale file");
        write_executable(&good.join("claude"));

        assert!(!RealLocateEnv.is_file(&stale.join("claude")));
        assert!(RealLocateEnv.is_file(&good.join("claude")));
        let stdout = shell_output("", &format!("{}:{}", stale.display(), good.display()), "");
        assert_eq!(
            claude_from_login_shell(&stdout, &RealLocateEnv),
            ClaudeLookup::Found(good.join("claude"))
        );
    }

    #[test]
    fn quoted_windows_path_entries_are_unquoted_once() {
        let env = FakeEnv::new(HostOs::Windows)
            .with_var("PATH", r#""C:\Program Files\nodejs";"D:\bin";"""#);
        assert_eq!(
            candidate_paths(&env),
            vec![
                join_all(Path::new(r"C:\Program Files\nodejs"), &["claude.exe"]),
                join_all(Path::new(r"D:\bin"), &["claude.exe"]),
            ]
        );
    }

    #[test]
    fn quotes_are_not_removed_from_unix_path_entries() {
        let env = FakeEnv::new(HostOs::Linux).with_var("PATH", r#""/a""#);
        assert_eq!(
            candidate_paths(&env),
            vec![
                PathBuf::from("/usr/local/bin/claude"),
                PathBuf::from("/opt/local/bin/claude"),
            ]
        );
    }

    #[test]
    fn probe_errors_map_to_locate_errors() {
        assert_eq!(
            probe_error(ProcessError::StartFailed),
            LocateError::StartFailed
        );
        assert_eq!(probe_error(ProcessError::TimedOut), LocateError::TimedOut);
        assert_eq!(
            probe_error(ProcessError::OutputTooLarge),
            LocateError::Unparseable
        );
        assert_eq!(probe_error(ProcessError::Io), LocateError::Unparseable);
    }

    #[tokio::test]
    async fn probe_reports_a_missing_program_as_not_started() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            probe_version(&dir.path().join("claude")).await,
            Err(LocateError::StartFailed)
        );
    }

    #[cfg(unix)]
    fn write_program(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write program");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make program executable");
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_reads_the_version_a_program_prints() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = write_program(dir.path(), "claude", "echo '2.1.296 (Claude Code)'");
        assert_eq!(probe_version(&program).await, Ok(version(2, 1, 296)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_reads_the_version_even_when_the_program_exits_non_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = write_program(dir.path(), "claude", "echo '2.1.296 (Claude Code)'\nexit 1");
        assert_eq!(probe_version(&program).await, Ok(version(2, 1, 296)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_rejects_output_that_is_not_a_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = write_program(dir.path(), "claude", "echo hello");
        assert_eq!(probe_version(&program).await, Err(LocateError::Unparseable));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_never_starts_a_batch_script() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = write_program(dir.path(), "claude.cmd", "echo '2.1.296 (Claude Code)'");
        assert_eq!(probe_version(&program).await, Err(LocateError::StartFailed));
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn login_shell_lookup_is_macos_only() {
        assert_eq!(login_shell_claude_path().await, ClaudeLookup::NotFound);
    }

    #[cfg(windows)]
    #[test]
    fn real_env_matches_names_case_insensitively_and_takes_home_from_userprofile() {
        std::env::set_var("LOCATE_TEST_MIXED_CASE", "value");
        let found = RealLocateEnv.var("locate_test_mixed_case");
        std::env::remove_var("LOCATE_TEST_MIXED_CASE");
        assert_eq!(found, Some(OsString::from("value")));
        assert_eq!(
            RealLocateEnv.home_dir(),
            std::env::var_os("USERPROFILE").map(PathBuf::from)
        );
    }
}
