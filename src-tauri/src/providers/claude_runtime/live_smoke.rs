//! Live smoke tests against the `claude` program installed and signed in on this
//! machine. Each test starts a real model turn, so they are ignored by default:
//! `cargo test -- --ignored live_`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::plan_providers::ClaudeEffort;

use super::adapter::ClaudeRuntimeAdapter;
use crate::agent::context::{ContextMessage, ContextRole};
use crate::providers::{
    ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent, ProviderToolSchema,
};

/// The smallest model in `CLAUDE_MODELS`, to spend as little of the plan as possible.
const SMALLEST_MODEL: &str = "claude_code/claude-haiku-5-5";
const TURN_LIMIT: Duration = Duration::from_secs(120);

#[tokio::test]
#[ignore = "needs a signed-in claude on this machine; run with --ignored"]
async fn live_plain_turn_streams_text_and_finishes() {
    let work = tempfile::tempdir().expect("tempdir");
    let adapter =
        ClaudeRuntimeAdapter::new(work.path().to_path_buf()).with_effort(ClaudeEffort::High);
    let request = chat_request(
        vec![ContextMessage::new(
            ContextRole::User,
            "Reply with exactly the word OK and nothing else.",
        )],
        Vec::new(),
    );

    let (result, events) = run_turn(&adapter, &request).await;

    expect_turn_finished(result);
    print_kinds("plain turn", &events);
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            ProviderStreamEvent::TextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::TextDelta(_))),
        "the stream emitted no text delta"
    );
    assert!(
        text.to_ascii_lowercase().contains("ok"),
        "the reply does not contain OK"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::Usage { .. })),
        "the stream emitted no usage event"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::Stopped { .. })),
        "the stream emitted no stopped event"
    );
    assert!(
        matches!(events.last(), Some(ProviderStreamEvent::Completed)),
        "the stream did not end with Completed"
    );
    assert_no_leftover_process(work.path());
}

#[tokio::test]
#[ignore = "needs a signed-in claude on this machine; run with --ignored"]
async fn live_declared_tool_is_requested_not_executed() {
    let work = tempfile::tempdir().expect("tempdir");
    let adapter = ClaudeRuntimeAdapter::new(work.path().to_path_buf());
    let tool = ProviderToolSchema {
        name: "lookup_weather".to_owned(),
        description: "Returns the weather for a city".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
    };
    let request = chat_request(
        vec![ContextMessage::new(
            ContextRole::User,
            "What is the weather in Paris? Use the lookup_weather tool.",
        )],
        vec![tool],
    );

    let (result, events) = run_turn(&adapter, &request).await;

    expect_turn_finished(result);
    print_kinds("declared tool turn", &events);
    let call = events.iter().find_map(|event| match event {
        ProviderStreamEvent::ToolCallRequested {
            name, arguments, ..
        } => Some((name.as_str(), arguments.as_str())),
        _ => None,
    });
    let (name, arguments) = call.expect("the stream emitted no tool-call event");
    assert_eq!(
        name, "lookup_weather",
        "the tool call does not carry the declared name"
    );
    let arguments: Value =
        serde_json::from_str(arguments).expect("the tool arguments are not JSON");
    assert!(
        arguments["city"].is_string(),
        "the tool arguments have no string city"
    );
    assert_no_leftover_process(work.path());
    assert_eq!(
        std::fs::read_dir(work.path()).expect("work root").count(),
        0,
        "the turn directory was not removed"
    );
}

#[tokio::test]
#[ignore = "needs a signed-in claude on this machine; run with --ignored"]
async fn live_every_effort_level_is_accepted() {
    for level in ClaudeEffort::ALL {
        let work = tempfile::tempdir().expect("tempdir");
        let adapter = ClaudeRuntimeAdapter::new(work.path().to_path_buf()).with_effort(level);
        let request = chat_request(
            vec![ContextMessage::new(
                ContextRole::User,
                "Reply with the single word ok.",
            )],
            Vec::new(),
        );

        let (result, events) = run_turn(&adapter, &request).await;

        print_kinds(&format!("--effort {}", level.flag()), &events);
        expect_turn_finished(result);
        assert!(
            matches!(events.last(), Some(ProviderStreamEvent::Completed)),
            "--effort {} did not end with Completed",
            level.flag()
        );
        assert_no_leftover_process(work.path());
    }
}

