use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ModelId;
use xfalcon_contracts::inference::{FinishReason, InferenceEvent};
use xfalcon_contracts::tools::{InvocationId, ToolId, ToolRisk, ToolSource};
use xfalcon_contracts::{ConversationId, RunId, RunState};

use crate::agent::engine::{run_with_tool_loop, RunEngine, RunEngineError, ToolLoopRunOutcome};
use crate::agent::executor::{
    DaRunObservation, ExecutorError, InferenceExecutor, ModelTurn, ObservedRun, ObservedRunEvent,
    RequestedToolCall, TurnError,
};
use crate::agent::run::{InMemoryRunJournal, Run, RunBudgets};
use crate::agent::tool_loop::{
    ToolCallOutcome, ToolExecutionError, ToolExecutorPort, ToolLoop, ToolLoopBudgets, ToolLoopError,
};
use crate::tools::invocation::{InMemoryInvocationJournal, ToolResult};
use crate::tools::policy::Actor;
use crate::tools::registry::{ToolDescriptor, ToolRegistry};

struct ScriptedObserver {
    runs: Vec<ObservedRun>,
    event_pages: Vec<Vec<ObservedRunEvent>>,
    calls: std::sync::Mutex<usize>,
}

impl ScriptedObserver {
    fn new(run: ObservedRun, events: Vec<ObservedRunEvent>) -> Self {
        Self {
            runs: vec![run],
            event_pages: vec![events],
            calls: std::sync::Mutex::new(0),
        }
    }
}

#[async_trait]
impl DaRunObservation for ScriptedObserver {
    async fn fetch_run(&self, _run_id: uuid::Uuid) -> Result<ObservedRun, ExecutorError> {
        Ok(self.runs[0].clone())
    }

    async fn fetch_new_events(
        &self,
        _run_id: uuid::Uuid,
        after_sequence: i64,
    ) -> Result<Vec<ObservedRunEvent>, ExecutorError> {
        let mut calls = self.calls.lock().expect("lock");
        let page = self.event_pages.get(*calls).cloned().unwrap_or_default();
        *calls += 1;
        Ok(page
            .into_iter()
            .filter(|event| event.sequence_no > after_sequence)
            .collect())
    }
}

fn budgets() -> RunBudgets {
    RunBudgets {
        event_budget: 128,
        output_byte_budget: 64 * 1024,
        wall_clock: Duration::from_secs(30),
    }
}

fn sample_events() -> Vec<ObservedRunEvent> {
    vec![
        ObservedRunEvent {
            sequence_no: 1,
            event_type: "run.start".to_owned(),
            payload: IndexMap::new(),
        },
        ObservedRunEvent {
            sequence_no: 2,
            event_type: "run.stream_delta".to_owned(),
            payload: IndexMap::from([("delta".to_owned(), json!("Hello"))]),
        },
        ObservedRunEvent {
            sequence_no: 3,
            event_type: "run.complete".to_owned(),
            payload: IndexMap::new(),
        },
    ]
}

#[tokio::test]
async fn engine_drives_run_to_completed_from_da_events() {
    let run_id = RunId::from(uuid::Uuid::new_v4().to_string());
    let conversation = ConversationId::from(uuid::Uuid::new_v4().to_string());
    let da_run_id = uuid::Uuid::parse_str(run_id.as_str()).expect("uuid run id");

    let observer = Arc::new(ScriptedObserver::new(
        ObservedRun {
            status: "completed".to_owned(),
            output_message: Some("Hello".to_owned()),
            error: None,
            provider: "openai".to_owned(),
            model: "gpt-test".to_owned(),
        },
        sample_events(),
    ));

    let mut run = Run::new(
        run_id.clone(),
        conversation,
        ModelId::from("gpt-test"),
        budgets(),
    );
    let mut journal = InMemoryRunJournal::new();

    let outcome = RunEngine::new(observer)
        .poll_da_run(
            &mut run,
            &mut journal,
            da_run_id,
            CancellationToken::new(),
            Duration::from_millis(1),
            |_event| {},
        )
        .await
        .expect("engine");

    assert_eq!(run.state(), RunState::Completed);
    assert_eq!(outcome.finish_reason, FinishReason::Stop);
    assert_eq!(outcome.final_text, "Hello");
    assert!(
        outcome
            .presentation
            .iter()
            .any(|event| matches!(event, InferenceEvent::Started { .. })),
        "expected started in presentation"
    );
    assert!(
        outcome
            .presentation
            .iter()
            .any(|event| matches!(event, InferenceEvent::TextDelta { .. })),
        "expected streamed deltas"
    );
}

