//! Pausing a tool-capable run on an approval, and resuming it afterwards.
//!
//! The invariant under test: a run that pauses must be resumable *as itself*.
//! Its `ToolLoop` holds the pending invocation, its turn driver holds the
//! conversation context earlier rounds built up, and its `Run` is a state
//! machine mid-flight — rebuilding any of them on resume would silently
//! produce a different run.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::credentials::{ModelId, ProviderId};
use xfalcon_contracts::inference::InferenceEvent;
use xfalcon_contracts::tools::{InvocationId, ToolId, ToolRisk, ToolSource};
use xfalcon_contracts::{ConversationId, RunId, RunState};
use zeroize::Zeroizing;

use crate::agent::context::{ContextMessage, ContextRole, ContextToolCall};
use crate::agent::local_executor::{DurableContextSnapshot, RunEventSink, RunEventSinkError};
use crate::agent::run::{InMemoryRunJournal, Run, RunBudgets};
use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort, ToolLoop, ToolLoopBudgets};
use crate::agent::tool_session::ToolSession;
use crate::agent::turn_driver::{LocalTurnDriver, LocalTurnDriverConfig};
use crate::chat::tool_run::ToolRun;
use crate::clients::da::chat_api::ModelExecutionMode;
use crate::clients::da::generated::{BatchRecord, Run as DaRun, RunFinalize, RunFinalizeResult};
use crate::providers::{
    ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStopReason, ProviderStreamEvent,
    ProviderToolSchema,
};
use crate::tools::invocation::{InMemoryInvocationJournal, ToolResult};
use crate::tools::policy::{Actor, PathAuthority, ProjectContext};
use crate::tools::registry::{ToolDescriptor, ToolRegistry};

const MUTATING_TOOL: &str = "warehouse.write_table";
const WIRE_NAME: &str = "warehouse_write_table";

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

struct ScriptedAdapter {
    turns: Mutex<Vec<Vec<ProviderStreamEvent>>>,
    fragment_delay_ms: std::sync::atomic::AtomicU64,
    seen: Mutex<Vec<ProviderChatRequest>>,
    wait_for_cancel: std::sync::atomic::AtomicBool,
    /// Blocks forever and ignores the cancel, like a socket that never answers.
    hang_ignoring_cancel: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl ProviderAdapter for ScriptedAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::OpenAi
    }

    async fn probe_native_compaction(
        &self,
        _model_id: &str,
        _route_identity: &str,
        _session_id: &str,
        _secret: &str,
        _cancel: &CancellationToken,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        _secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        if self
            .hang_ignoring_cancel
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            std::future::pending::<()>().await;
        }
        if self
            .wait_for_cancel
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            cancel.cancelled().await;
            return Err(ProviderError::Cancelled);
        }
        let turn = {
            let mut turns = self.turns.lock().expect("lock");
            assert!(!turns.is_empty(), "the run asked for an unscripted turn");
            turns.remove(0)
        };
        self.seen.lock().expect("lock").push(request.clone());
        for event in turn {
            let delay = self
                .fragment_delay_ms
                .load(std::sync::atomic::Ordering::SeqCst);
            if delay > 0
                && matches!(
                    event,
                    ProviderStreamEvent::TextDelta(_) | ProviderStreamEvent::Progress
                )
            {
                tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {},
                    () = cancel.cancelled() => return Err(ProviderError::Cancelled),
                }
            }
            on_event(event);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct RecordingSink {
    pub(super) finalized: Mutex<Vec<RunFinalize>>,
    pub(super) messages: Mutex<Vec<ContextMessage>>,
    hold_finalize: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    hold_round: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    fail_round: std::sync::atomic::AtomicBool,
    round_progress_delay_ms: std::sync::atomic::AtomicU64,
    round_progress_steps: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl RunEventSink for Arc<RecordingSink> {
    async fn apply_batch(
        &self,
        _run_id: Uuid,
        _records: Vec<BatchRecord>,
    ) -> Result<(), RunEventSinkError> {
        Ok(())
    }

    async fn finalize(
        &self,
        _run_id: Uuid,
        finalize: RunFinalize,
    ) -> Result<RunFinalizeResult, RunEventSinkError> {
        let held = self.hold_finalize.lock().expect("hold").take();
        if let Some(held) = held {
            let _ = held.await;
        }
        self.finalized.lock().expect("lock").push(finalize);
        Ok(RunFinalizeResult {
            run: DaRun::default(),
            message_id: None,
            message_sequence_no: None,
        })
    }
    async fn record_tool_round(
        &self,
        _conversation_id: Uuid,
        round: crate::agent::local_executor::ToolRoundRecord,
        on_progress: &mut (dyn FnMut() + Send),
    ) -> Result<Vec<String>, RunEventSinkError> {
        if self.fail_round.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(RunEventSinkError::Unavailable);
        }
        let held = self.hold_round.lock().expect("hold").take();
        if let Some(held) = held {
            let _ = held.await;
        }
        let delay_ms = self
            .round_progress_delay_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let steps = self
            .round_progress_steps
            .load(std::sync::atomic::Ordering::SeqCst);
        for _ in 0..steps {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            on_progress();
        }
        let mut messages = self.messages.lock().expect("lock");
        messages.push(ContextMessage::assistant_tool_calls(
            round.text,
            round
                .calls
                .iter()
                .map(|call| {
                    ContextToolCall::new(
                        call.call_id.clone(),
                        call.tool_name.clone(),
                        call.arguments.clone(),
                    )
                })
                .collect(),
        ));
        messages.extend(round.calls.iter().map(|call| {
            ContextMessage::tool_result(
                call.call_id.clone(),
                call.tool_name.clone(),
                call.result.clone(),
            )
        }));
        on_progress();
        Ok(round.calls.into_iter().map(|call| call.result).collect())
    }

    async fn load_context(
        &self,
        _conversation_id: Uuid,
    ) -> Result<DurableContextSnapshot, RunEventSinkError> {
        Ok(DurableContextSnapshot {
            active_compaction: None,
            history_revision: 0,
            snapshot_head_sequence_no: 1,
            boundary_sequence_no: 0,
            compactable_through_sequence_no: 1,
            compactable_source_message_count: 1,
            source_message_ids_sha256: "0".repeat(64),
            messages: self.messages.lock().expect("lock").clone(),
        })
    }
}

#[derive(Default)]
struct CountingExecutor {
    calls: Mutex<Vec<ToolId>>,
    wait_for_cancel: std::sync::atomic::AtomicBool,
    authorization_refused: std::sync::atomic::AtomicBool,
    cancel_after_success: std::sync::atomic::AtomicBool,
    owning_run_cancel: CancellationToken,
    started: tokio::sync::Notify,
}

#[async_trait]
impl ToolExecutorPort for CountingExecutor {
    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        self.calls.lock().expect("lock").push(tool_id.clone());
        if let Some(delay) = arguments.get("probe_delay_ms").and_then(Value::as_u64) {
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {},
                () = cancel.cancelled() => return Err(ToolExecutionError::Cancelled),
            }
        }
        if self
            .authorization_refused
            .load(std::sync::atomic::Ordering::SeqCst)
            || arguments
                .get("refuse_authorization")
                .and_then(Value::as_bool)
                == Some(true)
        {
            return Err(ToolExecutionError::AuthorizationRefused {
                label: "MCP server".to_owned(),
            });
        }
        if self
            .wait_for_cancel
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.started.notify_one();
            cancel.cancelled().await;
            return Err(ToolExecutionError::Cancelled);
        }
        let result = ToolResult::new(Vec::new(), Some(json!({"rows": 1})), ToolSource::Mcp)
            .map_err(|error| ToolExecutionError::Failed(error.to_string()));
        if self
            .cancel_after_success
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.owning_run_cancel.cancel();
        }
        result
    }
}

/// Trusts every path — path containment is `TrustedRoot`'s job and has its
/// own tests; this fixture is about the approval pause.
pub(super) struct AnyPath;

impl PathAuthority for AnyPath {
    fn contains(&self, _path: &std::path::Path) -> bool {
        true
    }
}

