//! One tool-capable run, in a form that survives an approval pause.
//!
//! [`crate::agent::engine::run_with_tool_loop`] returns to its caller the
//! moment a dispatched invocation needs human approval. Resuming that run
//! later means calling `ToolLoop::resume` against the *same* `ToolLoop` and
//! re-entering the orchestrator with the same `Run`, journals, turn driver,
//! and dispatcher. Reconstructing any of those would be wrong, not merely
//! wasteful: a fresh `ToolLoop` has no record of the pending invocation, a
//! fresh turn driver has lost the conversation context the model built up
//! across the run's earlier rounds, and a fresh `Run` would restart a state
//! machine that is mid-flight.
//!
//! So [`ToolRun`] owns all of it together, and [`ParkedToolRuns`] holds the
//! ones waiting on a person. A run is parked only while genuinely paused; it
//! is taken back out to be resumed, declined, or cancelled, and never left
//! behind on any path.
//!
//! # Restart while paused
//!
//! A parked run lives in memory, so quitting the app loses the pause itself.
//! What survives is what matters: the run's *journaled* state is
//! `RunState::AwaitingApproval`, which
//! `crate::agent::recovery::policy_for_interrupted` routes to
//! `RecoveryDecision::AwaitUser` — never an auto-resume, which would run a
//! tool nobody approved — and `is_in_flight_run_state` keeps the on-disk
//! journal so recovery has something to decide from. The tool never executed,
//! and the user is told the response was interrupted and can retry. Pinned by
//! `crate::chat::tests::tool_run::a_run_paused_for_approval_is_recovered_as_needing_the_user`.
//!
//! Continuing the *same* run across a restart, rather than retrying it, needs
//! the persistent invocation journal noted in `crate::chat::invocation_emit`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AsyncMutex, Notify};
use xfalcon_contracts::inference::{InferenceErrorCode, InferenceEvent};
use xfalcon_contracts::tools::{InvocationId, ToolInvocationEvent};
use xfalcon_contracts::{RunId, RunState};

use crate::agent::activity::RunActivity;
use crate::agent::engine::{run_with_tool_loop, RunEngineError, RunEngineOutcome};
use crate::agent::executor::InferenceExecutor;
use crate::agent::local_executor::RunEventSink;
use crate::agent::run::{Run, RunJournal};
use crate::agent::tool_loop::{ToolCallOutcome, ToolExecutorPort, ToolLoop, ToolLoopError};
use crate::agent::tool_session::ToolSession;
use crate::agent::turn_driver::LocalTurnDriver;
use crate::clients::da::chat_api::DaChatSession;
use crate::tools::invocation::{InvocationJournal, InvocationJournalError};
use crate::tools::policy::PathAuthority;
use tokio_util::sync::CancellationToken;

use super::invocation_emit::EmittingInvocationJournal;
use super::persisting_run_journal::PersistingEmittingRunJournal;

/// The concrete `ToolRun` the chat service parks: journals that persist and
/// emit to the frontend, over a DA-backed run-event sink.
pub(crate) type ChatToolRun = ToolRun<
    PersistingEmittingRunJournal<tauri::Wry>,
    EmittingInvocationJournal<tauri::Wry>,
    DaChatSession,
>;

/// How long a run may sit without producing one event before it is stopped.
///
/// Observes provider output, tool journal entries, and completed history writes.
/// A task making progress can continue up to the run's wall-clock budget.
const RUN_STALL_LIMIT: Duration = Duration::from_secs(300);
pub(crate) const RUN_WALL_CLOCK: Duration = Duration::from_secs(60 * 60);
/// Outer backstop for the run's own wall-clock budget (60 minutes). The budget
/// ends a run cleanly between steps; this catches one that is blocked inside a
/// step and never gets to check it.
const RUN_HARD_LIMIT: Duration = Duration::from_secs(RUN_WALL_CLOCK.as_secs() + 60);
/// How long a stopped run gets to write its results out before it is dropped.
const STOP_GRACE: Duration = Duration::from_secs(20);
const WATCHDOG_POLL: Duration = Duration::from_secs(2);

const STALLED_MESSAGE: &str =
    "This response stopped making progress, so it was stopped. Nothing you asked for is lost. You can retry.";