#[tokio::test]
async fn engine_emits_started_before_da_stream_events() {
    let run_id = RunId::from(uuid::Uuid::new_v4().to_string());
    let conversation = ConversationId::from(uuid::Uuid::new_v4().to_string());
    let da_run_id = uuid::Uuid::parse_str(run_id.as_str()).expect("uuid run id");

    let observer = Arc::new(ScriptedObserver::new(
        ObservedRun {
            status: "completed".to_owned(),
            output_message: Some("Hi".to_owned()),
            error: None,
            provider: "openai".to_owned(),
            model: "gpt-test".to_owned(),
        },
        sample_events(),
    ));

    let mut run = Run::new(
        run_id.clone(),
        conversation,
        ModelId::from("gpt-test"),
        budgets(),
    );
    let mut journal = InMemoryRunJournal::new();
    let mut order = Vec::new();

    RunEngine::new(observer)
        .poll_da_run(
            &mut run,
            &mut journal,
            da_run_id,
            CancellationToken::new(),
            Duration::from_millis(1),
            |event| {
                let tag = match event {
                    InferenceEvent::Started { .. } => "started",
                    InferenceEvent::TextDelta { .. } => "delta",
                    _ => "other",
                };
                order.push(tag.to_owned());
            },
        )
        .await
        .expect("engine");

    assert_eq!(order.first().map(String::as_str), Some("started"));
}

#[tokio::test]
async fn engine_marks_failed_when_tool_call_arrives() {
    let run_id = RunId::from(uuid::Uuid::new_v4().to_string());
    let conversation = ConversationId::from(uuid::Uuid::new_v4().to_string());
    let da_run_id = uuid::Uuid::parse_str(run_id.as_str()).expect("uuid");

    let events = vec![
        ObservedRunEvent {
            sequence_no: 1,
            event_type: "run.start".to_owned(),
            payload: IndexMap::new(),
        },
        ObservedRunEvent {
            sequence_no: 2,
            event_type: "tool_call.start".to_owned(),
            payload: IndexMap::new(),
        },
    ];

    let observer = Arc::new(ScriptedObserver::new(
        ObservedRun {
            status: "completed".to_owned(),
            output_message: None,
            error: None,
            provider: "openai".to_owned(),
            model: "gpt-test".to_owned(),
        },
        events,
    ));

    let mut run = Run::new(
        run_id.clone(),
        conversation,
        ModelId::from("gpt-test"),
        budgets(),
    );
    let mut journal = InMemoryRunJournal::new();
    let mut streamed = Vec::new();

    let err = RunEngine::new(observer)
        .poll_da_run(
            &mut run,
            &mut journal,
            da_run_id,
            CancellationToken::new(),
            Duration::from_millis(1),
            |event| streamed.push(event.clone()),
        )
        .await
        .expect_err("tool unavailable");

    assert!(matches!(err, RunEngineError::ToolsUnavailable));
    assert_eq!(run.state(), RunState::Failed);
    assert!(
        streamed
            .iter()
            .any(|event| matches!(event, InferenceEvent::Failed { .. })),
        "expected failed inference via callback, got {streamed:?}"
    );
}

struct FailingObserver;

#[async_trait]
impl DaRunObservation for FailingObserver {
    async fn fetch_run(&self, _run_id: uuid::Uuid) -> Result<ObservedRun, ExecutorError> {
        Err(ExecutorError::ObservationFailed)
    }

    async fn fetch_new_events(
        &self,
        _run_id: uuid::Uuid,
        _after_sequence: i64,
    ) -> Result<Vec<ObservedRunEvent>, ExecutorError> {
        Err(ExecutorError::ObservationFailed)
    }
}

