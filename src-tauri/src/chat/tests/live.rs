//! Whole runs against real model endpoints.
//!
//! Each test drives the real run loop (turn driver, tool loop, compaction)
//! with the real provider adapter, and only the saved-conversation store and
//! the tool are stand-ins. They cost money and minutes, so they are `#[ignore]`d
//! and read their keys from the environment:
//!
//! ```text
//! XF_LIVE_GATEWAY_KEY=... XF_LIVE_OPENROUTER_KEY=... \
//!   cargo test --lib chat::tests::live -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! What every test asserts is the person's experience: the run completes, with
//! an answer, and no failure was ever shown.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::credentials::{ModelId, ProviderId};
use xfalcon_contracts::inference::InferenceEvent;
use xfalcon_contracts::tools::{ToolId, ToolRisk, ToolSource};
use xfalcon_contracts::{ConversationId, RunId, RunState};
use zeroize::Zeroizing;

use super::tool_run::{AnyPath, RecordingSink};
use crate::agent::context::{ContextMessage, ContextRole, DurableMessageIdentity};
use crate::agent::context_budget::ContextCapacity;
use crate::agent::run::{InMemoryRunJournal, Run, RunBudgets};
use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort, ToolLoop, ToolLoopBudgets};
use crate::agent::tool_session::ToolSession;
use crate::agent::turn_driver::{LocalTurnDriver, LocalTurnDriverConfig};
use crate::chat::tool_run::ToolRun;
use crate::clients::da::chat_api::ModelExecutionMode;
use crate::providers::{ProviderAdapter, ProviderToolSchema};
use crate::tools::invocation::{InMemoryInvocationJournal, ToolResult};
use crate::tools::policy::{Actor, ProjectContext};
use crate::tools::registry::{ToolDescriptor, ToolRegistry};

const GATEWAY_URL: &str = "https://llm-gateway.iseyon.workers.dev/v1";
const GATEWAY_MODEL: &str = "ollama/qwen3.6:35b-a3b-coding-nvfp4";
const LUNA_MODEL: &str = "~openai/gpt-luna-latest";
const SONNET_MODEL: &str = "~anthropic/claude-sonnet-latest";
const PRICE_TOOL: &str = "catalog.lookup_price";
const PRICE_WIRE_NAME: &str = "catalog_lookup_price";
const PAGE_TOOL: &str = "catalog.fetch_page";
const PAGE_WIRE_NAME: &str = "catalog_fetch_page";
const DASH_TOOL: &str = "dash.save_dashboard";
const DASH_WIRE_NAME: &str = "dash_save_dashboard";
const PLAN_WIRE_NAME: &str = "project_update_plan";

fn key(variable: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| panic!("set {variable} to run the live tests"))
}

/// Looks prices up in a fixed table.
#[derive(Default)]
struct PriceExecutor {
    calls: Mutex<Vec<String>>,
    /// Every dashboard saved: its size and whether it ends as a whole document.
    dashboards: Mutex<Vec<(usize, bool)>>,
}

#[async_trait]
impl ToolExecutorPort for PriceExecutor {
    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        _cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        if tool_id.as_str() == DASH_TOOL {
            let html = arguments
                .get("html")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let whole = html.trim_end().to_lowercase().ends_with("</html>");
            self.dashboards
                .lock()
                .expect("lock")
                .push((html.len(), whole));
            self.calls
                .lock()
                .expect("lock")
                .push("dashboard".to_owned());
            return ToolResult::new(
                Vec::new(),
                Some(json!({"saved": true, "bytes": html.len()})),
                ToolSource::Mcp,
            )
            .map_err(|error| ToolExecutionError::Failed(error.to_string()));
        }
        if tool_id.as_str() == PAGE_TOOL {
            // About 20 KB of text per page, with a word to find in it.
            let page = arguments.get("page").and_then(Value::as_u64).unwrap_or(0);
            self.calls.lock().expect("lock").push(format!("page{page}"));
            let filler = (0..2_800)
                .map(|word| format!("lorem{word}"))
                .collect::<Vec<_>>()
                .join(" ");
            return ToolResult::new(
                Vec::new(),
                Some(json!({"page": page, "text": format!("{filler} The secret word for page {page} is WORD{page}.")})),
                ToolSource::Mcp,
            )
            .map_err(|error| ToolExecutionError::Failed(error.to_string()));
        }
        let item = arguments
            .get("item")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        self.calls.lock().expect("lock").push(item.clone());
        let price = match item.as_str() {
            "apple" => json!(3),
            "banana" => json!(2),
            "cherry" => json!(7),
            _ => Value::Null,
        };
        ToolResult::new(
            Vec::new(),
            Some(json!({"item": item, "price": price})),
            ToolSource::Mcp,
        )
        .map_err(|error| ToolExecutionError::Failed(error.to_string()))
    }
}