fn registry(risk: ToolRisk) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            ToolDescriptor::new(
                MUTATING_TOOL,
                ToolSource::Mcp,
                "Write a table",
                "Writes rows into a warehouse table.",
                json!({"type": "object", "properties": {}, "additionalProperties": true}),
                risk,
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

struct Harness {
    tool_run: ToolRun<InMemoryRunJournal, InMemoryInvocationJournal, Arc<RecordingSink>>,
    adapter: Arc<ScriptedAdapter>,
    sink: Arc<RecordingSink>,
    executor: Arc<CountingExecutor>,
    cancel: CancellationToken,
    events: Arc<Mutex<Vec<InferenceEvent>>>,
}

fn harness(turns: Vec<Vec<ProviderStreamEvent>>) -> Harness {
    harness_with_wall_clock(turns, std::time::Duration::from_secs(600))
}

fn harness_with_wall_clock(
    turns: Vec<Vec<ProviderStreamEvent>>,
    wall_clock: std::time::Duration,
) -> Harness {
    harness_with_risk(turns, wall_clock, ToolRisk::Mutating)
}

fn harness_with_risk(
    turns: Vec<Vec<ProviderStreamEvent>>,
    wall_clock: std::time::Duration,
    risk: ToolRisk,
) -> Harness {
    harness_with_session(turns, wall_clock, risk, None)
}

fn harness_with_session(
    turns: Vec<Vec<ProviderStreamEvent>>,
    wall_clock: std::time::Duration,
    risk: ToolRisk,
    session: Option<ToolSession>,
) -> Harness {
    harness_with_config(turns, wall_clock, risk, session, |_| {})
}

fn harness_with_config(
    turns: Vec<Vec<ProviderStreamEvent>>,
    wall_clock: std::time::Duration,
    risk: ToolRisk,
    session: Option<ToolSession>,
    configure: impl FnOnce(&mut LocalTurnDriverConfig),
) -> Harness {
    let adapter = Arc::new(ScriptedAdapter {
        turns: Mutex::new(turns),
        fragment_delay_ms: std::sync::atomic::AtomicU64::new(0),
        seen: Mutex::new(Vec::new()),
        wait_for_cancel: std::sync::atomic::AtomicBool::new(false),
        hang_ignoring_cancel: std::sync::atomic::AtomicBool::new(false),
    });
    let sink = Arc::new(RecordingSink::default());
    sink.messages
        .lock()
        .expect("lock")
        .push(ContextMessage::new(ContextRole::User, "load the table"));
    let cancel = CancellationToken::new();
    let executor = Arc::new(CountingExecutor {
        owning_run_cancel: cancel.clone(),
        ..CountingExecutor::default()
    });

    let registry = registry(risk);
    let allowlist = registry.iter().map(|d| d.id().clone()).collect();
    let tool_loop = ToolLoop::new(
        registry,
        allowlist,
        Actor::new("account-1").expect("actor"),
        RunId::from("run-1"),
        Some(ProjectContext::new("project-1")),
        None,
        ToolLoopBudgets::production(),
    );
    let tool_session = ToolSession {
        tool_loop,
        executor: executor.clone(),
        path_authority: Some(Arc::new(AnyPath)),
        has_project_tools: true,
        unavailable_mcp_servers: Vec::new(),
        tools: vec![
            ProviderToolSchema {
                name: WIRE_NAME.to_owned(),
                description: "Write a table".to_owned(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            ProviderToolSchema {
                name: PLAN_WIRE_NAME.to_owned(),
                description: "Update the task checklist".to_owned(),
                input_schema: json!({"type": "object"}),
            },
        ],
        tool_ids: HashMap::from([
            (WIRE_NAME.to_owned(), ToolId::from(MUTATING_TOOL)),
            (
                PLAN_WIRE_NAME.to_owned(),
                ToolId::from("project.update_plan"),
            ),
        ]),
    };

    let tool_session = session.unwrap_or(tool_session);
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded_events = events.clone();
    let on_inference: Arc<dyn Fn(&InferenceEvent) + Send + Sync> = Arc::new(move |event| {
        recorded_events.lock().expect("events").push(event.clone());
    });
    let mut config = LocalTurnDriverConfig {
        run_id: RunId::from("run-1"),
        da_run_id: Uuid::nil(),
        model_id: "gpt-example".to_owned(),
        route_identity: "credential-test".to_owned(),
        max_output_tokens: Some(512),
        execution_mode: ModelExecutionMode::NativeCompaction,
        tools: tool_session.tools.clone(),
        tool_ids: tool_session.tool_ids.clone(),
        seed_messages: vec![ContextMessage::new(ContextRole::User, "load the table")],
        conversation_id: Uuid::nil(),
        max_context_bytes: crate::agent::turn_driver::MAX_CONTEXT_BYTES,
        result_budget: crate::tools::ToolResultBudget::floor(),
        mcp_scope: None,
        active_compaction: None,
        history_revision: 0,
        snapshot_head_sequence_no: 1,
        boundary_sequence_no: 0,
        compactable_through_sequence_no: 1,
        compactable_source_message_count: 1,
        source_message_ids_sha256: "0".repeat(64),
    };
    configure(&mut config);
    *sink.messages.lock().expect("messages") = config.seed_messages.clone();
    let driver = LocalTurnDriver::new(
        sink.clone(),
        adapter.clone() as Arc<dyn ProviderAdapter>,
        Zeroizing::new("sk-test".to_owned()),
        on_inference.clone(),
        config,
    );

    let run = Run::new(
        RunId::from("run-1"),
        ConversationId::from("conversation-1"),
        ModelId::from("gpt-example"),
        RunBudgets {
            event_budget: 4_096,
            output_byte_budget: 512 * 1024,
            wall_clock,
        },
    );

    let tool_run = ToolRun::new(
        run,
        InMemoryRunJournal::new(),
        InMemoryInvocationJournal::new(),
        tool_session,
        driver,
        on_inference,
        cancel.clone(),
    );

    Harness {
        tool_run,
        adapter,
        sink,
        executor,
        cancel,
        events,
    }
}

#[tokio::test]
async fn a_blocked_turn_is_failed_at_the_run_wall_clock_deadline() {
    let mut harness = harness_with_wall_clock(Vec::new(), std::time::Duration::from_millis(50));
    harness
        .adapter
        .wait_for_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let outcome = harness
        .tool_run
        .drive(Vec::new())
        .await
        .expect_err("the hard deadline must interrupt a blocked provider turn");

    assert!(matches!(
        outcome,
        crate::agent::engine::RunEngineError::Run(crate::agent::run::RunError::DeadlineExceeded)
    ));
    assert_eq!(harness.tool_run.state(), RunState::Failed);
    assert!(harness.cancel.is_cancelled());
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "error");
    assert!(finalized[0]
        .error
        .as_deref()
        .is_some_and(|message| message.contains("deadline")));
}

fn quick_watchdog() -> crate::chat::tool_run::WatchdogLimits {
    crate::chat::tool_run::WatchdogLimits {
        stall: std::time::Duration::from_millis(60),
        hard: std::time::Duration::from_secs(600),
        stop_grace: std::time::Duration::from_millis(200),
        poll: std::time::Duration::from_millis(10),
    }
}

#[derive(Clone, Copy)]
enum RequiredCompletionStop {
    Stall,
    HardLimit,
    ExplicitCancel,
}

struct ReleaseProjectWriteGate(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);

impl ReleaseProjectWriteGate {
    fn release(&self) {
        let (released, wake) = &*self.0;
        *released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        wake.notify_all();
    }
}

impl Drop for ReleaseProjectWriteGate {
    fn drop(&mut self) {
        self.release();
    }
}

async fn started_project_write_is_drained(stop: RequiredCompletionStop) {
    use crate::agent::tool_session::{build_tool_session, wire_tool_name, ToolSessionRequest};
    use crate::approvals::ApprovalsService;
    use crate::persistence::InMemorySecretStore;
    use crate::projects::ProjectsService;
    use crate::security::account_cleanup::{CurrentAccountScope, ValidatedAccountId};
    use crate::skills::SkillStore;

    let app_root = tempfile::tempdir().expect("app root");
    let project_root = tempfile::tempdir().expect("project root");
    let account = ValidatedAccountId::parse("required-completion-account").expect("account");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native(account.as_str()).expect("bind account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(account.as_str(), [0x54; 32])
        .expect("account key");
    let projects = Arc::new(ProjectsService::new(
        app_root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let mut approvals = ApprovalsService::new(app_root.path().to_path_buf(), scope, secrets);

    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let release_gate = ReleaseProjectWriteGate(Arc::clone(&gate));
    let resolution_started = Arc::new(tokio::sync::Notify::new());
    let hook_gate = Arc::clone(&gate);
    let hook_started = Arc::clone(&resolution_started);
    approvals.resolution_hook = Some(Arc::new(move |_| {
        hook_started.notify_one();
        let (released, wake) = &*hook_gate;
        let mut released = released.lock().expect("write gate");
        while !*released {
            released = wake.wait(released).expect("write gate wait");
        }
    }));
    let approvals = Arc::new(approvals);

    projects
        .create(project_root.path().to_str().expect("project path"), None)
        .await
        .expect("trust project");
    projects
        .bind_active_conversation(&account, &ConversationId::from("conversation-1"))
        .expect("bind project");
    let paths = ["first.txt", "second.txt"];
    for (path, content) in paths.iter().zip(["old-first\n", "old-second\n"]) {
        std::fs::write(project_root.path().join(path), content).expect("seed project file");
    }
    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account.clone(),
        conversation_id: ConversationId::from("conversation-1"),
        run_id: RunId::from("run-1"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(app_root.path(), &account)),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;
    let tool_id = ToolId::from("project.propose_patch");
    let mut h = harness_with_session(
        vec![vec![
            ProviderStreamEvent::ToolCallRequested {
                call_id: "edit-two-files".to_owned(),
                name: wire_tool_name(&tool_id),
                arguments: json!({"operations":[
                    {"kind":"replace","path":paths[0],"edits":[{"old_string":"old-first","new_string":"new-first"}]},
                    {"kind":"replace","path":paths[1],"edits":[{"old_string":"old-second","new_string":"new-second"}]}
                ]})
                .to_string(),
            },
            ProviderStreamEvent::Completed,
        ]],
        std::time::Duration::from_secs(20),
        ToolRisk::ReadOnly,
        Some(session),
    );
    let grace = std::time::Duration::from_millis(60);
    let limits = match stop {
        RequiredCompletionStop::Stall => crate::chat::tool_run::WatchdogLimits {
            stall: std::time::Duration::from_secs(1),
            hard: std::time::Duration::from_secs(10),
            stop_grace: grace,
            poll: std::time::Duration::from_millis(5),
        },
        RequiredCompletionStop::HardLimit => crate::chat::tool_run::WatchdogLimits {
            stall: std::time::Duration::from_secs(10),
            hard: std::time::Duration::from_secs(1),
            stop_grace: grace,
            poll: std::time::Duration::from_millis(5),
        },
        RequiredCompletionStop::ExplicitCancel => crate::chat::tool_run::WatchdogLimits {
            stall: std::time::Duration::from_secs(10),
            hard: std::time::Duration::from_secs(20),
            stop_grace: grace,
            poll: std::time::Duration::from_millis(5),
        },
    };
    h.tool_run.set_watchdog(limits);
    let cancel = h.cancel.clone();
    let events = Arc::clone(&h.events);
    let mut driving = Box::pin(h.tool_run.drive(Vec::new()));

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            () = resolution_started.notified() => {},
            _ = &mut driving => panic!("the run ended before its local write started"),
        }
    })
    .await
    .expect("project write reached its local resolution step");
    for (path, original) in paths.iter().zip(["old-first\n", "old-second\n"]) {
        assert_eq!(
            std::fs::read_to_string(project_root.path().join(path)).expect("read gated file"),
            original,
            "the local write is still held at its gate"
        );
    }

    if matches!(stop, RequiredCompletionStop::ExplicitCancel) {
        cancel.cancel();
    }
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        tokio::select! {
            () = cancel.cancelled() => {},
            _ = &mut driving => panic!("the run ended before the selected stop was observed"),
        }
    })
    .await
    .expect("the selected stop condition cancels the run");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(500), &mut driving)
            .await
            .is_err(),
        "a started project write must outlive stop grace until its real result is known"
    );
    assert!(events.lock().expect("events").iter().all(|event| !matches!(
        event,
        InferenceEvent::Failed { .. } | InferenceEvent::Completed { .. }
    )));

    release_gate.release();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), &mut driving)
        .await
        .expect("the run ends after the write and bounded bookkeeping finish");
    assert!(result.is_err(), "the stopped run must report its stop");
    for (path, expected) in paths.iter().zip(["new-first\n", "new-second\n"]) {
        assert_eq!(
            std::fs::read_to_string(project_root.path().join(path)).expect("read written file"),
            expected,
            "both local operations finish before the run is failed"
        );
    }
    assert_eq!(
        events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| matches!(event, InferenceEvent::Failed { .. }))
            .count(),
        1,
        "the run emits one terminal failure only after the local write ends"
    );
}

