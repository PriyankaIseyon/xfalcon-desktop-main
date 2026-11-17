//! Starting, bounding and stopping the user's `claude` program.
//!
//! Every launch goes through [`command`]. It starts the program without a
//! console window on Windows, in its own process group on Unix, and without the
//! variables this app withholds from it. A started program can then be stopped as a tree with
//! [`kill_tree`], so a stray child never outlives the turn that started it.

use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, Command};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Variables with this prefix belong to the app (tenant endpoints, log levels).
/// They never reach the child.
const APP_ENV_PREFIX: &str = "XFALCON_";

/// Sets the effort level of the child. Anthropic ranks it with `--effort` and
/// `/effort`, so a user's value would override the level the chat chose.
const EFFORT_ENV_NAME: &str = "CLAUDE_CODE_EFFORT_LEVEL";

/// How long to wait for a killed child to be reaped before giving up.
const REAP_TIMEOUT: Duration = Duration::from_secs(5);

/// The newest bytes of a program's standard error that are kept for diagnostics.
pub(crate) const STDERR_TAIL_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ProcessError {
    #[error("the program could not be started")]
    StartFailed,
    #[error("the program did not finish in time")]
    TimedOut,
    #[error("the program wrote more output than allowed")]
    OutputTooLarge,
    #[error("reading or writing the program's streams failed")]
    Io,
}

/// What a bounded run produced.
#[derive(Debug)]
pub(crate) struct Captured {
    pub(crate) stdout: Vec<u8>,
    /// Callers read the output whatever the status, so only the tests read this.
    #[allow(dead_code)]
    pub(crate) success: bool,
}

/// Whether a variable is kept from the child. Names are compared case-insensitively,
/// because Windows treats them that way, so `xfalcon_x` is withheld like `XFALCON_X`.
fn is_withheld_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with(APP_ENV_PREFIX) || upper == EFFORT_ENV_NAME
}

/// A command that starts `program` the way every launch in this module must:
/// no console window (Windows), its own process group (Unix), the app's own
/// environment variables and the effort override removed, and the child killed if
/// its handle drops.
///
/// The caller sets arguments, the working directory and the stdio pipes.
pub(crate) fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if is_withheld_env_name(&key.to_string_lossy()) {
            command.env_remove(key);
        }
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Runs `command` to completion within `timeout`, optionally feeding `stdin`,
/// and returns at most `max_stdout` bytes of standard output. Standard error is
/// discarded. A run that exceeds either bound is killed as a tree.
pub(crate) async fn run_capped(
    mut command: Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
    max_stdout: usize,
) -> Result<Captured, ProcessError> {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| ProcessError::StartFailed)?;
    // Taken before any wait, because a reaped child reports no pid.
    let pid = child.id();

    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A program that exits before reading its input is not an error here:
        // its exit status tells the caller what happened.
        let _ = pipe.write_all(bytes).await;
        let _ = pipe.shutdown().await;
    }
    let Some(mut stdout) = child.stdout.take() else {
        kill_tree(&mut child).await;
        return Err(ProcessError::Io);
    };

    let read = async {
        let mut buffer = Vec::new();
        let mut limited = (&mut stdout).take(max_stdout as u64 + 1);
        limited
            .read_to_end(&mut buffer)
            .await
            .map_err(|_| ProcessError::Io)?;
        if buffer.len() > max_stdout {
            return Err(ProcessError::OutputTooLarge);
        }
        Ok(buffer)
    };
    let outcome = tokio::time::timeout(timeout, async {
        let buffer = read.await?;
        let status = child.wait().await.map_err(|_| ProcessError::Io)?;
        Ok::<_, ProcessError>((buffer, status.success()))
    })
    .await;

    match outcome {
        Ok(Ok((stdout, success))) => {
            // A program may leave children behind when it exits, and they must not outlive the run.
            if let Some(pid) = pid {
                terminate_tree(pid).await;
            }
            Ok(Captured { stdout, success })
        }
        Ok(Err(error)) => {
            kill_tree(&mut child).await;
            Err(error)
        }
        Err(_) => {
            kill_tree(&mut child).await;
            Err(ProcessError::TimedOut)
        }
    }
}