struct Outcome {
    state: RunState,
    content: String,
    error: Option<String>,
    status: String,
    failures_shown: Vec<String>,
    lookups: Vec<String>,
    dashboards: Vec<(usize, bool)>,
    elapsed: Duration,
}

struct Endpoint {
    mode: ModelExecutionMode,
    adapter: Arc<dyn ProviderAdapter>,
    secret: String,
    model_id: String,
    context_tokens: i64,
    max_output_tokens: i64,
    run_wall_clock: Duration,
}

fn gateway() -> Endpoint {
    Endpoint {
        mode: ModelExecutionMode::BoundedContext,
        adapter: crate::providers::custom_openai_adapter_with_sampling(
            GATEWAY_URL,
            GATEWAY_MODEL,
            xfalcon_contracts::custom_models::CustomModelSampling::default(),
        )
        .expect("gateway adapter"),
        secret: key("XF_LIVE_GATEWAY_KEY"),
        model_id: format!("custom/{}", Uuid::nil()),
        context_tokens: 131_072,
        max_output_tokens: 32_768,
        run_wall_clock: Duration::from_secs(25 * 60),
    }
}

/// Luna with a deliberately small window, so the same history that fits the
/// gateway model forces compaction here too.
fn luna(context_tokens: i64) -> Endpoint {
    openrouter(LUNA_MODEL, context_tokens)
}

fn openrouter(model_id: &str, context_tokens: i64) -> Endpoint {
    Endpoint {
        mode: ModelExecutionMode::BoundedContext,
        adapter: crate::providers::adapter_for(ProviderId::OpenRouter),
        secret: key("XF_LIVE_OPENROUTER_KEY"),
        model_id: model_id.to_owned(),
        context_tokens,
        max_output_tokens: 16_384,
        run_wall_clock: Duration::from_secs(10 * 60),
    }
}

/// Sonnet as the app runs it: OpenRouter's Anthropic Messages route with native
/// context compaction, a one-million-token window.
fn sonnet() -> Endpoint {
    Endpoint {
        mode: ModelExecutionMode::NativeCompaction,
        max_output_tokens: 64_000,
        run_wall_clock: Duration::from_secs(20 * 60),
        ..openrouter(SONNET_MODEL, 1_000_000)
    }
}

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            ToolDescriptor::new(
                PRICE_TOOL,
                ToolSource::Mcp,
                "Look up a price",
                "Returns the catalog price of one item. Call it once per item.",
                json!({
                    "type": "object",
                    "properties": {"item": {"type": "string", "description": "Item name"}},
                    "required": ["item"],
                }),
                ToolRisk::ReadOnly,
                Vec::new(),
                false,
            )
            .expect("descriptor"),
        )
        .expect("register");
    registry
        .register(
            ToolDescriptor::new(
                PAGE_TOOL,
                ToolSource::Mcp,
                "Fetch a page",
                "Returns the text of one numbered page.",
                json!({
                    "type": "object",
                    "properties": {"page": {"type": "integer", "description": "Page number"}},
                    "required": ["page"],
                }),
                ToolRisk::ReadOnly,
                Vec::new(),
                false,
            )
            .expect("descriptor"),
        )
        .expect("register");
    registry
        .register(
            ToolDescriptor::new(
                DASH_TOOL,
                ToolSource::Mcp,
                "Save a dashboard",
                "Saves a dashboard. `html` is the complete, self-contained HTML document.",
                json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "html": {"type": "string", "description": "Complete HTML document"}
                    },
                    "required": ["title", "html"],
                }),
                ToolRisk::ReadOnly,
                Vec::new(),
                false,
            )
            .expect("descriptor"),
        )
        .expect("register");
    registry
        .register(crate::tools::registry::project_update_plan_tool_descriptor().expect("plan tool"))
        .expect("register plan tool");
    registry
}