#[tokio::test]
async fn engine_marks_interrupted_when_observation_fails() {
    let run_id = RunId::from(uuid::Uuid::new_v4().to_string());
    let conversation = ConversationId::from(uuid::Uuid::new_v4().to_string());
    let da_run_id = uuid::Uuid::new_v4();

    let mut run = Run::new(
        run_id.clone(),
        conversation,
        ModelId::from("gpt-test"),
        budgets(),
    );
    let mut journal = InMemoryRunJournal::new();

    let err = RunEngine::new(Arc::new(FailingObserver))
        .poll_da_run(
            &mut run,
            &mut journal,
            da_run_id,
            CancellationToken::new(),
            Duration::from_millis(1),
            |_event| {},
        )
        .await
        .expect_err("observation failed");

    assert!(matches!(
        err,
        RunEngineError::Executor(ExecutorError::ObservationFailed)
    ));
    assert_eq!(run.state(), RunState::Interrupted);
}

// ---------------------------------------------------------------------
// Task 7: `run_with_tool_loop` — the bounded tool-continuation entry point.
// ---------------------------------------------------------------------

/// A scripted [`InferenceExecutor`] fake, mirroring `ScriptedObserver` above:
/// returns one queued [`ModelTurn`] per call, ignoring the tool outcomes fed
/// back (the tool-loop's own tests in `crate::agent::tests::tool_loop`
/// already prove those are handled correctly; this fixture only needs to
/// prove `run_with_tool_loop` drives [`Run`]'s state machine correctly).
struct ScriptedTurnDriver {
    turns: Mutex<VecDeque<ModelTurn>>,
    wait_for_cancel: bool,
}

impl ScriptedTurnDriver {
    fn new(turns: Vec<ModelTurn>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter().collect()),
            wait_for_cancel: false,
        }
    }
}

#[async_trait]
impl InferenceExecutor for ScriptedTurnDriver {
    async fn next_turn(
        &mut self,
        _tool_outcomes: Vec<ToolCallOutcome>,
        cancel: &CancellationToken,
    ) -> Result<ModelTurn, TurnError> {
        if self.wait_for_cancel {
            cancel.cancelled().await;
        }
        Ok(self
            .turns
            .lock()
            .expect("lock")
            .pop_front()
            .unwrap_or(ModelTurn::Final {
                finish_reason: FinishReason::Stop,
                final_text: "no more scripted turns".to_owned(),
            }))
    }
}

struct NoopToolExecutor;

#[async_trait]
impl ToolExecutorPort for NoopToolExecutor {
    async fn execute(
        &self,
        _tool_id: &ToolId,
        _arguments: &serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        ToolResult::new(Vec::new(), None, ToolSource::Internal).map_err(Into::into)
    }
}

struct AuthRefusingToolExecutor;

#[async_trait]
impl ToolExecutorPort for AuthRefusingToolExecutor {
    async fn execute(
        &self,
        _tool_id: &ToolId,
        _arguments: &serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        Err(ToolExecutionError::AuthorizationRefused {
            label: "MCP server".to_owned(),
        })
    }
}

fn engine_test_registry(risk: ToolRisk) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            ToolDescriptor::new(
                "test.tool",
                ToolSource::Internal,
                "Test tool",
                "A tool used only by engine-level tool-loop tests.",
                json!({ "type": "object", "properties": {} }),
                risk,
                Vec::new(),
                true,
            )
            .expect("valid descriptor"),
        )
        .expect("unique id");
    registry
}

fn engine_test_tool_loop(risk: ToolRisk) -> ToolLoop {
    let mut allowlist = HashSet::new();
    allowlist.insert(ToolId::from("test.tool"));
    ToolLoop::new(
        engine_test_registry(risk),
        allowlist,
        Actor::new("user-1").expect("valid actor"),
        RunId::from("run-tool-loop"),
        None,
        None,
        ToolLoopBudgets::production(),
    )
}