/// Stops `child` and every process it started, then reaps it. Best effort: a
/// child that is already gone is not an error.
pub(crate) async fn kill_tree(child: &mut Child) {
    let group_killed = match child.id() {
        Some(pid) => terminate_tree(pid).await,
        None => false,
    };
    if !group_killed {
        // The group could not be signalled, so the direct child is killed alone.
        let _ = child.start_kill();
    }
    let _ = tokio::time::timeout(REAP_TIMEOUT, child.wait()).await;
}

/// Waits up to `limit` for `child` to exit on its own. Its status if it did. That
/// reaps the child, so [`kill_tree`] can no longer read its pid: stop the rest with
/// [`kill_group`]. `None` if it is still running, and [`kill_tree`] stops it.
pub(crate) async fn exit_within(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    tokio::time::timeout(limit, child.wait()).await.ok()?.ok()
}

/// Stops every process in the group that `pid` leads. For a child that
/// [`exit_within`] has reaped, whose pid the caller took before the wait.
pub(crate) async fn kill_group(pid: u32) {
    let _ = terminate_tree(pid).await;
}

/// Reads a program's standard error until the pipe closes, and keeps only the newest
/// [`STDERR_TAIL_BYTES`] in `tail`. The bytes that are dropped are still read, so a
/// program that writes a lot is never blocked by a full pipe.
pub(crate) async fn read_stderr_tail(mut stderr: ChildStderr, tail: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 1024];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let mut kept = tail.lock().unwrap_or_else(PoisonError::into_inner);
                kept.extend_from_slice(&chunk[..read]);
                let excess = kept.len().saturating_sub(STDERR_TAIL_BYTES);
                kept.drain(..excess);
            }
        }
    }
}

/// The group is killed by the `kill` builtin of `/bin/sh`. Its `--` and negative
/// group id behave the same on Linux and macOS, where the `kill` binary differs.
#[cfg(unix)]
const GROUP_KILL_SCRIPT: &str = r#"kill -s KILL -- "-$1""#;

