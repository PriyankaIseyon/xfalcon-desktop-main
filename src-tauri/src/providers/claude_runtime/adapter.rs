//! Runs one model turn on the user's own Claude account by starting their
//! installed `claude` program, and reports whether that program is ready.
//!
//! Each turn starts one `claude -p` process in a directory the app owns. Desktop's
//! tools are declared to it through a loopback server and never executed by it,
//! so the tool loop stays in charge. See `openspec/changes/add-plan-model-providers/design.md`.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::credentials::ProviderId;
use xfalcon_contracts::plan_providers::ClaudeEffort;

use super::account::read_account;
use super::declare_server::{DeclareServer, ToolNameMap};
use super::locate::{
    discover_claude, probe_version, version_is_supported, ClaudeLookup, ClaudeVersion, LocateError,
};
use super::process::{self, kill_tree, TreeGuard};
use super::prompt::{render_prompt_with, RenderedPrompt};
use super::stream::StreamMapper;
use crate::providers::{ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent};

pub(crate) const MODEL_PREFIX: &str = "claude_code/";
pub(crate) const CLAUDE_MODELS: [(&str, &str); 3] = [
    ("claude-opus-5-5", "Opus"),
    ("claude-sonnet-5-5", "Sonnet"),
    ("claude-haiku-5-5", "Haiku"),
];
pub(crate) const CLAUDE_CONTEXT_TOKENS: i64 = 1_000_000;
pub(crate) const CLAUDE_MAX_OUTPUT_TOKENS: i64 = 128_000;

/// A turn that declares no tools still needs a valid `--mcp-config`, and this one
/// connects to nothing.
const EMPTY_MCP_CONFIG: &str = r#"{"mcpServers":{}}"#;
const LOOPBACK_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
/// `claude` writes a line for every event it makes, so a silent program has stalled.
const IDLE_LIMIT: Duration = Duration::from_secs(600);
/// Until the first line arrives, the prompt write and the wait for that line share
/// this bound. A program that has answered nothing in a minute has not started.
const FIRST_LINE_LIMIT: Duration = Duration::from_secs(60);
/// A program that printed nothing gets this long to exit before its status is unknown.
const EXIT_LIMIT: Duration = Duration::from_secs(2);
/// The most characters of a program's stderr tail that one debug log line carries.
const STDERR_LOG_CHARS: usize = 500;
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const SYSTEM_FILE: &str = "system.txt";
const MCP_FILE: &str = "mcp.json";
/// Windows keeps a file locked for a moment after its last user exits. Other
/// systems do not, so a failed removal is not retried there.
const REMOVE_ATTEMPTS: u32 = if cfg!(windows) { 4 } else { 1 };
const REMOVE_RETRY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProbeState {
    NotInstalled,
    Outdated,
    SignedOut,
    Ready,
    CheckFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProbeResult {
    pub(crate) state: ProbeState,
    pub(crate) installed_version: Option<ClaudeVersion>,
    pub(crate) plan: Option<String>,
    pub(crate) auth_method: Option<String>,
}

/// Checks whether the installed program can run a turn now. Reads only the
/// version and the sign-in state, never the credentials themselves.
pub(crate) async fn probe() -> ProbeResult {
    let binary = match discover_claude().await {
        ClaudeLookup::Found(binary) => binary,
        ClaudeLookup::NotFound => return probe_outcome(ProbeState::NotInstalled, None),
        // A lookup that did not finish says nothing about whether it is installed.
        ClaudeLookup::Failed => return probe_outcome(ProbeState::CheckFailed, None),
    };
    let version = match probe_version(&binary).await {
        Ok(version) => version,
        Err(LocateError::StartFailed) => {
            return probe_outcome(ProbeState::NotInstalled, None);
        }
        Err(LocateError::Unparseable | LocateError::TimedOut) => {
            return probe_outcome(ProbeState::CheckFailed, None);
        }
    };
    if !version_is_supported(version) {
        return probe_outcome(ProbeState::Outdated, Some(version));
    }
    let Ok(account) = read_account(&binary).await else {
        return probe_outcome(ProbeState::CheckFailed, Some(version));
    };
    if !account.is_ready() {
        return probe_outcome(ProbeState::SignedOut, Some(version));
    }
    ProbeResult {
        state: ProbeState::Ready,
        installed_version: Some(version),
        plan: account.plan,
        auth_method: account.auth_method,
    }
}

fn probe_outcome(state: ProbeState, installed_version: Option<ClaudeVersion>) -> ProbeResult {
    ProbeResult {
        state,
        installed_version,
        plan: None,
        auth_method: None,
    }
}

/// Accepts `claude_code/<id>` or `<id>`. `None` for any id not in [`CLAUDE_MODELS`].
pub(crate) fn model_slug(model_id: &str) -> Option<&'static str> {
    let bare = model_id.strip_prefix(MODEL_PREFIX).unwrap_or(model_id);
    CLAUDE_MODELS
        .iter()
        .find(|(id, _)| *id == bare)
        .map(|(id, _)| *id)
}