fn chat_request(
    messages: Vec<ContextMessage>,
    tools: Vec<ProviderToolSchema>,
) -> ProviderChatRequest {
    ProviderChatRequest {
        model_id: SMALLEST_MODEL.to_owned(),
        messages,
        max_output_tokens: None,
        allow_reasoning: false,
        tools,
        openrouter: None,
    }
}

/// Runs one turn, cancelled after `TURN_LIMIT`, and returns its result with every
/// event in order.
async fn run_turn(
    adapter: &ClaudeRuntimeAdapter,
    request: &ProviderChatRequest,
) -> (Result<(), ProviderError>, Vec<ProviderStreamEvent>) {
    let cancel = CancellationToken::new();
    let limit = cancel.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(TURN_LIMIT).await;
        limit.cancel();
    });
    let mut events = Vec::new();
    let result = {
        let mut sink = |event: ProviderStreamEvent| events.push(event);
        adapter.stream(request, "", &cancel, &mut sink).await
    };
    timer.abort();
    (result, events)
}

/// Fails the test with the cause when the turn did not finish. Nothing is skipped:
/// a missing or signed-out `claude` is a failure with its own message.
fn expect_turn_finished(result: Result<(), ProviderError>) {
    match result {
        Ok(()) => {}
        Err(ProviderError::RuntimeUnavailable) => {
            panic!("claude is missing, too old, or failed to start on this machine")
        }
        Err(ProviderError::PlanSignInRequired) => panic!(
            "claude is signed out on this machine: sign in with claude, then run this test again"
        ),
        Err(ProviderError::Cancelled) => panic!(
            "the turn did not finish within {} s and was cancelled",
            TURN_LIMIT.as_secs()
        ),
        Err(error) => panic!("the turn failed: {error}"),
    }
}

/// Prints the event kinds in order. The `Debug` form of an event shows byte counts
/// and never the text it carries.
fn print_kinds(label: &str, events: &[ProviderStreamEvent]) {
    let kinds: Vec<String> = events.iter().map(|event| format!("{event:?}")).collect();
    eprintln!("{label}: {} events: {}", kinds.len(), kinds.join(" -> "));
}

/// Fails if a process still names this work root. A turn's `claude` is started
/// with its files under the work root, so its command line carries the path.
#[cfg(unix)]
fn assert_no_leftover_process(work: &Path) {
    let output = Command::new("pgrep")
        .arg("-f")
        .arg(work)
        .output()
        .expect("pgrep must run for the leftover-process check, which is unix only");
    // pgrep exits 1 when nothing matched, and 2 or more when it fails itself.
    match output.status.code() {
        Some(1) => {}
        Some(0) => panic!(
            "a claude process is still running after the turn: pids {}",
            String::from_utf8_lossy(&output.stdout).trim()
        ),
        _ => panic!("pgrep failed with {}", output.status),
    }
}

/// The same check on Windows, where `pgrep` does not exist: it lists the
/// `claude.exe` processes whose command line names the work root.
#[cfg(windows)]
fn assert_no_leftover_process(work: &Path) {
    let path = work.to_str().expect("the work root path is not UTF-8");
    // A doubled single quote is one quote inside a PowerShell single-quoted string.
    let quoted = path.replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name='claude.exe'\" | \
         Where-Object {{ $_.CommandLine -and $_.CommandLine.Contains('{quoted}') }} | \
         ForEach-Object {{ $_.ProcessId }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(&script)
        .output()
        .expect("powershell.exe must run for the leftover-process check");
    assert!(
        output.status.success(),
        "the leftover-process check failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let pids = String::from_utf8_lossy(&output.stdout);
    assert!(
        pids.trim().is_empty(),
        "a claude process is still running after the turn: pids {}",
        pids.trim()
    );
}