/// Kills the tree whose root has `pid`, and reports whether the kill succeeded.
#[cfg(unix)]
async fn terminate_tree(pid: u32) -> bool {
    // The child leads its own process group (see `command`), so the group id is
    // its pid and a negative pid addresses the whole group.
    Command::new("/bin/sh")
        .args(["-c", GROUP_KILL_SCRIPT, "sh", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// Kills the tree whose root has `pid`, and reports whether the kill succeeded.
#[cfg(windows)]
async fn terminate_tree(pid: u32) -> bool {
    let mut taskkill = Command::new("taskkill");
    taskkill
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    taskkill.creation_flags(CREATE_NO_WINDOW);
    taskkill.status().await.is_ok_and(|status| status.success())
}

/// Stops the whole tree of a started program when it is dropped. A turn whose
/// future is dropped before its own `kill_tree` runs, such as a cancelled stream,
/// would otherwise stop only the direct child, through `kill_on_drop`. `Drop`
/// cannot await, so the kill runs synchronously. Disarm it once the awaited
/// `kill_tree` has finished.
pub(crate) struct TreeGuard {
    pid: Option<u32>,
}

impl TreeGuard {
    pub(crate) fn new(child: &Child) -> Self {
        Self { pid: child.id() }
    }

    pub(crate) fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            // Best effort: a failed kill leaves the direct child to `kill_on_drop`.
            kill_tree_now(pid);
        }
    }
}

#[cfg(unix)]
fn kill_tree_now(pid: u32) {
    let _ = std::process::Command::new("/bin/sh")
        .args(["-c", GROUP_KILL_SCRIPT, "sh", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn kill_tree_now(pid: u32) {
    use std::os::windows::process::CommandExt;

    let mut taskkill = std::process::Command::new("taskkill");
    taskkill
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    let _ = taskkill.status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut command = command(Path::new("/bin/sh"));
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_capped_returns_stdout_and_status() {
        let captured = run_capped(shell("printf hello"), None, Duration::from_secs(10), 1024)
            .await
            .expect("run");
        assert_eq!(captured.stdout, b"hello");
        assert!(captured.success);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_capped_feeds_stdin() {
        let captured = run_capped(
            shell("cat"),
            Some(b"from stdin"),
            Duration::from_secs(10),
            1024,
        )
        .await
        .expect("run");
        assert_eq!(captured.stdout, b"from stdin");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_capped_refuses_output_over_the_limit() {
        let error = run_capped(shell("printf 0123456789"), None, Duration::from_secs(10), 4)
            .await
            .expect_err("over limit");
        assert_eq!(error, ProcessError::OutputTooLarge);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_capped_stops_a_program_that_runs_too_long() {
        let error = run_capped(shell("sleep 30"), None, Duration::from_millis(200), 1024)
            .await
            .expect_err("timeout");
        assert_eq!(error, ProcessError::TimedOut);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_tree_stops_a_grandchild_too() {
        let mut command = shell("sleep 30 & echo $!; wait");
        command.stdout(Stdio::piped()).stdin(Stdio::null());
        let mut child = command.spawn().expect("spawn");
        let mut stdout = child.stdout.take().expect("stdout");
        let mut line = String::new();
        let mut byte = [0u8; 1];
        while stdout.read_exact(&mut byte).await.is_ok() && byte[0] != b'\n' {
            line.push(byte[0] as char);
        }
        let grandchild: u32 = line.trim().parse().expect("grandchild pid");
        kill_tree(&mut child).await;
        let alive = std::process::Command::new("kill")
            .args(["-0", &grandchild.to_string()])
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        assert!(!alive, "the grandchild must not outlive the tree kill");
    }

    /// A shell command that keeps a child alive for a long time on this OS.
    fn long_running(script_unix: &str, script_windows: &str) -> Command {
        #[cfg(unix)]
        {
            let _ = script_windows;
            let mut command = command(Path::new("/bin/sh"));
            command.args(["-c", script_unix]);
            command
        }
        #[cfg(windows)]
        {
            let _ = script_unix;
            let mut command = command(Path::new("cmd.exe"));
            command.args(["/D", "/C", script_windows]);
            command
        }
    }

    #[tokio::test]
    async fn run_capped_stops_a_long_program_on_every_os() {
        let started = std::time::Instant::now();
        let error = run_capped(
            long_running("sleep 30", "ping -n 30 127.0.0.1 > nul"),
            None,
            Duration::from_millis(300),
            1024,
        )
        .await
        .expect_err("timeout");
        assert_eq!(error, ProcessError::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the kill must return promptly"
        );
    }

    /// The pipe only reaches end-of-file once every process that inherited it is
    /// gone. If the tree kill left a grandchild alive, this read would block.
    #[tokio::test]
    async fn kill_tree_releases_the_output_pipe_on_every_os() {
        let mut command = long_running("sleep 30 & wait", "ping -n 60 127.0.0.1");
        command.stdout(Stdio::piped()).stdin(Stdio::null());
        let mut child = command.spawn().expect("spawn");
        let mut stdout = child.stdout.take().expect("stdout");
        // Give the shell a moment to start its own child.
        tokio::time::sleep(Duration::from_millis(500)).await;
        kill_tree(&mut child).await;
        let mut rest = Vec::new();
        let eof =
            tokio::time::timeout(Duration::from_secs(10), stdout.read_to_end(&mut rest)).await;
        assert!(
            eof.is_ok(),
            "a surviving grandchild would hold the pipe open"
        );
    }

    /// Reads the grandchild's pid, which the shell prints as its first line.
    #[cfg(unix)]
    async fn first_line_pid(stdout: &mut tokio::process::ChildStdout) -> u32 {
        let mut line = String::new();
        let mut byte = [0u8; 1];
        while stdout.read_exact(&mut byte).await.is_ok() && byte[0] != b'\n' {
            line.push(byte[0] as char);
        }
        line.trim().parse().expect("grandchild pid")
    }

    /// Whether `pid` is gone within three seconds. It is polled, because a killed
    /// process can take a moment to be reaped.
    #[cfg(unix)]
    async fn gone_within_three_seconds(pid: u32) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if !alive {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_tree_guard_kills_the_whole_tree() {
        let mut command = shell("sleep 30 & echo $!; wait");
        command.stdout(Stdio::piped()).stdin(Stdio::null());
        let mut child = command.spawn().expect("spawn");
        let mut stdout = child.stdout.take().expect("stdout");
        let grandchild = first_line_pid(&mut stdout).await;

        // Dropped while the child is still running, as a cancelled turn drops it.
        let guard = TreeGuard::new(&child);
        drop(guard);

        assert!(
            gone_within_three_seconds(grandchild).await,
            "the grandchild must not outlive the dropped guard"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_disarmed_tree_guard_does_not_kill_the_tree() {
        let mut command = shell("sleep 30 & echo $!; wait");
        command.stdout(Stdio::piped()).stdin(Stdio::null());
        let mut child = command.spawn().expect("spawn");
        let mut stdout = child.stdout.take().expect("stdout");
        let grandchild = first_line_pid(&mut stdout).await;

        let mut guard = TreeGuard::new(&child);
        guard.disarm();
        drop(guard);
        assert!(
            process_is_alive(grandchild),
            "a disarmed guard must leave the tree alone"
        );
        kill_tree(&mut child).await;
        assert!(gone_within_three_seconds(grandchild).await);
    }

    #[cfg(unix)]
    fn process_is_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_successful_run_leaves_no_child_behind() {
        // The grandchild keeps neither the output pipe nor the run open, so the run
        // succeeds and only the tree kill after it can stop the grandchild.
        let captured = run_capped(
            shell("sleep 30 >/dev/null 2>&1 & echo $!"),
            None,
            Duration::from_secs(10),
            1024,
        )
        .await
        .expect("run");
        assert!(captured.success);
        let grandchild: u32 = String::from_utf8_lossy(&captured.stdout)
            .trim()
            .parse()
            .expect("grandchild pid");
        assert!(
            gone_within_three_seconds(grandchild).await,
            "a finished run must not leave its children running"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn run_capped_returns_stdout_on_windows() {
        let mut command = command(Path::new("cmd.exe"));
        command.args(["/D", "/C", "echo hello"]);
        let captured = run_capped(command, None, Duration::from_secs(10), 1024)
            .await
            .expect("run");
        assert_eq!(String::from_utf8_lossy(&captured.stdout).trim(), "hello");
        assert!(captured.success);
    }

    #[test]
    fn app_variables_are_removed_from_the_child() {
        // Safe in a test binary: the variable is unique to this test.
        std::env::set_var("XFALCON_PROCESS_TEST_VALUE", "secret");
        let command = command(Path::new("claude"));
        let removed = command
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "XFALCON_PROCESS_TEST_VALUE" && value.is_none());
        std::env::remove_var("XFALCON_PROCESS_TEST_VALUE");
        assert!(removed);
    }

    #[test]
    fn app_and_effort_variables_are_withheld_whatever_their_case() {
        assert!(is_withheld_env_name("XFALCON_X"));
        assert!(is_withheld_env_name("xfalcon_x"));
        assert!(is_withheld_env_name("CLAUDE_CODE_EFFORT_LEVEL"));
        assert!(is_withheld_env_name("claude_code_effort_level"));
    }

    #[test]
    fn the_other_variables_reach_the_child() {
        assert!(!is_withheld_env_name("PATH"));
        assert!(!is_withheld_env_name("CLAUDE_CODE_OTHER"));
        assert!(!is_withheld_env_name("ANTHROPIC_API_KEY"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_stderr_tail_keeps_only_the_newest_bytes() {
        let mut command = shell("head -c 10000 /dev/zero | tr '\\0' x >&2; printf END >&2");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn");
        let tail = Arc::new(Mutex::new(Vec::new()));
        let stderr = child.stderr.take().expect("stderr");
        read_stderr_tail(stderr, Arc::clone(&tail)).await;
        child.wait().await.expect("wait");

        let kept = tail.lock().expect("tail").clone();
        assert_eq!(kept.len(), STDERR_TAIL_BYTES);
        assert!(kept.ends_with(b"END"));
    }
}