#[tokio::test]
async fn watchdog_and_explicit_stop_drain_started_multi_file_project_writes() {
    for stop in [
        RequiredCompletionStop::Stall,
        RequiredCompletionStop::HardLimit,
        RequiredCompletionStop::ExplicitCancel,
    ] {
        started_project_write_is_drained(stop).await;
    }
}

fn failure_events(harness: &Harness) -> Vec<(String, bool)> {
    harness
        .events
        .lock()
        .expect("lock")
        .iter()
        .filter_map(|event| match event {
            InferenceEvent::Failed {
                message, retryable, ..
            } => Some((message.clone(), *retryable)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_run_that_goes_quiet_is_stopped_and_the_person_is_told_to_retry() {
    let mut harness = harness(Vec::new());
    harness
        .adapter
        .wait_for_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness.tool_run.set_watchdog(quick_watchdog());

    let error = harness
        .tool_run
        .drive(Vec::new())
        .await
        .expect_err("a stalled run must end");

    assert!(matches!(
        error,
        crate::agent::engine::RunEngineError::Turn(_)
    ));
    // A run that heard the cancel closes as cancelled; the failure the person
    // sees and the DA write below are what mark it as a stop.
    assert!(matches!(
        harness.tool_run.state(),
        RunState::Cancelled | RunState::Failed
    ));
    let failures = failure_events(&harness);
    assert_eq!(
        failures.len(),
        1,
        "exactly one terminal event: {failures:?}"
    );
    assert!(failures[0].0.contains("retry"), "{failures:?}");
    assert!(failures[0].1, "a stopped run is retryable");
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "error");
}

#[tokio::test]
async fn a_run_blocked_where_it_cannot_hear_the_cancel_is_still_ended() {
    let mut harness = harness(Vec::new());
    harness
        .adapter
        .hang_ignoring_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness.tool_run.set_watchdog(quick_watchdog());

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        harness.tool_run.drive(Vec::new()),
    )
    .await
    .expect("the watchdog must not wait on a run that ignores the cancel")
    .expect_err("a hung run must end");

    assert!(matches!(
        error,
        crate::agent::engine::RunEngineError::Turn(_)
    ));
    assert_eq!(failure_events(&harness).len(), 1);
    assert_eq!(harness.sink.finalized.lock().expect("lock").len(), 1);
}

#[tokio::test]
async fn run_deadline_does_not_drop_an_in_flight_terminal_write() {
    let mut harness = harness_with_wall_clock(
        vec![vec![
            text("Completed answer."),
            ProviderStreamEvent::Completed,
        ]],
        std::time::Duration::from_millis(50),
    );
    let (release, held) = tokio::sync::oneshot::channel();
    *harness.sink.hold_finalize.lock().expect("hold") = Some(held);
    let sink = harness.sink.clone();
    let cancel = harness.cancel.clone();
    let release_write = async move {
        cancel.cancelled().await;
        let _ = release.send(());
    };
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(harness.tool_run.drive(Vec::new()), release_write)
    })
    .await
    .expect("deadline signals cancellation and drains finalization");
    assert!(outcome.expect("started finalization completes").is_some());
    assert_eq!(harness.tool_run.state(), RunState::Completed);
    assert_eq!(
        sink.finalized.lock().expect("finalized").len(),
        1,
        "a deadline must drain the already-started terminal write exactly once"
    );
    assert_eq!(
        sink.finalized.lock().expect("finalized")[0].status,
        "completed"
    );
}

#[tokio::test]
async fn run_deadline_drains_tool_result_persistence_before_finalizing() {
    let mut harness = harness_with_wall_clock(
        vec![vec![call("call_a"), ProviderStreamEvent::Completed]],
        std::time::Duration::from_millis(50),
    );
    harness.tool_run.drive(Vec::new()).await.expect("pause");
    let invocation = harness.tool_run.pending_invocation_ids()[0].clone();
    harness
        .tool_run
        .resolve_approval(&invocation, true)
        .await
        .expect("approve");
    let (release, held) = tokio::sync::oneshot::channel();
    *harness.sink.hold_round.lock().expect("hold") = Some(held);
    let cancel = harness.cancel.clone();
    let release_write = async move {
        cancel.cancelled().await;
        let _ = release.send(());
    };
    let resumed = harness.tool_run.take_resolved_outcomes();
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(harness.tool_run.drive(resumed), release_write)
    })
    .await
    .expect("deadline signals cancellation and drains the tool round");
    assert!(matches!(
        outcome,
        Err(crate::agent::engine::RunEngineError::Run(
            crate::agent::run::RunError::DeadlineExceeded
        ))
    ));
    let messages = harness.sink.messages.lock().expect("messages");
    let results: Vec<_> = messages
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .collect();
    assert_eq!(results.len(), 1, "persist the completed tool result once");
    let result: Value = serde_json::from_str(&results[0].content).expect("tool result");
    assert_eq!(result["status"], "completed");
    assert_eq!(harness.adapter.seen.lock().expect("requests").len(), 1);
    let finalized = harness.sink.finalized.lock().expect("finalized");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "error");
}

#[tokio::test]
async fn run_deadline_cancels_and_drains_an_executing_tool_round() {
    let mut harness = harness_with_risk(
        vec![vec![call("call_a"), ProviderStreamEvent::Completed]],
        std::time::Duration::from_millis(50),
        ToolRisk::ReadOnly,
    );
    harness
        .executor
        .wait_for_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        harness.tool_run.drive(Vec::new()),
    )
    .await
    .expect("deadline cancels the tool");
    assert!(matches!(
        outcome,
        Err(crate::agent::engine::RunEngineError::Run(
            crate::agent::run::RunError::DeadlineExceeded
        ))
    ));
    assert_eq!(harness.tool_run.state(), RunState::Failed);
    let messages = harness.sink.messages.lock().expect("messages");
    let result = messages
        .iter()
        .find(|message| message.role == ContextRole::Tool)
        .expect("persisted tool result");
    let result: Value = serde_json::from_str(&result.content).expect("tool result");
    assert_eq!(result["status"], "cancelled");
    assert_eq!(harness.sink.finalized.lock().expect("finalized").len(), 1);
}