/// The arguments of one `claude -p` turn. The order is part of the contract that
/// the verified `claude` version accepts, and the empty arguments are deliberate.
/// `--effort` comes last, and only when a level is given.
pub(crate) fn build_args(
    model: &str,
    system_file: &Path,
    mcp_file: &Path,
    allowed_tools: Option<&str>,
    effort: Option<ClaudeEffort>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
    ]
    .map(OsString::from)
    .into();
    args.push(OsString::from(mcp_file));
    if let Some(allowed) = allowed_tools {
        args.extend(["--allowedTools", allowed].map(OsString::from));
    }
    args.extend(
        [
            "--permission-mode",
            "dontAsk",
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--system-prompt-file",
        ]
        .map(OsString::from),
    );
    args.push(OsString::from(system_file));
    args.extend(["--model", model].map(OsString::from));
    if let Some(effort) = effort {
        args.extend(["--effort", effort.flag()].map(OsString::from));
    }
    args
}

/// The `NO_PROXY` value for the child. A proxy in the user's environment must not
/// capture the loopback request to the declare server, so the loopback hosts are
/// always listed. Entries from both spellings of the variable are kept once each,
/// in the order they are found.
pub(crate) fn merged_no_proxy(values: &[Option<&str>]) -> String {
    let mut entries: Vec<&str> = Vec::new();
    let found = values
        .iter()
        .flatten()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|entry| !entry.is_empty());
    for entry in found.chain(LOOPBACK_HOSTS) {
        if !entries.iter().any(|kept| kept.eq_ignore_ascii_case(entry)) {
            entries.push(entry);
        }
    }
    entries.join(",")
}

/// Runs Claude turns through the user's own `claude` program.
pub(crate) struct ClaudeRuntimeAdapter {
    work_root: PathBuf,
    /// Set only by tests, which run a stand-in program in place of the installed one.
    binary_override: Option<PathBuf>,
    /// The level every turn passes as `--effort`. Without one, `claude` uses its default.
    effort: Option<ClaudeEffort>,
}

impl ClaudeRuntimeAdapter {
    pub(crate) fn new(work_root: PathBuf) -> Self {
        Self {
            work_root,
            binary_override: None,
            effort: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_binary(work_root: PathBuf, binary: PathBuf) -> Self {
        Self {
            work_root,
            binary_override: Some(binary),
            effort: None,
        }
    }

    /// Makes every turn run at `effort`, so the level chosen in the chat is the one
    /// `claude` uses.
    pub(crate) fn with_effort(mut self, effort: ClaudeEffort) -> Self {
        self.effort = Some(effort);
        self
    }

    /// The installed program is searched for once and then reused. The cached path is
    /// reused only while the file there is unchanged, so a removed, moved or replaced
    /// program is searched for and its version probed again.
    async fn resolve_binary(&self) -> Result<PathBuf, ProviderError> {
        if let Some(binary) = &self.binary_override {
            return Ok(binary.clone());
        }
        // The guard is a temporary, so it is released before the first await.
        let cached = lock_cache().clone();
        if let Some((path, _, _)) = cached.filter(|(path, key, _)| binary_key(path) == Some(*key)) {
            return Ok(path);
        }
        let path = match discover_claude().await {
            ClaudeLookup::Found(path) => path,
            ClaudeLookup::NotFound | ClaudeLookup::Failed => {
                return Err(ProviderError::RuntimeUnavailable)
            }
        };
        // Keyed before the probe, so a file replaced during the probe is probed again.
        let key = binary_key(&path).ok_or(ProviderError::RuntimeUnavailable)?;
        let version = probe_version(&path)
            .await
            .map_err(|_| ProviderError::RuntimeUnavailable)?;
        if !version_is_supported(version) {
            return Err(ProviderError::RuntimeUnavailable);
        }
        *lock_cache() = Some((path.clone(), key, version));
        Ok(path)
    }

    /// Owns the declare server for the turn. Every exit below the server's start
    /// reaches `shutdown`, after `run_turn` has killed the process and removed the
    /// turn directory.
    async fn turn(
        &self,
        request: &ProviderChatRequest,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let model = model_slug(&request.model_id).ok_or(ProviderError::InvalidResponse)?;
        let binary = self.resolve_binary().await?;
        let names =
            Arc::new(ToolNameMap::new(&request.tools).map_err(|_| ProviderError::InvalidResponse)?);
        let server = if request.tools.is_empty() {
            None
        } else {
            Some(
                DeclareServer::start(&request.tools, &names)
                    .await
                    .map_err(|_| ProviderError::Unavailable)?,
            )
        };
        let (mcp_config, allowed_tools) = match &server {
            Some(server) => (server.mcp_config_json(), Some(names.allowed_tools_arg())),
            None => (EMPTY_MCP_CONFIG.to_owned(), None),
        };
        let plan = TurnPlan {
            binary,
            model,
            prompt: render_prompt_with(&request.messages, &|wire| names.callable_name(wire)),
            mcp_config,
            allowed_tools,
            names,
            effort: self.effort,
        };
        let outcome = self.run_turn(plan, cancel, on_event).await;
        if let Some(server) = server {
            server.shutdown().await;
        }
        outcome
    }

    async fn run_turn(
        &self,
        plan: TurnPlan,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        // Declared first, so it is dropped last: the directory goes after the process.
        let turn = TurnDir::create(&self.work_root, &plan.prompt.system, &plan.mcp_config)?;
        let args = build_args(
            plan.model,
            &turn.system_file(),
            &turn.mcp_file(),
            plan.allowed_tools.as_deref(),
            plan.effort,
        );
        let no_proxy = merged_no_proxy(&[
            existing_no_proxy("NO_PROXY").as_deref(),
            existing_no_proxy("no_proxy").as_deref(),
        ]);
        let mut command = process::command(&plan.binary);
        command
            .args(&args)
            .current_dir(&turn.path)
            .env("NO_PROXY", &no_proxy)
            .env("no_proxy", &no_proxy)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|_| ProviderError::RuntimeUnavailable)?;
        // Declared after `turn`, so it drops before the directory is removed. It stops
        // the tree if this future is dropped before the `kill_tree` below runs.
        let mut tree = TreeGuard::new(&child);
        // Taken before any wait, because a reaped child reports no pid.
        let pid = child.id();
        let Some(stderr) = child.stderr.take() else {
            return Err(ProviderError::RuntimeUnavailable);
        };
        // Read for the whole turn, so a full pipe never blocks the program. The set aborts
        // its task when it is dropped, which is when this turn returns.
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let mut stderr_tasks = JoinSet::new();
        stderr_tasks.spawn(process::read_stderr_tail(stderr, Arc::clone(&stderr_tail)));
        let mut emitted = false;
        let outcome = {
            let mut sink = |event: ProviderStreamEvent| {
                emitted = true;
                on_event(event);
            };
            tokio::select! {
                biased;
                () = cancel.cancelled() => Err(ProviderError::Cancelled),
                outcome = relay(&mut child, &plan.prompt.user, Arc::clone(&plan.names), &mut sink, FIRST_LINE_LIMIT, IDLE_LIMIT) => outcome,
            }
        };
        let no_output = !emitted && matches!(outcome, Err(ProviderError::Unavailable));
        // The status is read only when nothing was streamed. A program that has closed
        // its output may still be exiting, so it gets `EXIT_LIMIT` before it is stopped.
        let exit = if no_output {
            process::exit_within(&mut child, EXIT_LIMIT).await
        } else {
            None
        };
        match (&exit, pid) {
            (Some(_), Some(pid)) => process::kill_group(pid).await,
            _ => kill_tree(&mut child).await,
        }
        tree.disarm();
        if !no_output {
            return outcome;
        }
        // The pipe closes with the tree, so the tail is complete when the task ends. The
        // bound covers a process outside the tree that still holds the pipe open.
        let _ = tokio::time::timeout(EXIT_LIMIT, stderr_tasks.join_next()).await;
        let tail = stderr_tail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Err(no_output_outcome(exit, &tail))
    }
}

#[async_trait]
impl ProviderAdapter for ClaudeRuntimeAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::ClaudeCode
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        _secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        self.turn(request, cancel, on_event).await
    }
}