fn saved(role: ContextRole, text: String, sequence_no: i64) -> ContextMessage {
    let name = match role {
        ContextRole::User => "user",
        _ => "assistant",
    };
    ContextMessage::new(role, text).with_durable_identity(DurableMessageIdentity {
        id: Uuid::new_v4(),
        role: name.to_owned(),
        sequence_no,
    })
}

/// A saved conversation: `earlier` turns of filler, with a fact planted in the
/// first, then the question.
fn conversation(earlier: usize, filler_words: usize, question: &str) -> Vec<ContextMessage> {
    let filler = |turn: usize| {
        (0..filler_words)
            .map(|word| format!("item{turn}-{word}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut messages = Vec::new();
    let mut sequence = 1;
    for turn in 0..earlier {
        let ask = if turn == 0 {
            format!(
                "Remember this: the project codename is BLUEHERON. Here is my data: {}",
                filler(turn)
            )
        } else {
            format!(
                "Here is more of my data, just acknowledge it briefly: {}",
                filler(turn)
            )
        };
        messages.push(saved(ContextRole::User, ask, sequence));
        messages.push(saved(
            ContextRole::Assistant,
            format!("Noted, I have the data from message {turn}."),
            sequence + 1,
        ));
        sequence += 2;
    }
    messages.push(saved(ContextRole::User, question.to_owned(), sequence));
    messages
}

async fn run(endpoint: Endpoint, seed: Vec<ContextMessage>) -> Outcome {
    run_with(endpoint, seed, None).await
}

/// `cancel_after` presses Stop that long after the run starts.
async fn run_with(
    endpoint: Endpoint,
    seed: Vec<ContextMessage>,
    cancel_after: Option<Duration>,
) -> Outcome {
    let cancel = CancellationToken::new();
    if let Some(after) = cancel_after {
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            stop.cancel();
        });
    }
    let sink = Arc::new(RecordingSink::default());
    *sink.messages.lock().expect("lock") = seed.clone();
    let executor = Arc::new(PriceExecutor::default());

    let registry = registry();
    let allowlist = registry.iter().map(|d| d.id().clone()).collect();
    let tool_loop = ToolLoop::new(
        registry,
        allowlist,
        Actor::new("account-1").expect("actor"),
        RunId::from("run-live"),
        Some(ProjectContext::new("project-1")),
        None,
        ToolLoopBudgets::production(),
    );
    let tools = vec![
        ProviderToolSchema {
            name: PRICE_WIRE_NAME.to_owned(),
            description: "Returns the catalog price of one item. Call it once per item.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"item": {"type": "string"}},
                "required": ["item"],
            }),
        },
        ProviderToolSchema {
            name: PAGE_WIRE_NAME.to_owned(),
            description: "Returns the text of one numbered page.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"page": {"type": "integer"}},
                "required": ["page"],
            }),
        },
        ProviderToolSchema {
            name: DASH_WIRE_NAME.to_owned(),
            description: "Saves a dashboard. `html` is the complete, self-contained HTML document."
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "html": {"type": "string"}
                },
                "required": ["title", "html"],
            }),
        },
        ProviderToolSchema {
            name: PLAN_WIRE_NAME.to_owned(),
            description: "Update the task checklist".to_owned(),
            input_schema: json!({"type": "object"}),
        },
    ];
    let tool_ids = HashMap::from([
        (PRICE_WIRE_NAME.to_owned(), ToolId::from(PRICE_TOOL)),
        (PAGE_WIRE_NAME.to_owned(), ToolId::from(PAGE_TOOL)),
        (DASH_WIRE_NAME.to_owned(), ToolId::from(DASH_TOOL)),
        (
            PLAN_WIRE_NAME.to_owned(),
            ToolId::from("project.update_plan"),
        ),
    ]);
    let tool_session = ToolSession {
        tool_loop,
        executor: executor.clone(),
        path_authority: Some(Arc::new(AnyPath)),
        has_project_tools: true,
        unavailable_mcp_servers: Vec::new(),
        tools: tools.clone(),
        tool_ids: tool_ids.clone(),
    };

    let capacity = ContextCapacity::new(endpoint.context_tokens, endpoint.max_output_tokens)
        .expect("capacity");
    let events: Arc<Mutex<Vec<InferenceEvent>>> = Arc::default();
    let driver = LocalTurnDriver::new(
        sink.clone(),
        endpoint.adapter.clone(),
        Zeroizing::new(endpoint.secret.clone()),
        Arc::new(|_: &InferenceEvent| {}),
        LocalTurnDriverConfig {
            run_id: RunId::from("run-live"),
            da_run_id: Uuid::nil(),
            model_id: endpoint.model_id.clone(),
            route_identity: "live".to_owned(),
            max_output_tokens: Some(u32::try_from(capacity.max_output_tokens()).expect("u32")),
            execution_mode: endpoint.mode,
            tools,
            tool_ids,
            seed_messages: seed,
            conversation_id: Uuid::nil(),
            max_context_bytes: usize::try_from(capacity.hard_input_tokens()).expect("usize"),
            result_budget: crate::tools::ToolResultBudget::floor(),
            mcp_scope: None,
            active_compaction: None,
            history_revision: 0,
            snapshot_head_sequence_no: 1,
            boundary_sequence_no: 0,
            compactable_through_sequence_no: 1,
            compactable_source_message_count: 1,
            source_message_ids_sha256: "0".repeat(64),
        },
    );
    let run = Run::new(
        RunId::from("run-live"),
        ConversationId::from("conversation-live"),
        ModelId::from(endpoint.model_id),
        RunBudgets {
            event_budget: 4_096,
            output_byte_budget: 512 * 1024,
            wall_clock: endpoint.run_wall_clock,
        },
    );
    let recorded = events.clone();
    let mut tool_run = ToolRun::new(
        run,
        InMemoryRunJournal::new(),
        InMemoryInvocationJournal::new(),
        tool_session,
        driver,
        Arc::new(move |event: &InferenceEvent| {
            recorded.lock().expect("events").push(event.clone());
        }),
        cancel,
    );
    // Raise the stall limit to what a queued gateway request can need; the
    // service does the same for the gateway.
    tool_run.set_watchdog(crate::chat::tool_run::WatchdogLimits {
        stall: Duration::from_secs(3_720),
        hard: endpoint.run_wall_clock + Duration::from_secs(60),
        ..crate::chat::tool_run::WatchdogLimits::default()
    });

    let started = Instant::now();
    let _ = tool_run.drive(Vec::new()).await;
    let elapsed = started.elapsed();

    let finalized = sink.finalized.lock().expect("lock");
    let last = finalized.last();
    let failures_shown = events
        .lock()
        .expect("events")
        .iter()
        .filter(|event| matches!(event, InferenceEvent::Failed { .. }))
        .map(|event| format!("{event:?}"))
        .collect();
    let outcome = Outcome {
        state: tool_run.state(),
        content: last.and_then(|f| f.content.clone()).unwrap_or_default(),
        error: last.and_then(|f| f.error.clone()),
        status: last.map(|f| f.status.clone()).unwrap_or_default(),
        failures_shown,
        lookups: executor.calls.lock().expect("lock").clone(),
        dashboards: executor.dashboards.lock().expect("lock").clone(),
        elapsed,
    };
    println!(
        "[{:?}] dashboards={:?} state={:?} status={} lookups={:?} content({} chars)={:?}",
        outcome.elapsed,
        outcome.dashboards,
        outcome.state,
        outcome.status,
        outcome.lookups,
        outcome.content.len(),
        outcome.content.chars().take(300).collect::<String>()
    );
    outcome
}