fn engine_test_run() -> Run {
    Run::new(
        RunId::from("run-tool-loop"),
        ConversationId::from("conv-1"),
        ModelId::from("model-1"),
        RunBudgets {
            event_budget: 128,
            output_byte_budget: 64 * 1024,
            wall_clock: Duration::from_secs(30),
        },
    )
}

#[tokio::test]
async fn run_with_tool_loop_completes_when_the_driver_needs_no_tools() {
    let mut run = engine_test_run();
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let mut driver = ScriptedTurnDriver::new(vec![ModelTurn::Final {
        finish_reason: FinishReason::Stop,
        final_text: "hello".to_owned(),
    }]);
    let mut tool_loop = engine_test_tool_loop(ToolRisk::ReadOnly);
    let tool_executor = NoopToolExecutor;

    let outcome = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver,
        &mut tool_loop,
        &tool_executor,
        None,
        Vec::new(),
        CancellationToken::new(),
        |_event| {},
    )
    .await
    .expect("run completes");

    assert_eq!(run.state(), RunState::Completed);
    match outcome {
        ToolLoopRunOutcome::Completed(completed) => {
            assert_eq!(completed.final_text, "hello");
            assert_eq!(completed.finish_reason, FinishReason::Stop);
        }
        ToolLoopRunOutcome::AwaitingApproval => panic!("expected completion"),
    }
}

#[tokio::test]
async fn deadline_during_a_turn_prevents_dispatching_its_late_tool_calls() {
    let mut run = Run::new(
        RunId::from("run-tool-loop"),
        ConversationId::from("conv-1"),
        ModelId::from("model-1"),
        RunBudgets {
            wall_clock: Duration::from_millis(50),
            ..budgets()
        },
    );
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let mut driver = ScriptedTurnDriver::new(vec![ModelTurn::ToolCalls(vec![RequestedToolCall {
        invocation_id: InvocationId::from("inv-1"),
        tool_id: ToolId::from("test.tool"),
        arguments: json!({}),
    }])]);
    driver.wait_for_cancel = true;
    let mut tool_loop = engine_test_tool_loop(ToolRisk::ReadOnly);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        run_with_tool_loop(
            &mut run,
            &mut run_journal,
            &mut invocation_journal,
            &mut driver,
            &mut tool_loop,
            &NoopToolExecutor,
            None,
            Vec::new(),
            CancellationToken::new(),
            |_event| {},
        ),
    )
    .await
    .expect("deadline signals cancellation");
    assert!(matches!(
        result,
        Err(RunEngineError::Run(
            crate::agent::run::RunError::DeadlineExceeded
        ))
    ));
    assert_eq!(run.state(), RunState::Failed);
    assert!(
        invocation_journal.entries().is_empty(),
        "no tool may be admitted after the turn's deadline"
    );
}

#[tokio::test]
async fn run_with_tool_loop_fails_when_mcp_authorization_is_invalidated() {
    let mut run = engine_test_run();
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let mut driver = ScriptedTurnDriver::new(vec![
        ModelTurn::ToolCalls(vec![RequestedToolCall {
            invocation_id: InvocationId::from("inv-1"),
            tool_id: ToolId::from("test.tool"),
            arguments: json!({}),
        }]),
        ModelTurn::Final {
            finish_reason: FinishReason::Stop,
            final_text: "must not run".to_owned(),
        },
    ]);
    let mut tool_loop = engine_test_tool_loop(ToolRisk::ReadOnly);

    let error = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver,
        &mut tool_loop,
        &AuthRefusingToolExecutor,
        None,
        Vec::new(),
        CancellationToken::new(),
        |_event| {},
    )
    .await
    .expect_err("an invalid MCP authorization must fail the run");

    assert!(matches!(
        error,
        RunEngineError::ToolLoop(ToolLoopError::AuthorizationRefused)
    ));
    assert_eq!(run.state(), RunState::Failed);
    assert_eq!(
        driver.turns.lock().expect("lock").len(),
        1,
        "the model must not get another turn after MCP authorization expires"
    );
}