/// Everything one `claude -p` run is started with, resolved before anything is
/// written or spawned.
struct TurnPlan {
    binary: PathBuf,
    model: &'static str,
    prompt: RenderedPrompt,
    mcp_config: String,
    allowed_tools: Option<String>,
    names: Arc<ToolNameMap>,
    effort: Option<ClaudeEffort>,
}

/// Writes the prompt on stdin, then maps stdout until the turn ends or the output
/// does. The caller kills the process afterwards, whatever this returns.
///
/// `first_line` bounds the prompt write and the wait for the first line; `idle`
/// bounds each wait after that. Both expire as `RuntimeUnavailable`, which the
/// retry wrapper does not repeat. A retry would wait out the same bound again.
async fn relay(
    child: &mut Child,
    user: &str,
    names: Arc<ToolNameMap>,
    on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    first_line: Duration,
    idle: Duration,
) -> Result<(), ProviderError> {
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(ProviderError::RuntimeUnavailable);
    };
    // A program that exits before reading its input is not an error here: its
    // output says what happened. One that never reads it has not answered.
    if tokio::time::timeout(first_line, stdin.write_all(user.as_bytes()))
        .await
        .is_err()
    {
        return Err(ProviderError::RuntimeUnavailable);
    }
    // The pipe closes only when dropped: `claude` reads its input to end-of-file.
    drop(stdin);

    let mut mapper = StreamMapper::new(Box::new(move |called: &str| names.resolve(called)));
    let mut lines = BufReader::new(stdout);
    let mut seen_line = false;
    loop {
        let bound = if seen_line { idle } else { first_line };
        let line =
            match tokio::time::timeout(bound, read_line_capped(&mut lines, MAX_LINE_BYTES)).await {
                Err(_) => return Err(ProviderError::RuntimeUnavailable),
                Ok(Err(LineError::TooLong)) => return Err(ProviderError::InvalidResponse),
                Ok(Err(LineError::Io)) => return Err(ProviderError::Unavailable),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(line))) => line,
            };
        seen_line = true;
        let mapped = mapper.push_line(&line);
        for event in mapped.events {
            on_event(event);
        }
        if let Some(error) = mapped.error {
            return Err(error);
        }
        if mapped.stop_process {
            return Ok(());
        }
    }
    let mapped = mapper.finish();
    for event in mapped.events {
        on_event(event);
    }
    match mapped.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The outcome of a turn whose stream produced no event. A program that exited