/// The person-visible contract of every run.
fn assert_smooth(outcome: &Outcome) {
    assert_eq!(
        outcome.state,
        RunState::Completed,
        "error={:?} failures={:?}",
        outcome.error,
        outcome.failures_shown
    );
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(
        outcome.failures_shown.is_empty(),
        "{:?}",
        outcome.failures_shown
    );
    assert!(!outcome.content.trim().is_empty(), "the answer is empty");
    assert!(
        !outcome
            .content
            .contains("was interrupted before it finished")
            && !outcome
                .content
                .contains("cut off at the model's output limit"),
        "{}",
        outcome.content
    );
}

fn question(text: &str) -> Vec<ContextMessage> {
    vec![saved(ContextRole::User, text.to_owned(), 1)]
}

const PRICES: &str = "Use the lookup tool once for each of apple, banana and cherry, then tell me the total price of all three.";

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_answers_a_plain_question() {
    let outcome = run(
        gateway(),
        question("What is the capital of France? Answer in one sentence."),
    )
    .await;
    assert_smooth(&outcome);
    assert!(outcome.content.contains("Paris"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_runs_a_tool_loop() {
    let outcome = run(gateway(), question(PRICES)).await;
    assert_smooth(&outcome);
    assert!(outcome.lookups.len() >= 3, "{:?}", outcome.lookups);
    assert!(outcome.content.contains("12"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_writes_a_long_answer() {
    let outcome = run(
        gateway(),
        question("Write a detailed 700 word essay about the history of the compass."),
    )
    .await;
    assert_smooth(&outcome);
    assert!(
        outcome.content.split_whitespace().count() >= 400,
        "{} words",
        outcome.content.split_whitespace().count()
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_compacts_a_long_conversation_and_remembers_the_start() {
    // About 90,000 estimated tokens against 81,920 of input room: over the
    // limit before the first request.
    let seed = conversation(
        30,
        1_100,
        "What was the project codename I asked you to remember at the start? Answer with the codename.",
    );
    let outcome = run(gateway(), seed).await;
    assert_smooth(&outcome);
    assert!(
        outcome.content.to_uppercase().contains("BLUEHERON"),
        "{}",
        outcome.content
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_answers_a_plain_question() {
    let outcome = run(
        luna(200_000),
        question("What is the capital of France? Answer in one sentence."),
    )
    .await;
    assert_smooth(&outcome);
    assert!(outcome.content.contains("Paris"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_runs_a_tool_loop() {
    let outcome = run(luna(200_000), question(PRICES)).await;
    assert_smooth(&outcome);
    assert!(outcome.lookups.len() >= 3, "{:?}", outcome.lookups);
    assert!(outcome.content.contains("12"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_compacts_a_long_conversation_and_remembers_the_start() {
    // A 60,000-token window leaves 27,000 estimated tokens of input room.
    let seed = conversation(
        14,
        700,
        "What was the project codename I asked you to remember at the start? Answer with the codename.",
    );
    let outcome = run(luna(60_000), seed).await;
    assert_smooth(&outcome);
    assert!(
        outcome.content.to_uppercase().contains("BLUEHERON"),
        "{}",
        outcome.content
    );
}

const PAGES: &str = "Fetch pages 1 through 14 with the page tool, one call per page, and read each one. Then tell me the secret word of page 14 and of page 1.";

fn assert_read_every_page(outcome: &Outcome) {
    assert_smooth(outcome);
    assert!(
        outcome
            .lookups
            .iter()
            .filter(|call| call.starts_with("page"))
            .count()
            >= 14,
        "{:?}",
        outcome.lookups
    );
    assert!(outcome.content.contains("WORD14"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_finishes_a_long_run_of_tool_calls() {
    // About 14 calls of 20 KB each fill the input room several times over, so
    // the run condenses its own history while it works.
    assert_read_every_page(&run(gateway(), question(PAGES)).await);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_finishes_a_long_run_of_tool_calls() {
    assert_read_every_page(&run(luna(60_000), question(PAGES)).await);
}

/// Pressing Stop ends the run promptly, as a stop and not as a failure.
fn assert_stopped(outcome: &Outcome, pressed_at: Duration) {
    assert_eq!(outcome.state, RunState::Cancelled, "{:?}", outcome.error);
    assert!(
        outcome.failures_shown.is_empty(),
        "{:?}",
        outcome.failures_shown
    );
    assert!(
        outcome.elapsed < pressed_at + Duration::from_secs(10),
        "stopping took {:?}",
        outcome.elapsed
    );
}

const ESSAY: &str = "Write a detailed 900 word essay about the history of the compass.";

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_stops_when_stopped_while_the_answer_is_pending() {
    let after = Duration::from_secs(4);
    assert_stopped(
        &run_with(gateway(), question(ESSAY), Some(after)).await,
        after,
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_stops_between_tool_calls_and_can_run_again() {
    let after = Duration::from_secs(25);
    assert_stopped(
        &run_with(gateway(), question(PAGES), Some(after)).await,
        after,
    );
    // The slot the stopped run held is free: the next message is answered.
    assert_smooth(&run(gateway(), question("Say hello in five words.")).await);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_stops_mid_answer() {
    let after = Duration::from_secs(3);
    assert_stopped(
        &run_with(luna(200_000), question(ESSAY), Some(after)).await,
        after,
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_stops_between_tool_calls() {
    let after = Duration::from_secs(6);
    assert_stopped(
        &run_with(luna(60_000), question(PAGES), Some(after)).await,
        after,
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_answers_a_plain_question() {
    let outcome = run(
        sonnet(),
        question("What is the capital of France? Answer in one sentence."),
    )
    .await;
    assert_smooth(&outcome);
    assert!(outcome.content.contains("Paris"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_runs_a_tool_loop() {
    let outcome = run(sonnet(), question(PRICES)).await;
    assert_smooth(&outcome);
    assert!(outcome.lookups.len() >= 3, "{:?}", outcome.lookups);
    assert!(outcome.content.contains("12"), "{}", outcome.content);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_reads_fourteen_big_pages() {
    assert_read_every_page(&run(sonnet(), question(PAGES)).await);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_writes_a_long_answer() {
    let outcome = run(sonnet(), question(ESSAY)).await;
    assert_smooth(&outcome);
    assert!(outcome.content.split_whitespace().count() >= 500);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_stops_mid_answer() {
    let after = Duration::from_secs(3);
    assert_stopped(
        &run_with(sonnet(), question(ESSAY), Some(after)).await,
        after,
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_stops_between_tool_calls() {
    let after = Duration::from_secs(8);
    assert_stopped(
        &run_with(sonnet(), question(PAGES), Some(after)).await,
        after,
    );
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_remembers_an_early_fact_in_a_long_saved_conversation() {
    // About 150,000 tokens of saved history, well inside the window: no
    // compaction yet, but a real long request on the native route.
    let seed = conversation(
        30,
        1_600,
        "What was the project codename I asked you to remember at the start? Answer with the codename.",
    );
    let outcome = run(sonnet(), seed).await;
    assert_smooth(&outcome);
    assert!(
        outcome.content.to_uppercase().contains("BLUEHERON"),
        "{}",
        outcome.content
    );
}

const DASHBOARD: &str = "Build an ultra cool, interactive, ultra premium single-file HTML dashboard for monthly sales by region, with inline CSS, inline JavaScript and SVG or canvas charts, no external libraries. Save it with the save_dashboard tool, then tell me it is saved.";

/// A saved dashboard must reach the tool whole.
fn assert_dashboard_saved_whole(outcome: &Outcome) {
    assert_smooth(outcome);
    let last = outcome.dashboards.last().expect("a dashboard was saved");
    assert!(last.0 > 6_000, "the dashboard is only {} bytes", last.0);
    assert!(last.1, "the saved html does not end with </html>");
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_GATEWAY_KEY"]
async fn gateway_saves_a_large_dashboard_whole() {
    assert_dashboard_saved_whole(&run(gateway(), question(DASHBOARD)).await);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn luna_saves_a_large_dashboard_whole() {
    assert_dashboard_saved_whole(&run(luna(200_000), question(DASHBOARD)).await);
}

#[tokio::test]
#[ignore = "live: needs XF_LIVE_OPENROUTER_KEY"]
async fn sonnet_saves_a_large_dashboard_whole() {
    assert_dashboard_saved_whole(&run(sonnet(), question(DASHBOARD)).await);
}