#[tokio::test]
async fn run_with_tool_loop_pauses_the_run_when_a_mutating_tool_is_requested() {
    let mut run = engine_test_run();
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let mut driver = ScriptedTurnDriver::new(vec![ModelTurn::ToolCalls(vec![RequestedToolCall {
        invocation_id: InvocationId::from("inv-1"),
        tool_id: ToolId::from("test.tool"),
        arguments: json!({}),
    }])]);
    let mut tool_loop = engine_test_tool_loop(ToolRisk::Mutating);
    let tool_executor = NoopToolExecutor;

    let outcome = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver,
        &mut tool_loop,
        &tool_executor,
        None,
        Vec::new(),
        CancellationToken::new(),
        |_event| {},
    )
    .await
    .expect("run pauses rather than erroring");

    assert!(matches!(outcome, ToolLoopRunOutcome::AwaitingApproval));
    assert_eq!(run.state(), RunState::AwaitingApproval);
    assert!(tool_loop.has_pending_approval(&InvocationId::from("inv-1")));
}

#[tokio::test]
async fn run_with_tool_loop_resumes_after_approval_and_completes() {
    let mut run = engine_test_run();
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let mut tool_loop = engine_test_tool_loop(ToolRisk::Mutating);
    let tool_executor = NoopToolExecutor;
    let cancel = CancellationToken::new();

    // Round 1: the model requests the mutating tool, pausing the run.
    let mut driver = ScriptedTurnDriver::new(vec![ModelTurn::ToolCalls(vec![RequestedToolCall {
        invocation_id: InvocationId::from("inv-1"),
        tool_id: ToolId::from("test.tool"),
        arguments: json!({}),
    }])]);
    let paused = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver,
        &mut tool_loop,
        &tool_executor,
        None,
        Vec::new(),
        cancel.clone(),
        |_event| {},
    )
    .await
    .expect("run pauses");
    assert!(matches!(paused, ToolLoopRunOutcome::AwaitingApproval));
    assert_eq!(run.state(), RunState::AwaitingApproval);

    // The (future) approval UI approves the exact pending invocation.
    let token = tool_loop
        .pending_approval_request(&InvocationId::from("inv-1"))
        .expect("pending approval exists")
        .clone()
        .approve();
    let resumed_outcome = tool_loop
        .resume(
            &InvocationId::from("inv-1"),
            token,
            &tool_executor,
            None,
            &mut invocation_journal,
            &cancel,
        )
        .await
        .expect("resume succeeds");
    assert!(matches!(resumed_outcome, ToolCallOutcome::Completed { .. }));

    // Round 2: re-entering `run_with_tool_loop` transitions
    // `AwaitingApproval -> Running` and hands the resumed outcome to the
    // driver's next turn, which now finishes the run.
    let mut driver2 = ScriptedTurnDriver::new(vec![ModelTurn::Final {
        finish_reason: FinishReason::Stop,
        final_text: "done".to_owned(),
    }]);
    let completed = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver2,
        &mut tool_loop,
        &tool_executor,
        None,
        vec![resumed_outcome],
        cancel,
        |_event| {},
    )
    .await
    .expect("run completes after resume");

    assert_eq!(run.state(), RunState::Completed);
    match completed {
        ToolLoopRunOutcome::Completed(outcome) => assert_eq!(outcome.final_text, "done"),
        ToolLoopRunOutcome::AwaitingApproval => panic!("expected completion"),
    }
}