/// non-zero failed to start, so the failure is `RuntimeUnavailable`, which the retry
/// wrapper does not repeat. Otherwise the outcome stays `Unavailable`. `exit` is
/// `None` when the program was still running at its bound and has been stopped.
fn no_output_outcome(exit: Option<ExitStatus>, stderr_tail: &[u8]) -> ProviderError {
    tracing::warn!(exit = ?exit, "claude printed no output");
    tracing::debug!(stderr = %stderr_for_log(stderr_tail), "claude stderr tail");
    match exit {
        Some(status) if !status.success() => ProviderError::RuntimeUnavailable,
        _ => ProviderError::Unavailable,
    }
}

/// The stderr tail as one log line, without control characters, at most
/// [`STDERR_LOG_CHARS`] characters long.
fn stderr_for_log(tail: &[u8]) -> String {
    String::from_utf8_lossy(tail)
        .chars()
        .filter(|character| !character.is_control())
        .take(STDERR_LOG_CHARS)
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum LineError {
    #[error("a line is longer than allowed")]
    TooLong,
    #[error("reading the program's output failed")]
    Io,
}

/// Reads one line without its line ending. `Ok(None)` is end of input. The line
/// is refused as soon as it passes `limit` bytes, so a program that never sends a
/// newline cannot grow the buffer without bound.
async fn read_line_capped<R>(reader: &mut R, limit: usize) -> Result<Option<String>, LineError>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|_| LineError::Io)?;
        if available.is_empty() {
            // A last line without a newline is still a line.
            return Ok((!line.is_empty()).then(|| decode_line(&line)));
        }
        let newline = available.iter().position(|&byte| byte == b'\n');
        let content = newline.unwrap_or(available.len());
        if line.len() + content > limit {
            return Err(LineError::TooLong);
        }
        line.extend_from_slice(&available[..content]);
        let consumed = newline.map_or(content, |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(decode_line(&line)));
        }
    }
}

fn decode_line(bytes: &[u8]) -> String {
    // A `\r` before the newline belongs to the line ending. Output that is not
    // UTF-8 is not JSON either, and the stream mapper ignores lines it cannot read.
    let bytes = match bytes {
        [head @ .., b'\r'] => head,
        _ => bytes,
    };
    String::from_utf8_lossy(bytes).into_owned()
}

/// The app-owned directory one turn runs in. Dropping it removes the directory,
/// so `run_turn` lets it drop only after the process is dead.
struct TurnDir {
    path: PathBuf,
}

impl TurnDir {
    fn create(work_root: &Path, system: &str, mcp_config: &str) -> Result<Self, ProviderError> {
        let turn = Self {
            path: work_root.join(format!("turn-{}", Uuid::new_v4())),
        };
        // The guard exists before anything is written, so a failure part way
        // removes whatever was made.
        turn.write_files(work_root, system, mcp_config)
            .map_err(|_| ProviderError::RuntimeUnavailable)?;
        Ok(turn)
    }

    fn write_files(&self, work_root: &Path, system: &str, mcp_config: &str) -> io::Result<()> {
        std::fs::create_dir_all(work_root)?;
        std::fs::create_dir(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o700))?;
        }
        write_private(&self.system_file(), system)?;
        write_private(&self.mcp_file(), mcp_config)
    }

    fn system_file(&self) -> PathBuf {
        self.path.join(SYSTEM_FILE)
    }

    fn mcp_file(&self) -> PathBuf {
        self.path.join(MCP_FILE)
    }
}

impl Drop for TurnDir {
    fn drop(&mut self) {
        // Best effort: a directory that cannot be removed holds only the prompt
        // text and a loopback token, and it is never read again. The first attempt
        // is made here, so a directory that goes at once is gone when this returns.
        if remove_turn_dir(&self.path, 1, REMOVE_RETRY) || REMOVE_ATTEMPTS == 1 {
            return;
        }
        // The retries wait between attempts. That wait must not hold a runtime
        // thread, so it runs on a thread of its own.
        let path = self.path.clone();
        let _ = std::thread::Builder::new().spawn(move || {
            std::thread::sleep(REMOVE_RETRY);
            remove_turn_dir(&path, REMOVE_ATTEMPTS - 1, REMOVE_RETRY);
        });
    }
}

/// Removes the turn directory in up to `attempts` tries, `wait` apart. Returns
/// whether it is gone.
fn remove_turn_dir(path: &Path, attempts: u32, wait: Duration) -> bool {
    for attempt in 1..=attempts {
        if std::fs::remove_dir_all(path).is_ok() || !path.exists() {
            return true;
        }
        if attempt < attempts {
            std::thread::sleep(wait);
        }
    }
    false
}

/// Creates a new file that only the current user can read on Unix.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// The proxy exclusions set under one spelling of the variable.
fn existing_no_proxy(name: &str) -> Option<String> {
    std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
}

/// The size and modified time of a program file. Both change when the file is
/// replaced, so a different key means the program has to be probed again.
type BinaryKey = (u64, Option<SystemTime>);