const OVERRAN_MESSAGE: &str =
    "This run took too long and was stopped. Ask for a narrower slice, then retry.";

/// Limits the watchdog enforces on one [`ToolRun::drive`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct WatchdogLimits {
    pub(crate) stall: Duration,
    pub(crate) hard: Duration,
    pub(crate) stop_grace: Duration,
    pub(crate) poll: Duration,
}

impl Default for WatchdogLimits {
    fn default() -> Self {
        Self {
            stall: RUN_STALL_LIMIT,
            hard: RUN_HARD_LIMIT,
            stop_grace: STOP_GRACE,
            poll: WATCHDOG_POLL,
        }
    }
}

struct ProgressInvocationJournal<I> {
    inner: I,
    activity: Arc<RunActivity>,
}

/// Tracks only executor calls whose effects must be allowed to finish once
/// they start. The watchdog uses this to distinguish a durable local write
/// from an otherwise stuck tool run.
#[derive(Default)]
struct RequiredCompletionTracker {
    active: AtomicUsize,
    idle: Notify,
}

impl RequiredCompletionTracker {
    fn enter(self: &Arc<Self>) -> RequiredCompletionGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        RequiredCompletionGuard(Arc::clone(self))
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire) > 0
    }

    async fn wait_until_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.is_active() {
                return;
            }
            notified.await;
        }
    }
}

struct RequiredCompletionGuard(Arc<RequiredCompletionTracker>);

impl Drop for RequiredCompletionGuard {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

struct CompletionTrackingExecutor<'a> {
    inner: &'a dyn ToolExecutorPort,
    tracker: Arc<RequiredCompletionTracker>,
}

#[async_trait::async_trait]
impl ToolExecutorPort for CompletionTrackingExecutor<'_> {
    fn requires_completion(&self, tool_id: &xfalcon_contracts::tools::ToolId) -> bool {
        self.inner.requires_completion(tool_id)
    }

    async fn execute(
        &self,
        tool_id: &xfalcon_contracts::tools::ToolId,
        arguments: &serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<crate::tools::invocation::ToolResult, crate::agent::tool_loop::ToolExecutionError>
    {
        let _guard = self
            .inner
            .requires_completion(tool_id)
            .then(|| self.tracker.enter());
        self.inner.execute(tool_id, arguments, cancel).await
    }
}

impl<I: InvocationJournal> InvocationJournal for ProgressInvocationJournal<I> {
    fn record(
        &mut self,
        invocation_id: &InvocationId,
        sequence: u64,
        event: ToolInvocationEvent,
    ) -> Result<(), InvocationJournalError> {
        self.inner.record(invocation_id, sequence, event)?;
        self.activity.touch();
        Ok(())
    }
}

/// Everything one tool-capable run needs to keep running, including across a
/// pause.
///
/// Generic over its journals and run-event sink rather than over
/// `tauri::Runtime`: nothing here is about Tauri, and the ports it actually
/// depends on are `RunJournal`, `InvocationJournal`, and `RunEventSink`. The
/// chat service instantiates [`ChatToolRun`]; tests instantiate in-memory
/// journals and a fake executor, with no app handle in sight.
pub(crate) struct ToolRun<J, I, S>
where
    J: RunJournal,
    I: InvocationJournal,
    S: RunEventSink,
{
    run: Run,
    run_journal: J,
    invocation_journal: ProgressInvocationJournal<I>,
    tool_loop: ToolLoop,
    executor: Arc<dyn ToolExecutorPort>,
    /// `None` for an MCP-only run — see [`ToolSession::path_authority`]. The
    /// policy engine denies any path-taking tool without one, so the absence
    /// needs no stand-in here.
    path_authority: Option<Arc<dyn PathAuthority>>,
    driver: LocalTurnDriver<S>,
    required_completions: Arc<RequiredCompletionTracker>,
    on_inference: Arc<dyn Fn(&InferenceEvent) + Send + Sync>,
    cancel: CancellationToken,
    watchdog: WatchdogLimits,
    /// Outcomes of invocations resolved while parked, waiting to seed the
    /// next model turn.
    ///
    /// Buffered rather than passed straight to [`Self::drive`] because a
    /// round can pause on several invocations at once, and the run must not
    /// resume until every one of them has a decision — otherwise the next
    /// turn would see a tool call it never got an answer for, which OpenAI
    /// and Anthropic both reject outright.
    resolved: Vec<ToolCallOutcome>,
}