#[tokio::test]
async fn explicit_stop_remains_cancelled_when_persistence_drains_past_the_deadline() {
    let mut harness = harness_with_wall_clock(
        vec![vec![call("call_a"), ProviderStreamEvent::Completed]],
        std::time::Duration::from_millis(100),
    );
    harness.tool_run.drive(Vec::new()).await.expect("pause");
    let invocation = harness.tool_run.pending_invocation_ids()[0].clone();
    harness
        .tool_run
        .resolve_approval(&invocation, true)
        .await
        .expect("approve");
    let (release, held) = tokio::sync::oneshot::channel();
    *harness.sink.hold_round.lock().expect("hold") = Some(held);
    let cancel = harness.cancel.clone();
    let stop = async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cancel.cancel();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        release.send(()).expect("persistence is still owned");
    };
    let resumed = harness.tool_run.take_resolved_outcomes();
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(harness.tool_run.drive(resumed), stop)
    })
    .await
    .expect("Stop drains persistence");
    assert!(matches!(
        outcome,
        Err(crate::agent::engine::RunEngineError::Cancelled)
    ));
    assert_eq!(harness.tool_run.state(), RunState::Cancelled);
    assert!(
        harness.sink.finalized.lock().expect("finalized").is_empty(),
        "explicit Stop still owns the terminal write"
    );
}

#[tokio::test]
async fn an_invalidated_mcp_sign_in_fails_the_run_before_another_model_turn() {
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");
    let pending = harness.tool_run.pending_invocation_ids();
    harness
        .executor
        .authorization_refused
        .store(true, std::sync::atomic::Ordering::SeqCst);

    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("approve");
    let resumed = harness.tool_run.take_resolved_outcomes();
    let outcome = harness
        .tool_run
        .drive(resumed)
        .await
        .expect_err("an invalidated MCP grant must end this run");

    assert!(matches!(
        outcome,
        crate::agent::engine::RunEngineError::ToolLoop(
            crate::agent::tool_loop::ToolLoopError::AuthorizationRefused
        )
    ));
    assert_eq!(harness.tool_run.state(), RunState::Failed);
    assert_eq!(harness.adapter.seen.lock().expect("lock").len(), 1);
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "error");
}

#[tokio::test]
async fn mcp_authorization_failure_preserves_already_completed_tool_results() {
    let mut harness = harness(vec![vec![
        call("call_a"),
        call("call_b"),
        ProviderStreamEvent::Completed,
    ]]);
    harness.tool_run.drive(Vec::new()).await.expect("pause");
    let pending = harness.tool_run.pending_invocation_ids();
    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("first write succeeds");
    harness
        .executor
        .authorization_refused
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness
        .tool_run
        .resolve_approval(&pending[1], true)
        .await
        .expect("second call records authorization failure");
    let resumed = harness.tool_run.take_resolved_outcomes();
    harness
        .tool_run
        .drive(resumed)
        .await
        .expect_err("authorization failure ends the run");

    let messages = harness.sink.messages.lock().expect("messages");
    let tool_results: Vec<Value> = messages
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .map(|message| serde_json::from_str(&message.content).expect("tool result JSON"))
        .collect();
    assert!(
        tool_results.iter().any(|result| result["status"] == "completed"),
        "the next turn must retain the successful write instead of a retryable missing-result error"
    );
}

#[tokio::test]
async fn mcp_authorization_failure_preserves_parallel_round_results() {
    let mut refused = call("call_b");
    if let ProviderStreamEvent::ToolCallRequested { arguments, .. } = &mut refused {
        *arguments = json!({"refuse_authorization": true}).to_string();
    }
    let mut harness = harness_with_risk(
        vec![vec![
            call("call_a"),
            refused,
            ProviderStreamEvent::Completed,
        ]],
        std::time::Duration::from_secs(600),
        ToolRisk::ReadOnly,
    );
    harness
        .tool_run
        .drive(Vec::new())
        .await
        .expect_err("authorization failure");
    let messages = harness.sink.messages.lock().expect("messages");
    let results: Vec<Value> = messages
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .map(|message| serde_json::from_str(&message.content).expect("tool result"))
        .collect();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|result| result["status"] == "completed"));
    assert!(results.iter().any(|result| result["error"]
        .as_str()
        .is_some_and(|error| error.contains("sign-in"))));
    assert_eq!(harness.adapter.seen.lock().expect("requests").len(), 1);
}

#[tokio::test]
async fn mcp_authorization_failure_closes_unresolved_approval_siblings() {
    let mut harness = harness(vec![vec![
        call("call_a"),
        call("call_b"),
        ProviderStreamEvent::Completed,
    ]]);
    harness.tool_run.drive(Vec::new()).await.expect("pause");
    let pending = harness.tool_run.pending_invocation_ids();
    harness
        .executor
        .authorization_refused
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("resolve");
    assert!(harness.tool_run.pending_invocation_ids().is_empty());
    let resumed = harness.tool_run.take_resolved_outcomes();
    harness
        .tool_run
        .drive(resumed)
        .await
        .expect_err("authorization failure");
    assert_eq!(harness.executor.calls.lock().expect("calls").len(), 1);
    let messages = harness.sink.messages.lock().expect("messages");
    let results: Vec<Value> = messages
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .map(|message| serde_json::from_str(&message.content).expect("tool result"))
        .collect();
    assert!(results.iter().any(|result| result["status"] == "denied"));
}

const PLAN_WIRE_NAME: &str = "project_update_plan";

fn plan_call(call_id: &str, items: &[(&str, &str)]) -> ProviderStreamEvent {
    let plan: Vec<Value> = items
        .iter()
        .map(|(content, status)| json!({ "content": content, "status": status }))
        .collect();
    ProviderStreamEvent::ToolCallRequested {
        call_id: call_id.to_owned(),
        name: PLAN_WIRE_NAME.to_owned(),
        arguments: json!({ "plan": plan }).to_string(),
    }
}

fn text(chunk: &str) -> ProviderStreamEvent {
    ProviderStreamEvent::TextDelta(chunk.to_owned())
}

fn call(call_id: &str) -> ProviderStreamEvent {
    ProviderStreamEvent::ToolCallRequested {
        call_id: call_id.to_owned(),
        name: WIRE_NAME.to_owned(),
        arguments: "{}".to_owned(),
    }
}

// ---------------------------------------------------------------------
// Pausing
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_mutating_tool_pauses_the_run_and_executes_nothing() {
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);

    let outcome = harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert!(outcome.is_none(), "a paused run has no outcome yet");
    assert_eq!(harness.tool_run.state(), RunState::AwaitingApproval);
    assert!(
        harness.executor.calls.lock().expect("lock").is_empty(),
        "nothing may execute before a person decides"
    );
    assert!(
        harness.sink.finalized.lock().expect("lock").is_empty(),
        "a paused run must not be finalized on DA -- it is not over"
    );
}

// ---------------------------------------------------------------------
// Resuming
// ---------------------------------------------------------------------

#[tokio::test]
async fn approving_executes_the_call_and_the_run_continues_to_a_final_answer() {
    let mut harness = harness(vec![
        vec![call("call_a"), ProviderStreamEvent::Completed],
        vec![text("Loaded."), ProviderStreamEvent::Completed],
    ]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");
    let pending = harness.tool_run.pending_invocation_ids();
    assert_eq!(pending.len(), 1);

    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("approve");
    let resumed = harness.tool_run.take_resolved_outcomes();
    let outcome = harness.tool_run.drive(resumed).await.expect("resume");

    assert!(outcome.is_some());
    assert_eq!(harness.tool_run.state(), RunState::Completed);
    assert_eq!(
        harness.executor.calls.lock().expect("lock").as_slice(),
        [ToolId::from(MUTATING_TOOL)]
    );
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "completed");
}