fn binary_key(path: &Path) -> Option<BinaryKey> {
    let metadata = path.metadata().ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

/// The version found by the last successful search, the path it was found at, and
/// the key of that file then. Process-wide, so every adapter instance shares one search.
static BINARY_CACHE: OnceLock<Mutex<Option<(PathBuf, BinaryKey, ClaudeVersion)>>> = OnceLock::new();

fn lock_cache() -> MutexGuard<'static, Option<(PathBuf, BinaryKey, ClaudeVersion)>> {
    // A poisoned cache still holds a complete path and version, so it is used as is.
    BINARY_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::{ContextMessage, ContextRole};
    #[cfg(unix)]
    use crate::providers::claude_runtime::prompt::render_prompt;
    #[cfg(unix)]
    use crate::providers::with_retry;
    #[cfg(unix)]
    use crate::providers::ProviderStopReason;
    use crate::providers::ProviderToolSchema;
    #[cfg(unix)]
    use serde_json::json;
    #[cfg(unix)]
    use std::time::Instant;

    #[cfg(unix)]
    const TOOL_USE_TURN: &str = include_str!("fixtures/turn_tool_use.jsonl");
    #[cfg(unix)]
    const TEXT_TURN: &str = include_str!("fixtures/turn_text.jsonl");
    #[cfg(unix)]
    const NOT_LOGGED_IN_RESULT: &str =
        r#"{"type":"result","is_error":true,"result":"Not logged in · Please run /login"}"#;

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn chat_request(model_id: &str, tools: Vec<ProviderToolSchema>) -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: model_id.to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "Find the parser.")],
            max_output_tokens: None,
            allow_reasoning: false,
            tools,
            openrouter: None,
        }
    }

    #[cfg(unix)]
    fn search_tool() -> ProviderToolSchema {
        ProviderToolSchema {
            name: "project_search_text".to_owned(),
            description: "Search project text.".to_owned(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn run_stream(
        adapter: &ClaudeRuntimeAdapter,
        request: &ProviderChatRequest,
        cancel: &CancellationToken,
    ) -> (Result<(), ProviderError>, Vec<ProviderStreamEvent>) {
        let mut events = Vec::new();
        let result = {
            let mut sink = |event: ProviderStreamEvent| events.push(event);
            adapter.stream(request, "", cancel, &mut sink).await
        };
        (result, events)
    }

    fn turn_dirs(work_root: &Path) -> usize {
        std::fs::read_dir(work_root).expect("work root").count()
    }

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[cfg(unix)]
    fn read_pid(path: &Path) -> u32 {
        std::fs::read_to_string(path)
            .expect("pid file")
            .trim()
            .parse()
            .expect("pid")
    }

    /// Whether a process with this id still exists, as `kill -0` reports it.
    #[cfg(unix)]
    fn process_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn build_args_without_a_declared_server_allows_no_tools() {
        let args = build_args(
            "claude-opus-5-5",
            Path::new("/work/system.txt"),
            Path::new("/work/mcp.json"),
            None,
            None,
        );
        assert_eq!(
            strings(&args),
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--tools",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                "/work/mcp.json",
                "--permission-mode",
                "dontAsk",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--disable-slash-commands",
                "--system-prompt-file",
                "/work/system.txt",
                "--model",
                "claude-opus-5-5",
            ]
        );
    }

    #[test]
    fn build_args_with_a_declared_server_allows_its_tools_before_the_permission_mode() {
        let args = build_args(
            "claude-sonnet-5-5",
            Path::new("/work/system.txt"),
            Path::new("/work/mcp.json"),
            Some("mcp__xf__*"),
            None,
        );
        assert_eq!(
            strings(&args),
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--tools",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                "/work/mcp.json",
                "--allowedTools",
                "mcp__xf__*",
                "--permission-mode",
                "dontAsk",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--disable-slash-commands",
                "--system-prompt-file",
                "/work/system.txt",
                "--model",
                "claude-sonnet-5-5",
            ]
        );
    }

    #[test]
    fn build_args_adds_each_effort_level_last_after_the_model() {
        for allowed_tools in [None, Some("mcp__xf__*")] {
            let without = strings(&build_args(
                "claude-haiku-5-5",
                Path::new("/work/system.txt"),
                Path::new("/work/mcp.json"),
                allowed_tools,
                None,
            ));
            assert!(!without.iter().any(|arg| arg == "--effort"));
            for level in ClaudeEffort::ALL {
                let args = strings(&build_args(
                    "claude-haiku-5-5",
                    Path::new("/work/system.txt"),
                    Path::new("/work/mcp.json"),
                    allowed_tools,
                    Some(level),
                ));
                assert_eq!(
                    &args[args.len() - 4..],
                    ["--model", "claude-haiku-5-5", "--effort", level.flag()]
                );
                assert_eq!(args.iter().filter(|arg| *arg == "--effort").count(), 1);
                assert_eq!(args[..args.len() - 2], without[..]);
            }
        }
    }

    #[test]
    fn merged_no_proxy_is_loopback_when_nothing_is_set() {
        assert_eq!(merged_no_proxy(&[None, None]), "127.0.0.1,localhost");
        assert_eq!(merged_no_proxy(&[Some("")]), "127.0.0.1,localhost");
        assert_eq!(merged_no_proxy(&[Some("  ")]), "127.0.0.1,localhost");
    }

    #[test]
    fn merged_no_proxy_adds_only_the_loopback_hosts_that_are_missing() {
        assert_eq!(
            merged_no_proxy(&[Some("corp.example,127.0.0.1")]),
            "corp.example,127.0.0.1,localhost"
        );
        assert_eq!(
            merged_no_proxy(&[Some("corp.example")]),
            "corp.example,127.0.0.1,localhost"
        );
    }

    #[test]
    fn merged_no_proxy_matches_existing_entries_case_insensitively() {
        assert_eq!(
            merged_no_proxy(&[Some("LOCALHOST, 10.0.0.1")]),
            "LOCALHOST,10.0.0.1,127.0.0.1"
        );
    }

    #[test]
    fn merged_no_proxy_keeps_the_entries_of_both_spellings_once_each() {
        assert_eq!(
            merged_no_proxy(&[Some("corp.example, 10.0.0.1"), Some("10.0.0.1,localhost")]),
            "corp.example,10.0.0.1,localhost,127.0.0.1"
        );
        // A lower-case variable alone is honoured, not replaced by the loopback hosts.
        assert_eq!(
            merged_no_proxy(&[None, Some("only.lower")]),
            "only.lower,127.0.0.1,localhost"
        );
    }

    #[test]
    fn a_replaced_program_has_a_different_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("claude");
        std::fs::write(&path, "2.1.296").expect("write");
        let first = binary_key(&path).expect("key");

        // A different size: a downgraded build is rarely the same length.
        std::fs::write(&path, "2.1.1").expect("rewrite");
        let resized = binary_key(&path).expect("key");
        assert_ne!(resized, first);

        // The same size, with an older modified time.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open");
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000))
            .expect("set modified");
        assert_ne!(binary_key(&path), Some(resized));
        assert_eq!(binary_key(&dir.path().join("missing")), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_turn_directory_that_cannot_be_removed_is_given_up_after_its_attempts() {
        use std::os::unix::fs::PermissionsExt;

        let work = tempfile::tempdir().expect("tempdir");
        let turn = work.path().join("turn");
        std::fs::create_dir(&turn).expect("turn");
        std::fs::write(turn.join("file"), "x").expect("file");
        // Without write permission the file inside cannot be unlinked.
        std::fs::set_permissions(&turn, std::fs::Permissions::from_mode(0o500)).expect("chmod");

        assert!(!remove_turn_dir(&turn, 3, Duration::from_millis(1)));
        assert!(turn.exists());

        std::fs::set_permissions(&turn, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert!(remove_turn_dir(&turn, 1, Duration::from_millis(1)));
        assert!(!turn.exists());
    }

    #[test]
    fn model_slug_accepts_prefixed_and_bare_ids_and_rejects_the_rest() {
        assert_eq!(
            model_slug("claude_code/claude-opus-5-5"),
            Some("claude-opus-5-5")
        );
        assert_eq!(model_slug("claude-haiku-5-5"), Some("claude-haiku-5-5"));
        assert_eq!(model_slug("claude_code/claude-unknown"), None);
        assert_eq!(model_slug("claude-unknown"), None);
        assert_eq!(model_slug(""), None);
        assert_eq!(model_slug("claude_code/"), None);
    }

    #[tokio::test]
    async fn line_reader_returns_each_line_without_its_line_ending() {
        let mut reader = BufReader::new(&b"first\r\nsecond\n\nthird"[..]);
        assert_eq!(
            read_line_capped(&mut reader, 64).await,
            Ok(Some("first".to_owned()))
        );
        assert_eq!(
            read_line_capped(&mut reader, 64).await,
            Ok(Some("second".to_owned()))
        );
        assert_eq!(
            read_line_capped(&mut reader, 64).await,
            Ok(Some(String::new()))
        );
        assert_eq!(
            read_line_capped(&mut reader, 64).await,
            Ok(Some("third".to_owned()))
        );
        assert_eq!(read_line_capped(&mut reader, 64).await, Ok(None));
    }

    #[tokio::test]
    async fn line_reader_refuses_a_line_longer_than_the_limit() {
        // A four-byte buffer makes the line arrive in several chunks, so the limit
        // is checked against the whole line and not one chunk.
        let mut reader = BufReader::with_capacity(4, &b"0123456789\nok\n"[..]);
        assert_eq!(
            read_line_capped(&mut reader, 6).await,
            Err(LineError::TooLong)
        );

        let mut reader = BufReader::with_capacity(4, &b"012345\nok\n"[..]);
        assert_eq!(
            read_line_capped(&mut reader, 6).await,
            Ok(Some("012345".to_owned()))
        );
    }

    #[tokio::test]
    async fn an_unknown_model_is_refused_before_anything_starts() {
        let work = tempfile::tempdir().expect("tempdir");
        let adapter = ClaudeRuntimeAdapter::with_binary(
            work.path().to_path_buf(),
            work.path().join("claude"),
        );
        let request = chat_request("claude_code/claude-unknown", Vec::new());
        let (result, events) = run_stream(&adapter, &request, &CancellationToken::new()).await;
        assert_eq!(result, Err(ProviderError::InvalidResponse));
        assert!(events.is_empty());
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[tokio::test]
    async fn a_missing_binary_is_reported_as_runtime_unavailable() {
        let work = tempfile::tempdir().expect("tempdir");
        let adapter = ClaudeRuntimeAdapter::with_binary(
            work.path().to_path_buf(),
            work.path().join("no-such-claude"),
        );
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());
        let (result, events) = run_stream(&adapter, &request, &CancellationToken::new()).await;
        assert_eq!(result, Err(ProviderError::RuntimeUnavailable));
        assert!(events.is_empty());
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_declared_tool_call_ends_the_turn_and_stops_the_program() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let fixture = bin.path().join("turn.jsonl");
        std::fs::write(&fixture, TOOL_USE_TURN).expect("fixture");
        let pid_file = bin.path().join("pid");
        let binary = script(
            bin.path(),
            &format!(
                "echo $$ > '{}'\ncat '{}'\nsleep 30",
                pid_file.display(),
                fixture.display()
            ),
        );
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-opus-5-5", vec![search_tool()]);

        let started = Instant::now();
        let (result, events) = run_stream(&adapter, &request, &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the turn must not wait for the program's sleep"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::ToolCallRequested { name, .. }
                if name.as_str() == "project_search_text"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::ToolUse,
                ..
            }
        )));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
        assert!(!process_alive(read_pid(&pid_file)));
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_text_turn_streams_its_text_and_completes() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let fixture = bin.path().join("turn.jsonl");
        std::fs::write(&fixture, TEXT_TURN).expect("fixture");
        let binary = script(bin.path(), &format!("cat '{}'", fixture.display()));
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-haiku-5-5", Vec::new());

        let (result, events) = run_stream(&adapter, &request, &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        assert!(events.iter().any(
            |event| matches!(event, ProviderStreamEvent::TextDelta(text) if !text.is_empty())
        ));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_result_error_is_reported_as_a_sign_in_problem() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let result_file = bin.path().join("result.jsonl");
        std::fs::write(&result_file, format!("{NOT_LOGGED_IN_RESULT}\n")).expect("result");
        let binary = script(
            bin.path(),
            &format!("cat '{}'\nsleep 30", result_file.display()),
        );
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());

        let started = Instant::now();
        let (result, _) = run_stream(&adapter, &request, &CancellationToken::new()).await;

        assert_eq!(result, Err(ProviderError::PlanSignInRequired));
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_fails_before_any_output_is_started_once_and_not_retried() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let starts = bin.path().join("starts.txt");
        let binary = script(
            bin.path(),
            &format!("echo start >> '{}'\nexit 3", starts.display()),
        );
        // The retry wrapper is in the path, as it is for the stream the app runs.
        let adapter = with_retry(Arc::new(ClaudeRuntimeAdapter::with_binary(
            work.path().to_path_buf(),
            binary,
        )));
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());
        let mut events = Vec::new();
        let result = {
            let mut sink = |event: ProviderStreamEvent| events.push(event);
            adapter
                .stream(&request, "", &CancellationToken::new(), &mut sink)
                .await
        };

        assert_eq!(result, Err(ProviderError::RuntimeUnavailable));
        assert!(events.is_empty());
        let text = std::fs::read_to_string(&starts).expect("starts");
        assert_eq!(text.lines().count(), 1, "the program must start once");
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_writing_a_lot_to_stderr_is_not_blocked() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let fixture = bin.path().join("turn.jsonl");
        std::fs::write(&fixture, TEXT_TURN).expect("fixture");
        // Far more than a pipe holds, so the program stops here unless stderr is read.
        let binary = script(
            bin.path(),
            &format!(
                "head -c 1048576 /dev/zero | tr '\\0' x >&2\ncat '{}'",
                fixture.display()
            ),
        );
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-haiku-5-5", Vec::new());

        let started = Instant::now();
        let (result, events) = run_stream(&adapter, &request, &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the turn must not wait for the first-line bound"
        );
        assert!(events.iter().any(
            |event| matches!(event, ProviderStreamEvent::TextDelta(text) if !text.is_empty())
        ));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_exits_zero_before_any_output_stays_unavailable() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let binary = script(bin.path(), "exit 0");
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());

        let (result, events) = run_stream(&adapter, &request, &CancellationToken::new()).await;

        assert_eq!(result, Err(ProviderError::Unavailable));
        assert!(events.is_empty());
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_the_turn_stops_the_program() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let pid_file = bin.path().join("pid");
        let binary = script(
            bin.path(),
            &format!("echo $$ > '{}'\nsleep 30", pid_file.display()),
        );
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            trigger.cancel();
        });

        let started = Instant::now();
        let (result, _) = run_stream(&adapter, &request, &cancel).await;

        assert_eq!(result, Err(ProviderError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(!process_alive(read_pid(&pid_file)));
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_program_gets_the_declared_arguments_and_the_prompt_on_stdin() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let fixture = bin.path().join("turn.jsonl");
        std::fs::write(&fixture, TEXT_TURN).expect("fixture");
        let args_file = bin.path().join("args.txt");
        let stdin_file = bin.path().join("stdin.txt");
        let system_copy = bin.path().join("system-copy.txt");
        // The system prompt file must exist while the program runs, so the script
        // copies it out before it prints the answer.
        let binary = script(
            bin.path(),
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--system-prompt-file\" ]; then cp \"$2\" '{}'; fi\n  shift\ndone\ncat '{}'",
                args_file.display(),
                stdin_file.display(),
                system_copy.display(),
                fixture.display()
            ),
        );
        let adapter = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());

        let (result, _) = run_stream(&adapter, &request, &CancellationToken::new()).await;
        assert_eq!(result, Ok(()));

        let text = std::fs::read_to_string(&args_file).expect("args");
        let args: Vec<&str> = text.lines().collect();
        let tools = args
            .iter()
            .position(|arg| *arg == "--tools")
            .expect("--tools");
        assert_eq!(args[tools + 1], "");
        let system = args
            .iter()
            .position(|arg| *arg == "--system-prompt-file")
            .expect("--system-prompt-file");
        assert!(Path::new(args[system + 1]).ends_with(SYSTEM_FILE));
        let model = args
            .iter()
            .position(|arg| *arg == "--model")
            .expect("--model");
        assert_eq!(args[model + 1], "claude-opus-5-5");
        assert!(!args.contains(&"--allowedTools"));

        let rendered = render_prompt(&request.messages);
        assert_eq!(
            std::fs::read_to_string(&stdin_file).expect("stdin"),
            rendered.user
        );
        assert_eq!(
            std::fs::read_to_string(&system_copy).expect("system copy"),
            rendered.system
        );
        assert_eq!(turn_dirs(work.path()), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_turn_passes_the_effort_level_only_when_one_is_set() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let fixture = bin.path().join("turn.jsonl");
        std::fs::write(&fixture, TEXT_TURN).expect("fixture");
        let args_file = bin.path().join("args.txt");
        let binary = script(
            bin.path(),
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\ncat > /dev/null\ncat '{}'",
                args_file.display(),
                fixture.display()
            ),
        );
        let request = chat_request("claude_code/claude-haiku-5-5", Vec::new());

        let with_effort =
            ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary.clone())
                .with_effort(ClaudeEffort::Xhigh);
        let (result, _) = run_stream(&with_effort, &request, &CancellationToken::new()).await;
        assert_eq!(result, Ok(()));
        let text = std::fs::read_to_string(&args_file).expect("args");
        let args: Vec<&str> = text.lines().collect();
        assert_eq!(args.iter().filter(|arg| **arg == "--effort").count(), 1);
        assert_eq!(&args[args.len() - 2..], ["--effort", "xhigh"]);

        let without = ClaudeRuntimeAdapter::with_binary(work.path().to_path_buf(), binary);
        let (result, _) = run_stream(&without, &request, &CancellationToken::new()).await;
        assert_eq!(result, Ok(()));
        let text = std::fs::read_to_string(&args_file).expect("args");
        assert!(!text.lines().any(|arg| arg == "--effort"));
        assert_eq!(turn_dirs(work.path()), 0);
    }

    /// Runs `relay` against a stand-in program whose body is `script_body`, with the
    /// given bounds. The program is killed afterwards, as `run_turn` does.
    #[cfg(unix)]
    async fn relay_against(
        script_body: &str,
        user: &str,
        first_line: Duration,
        idle: Duration,
    ) -> Result<(), ProviderError> {
        let bin = tempfile::tempdir().expect("tempdir");
        let binary = script(bin.path(), script_body);
        let mut child = process::command(&binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let names = Arc::new(ToolNameMap::new(&[]).expect("names"));
        let mut sink = |_: ProviderStreamEvent| {};
        let result = relay(&mut child, user, names, &mut sink, first_line, idle).await;
        kill_tree(&mut child).await;
        result
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_silent_before_its_first_line_is_refused_at_the_first_line_bound() {
        let started = Instant::now();
        let result = relay_against(
            "sleep 30",
            "prompt",
            Duration::from_millis(200),
            Duration::from_secs(600),
        )
        .await;
        assert_eq!(result, Err(ProviderError::RuntimeUnavailable));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_prompt_the_program_never_reads_is_refused_at_the_first_line_bound() {
        // Far more than a pipe holds, so the write cannot finish while nothing reads.
        let prompt = "x".repeat(4 * 1024 * 1024);
        let started = Instant::now();
        let result = relay_against(
            "sleep 30",
            &prompt,
            Duration::from_millis(200),
            Duration::from_secs(600),
        )
        .await;
        assert_eq!(result, Err(ProviderError::RuntimeUnavailable));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_silent_after_its_first_line_is_refused_at_the_idle_bound() {
        let started = Instant::now();
        let result = relay_against(
            "echo '{\"type\":\"system\"}'\nsleep 30",
            "prompt",
            Duration::from_secs(600),
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(result, Err(ProviderError::RuntimeUnavailable));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_a_retried_turn_stops_the_whole_tree() {
        let bin = tempfile::tempdir().expect("tempdir");
        let work = tempfile::tempdir().expect("tempdir");
        let grandchild_file = bin.path().join("grandchild");
        // The program starts a child that keeps running unless the whole tree is stopped.
        let binary = script(
            bin.path(),
            &format!(
                "sleep 30 &\necho $! > '{}'\nwait",
                grandchild_file.display()
            ),
        );
        let adapter = with_retry(Arc::new(ClaudeRuntimeAdapter::with_binary(
            work.path().to_path_buf(),
            binary,
        )));
        let request = chat_request("claude_code/claude-opus-5-5", Vec::new());
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let watched = grandchild_file.clone();
        tokio::spawn(async move {
            // Cancel once the grandchild has written its pid.
            while std::fs::read_to_string(&watched).map_or(true, |text| text.trim().is_empty()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            trigger.cancel();
        });

        let mut events = Vec::new();
        let result = {
            let mut sink = |event: ProviderStreamEvent| events.push(event);
            adapter.stream(&request, "", &cancel, &mut sink).await
        };

        assert_eq!(result, Err(ProviderError::Cancelled));
        let grandchild = read_pid(&grandchild_file);
        let deadline = Instant::now() + Duration::from_secs(3);
        while process_alive(grandchild) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            !process_alive(grandchild),
            "a cancelled turn must not leave its grandchild running"
        );
        assert_eq!(turn_dirs(work.path()), 0);
    }
}