impl<J, I, S> ToolRun<J, I, S>
where
    J: RunJournal,
    I: InvocationJournal,
    S: RunEventSink,
{
    pub(crate) fn new(
        run: Run,
        run_journal: J,
        invocation_journal: I,
        tool_session: ToolSession,
        driver: LocalTurnDriver<S>,
        on_inference: Arc<dyn Fn(&InferenceEvent) + Send + Sync>,
        cancel: CancellationToken,
    ) -> Self {
        let ToolSession {
            tool_loop,
            executor,
            path_authority,
            ..
        } = tool_session;
        let invocation_journal = ProgressInvocationJournal {
            inner: invocation_journal,
            activity: driver.activity(),
        };
        Self {
            run,
            run_journal,
            invocation_journal,
            tool_loop,
            executor,
            path_authority,
            driver,
            required_completions: Arc::new(RequiredCompletionTracker::default()),
            on_inference,
            cancel,
            watchdog: WatchdogLimits::default(),
            resolved: Vec::new(),
        }
    }

    pub(crate) fn set_watchdog(&mut self, limits: WatchdogLimits) {
        self.watchdog = limits;
    }

    #[must_use]
    pub(crate) fn state(&self) -> RunState {
        self.run.state()
    }

    #[must_use]
    pub(crate) fn run_id(&self) -> &RunId {
        self.run.run_id()
    }

    /// Every invocation currently waiting on a person.
    #[must_use]
    pub(crate) fn pending_invocation_ids(&self) -> Vec<InvocationId> {
        self.tool_loop.pending_invocation_ids()
    }

    /// Drives the run until it finishes, pauses, or fails.
    ///
    /// `Ok(None)` means paused: no outcome yet, nothing terminal written to
    /// DA, and the caller must park this run rather than report it ended.
    ///
    /// A run that goes quiet, or outlives its hard limit, is stopped here. A
    /// local write that already started may report its true outcome; remaining
    /// cleanup gets a short grace before the run is dropped and the person is
    /// told to retry. Nothing else below this can hold a run in "thinking"
    /// indefinitely.
    pub(crate) async fn drive(
        &mut self,
        resumed: Vec<ToolCallOutcome>,
    ) -> Result<Option<RunEngineOutcome>, RunEngineError> {
        let limits = self.watchdog;
        let cancel = self.cancel.clone();
        let activity = self.driver.activity();
        let required_completions = Arc::clone(&self.required_completions);
        activity.touch();
        let started = Instant::now();
        let tripped = {
            let inner = self.drive_unwatched(resumed, Arc::clone(&activity));
            tokio::pin!(inner);
            let mut poll = tokio::time::interval(limits.poll);
            let mut cancelled_since: Option<Instant> = None;
            let message = loop {
                tokio::select! {
                    result = &mut inner => return result,
                    _ = poll.tick() => {
                        if cancel.is_cancelled() {
                            // A stop is already unwinding: a person's Stop, or
                            // the run's own deadline. It gets longer than a
                            // watchdog stop to save finished tool results, and
                            // is only cut short if it is itself stuck.
                            let since = *cancelled_since.get_or_insert_with(Instant::now);
                            if since.elapsed() >= limits.stop_grace * 3 {
                                break STALLED_MESSAGE;
                            }
                        } else if activity.idle() >= limits.stall {
                            break STALLED_MESSAGE;
                        } else if started.elapsed() >= limits.hard {
                            break OVERRAN_MESSAGE;
                        }
                    }
                }
            };
            cancel.cancel();
            // Poll cancellation cleanup for its normal grace. If it reaches an
            // active required-completion call, that call owns a local durable
            // mutation and must report its real result before the run can end.
            let stop_grace = tokio::time::sleep(limits.stop_grace);
            tokio::pin!(stop_grace);
            let stopped = tokio::select! {
                biased;
                result = &mut inner => Some(result),
                () = &mut stop_grace => None,
            };
            let mut active_required_completion = match stopped {
                Some(Ok(Some(finished))) => return Ok(Some(finished)),
                Some(_) => false,
                None => required_completions.is_active(),
            };
            if active_required_completion {
                // Keep polling the run while the executor unwinds. Its reads,
                // provider calls, and later bookkeeping still receive only a
                // bounded grace once the required call has completed.
                let drained = tokio::select! {
                    biased;
                    result = &mut inner => Some(result),
                    () = required_completions.wait_until_idle() => None,
                };
                active_required_completion = match drained {
                    Some(Ok(Some(finished))) => return Ok(Some(finished)),
                    Some(_) => false,
                    None => true,
                };
            }
            if active_required_completion {
                if let Ok(Ok(Some(finished))) =
                    tokio::time::timeout(limits.stop_grace, &mut inner).await
                {
                    return Ok(Some(finished));
                }
            }
            message
        };
        tracing::error!(
            run_id = self.run.run_id().as_str(),
            message = tripped,
            "run stopped by the watchdog"
        );
        // The person hears first: closing the run on the server can itself be
        // slow, and the chat must not keep spinning while it is.
        if !activity.ended() {
            (self.on_inference)(&InferenceEvent::failed(
                self.run.run_id().clone(),
                InferenceErrorCode::Internal,
                tripped,
                true,
            ));
        }
        let _ = self.run.transition(RunState::Failed, &mut self.run_journal);
        if tokio::time::timeout(limits.stop_grace, self.driver.abandon(tripped))
            .await
            .is_err()
        {
            tracing::error!(
                run_id = self.run.run_id().as_str(),
                "stopped run could not be closed on the server in time"
            );
        }
        Err(RunEngineError::Turn(tripped.to_owned()))
    }

    async fn drive_unwatched(
        &mut self,
        resumed: Vec<ToolCallOutcome>,
        activity: Arc<RunActivity>,
    ) -> Result<Option<RunEngineOutcome>, RunEngineError> {
        let on_inference = Arc::clone(&self.on_inference);
        let executor = CompletionTrackingExecutor {
            inner: self.executor.as_ref(),
            tracker: Arc::clone(&self.required_completions),
        };
        activity.touch();
        let result = run_with_tool_loop(
            &mut self.run,
            &mut self.run_journal,
            &mut self.invocation_journal,
            &mut self.driver,
            &mut self.tool_loop,
            &executor,
            self.path_authority.as_deref(),
            resumed,
            self.cancel.clone(),
            |event: &InferenceEvent| {
                activity.saw(event);
                on_inference(event)
            },
        )
        .await;

        match result {
            Ok(crate::agent::engine::ToolLoopRunOutcome::Completed(outcome)) => Ok(Some(outcome)),
            Ok(crate::agent::engine::ToolLoopRunOutcome::AwaitingApproval) => Ok(None),
            Err(error) => {
                // The turn driver finalizes DA for anything it observed
                // itself. These are the failures it never sees — the round
                // cap, a round timeout, the run's wall-clock deadline — and
                // without this the DA run would sit non-terminal forever.
                // `abandon` is idempotent, so a run the driver already
                // finalized is unaffected. Cancellation is excluded: the
                // cancel path owns that terminal write.
                if matches!(error, RunEngineError::Cancelled) {
                    // Stop owns the terminal write, but this owner still
                    // owes the conversation every completed tool result.
                    if let Err(error) = self.driver.flush_pending_round().await {
                        let message = format!("tool results could not be persisted: {error}");
                        tracing::error!(run_id = self.run.run_id().as_str(), %error, "cancelled run: tool result persistence failed");
                        (self.on_inference)(&InferenceEvent::failed(
                            self.run.run_id().clone(),
                            InferenceErrorCode::Internal,
                            &message,
                            false,
                        ));
                        return Err(RunEngineError::Turn(message));
                    }
                } else {
                    self.driver.abandon(&format!("{error}")).await;
                }
                Err(error)
            }
        }
    }

    /// Resolves one paused invocation and returns the outcome to feed the
    /// next model turn.
    ///
    /// An approval mints an [`crate::tools::policy::ApprovalToken`] from the
    /// pending request itself, so it is scoped to that exact invocation and
    /// tool pair — never a bare boolean, and never anything derived from tool
    /// output. `PolicyEngine::authorize` re-checks it from scratch inside
    /// `ToolLoop::resume` regardless.
    pub(crate) async fn resolve_approval(
        &mut self,
        invocation_id: &InvocationId,
        approve: bool,
    ) -> Result<(), ToolLoopError> {
        let outcome = if approve {
            let token = self
                .tool_loop
                .pending_approval_request(invocation_id)
                .ok_or_else(|| ToolLoopError::UnknownPendingInvocation(invocation_id.clone()))?
                .approve();
            self.tool_loop
                .resume(
                    invocation_id,
                    token,
                    self.executor.as_ref(),
                    self.path_authority.as_deref(),
                    &mut self.invocation_journal,
                    &self.cancel,
                )
                .await?
        } else {
            self.tool_loop
                .decline_pending_approval(invocation_id, &mut self.invocation_journal)
                .await?
        };
        self.resolved.push(outcome);
        if self.tool_loop.authorization_refused() {
            for pending in self.tool_loop.pending_invocation_ids() {
                self.resolved.push(
                    self.tool_loop
                        .decline_pending_approval(&pending, &mut self.invocation_journal)
                        .await?,
                );
            }
        }
        Ok(())
    }

    /// Takes the outcomes resolved since the run parked, to seed the next
    /// model turn.
    #[must_use]
    pub(crate) fn take_resolved_outcomes(&mut self) -> Vec<ToolCallOutcome> {
        std::mem::take(&mut self.resolved)
    }

    /// Owns teardown independently of a caller's bounded wait. Dropping an
    /// abandon future mid-request would lose pending persistence state.
    pub(crate) fn abandon_in_background(
        mut self,
        message: &'static str,
    ) -> tokio::task::JoinHandle<()>
    where
        J: 'static,
        I: 'static,
        S: 'static,
    {
        tokio::spawn(async move { self.abandon(message).await })
    }

    /// Declines every pending invocation and writes the run's terminal DA
    /// record. Used when a paused run is being torn down for a reason no
    /// decision is coming for — the user cancelled, or the app is discarding
    /// the run.
    pub(crate) async fn abandon(&mut self, message: &str) {
        self.driver
            .record_outcomes(std::mem::take(&mut self.resolved));
        let mut message = message.to_owned();
        for invocation_id in self.tool_loop.pending_invocation_ids() {
            match self
                .tool_loop
                .decline_pending_approval(&invocation_id, &mut self.invocation_journal)
                .await
            {
                Ok(outcome) => self.driver.record_outcomes(vec![outcome]),
                Err(error) => {
                    tracing::error!(run_id = self.run.run_id().as_str(), %error, "abandoned run: pending approval could not be declined");
                    message.push_str(&format!(
                        "; pending approval could not be declined: {error}"
                    ));
                }
            }
        }
        self.driver.abandon(&message).await;
    }
}

/// Runs currently paused awaiting a human decision, keyed by run id.
pub(crate) struct ParkedToolRuns {
    runs: AsyncMutex<HashMap<String, ChatToolRun>>,
}

impl Default for ParkedToolRuns {
    fn default() -> Self {
        Self {
            runs: AsyncMutex::new(HashMap::new()),
        }
    }
}

impl ParkedToolRuns {
    pub(crate) async fn park(&self, run_id: String, run: ChatToolRun) {
        self.runs.lock().await.insert(run_id, run);
    }

    /// Removes a parked run so the caller can drive it.
    ///
    /// Deliberately a `take`, not a borrow: driving a run holds it across
    /// awaits and may park it again, and holding the map's lock for that whole
    /// time would serialize every other run's approval behind it.
    pub(crate) async fn take(&self, run_id: &str) -> Option<ChatToolRun> {
        self.runs.lock().await.remove(run_id)
    }

    /// Whether this process is holding the run parked on an approval.
    /// Used by startup zombie finalization to tell "paused but alive here"
    /// from "abandoned by a process that no longer exists".
    pub(crate) async fn contains(&self, run_id: &str) -> bool {
        self.runs.lock().await.contains_key(run_id)
    }
}