#[tokio::test]
async fn the_resumed_run_keeps_the_context_its_earlier_rounds_built() {
    // Rebuilding the turn driver on resume would lose the assistant turn that
    // requested the tool, and the follow-up request would carry a tool result
    // answering a call the model never sees itself making.
    let mut harness = harness(vec![
        vec![
            text("Working. "),
            call("call_a"),
            ProviderStreamEvent::Completed,
        ],
        vec![text("Loaded."), ProviderStreamEvent::Completed],
    ]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");
    let pending = harness.tool_run.pending_invocation_ids();
    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("approve");
    let resumed = harness.tool_run.take_resolved_outcomes();
    harness.tool_run.drive(resumed).await.expect("resume");

    let second = harness.adapter.seen.lock().expect("lock")[1].clone();
    let assistant = second
        .messages
        .iter()
        .find(|message| !message.tool_calls.is_empty())
        .expect("the assistant turn that requested the tool survived the pause");
    assert_eq!(assistant.content, "Working. ");
    let answered = second
        .messages
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .count();
    assert_eq!(answered, 1, "the approved call is answered exactly once");
}

#[tokio::test]
async fn declining_never_executes_the_call_but_still_lets_the_run_finish() {
    let mut harness = harness(vec![
        vec![call("call_a"), ProviderStreamEvent::Completed],
        vec![text("Understood."), ProviderStreamEvent::Completed],
    ]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");
    let pending = harness.tool_run.pending_invocation_ids();

    harness
        .tool_run
        .resolve_approval(&pending[0], false)
        .await
        .expect("decline");
    let resumed = harness.tool_run.take_resolved_outcomes();
    harness.tool_run.drive(resumed).await.expect("resume");

    assert!(
        harness.executor.calls.lock().expect("lock").is_empty(),
        "a declined call must never reach the executor"
    );
    assert_eq!(harness.tool_run.state(), RunState::Completed);
}

#[tokio::test]
async fn resolving_an_invocation_the_run_is_not_paused_on_is_rejected() {
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");

    let result = harness
        .tool_run
        .resolve_approval(&InvocationId::from("not-a-real-invocation"), true)
        .await;

    assert!(result.is_err());
    assert_eq!(
        harness.tool_run.pending_invocation_ids().len(),
        1,
        "the real pending invocation is untouched"
    );
}

// ---------------------------------------------------------------------
// Abandoning
// ---------------------------------------------------------------------

#[tokio::test]
async fn stopping_a_partly_approved_round_preserves_completed_and_denied_results() {
    let mut harness = harness(vec![vec![
        call("call_a"),
        call("call_b"),
        ProviderStreamEvent::Completed,
    ]]);
    harness.tool_run.drive(Vec::new()).await.expect("pause");
    let approved = harness.tool_run.pending_invocation_ids()[0].clone();
    harness
        .tool_run
        .resolve_approval(&approved, true)
        .await
        .expect("write succeeds");
    assert_eq!(harness.tool_run.pending_invocation_ids().len(), 1);

    // ChatService reparks this owner until the sibling gets a decision. Stop
    // takes that same owner and calls abandon without taking its outcomes.
    harness.cancel.cancel();
    harness
        .tool_run
        .abandon("run cancelled while awaiting approval")
        .await;
    let results: Vec<Value> = harness
        .sink
        .messages
        .lock()
        .expect("messages")
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .map(|message| serde_json::from_str(&message.content).expect("tool result"))
        .collect();
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().any(|result| result["status"] == "completed"),
        "the completed write must survive Stop"
    );
    let completed = results
        .iter()
        .find(|result| result["status"] == "completed")
        .expect("completed write");
    assert_eq!(completed["result"], json!({"rows": 1}));
    assert!(results.iter().all(|result| result["retryable"] != true));
    assert!(
        results.iter().any(|result| result["status"] == "denied"),
        "the unexecuted sibling must be recorded as declined"
    );
    assert_eq!(harness.executor.calls.lock().expect("calls").len(), 1);
    assert_eq!(harness.sink.finalized.lock().expect("finalized").len(), 1);
    let persisted_count = harness.sink.messages.lock().expect("messages").len();
    harness.tool_run.abandon("run cancelled").await;
    assert_eq!(
        harness.sink.messages.lock().expect("messages").len(),
        persisted_count
    );
    assert_eq!(harness.sink.finalized.lock().expect("finalized").len(), 1);
}

#[tokio::test]
async fn stopping_after_tool_completion_persists_its_result_without_a_terminal_write() {
    let mut harness = harness_with_risk(
        vec![vec![call("call_a"), ProviderStreamEvent::Completed]],
        std::time::Duration::from_secs(600),
        ToolRisk::ReadOnly,
    );
    harness
        .executor
        .cancel_after_success
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = harness.tool_run.drive(Vec::new()).await;
    assert!(matches!(
        result,
        Err(crate::agent::engine::RunEngineError::Cancelled)
    ));
    let messages = harness.sink.messages.lock().expect("messages");
    let results: Vec<Value> = messages
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .map(|message| serde_json::from_str(&message.content).expect("tool result"))
        .collect();
    assert_eq!(
        results.len(),
        1,
        "completed round must be durable before the owner returns"
    );
    assert_eq!(results[0]["status"], "completed");
    assert_eq!(results[0]["result"], json!({"rows": 1}));
    assert_ne!(results[0]["retryable"], true);
    assert!(
        harness.sink.finalized.lock().expect("finalized").is_empty(),
        "ChatService Stop retains terminal-write ownership"
    );
    assert_eq!(harness.adapter.seen.lock().expect("requests").len(), 1);
}

#[tokio::test]
async fn stopping_after_the_last_approval_preserves_the_resumed_result() {
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("pause");
    let pending = harness.tool_run.pending_invocation_ids();
    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("approved write");
    let resumed = harness.tool_run.take_resolved_outcomes();
    harness.cancel.cancel();
    assert!(matches!(
        harness.tool_run.drive(resumed).await,
        Err(crate::agent::engine::RunEngineError::Cancelled)
    ));
    let messages = harness.sink.messages.lock().expect("messages");
    let tool = messages
        .iter()
        .find(|message| message.role == ContextRole::Tool)
        .expect("durable tool result");
    let result: Value = serde_json::from_str(&tool.content).expect("tool result");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["result"], json!({"rows": 1}));
    assert_ne!(result["retryable"], true);
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == ContextRole::Tool)
            .count(),
        1
    );
    assert!(harness.sink.finalized.lock().expect("finalized").is_empty());
    assert_eq!(harness.adapter.seen.lock().expect("requests").len(), 1);
}

#[tokio::test]
async fn stopping_reports_result_persistence_failure_without_a_terminal_write() {
    let mut harness = harness_with_risk(
        vec![vec![call("call_a"), ProviderStreamEvent::Completed]],
        std::time::Duration::from_secs(600),
        ToolRisk::ReadOnly,
    );
    harness
        .executor
        .cancel_after_success
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness
        .sink
        .fail_round
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        harness.tool_run.drive(Vec::new()).await,
        Err(crate::agent::engine::RunEngineError::Turn(message))
            if message.contains("tool results could not be persisted")
    ));
    assert!(harness.events.lock().expect("events").iter().any(|event| {
        matches!(event, InferenceEvent::Failed { retryable: false, message, .. }
            if message.contains("tool results could not be persisted"))
    }));
    assert_eq!(
        harness
            .events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| matches!(event, InferenceEvent::Failed { .. }))
            .count(),
        1
    );
    assert!(harness.sink.finalized.lock().expect("finalized").is_empty());
}

#[tokio::test]
async fn abandoning_a_paused_run_declines_every_pending_call_and_finalizes_it() {
    // Cancelling a parked run only cancels its token; nothing is driving it,
    // so without this its invocations would stay `AwaitingApproval` forever
    // and its DA run would never reach a terminal state.
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");

    harness.tool_run.abandon("run cancelled").await;

    assert!(harness.tool_run.pending_invocation_ids().is_empty());
    assert!(
        harness.executor.calls.lock().expect("lock").is_empty(),
        "abandoning must never execute the call it was waiting on"
    );
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "error");
}

#[tokio::test]
async fn resolved_outcomes_are_taken_once_and_not_replayed() {
    // A second `drive` seeded with the same outcomes would answer the same
    // call twice in the model's context.
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");
    let pending = harness.tool_run.pending_invocation_ids();
    harness
        .tool_run
        .resolve_approval(&pending[0], true)
        .await
        .expect("approve");

    assert_eq!(harness.tool_run.take_resolved_outcomes().len(), 1);
    assert!(harness.tool_run.take_resolved_outcomes().is_empty());
}