/// Three independent review lenses flagged the same gap: a `ToolLoopError`
/// (round-cap hit, too-many-parallel-calls, round timeout) used to bubble
/// out of `run_with_tool_loop` via `?` with no `Run` transition at all,
/// permanently stranding the run in the non-terminal `RunState::Running`.
/// Drives `run_round` past a deliberately tiny round cap through
/// `run_with_tool_loop` itself (not just `ToolLoop::run_round` in
/// isolation) and asserts `run.state() == RunState::Failed` afterward.
#[tokio::test]
async fn run_with_tool_loop_fails_the_run_when_the_round_cap_is_exceeded() {
    let mut run = engine_test_run();
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let tool_executor = NoopToolExecutor;

    let mut allowlist = HashSet::new();
    allowlist.insert(ToolId::from("test.tool"));
    let budgets = ToolLoopBudgets {
        max_rounds: 1,
        ..ToolLoopBudgets::production()
    };
    let mut tool_loop = ToolLoop::new(
        engine_test_registry(ToolRisk::ReadOnly),
        allowlist,
        Actor::new("user-1").expect("valid actor"),
        RunId::from("run-tool-loop"),
        None,
        None,
        budgets,
    );

    // The read-only tool never pauses, so the driver is asked for a second
    // round — which must exceed the 1-round cap.
    let mut driver = ScriptedTurnDriver::new(vec![
        ModelTurn::ToolCalls(vec![RequestedToolCall {
            invocation_id: InvocationId::from("inv-1"),
            tool_id: ToolId::from("test.tool"),
            arguments: json!({}),
        }]),
        ModelTurn::ToolCalls(vec![RequestedToolCall {
            invocation_id: InvocationId::from("inv-2"),
            tool_id: ToolId::from("test.tool"),
            arguments: json!({}),
        }]),
    ]);

    let error = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver,
        &mut tool_loop,
        &tool_executor,
        None,
        Vec::new(),
        CancellationToken::new(),
        |_event| {},
    )
    .await
    .expect_err("the second round must exceed the 1-round cap");

    assert!(matches!(
        error,
        RunEngineError::ToolLoop(ToolLoopError::MaxRoundsExceeded(1))
    ));
    assert_eq!(
        run.state(),
        RunState::Failed,
        "a ToolLoopError must still leave the run in a terminal, journaled state, not \
         stranded in Running"
    );
}

/// A `Run` cancelled while paused (`RunState::AwaitingApproval ->
/// Cancelled` is a legal transition at the `Run` level alone) must not
/// leave the `ToolInvocation` that caused the pause durably stuck at
/// `AwaitingApproval` in `ToolLoop`'s own `pending_approval` map with
/// nothing left able to resolve it — see `run::allowed_transition`'s own
/// doc comment on the required call-ordering contract this closes for
/// `run_with_tool_loop`'s own re-entry path.
#[tokio::test]
async fn cancelling_a_paused_run_also_declines_its_pending_tool_invocation() {
    let mut run = engine_test_run();
    let mut run_journal = InMemoryRunJournal::new();
    let mut invocation_journal = InMemoryInvocationJournal::new();
    let mut tool_loop = engine_test_tool_loop(ToolRisk::Mutating);
    let tool_executor = NoopToolExecutor;
    let cancel = CancellationToken::new();

    let mut driver = ScriptedTurnDriver::new(vec![ModelTurn::ToolCalls(vec![RequestedToolCall {
        invocation_id: InvocationId::from("inv-1"),
        tool_id: ToolId::from("test.tool"),
        arguments: json!({}),
    }])]);
    let paused = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver,
        &mut tool_loop,
        &tool_executor,
        None,
        Vec::new(),
        cancel.clone(),
        |_event| {},
    )
    .await
    .expect("run pauses");
    assert!(matches!(paused, ToolLoopRunOutcome::AwaitingApproval));
    assert_eq!(run.state(), RunState::AwaitingApproval);
    assert!(tool_loop.has_pending_approval(&InvocationId::from("inv-1")));

    // The run is cancelled while still paused (e.g. the user closed the
    // chat before ever resolving the approval prompt).
    cancel.cancel();
    let mut driver_after_cancel = ScriptedTurnDriver::new(Vec::new());
    let error = run_with_tool_loop(
        &mut run,
        &mut run_journal,
        &mut invocation_journal,
        &mut driver_after_cancel,
        &mut tool_loop,
        &tool_executor,
        None,
        Vec::new(),
        cancel,
        |_event| {},
    )
    .await
    .expect_err("re-entering a paused, already-cancelled run must fail with Cancelled");

    assert!(matches!(error, RunEngineError::Cancelled));
    assert_eq!(run.state(), RunState::Cancelled);
    assert!(
        !tool_loop.has_pending_approval(&InvocationId::from("inv-1")),
        "the pending invocation that caused the pause must be resolved (declined) when its \
         owning run is cancelled, never left stuck at AwaitingApproval with nothing able to \
         resume or clean it up"
    );
}