// ---------------------------------------------------------------------
// Restart while awaiting approval
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_run_paused_for_approval_is_recovered_as_needing_the_user() {
    // Slice-4 exit gate item 6. A parked run lives in memory, so quitting the
    // app loses the pause. What must survive is the *safety* of it: the run's
    // journaled state says `AwaitingApproval`, recovery routes that to
    // `AwaitUser` rather than auto-resuming, and the invocation it was waiting
    // on never executed. An auto-resume here would run a tool nobody
    // approved.
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");

    let state = harness.tool_run.state();
    assert_eq!(state, RunState::AwaitingApproval);
    assert_eq!(
        crate::agent::recovery::policy_for_interrupted(state.clone()),
        crate::agent::recovery::RecoveryDecision::AwaitUser,
    );
    assert!(
        crate::agent::recovery::is_in_flight_run_state(state),
        "the run journal must be retained across restart, or recovery has \
         nothing to decide from"
    );
    assert!(harness.executor.calls.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn timed_out_abandon_wait_does_not_cancel_pending_finalization() {
    let mut harness = harness(vec![vec![call("call_a"), ProviderStreamEvent::Completed]]);
    harness.tool_run.drive(Vec::new()).await.expect("drive");
    let (release, held) = tokio::sync::oneshot::channel();
    *harness.sink.hold_finalize.lock().expect("hold") = Some(held);
    let mut teardown = harness
        .tool_run
        .abandon_in_background("conversation deleted");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut teardown)
            .await
            .is_err()
    );
    assert!(harness.sink.finalized.lock().expect("finalized").is_empty());
    release.send(()).expect("release finalization");
    teardown
        .await
        .expect("retry wait completes original teardown");
    assert_eq!(harness.sink.finalized.lock().expect("finalized").len(), 1);
    assert!(harness.executor.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn cancelling_an_approved_slow_tool_leaves_sibling_for_safe_teardown() {
    let mut harness = harness(vec![vec![
        call("call_a"),
        call("call_b"),
        ProviderStreamEvent::Completed,
    ]]);
    harness
        .tool_run
        .drive(Vec::new())
        .await
        .expect("pause for approvals");
    let invocation = harness.tool_run.pending_invocation_ids()[0].clone();
    harness
        .executor
        .wait_for_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let stop = async {
        harness.executor.started.notified().await;
        harness.cancel.cancel();
    };
    let (resolved, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(harness.tool_run.resolve_approval(&invocation, true), stop)
    })
    .await
    .expect("approved tool observes cancellation");
    resolved.expect("cancelled tool outcome recorded");
    assert_eq!(harness.tool_run.pending_invocation_ids().len(), 1);
    harness
        .tool_run
        .abandon_in_background("run cancelled")
        .await
        .expect("finish sibling teardown");
    assert_eq!(harness.executor.calls.lock().expect("calls").len(), 1);
    assert_eq!(harness.sink.finalized.lock().expect("finalized").len(), 1);
}

// ---------------------------------------------------------------------
// Output-limit recovery
// ---------------------------------------------------------------------

fn stopped(reason: ProviderStopReason, discarded_tool_calls: u32) -> ProviderStreamEvent {
    ProviderStreamEvent::Stopped {
        reason,
        discarded_tool_calls,
    }
}

fn run_status_of(request: &ProviderChatRequest) -> String {
    request
        .messages
        .last()
        .map(|message| message.content.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_tool_call_cut_off_by_the_output_limit_is_retried_with_a_notice() {
    let mut harness = harness(vec![
        vec![
            text("Writing the pages. "),
            stopped(ProviderStopReason::OutputLimit, 1),
            ProviderStreamEvent::Completed,
        ],
        vec![text("All pages written."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    let seen = harness.adapter.seen.lock().expect("lock");
    assert_eq!(seen.len(), 2, "the cut-off turn is answered with a retry");
    let retry = &seen[1];
    let status = run_status_of(retry);
    assert!(status.contains("output limit"), "{status}");
    assert!(status.contains("one file per call"), "{status}");
    assert!(
        retry.messages.iter().any(|message| {
            message.role == ContextRole::Assistant && message.content == "Writing the pages. "
        }),
        "the prose the model already wrote stays in its own context"
    );
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].status, "completed");
    assert_eq!(
        finalized[0].content.as_deref(),
        Some("Writing the pages. All pages written."),
        "the durable answer keeps the prose from the cut-off turn"
    );
}

#[tokio::test]
async fn a_prose_answer_cut_off_by_the_output_limit_is_continued() {
    let mut harness = harness(vec![
        vec![
            text("First half"),
            stopped(ProviderStopReason::OutputLimit, 0),
            ProviderStreamEvent::Completed,
        ],
        vec![text(" and the rest."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    let seen = harness.adapter.seen.lock().expect("lock");
    assert!(run_status_of(&seen[1]).contains("Continue exactly where you stopped"));
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(
        finalized[0].content.as_deref(),
        Some("First half and the rest.")
    );
}

#[tokio::test]
async fn a_run_that_keeps_hitting_the_output_limit_inside_a_tool_call_fails_with_advice() {
    let cut = || {
        vec![
            stopped(ProviderStopReason::OutputLimit, 1),
            ProviderStreamEvent::Completed,
        ]
    };
    let mut harness = harness(vec![cut(), cut(), cut(), cut()]);

    let _ = harness.tool_run.drive(Vec::new()).await;

    assert_eq!(harness.tool_run.state(), RunState::Failed);
    assert_eq!(harness.adapter.seen.lock().expect("lock").len(), 4);
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "error");
    assert!(finalized[0].error.as_deref().is_some_and(
        |message| message.contains("output limit") && message.contains("fewer files at a time")
    ));
}

#[tokio::test]
async fn an_answer_still_cut_off_after_every_continuation_is_marked_as_cut_off() {
    let cut = |chunk: &str| {
        vec![
            text(chunk),
            stopped(ProviderStopReason::OutputLimit, 0),
            ProviderStreamEvent::Completed,
        ]
    };
    let mut harness = harness(vec![cut("a"), cut("b"), cut("c"), cut("d")]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].status, "completed");
    let content = finalized[0].content.as_deref().expect("content");
    assert!(content.starts_with("abcd"), "{content}");
    assert!(content.contains("cut off at the model's output limit"));
}

#[tokio::test]
async fn a_response_cut_off_by_the_connection_is_continued_not_reported_as_an_error() {
    // The stream closed mid-answer with no stop reason; the adapter reports
    // that as a cut-off turn, and the run asks for the rest like it does for an
    // output limit, in words that do not blame a limit.
    let mut harness = harness(vec![
        vec![
            text("First half"),
            stopped(ProviderStopReason::StreamCut, 0),
            ProviderStreamEvent::Completed,
        ],
        vec![text(" and the rest."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    let seen = harness.adapter.seen.lock().expect("lock");
    let status = run_status_of(&seen[1]);
    assert!(
        status.contains("interrupted before it finished"),
        "{status}"
    );
    assert!(!status.contains("output limit"), "{status}");
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].status, "completed");
    assert_eq!(
        finalized[0].content.as_deref(),
        Some("First half and the rest.")
    );
}

#[tokio::test]
async fn a_tool_call_cut_off_by_the_connection_is_sent_again_in_full() {
    let mut harness = harness(vec![
        vec![
            text("Writing the page. "),
            stopped(ProviderStopReason::StreamCut, 1),
            ProviderStreamEvent::Completed,
        ],
        vec![text("Page written."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    let seen = harness.adapter.seen.lock().expect("lock");
    let status = run_status_of(&seen[1]);
    assert!(
        status.contains("interrupted before it finished"),
        "{status}"
    );
    assert!(status.contains("send it again in full"), "{status}");
    assert!(
        !status.contains("smaller") && !status.contains("one file per call"),
        "a dropped connection is not a reason to shorten the work: {status}"
    );
}

#[tokio::test]
async fn an_answer_the_connection_keeps_cutting_ends_marked_as_interrupted() {
    let cut = |chunk: &str| {
        vec![
            text(chunk),
            stopped(ProviderStopReason::StreamCut, 0),
            ProviderStreamEvent::Completed,
        ]
    };
    // Eight silent continuations, then the three an output limit gets, then
    // the marker.
    let pieces = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];
    let mut harness = harness(pieces.iter().map(|piece| cut(piece)).collect());

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    let finalized = harness.sink.finalized.lock().expect("lock");
    let content = finalized[0].content.as_deref().expect("content");
    assert!(content.starts_with("abcdefghijkl"), "{content}");
    assert!(content.contains("interrupted before it finished"));
    assert!(!content.contains("output limit"));
}

#[tokio::test]
async fn a_connection_that_breaks_before_any_answer_is_asked_again_without_a_notice() {
    // Only thinking arrived. Nothing is kept and the model is told nothing: the
    // same request goes out again and the reader sees one clean answer.
    let mut harness = harness(vec![
        vec![
            ProviderStreamEvent::ReasoningDelta("Let me think".to_owned()),
            stopped(ProviderStopReason::StreamCut, 0),
            ProviderStreamEvent::Completed,
        ],
        vec![
            stopped(ProviderStopReason::StreamCut, 0),
            ProviderStreamEvent::Completed,
        ],
        vec![text("The answer."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    let seen = harness.adapter.seen.lock().expect("lock");
    assert_eq!(seen.len(), 3);
    assert_eq!(run_status_of(&seen[1]), run_status_of(&seen[0]));
    assert_eq!(run_status_of(&seen[2]), run_status_of(&seen[0]));
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].status, "completed");
    assert_eq!(finalized[0].content.as_deref(), Some("The answer."));
}

#[tokio::test]
async fn a_turn_that_spends_its_whole_budget_thinking_is_asked_again_without_thinking() {
    let mut harness = harness(vec![
        vec![
            ProviderStreamEvent::ReasoningDelta("Hmm, let me think".to_owned()),
            stopped(ProviderStopReason::OutputLimit, 0),
            ProviderStreamEvent::Completed,
        ],
        vec![text("Hello there."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    let seen = harness.adapter.seen.lock().expect("lock");
    assert!(seen[0].allow_reasoning, "a new message is thought about");
    assert!(
        !seen[1].allow_reasoning,
        "the retry after a budget spent on thinking does not think"
    );
    assert!(
        run_status_of(&seen[1]).contains("on thinking and wrote no answer"),
        "{}",
        run_status_of(&seen[1])
    );
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].content.as_deref(), Some("Hello there."));
}

#[tokio::test]
async fn a_refusal_fails_the_run_with_a_clear_message() {
    let mut harness = harness(vec![vec![
        stopped(ProviderStopReason::Refusal, 0),
        ProviderStreamEvent::Completed,
    ]]);

    let _ = harness.tool_run.drive(Vec::new()).await;

    assert_eq!(harness.tool_run.state(), RunState::Failed);
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert!(finalized[0]
        .error
        .as_deref()
        .is_some_and(|message| message.contains("declined")));
}

// ---------------------------------------------------------------------
// Finishing the job
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_reply_that_ends_with_plan_items_open_is_sent_back_to_work() {
    let mut harness = harness(vec![
        vec![
            plan_call(
                "plan_1",
                &[("write a.html", "completed"), ("write b.html", "pending")],
            ),
            ProviderStreamEvent::Completed,
        ],
        // Ends its reply with b.html still open.
        vec![text("Done with a."), ProviderStreamEvent::Completed],
        // Sent back to work, it finishes b and closes the plan.
        vec![
            plan_call(
                "plan_2",
                &[("write a.html", "completed"), ("write b.html", "completed")],
            ),
            ProviderStreamEvent::Completed,
        ],
        vec![
            text("Both pages are written."),
            ProviderStreamEvent::Completed,
        ],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    let seen = harness.adapter.seen.lock().expect("lock");
    assert_eq!(
        seen.len(),
        4,
        "the early stop is answered with another turn"
    );
    let status = run_status_of(&seen[2]);
    assert!(status.contains("write b.html"), "{status}");
    assert!(
        !status.contains("write a.html"),
        "finished items are not repeated: {status}"
    );
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].status, "completed");
    assert_eq!(
        finalized[0].content.as_deref(),
        Some("Both pages are written.")
    );
}

#[tokio::test]
async fn a_reply_with_every_plan_item_done_is_left_alone() {
    let mut harness = harness(vec![
        vec![
            plan_call("plan_1", &[("write a.html", "completed")]),
            ProviderStreamEvent::Completed,
        ],
        vec![text("All done."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.adapter.seen.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn a_model_that_keeps_stopping_early_is_nudged_a_bounded_number_of_times() {
    let mut turns = vec![vec![
        plan_call("plan_1", &[("write b.html", "pending")]),
        ProviderStreamEvent::Completed,
    ]];
    for _ in 0..5 {
        turns.push(vec![text("Stopping."), ProviderStreamEvent::Completed]);
    }
    let mut harness = harness(turns);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    // One plan turn, then the reply and three nudged retries.
    assert_eq!(harness.adapter.seen.lock().expect("lock").len(), 5);
    assert_eq!(harness.tool_run.state(), RunState::Completed);
}

// ---------------------------------------------------------------------
// Audit fixes: reasoning-only turns and separators
// ---------------------------------------------------------------------

fn thinking_block() -> ProviderStreamEvent {
    ProviderStreamEvent::ReasoningDetails(vec![
        serde_json::json!({"type": "thinking", "thinking": "weighing it", "signature": "sig"}),
    ])
}

#[tokio::test]
async fn a_turn_that_spends_its_whole_limit_thinking_is_retried_not_failed() {
    let mut harness = harness(vec![
        vec![
            thinking_block(),
            stopped(ProviderStopReason::OutputLimit, 0),
            ProviderStreamEvent::Completed,
        ],
        vec![text("Here is the answer."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    assert_eq!(harness.adapter.seen.lock().expect("lock").len(), 2);
    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(finalized[0].status, "completed");
    assert_eq!(finalized[0].content.as_deref(), Some("Here is the answer."));
}

#[tokio::test]
async fn a_thinking_only_reply_with_plan_items_open_is_sent_back_to_work() {
    let mut harness = harness(vec![
        vec![
            plan_call("plan_1", &[("write a.html", "pending")]),
            ProviderStreamEvent::Completed,
        ],
        vec![thinking_block(), ProviderStreamEvent::Completed],
        vec![
            plan_call("plan_2", &[("write a.html", "completed")]),
            ProviderStreamEvent::Completed,
        ],
        vec![text("Written."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    assert_eq!(harness.tool_run.state(), RunState::Completed);
    assert_eq!(harness.adapter.seen.lock().expect("lock").len(), 4);
}

#[tokio::test]
async fn prose_written_before_a_nudge_is_separated_from_what_follows() {
    let mut harness = harness(vec![
        vec![
            plan_call("plan_1", &[("write a.html", "pending")]),
            ProviderStreamEvent::Completed,
        ],
        vec![text("Started."), ProviderStreamEvent::Completed],
        vec![text("Still going."), ProviderStreamEvent::Completed],
        vec![text("More."), ProviderStreamEvent::Completed],
        vec![text("Finished."), ProviderStreamEvent::Completed],
    ]);

    harness.tool_run.drive(Vec::new()).await.expect("drive");

    let finalized = harness.sink.finalized.lock().expect("lock");
    assert_eq!(
        finalized[0].content.as_deref(),
        Some("Started.\n\nStill going.\n\nMore.\n\nFinished.")
    );
}

// Real ToolRun/LocalTurnDriver regression for progress that used to bypass the watchdog.
fn progress_watchdog() -> crate::chat::tool_run::WatchdogLimits {
    crate::chat::tool_run::WatchdogLimits {
        stall: std::time::Duration::from_millis(600),
        hard: std::time::Duration::from_secs(20),
        stop_grace: std::time::Duration::from_millis(200),
        poll: std::time::Duration::from_millis(5),
    }
}

fn project_work_turns(dashboards: usize) -> Vec<Vec<ProviderStreamEvent>> {
    let mut turns = Vec::new();
    for dashboard in 0..dashboards {
        for stage in 0..2 {
            turns.push(vec![ProviderStreamEvent::ToolCallRequested {
                call_id: format!("dashboard-{dashboard}-stage-{stage}"),
                name: WIRE_NAME.to_owned(),
                arguments: json!({"path": format!("dashboard-{dashboard}.html"), "probe_delay_ms": 200}).to_string(),
            }, ProviderStreamEvent::Completed]);
        }
    }
    turns.push(vec![
        text("All requested dashboards updated."),
        ProviderStreamEvent::Completed,
    ]);
    turns
}

#[tokio::test]
async fn a_single_short_tool_workflow_completes() {
    let mut h = harness_with_risk(
        project_work_turns(1),
        std::time::Duration::from_secs(20),
        ToolRisk::ReadOnly,
    );
    h.tool_run.set_watchdog(progress_watchdog());
    let result = h.tool_run.drive(Vec::new()).await;
    eprintln!(
        "single dashboard: tools={}, failures={:?}, state={:?}",
        h.executor.calls.lock().unwrap().len(),
        failure_events(&h),
        h.tool_run.state()
    );
    assert!(result.expect("single dashboard should complete").is_some());
    assert_eq!(h.tool_run.state(), RunState::Completed);
}

#[tokio::test]
async fn a_long_tool_workflow_keeps_the_watchdog_alive() {
    let mut h = harness_with_risk(
        project_work_turns(2),
        std::time::Duration::from_secs(20),
        ToolRisk::ReadOnly,
    );
    h.tool_run.set_watchdog(progress_watchdog());
    let result = h.tool_run.drive(Vec::new()).await;
    eprintln!(
        "two dashboards: tools={}, failures={:?}, state={:?}",
        h.executor.calls.lock().unwrap().len(),
        failure_events(&h),
        h.tool_run.state()
    );
    assert!(result
        .expect("healthy progress across two dashboards should complete")
        .is_some());
    assert_eq!(h.tool_run.state(), RunState::Completed);
}

#[tokio::test]
async fn streamed_answer_progress_keeps_the_watchdog_alive() {
    let mut events = (0..10).map(|_| text("progress ")).collect::<Vec<_>>();
    events.push(ProviderStreamEvent::Completed);
    let mut h = harness(vec![events]);
    h.adapter
        .fragment_delay_ms
        .store(100, std::sync::atomic::Ordering::SeqCst);
    h.tool_run.set_watchdog(progress_watchdog());
    let result = h.tool_run.drive(Vec::new()).await;
    let streamed = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, InferenceEvent::TextDelta { .. }))
        .count();
    eprintln!(
        "regular streamed progress: text_events={streamed}, failures={:?}, state={:?}",
        failure_events(&h),
        h.tool_run.state()
    );
    assert!(
        streamed >= 3,
        "prove response bytes reached the real callback before the watchdog stop"
    );
    assert!(result
        .expect("healthy streamed progress should complete")
        .is_some());
    assert_eq!(h.tool_run.state(), RunState::Completed);
}

#[tokio::test]
async fn durable_tool_round_writes_keep_the_watchdog_alive() {
    let mut h = harness_with_risk(
        vec![
            vec![call("read-a"), ProviderStreamEvent::Completed],
            vec![text("Done."), ProviderStreamEvent::Completed],
        ],
        std::time::Duration::from_secs(2),
        ToolRisk::ReadOnly,
    );
    h.sink
        .round_progress_delay_ms
        .store(60, std::sync::atomic::Ordering::SeqCst);
    h.sink
        .round_progress_steps
        .store(4, std::sync::atomic::Ordering::SeqCst);
    h.tool_run
        .set_watchdog(crate::chat::tool_run::WatchdogLimits {
            stall: std::time::Duration::from_millis(100),
            hard: std::time::Duration::from_secs(2),
            stop_grace: std::time::Duration::from_millis(100),
            poll: std::time::Duration::from_millis(5),
        });

    assert!(h
        .tool_run
        .drive(Vec::new())
        .await
        .expect("confirmed durable writes keep the run active")
        .is_some());
    assert_eq!(h.tool_run.state(), RunState::Completed);
    assert!(failure_events(&h).is_empty());
}

#[tokio::test]
async fn oversized_streamed_text_fails_before_it_reaches_the_reader_or_persistence() {
    let oversized = "x".repeat(crate::chat::domain::MAX_MESSAGE_TEXT_BYTES + 1);
    let mut h = harness(vec![vec![text(&oversized), ProviderStreamEvent::Completed]]);

    let outcome = h.tool_run.drive(Vec::new()).await;

    assert!(matches!(
        outcome,
        Err(crate::agent::engine::RunEngineError::Turn(ref message))
            if message == "assistant output byte budget exceeded"
    ));
    assert_eq!(h.tool_run.state(), RunState::Failed);
    assert!(h
        .events
        .lock()
        .expect("events")
        .iter()
        .all(|event| !matches!(event, InferenceEvent::TextDelta { .. })));
    assert_eq!(h.sink.finalized.lock().expect("finalized").len(), 1);
    assert_eq!(
        h.sink.finalized.lock().expect("finalized")[0]
            .error
            .as_deref(),
        Some("assistant output byte budget exceeded")
    );
}

#[tokio::test]
async fn a_project_run_can_continue_past_sixty_four_tool_rounds() {
    let mut turns = (0..80)
        .map(|index| {
            vec![
                call(&format!("read-{index}")),
                ProviderStreamEvent::Completed,
            ]
        })
        .collect::<Vec<_>>();
    turns.push(vec![
        text("All files checked."),
        ProviderStreamEvent::Completed,
    ]);
    let mut h = harness_with_risk(
        turns,
        std::time::Duration::from_secs(20),
        ToolRisk::ReadOnly,
    );
    assert!(h
        .tool_run
        .drive(Vec::new())
        .await
        .expect("long run")
        .is_some());
    assert_eq!(h.executor.calls.lock().expect("calls").len(), 80);
    assert_eq!(h.tool_run.state(), RunState::Completed);
    assert!(failure_events(&h).is_empty());
}

#[tokio::test]
async fn two_real_dashboards_are_fully_paged_edited_and_read_back_in_one_run() {
    use crate::agent::tool_session::{build_tool_session, ToolSessionRequest};
    use crate::approvals::ApprovalsService;
    use crate::persistence::InMemorySecretStore;
    use crate::projects::ProjectsService;
    use crate::security::account_cleanup::{CurrentAccountScope, ValidatedAccountId};
    use crate::skills::SkillStore;

    let app_root = tempfile::tempdir().expect("app root");
    let project_root = tempfile::tempdir().expect("project root");
    let account = ValidatedAccountId::parse("multi-file-account").expect("account");
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native(account.as_str()).expect("bind account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(account.as_str(), [0x33; 32])
        .expect("account key");
    let projects = Arc::new(ProjectsService::new(
        app_root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        app_root.path().to_path_buf(),
        scope,
        secrets,
    ));
    projects
        .create(project_root.path().to_str().expect("project path"), None)
        .await
        .expect("trust project");
    projects
        .bind_active_conversation(&account, &ConversationId::from("conversation-1"))
        .expect("bind project");
    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account.clone(),
        conversation_id: ConversationId::from("conversation-1"),
        run_id: RunId::from("run-1"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(app_root.path(), &account)),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;
    let paths = ["06-marketing-attribution.html", "07-brand-performance.html"];
    let originals = paths
        .iter()
        .map(|path| {
            let mut content = format!("<html data-refresh=\"off\"><title>{path}</title>\n");
            for metric in 1..=100 {
                content.push_str(&format!(
                    "<div>{path}-metric-{metric} {}</div>\n",
                    "x".repeat(512)
                ));
            }
            content.push_str(&format!("<!-- {path}-complete-tail -->\n</html>\n"));
            std::fs::write(project_root.path().join(path), &content).expect("seed dashboard");
            content
        })
        .collect::<Vec<_>>();
    let wire_call = |id: String, tool: &str, args: Value| ProviderStreamEvent::ToolCallRequested {
        call_id: id,
        name: crate::agent::tool_session::wire_tool_name(&ToolId::from(tool)),
        arguments: args.to_string(),
    };
    let mut turns = Vec::new();
    for (page, (start, end)) in [(1, 25), (26, 50), (51, 75), (76, 125)]
        .into_iter()
        .enumerate()
    {
        let mut turn = if page == 0 {
            vec![ProviderStreamEvent::Progress; 8]
        } else {
            Vec::new()
        };
        for path in paths {
            turn.push(wire_call(
                format!("read-{page}-{path}"),
                "project.read_file",
                json!({"path":path,"start_line":start,"end_line":end}),
            ));
        }
        turn.push(ProviderStreamEvent::Completed);
        turns.push(turn);
    }
    turns.push(paths.iter().map(|path| wire_call(format!("edit-{path}"), "project.propose_patch", json!({"operations":[{
        "kind":"replace", "path":path, "edits":[{"old_string":"data-refresh=\"off\"", "new_string":"data-refresh=\"on\""}]
    }]}))).chain([ProviderStreamEvent::Completed]).collect());
    turns.push(
        paths
            .iter()
            .map(|path| {
                wire_call(
                    format!("verify-{path}"),
                    "project.read_file",
                    json!({"path":path,"start_line":1,"end_line":1}),
                )
            })
            .chain([ProviderStreamEvent::Completed])
            .collect(),
    );
    turns.push(vec![
        text("Both dashboards updated."),
        ProviderStreamEvent::Completed,
    ]);
    let mut h = harness_with_session(
        turns,
        std::time::Duration::from_secs(20),
        ToolRisk::ReadOnly,
        Some(session),
    );
    h.adapter
        .fragment_delay_ms
        .store(100, std::sync::atomic::Ordering::SeqCst);
    assert!(h
        .tool_run
        .drive(Vec::new())
        .await
        .expect("both dashboards finish")
        .is_some());
    assert_eq!(h.tool_run.state(), RunState::Completed);
    assert!(failure_events(&h).is_empty());
    let result_guard = h.sink.messages.lock().expect("recorded results");
    let failed = result_guard
        .iter()
        .filter(|message| message.role == ContextRole::Tool)
        .map(|message| serde_json::from_str::<Value>(&message.content).expect("result"))
        .filter(|result| result["status"] != "completed")
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "real project tools failed: {failed:?}");
    drop(result_guard);
    for (path, original) in paths.iter().zip(&originals) {
        assert_eq!(
            std::fs::read_to_string(project_root.path().join(path)).expect("read edited dashboard"),
            original.replace("data-refresh=\"off\"", "data-refresh=\"on\"")
        );
    }
    let seen = h.adapter.seen.lock().expect("requests");
    let before_edit = &seen[4].messages;
    for (path, original) in paths.iter().zip(&originals) {
        let pages = before_edit
            .iter()
            .filter(|message| message.role == ContextRole::Tool)
            .map(|message| serde_json::from_str::<Value>(&message.content).expect("read result"))
            .filter(|result| result["result"]["path"] == *path)
            .collect::<Vec<_>>();
        assert_eq!(pages.len(), 4, "all four pages of {path} reach the model");
        let complete = pages
            .iter()
            .map(|page| page["result"]["content"].as_str().expect("page content"))
            .collect::<String>();
        assert_eq!(
            &complete, original,
            "every byte of {path} reaches the model"
        );
    }
    let results = h.sink.messages.lock().expect("messages");
    let tool_results = results
        .iter()
        .filter(|m| m.role == ContextRole::Tool)
        .map(|m| serde_json::from_str::<Value>(&m.content).expect("result envelope"))
        .collect::<Vec<_>>();
    assert_eq!(tool_results.len(), 12);
    assert!(
        tool_results
            .iter()
            .all(|result| result["status"] == "completed"),
        "every real read and write succeeded: {tool_results:?}"
    );
    for (path, original) in paths.iter().zip(&originals) {
        let readback = tool_results[10..]
            .iter()
            .find(|result| result["result"]["path"] == *path)
            .expect("both updated files were read back");
        let expected = format!(
            "{}\n",
            original
                .lines()
                .next()
                .expect("first line")
                .replace("data-refresh=\"off\"", "data-refresh=\"on\"")
        );
        assert_eq!(readback["result"]["content"], expected);
    }
}

#[tokio::test]
async fn a_slow_bounded_context_summary_keeps_the_watchdog_alive_without_leaking_text() {
    let mut summary = (0..10)
        .map(|_| text("Earlier project edits were verified. "))
        .collect::<Vec<_>>();
    summary.push(ProviderStreamEvent::Completed);
    let mut h = harness_with_config(
        vec![
            summary,
            vec![text("Both files updated."), ProviderStreamEvent::Completed],
        ],
        std::time::Duration::from_secs(20),
        ToolRisk::ReadOnly,
        None,
        |config| {
            config.execution_mode = ModelExecutionMode::BoundedContext;
            config.max_context_bytes = 100_000;
            config.conversation_id = Uuid::new_v4();
            config.seed_messages = (0..20)
                .map(|index| {
                    let role = if index % 2 == 0 {
                        ContextRole::User
                    } else {
                        ContextRole::Assistant
                    };
                    let durable_role = if index % 2 == 0 { "user" } else { "assistant" };
                    ContextMessage::new(
                        role,
                        format!("Earlier exchange {index}: {}", "x".repeat(16_000)),
                    )
                    .with_durable_identity(
                        crate::agent::context::DurableMessageIdentity {
                            id: Uuid::from_u128(index as u128 + 1),
                            role: durable_role.to_owned(),
                            sequence_no: index + 1,
                        },
                    )
                })
                .chain([ContextMessage::new(
                    ContextRole::User,
                    "Update both project files.",
                )])
                .collect();
        },
    );
    h.adapter
        .fragment_delay_ms
        .store(100, std::sync::atomic::Ordering::SeqCst);
    h.tool_run.set_watchdog(progress_watchdog());
    assert!(h
        .tool_run
        .drive(Vec::new())
        .await
        .expect("summary and answer finish")
        .is_some());
    assert_eq!(h.tool_run.state(), RunState::Completed);
    assert!(failure_events(&h).is_empty());
    let requests = h.adapter.seen.lock().expect("requests");
    assert_eq!(
        requests.len(),
        2,
        "one summary request followed by the answer"
    );
    assert!(requests[0].tools.is_empty());
    assert!(requests[0].messages[0].content.contains("condense"));
    assert!(requests[1]
        .messages
        .iter()
        .any(|message| message.content.contains("<conversation_summary>")));
    let events = h.events.lock().expect("events");
    let visible = events
        .iter()
        .filter_map(|event| match event {
            InferenceEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(
        visible, "Both files updated.",
        "internal summary text stays hidden"
    );
}
