//! Journal-first run state machine.

use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ModelId;
use xfalcon_contracts::{ConversationId, RunEvent, RunId, RunState, CONTRACT_VERSION};

/// Budget and lifetime limits owned by a live [`Run`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunBudgets {
    pub event_budget: u32,
    pub output_byte_budget: usize,
    pub wall_clock: Duration,
}

/// Errors from validating or journaling a run transition / budget consume.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RunError {
    #[error("illegal run state transition from {from:?} to {to:?}")]
    IllegalTransition { from: RunState, to: RunState },
    #[error("run is in immutable terminal state {0:?}")]
    TerminalImmutable(RunState),
    #[error("event budget exhausted")]
    EventBudgetExhausted,
    #[error("output byte budget exhausted")]
    OutputByteBudgetExhausted,
    #[error("run wall-clock deadline exceeded")]
    DeadlineExceeded,
    #[error("run journal rejected the transition: {0}")]
    Journal(&'static str),
}

/// Side-effect intent returned only after the transition has been journaled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionIntent {
    Enqueue,
    StartInference,
    Complete,
    Fail,
    Cancel,
    Interrupt,
    BeginRecovery,
    Resume,
    /// Cancel requested while already cancelled — no new journal entry.
    AlreadyCancelled,
    /// A live `crate::tools::invocation::ToolInvocation` owned by this run
    /// entered `xfalcon_contracts::tools::ToolInvocationStatus::AwaitingApproval`
    /// (a `xfalcon_contracts::tools::ToolRisk::Mutating` tool call with no
    /// approval token yet) — distinct from [`Self::Interrupt`]/[`Self::BeginRecovery`]:
    /// this is an expected control-flow pause the agent loop itself requests,
    /// never a crash/failure signal.
    PauseForApproval,
    /// The invocation that caused [`Self::PauseForApproval`] was resolved
    /// (approved and executed, or denied/cancelled) and the run resumes
    /// inference. Distinct from [`Self::Resume`] (crash recovery): this never
    /// bumps [`Run::attempt`] — pausing for approval is not a retry.
    ResumeAfterApproval,
}

/// Validated transition result: semantic event plus post-journal side-effect intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionOutcome {
    pub event: RunEvent,
    pub intent: TransitionIntent,
    pub sequence: u64,
    pub state: RunState,
}

/// Narrow journal port — persistence adapters bind later; unit tests use memory.
pub trait RunJournal: Send {
    fn record_transition(
        &mut self,
        run_id: &RunId,
        sequence: u64,
        from: RunState,
        to: RunState,
    ) -> Result<(), RunError>;
}

/// One journaled state transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalEntry {
    pub run_id: RunId,
    pub sequence: u64,
    pub from: RunState,
    pub to: RunState,
}

/// In-memory journal for proving journal-before-side-effect ordering.
#[derive(Clone, Debug, Default)]
pub struct InMemoryRunJournal {
    entries: Vec<JournalEntry>,
}

impl InMemoryRunJournal {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }
}

impl RunJournal for InMemoryRunJournal {
    fn record_transition(
        &mut self,
        run_id: &RunId,
        sequence: u64,
        from: RunState,
        to: RunState,
    ) -> Result<(), RunError> {
        self.entries.push(JournalEntry {
            run_id: run_id.clone(),
            sequence,
            from,
            to,
        });
        Ok(())
    }
}

/// Live agent run — callers never mutate state fields directly.
#[derive(Debug)]
pub struct Run {
    run_id: RunId,
    conversation_id: ConversationId,
    model_id: ModelId,
    state: RunState,
    cancel: CancellationToken,
    attempt: u32,
    sequence: u64,
    deadline: Instant,
    event_budget: u32,
    events_emitted: u32,
    output_byte_budget: usize,
    output_bytes_used: usize,
}

impl Run {
    /// Creates a run in [`RunState::Draft`] with sequence `0` (no transitions yet).
    #[must_use]
    pub fn new(
        run_id: RunId,
        conversation_id: ConversationId,
        model_id: ModelId,
        budgets: RunBudgets,
    ) -> Self {
        Self {
            run_id,
            conversation_id,
            model_id,
            state: RunState::Draft,
            cancel: CancellationToken::new(),
            attempt: 0,
            sequence: 0,
            deadline: Instant::now() + budgets.wall_clock,
            event_budget: budgets.event_budget,
            events_emitted: 0,
            output_byte_budget: budgets.output_byte_budget,
            output_bytes_used: 0,
        }
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    #[must_use]
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    #[must_use]
    pub fn state(&self) -> RunState {
        self.state.clone()
    }

    #[must_use]
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    #[must_use]
    pub fn event_budget_remaining(&self) -> u32 {
        self.event_budget.saturating_sub(self.events_emitted)
    }

    #[must_use]
    pub fn output_byte_budget_remaining(&self) -> usize {
        self.output_byte_budget
            .saturating_sub(self.output_bytes_used)
    }

    /// Validates `to`, journals the transition, then returns the side-effect intent.
    pub fn transition(
        &mut self,
        to: RunState,
        journal: &mut dyn RunJournal,
    ) -> Result<TransitionOutcome, RunError> {
        let from = self.state.clone();

        if is_terminal(&from) {
            return Err(RunError::TerminalImmutable(from));
        }

        if !allowed_transition(&from, &to) {
            return Err(RunError::IllegalTransition { from, to });
        }

        let intent = intent_for(&from, &to);
        let next_sequence = self.sequence + 1;

        journal.record_transition(&self.run_id, next_sequence, from, to.clone())?;

        self.sequence = next_sequence;
        self.state = to.clone();
        if matches!(intent, TransitionIntent::Resume) {
            self.attempt = self.attempt.saturating_add(1);
        }
        if matches!(intent, TransitionIntent::Cancel) {
            self.cancel.cancel();
        }

        Ok(TransitionOutcome {
            event: state_changed_event(&self.run_id, to.clone()),
            intent,
            sequence: next_sequence,
            state: to,
        })
    }

    /// Requests cancellation. Idempotent when already [`RunState::Cancelled`].
    pub fn request_cancel(
        &mut self,
        journal: &mut dyn RunJournal,
    ) -> Result<TransitionOutcome, RunError> {
        if self.state == RunState::Cancelled {
            return Ok(TransitionOutcome {
                event: state_changed_event(&self.run_id, RunState::Cancelled),
                intent: TransitionIntent::AlreadyCancelled,
                sequence: self.sequence,
                state: RunState::Cancelled,
            });
        }

        self.transition(RunState::Cancelled, journal)
    }

    /// Consumes one unit of the event budget (presentation / semantic events).
    pub fn consume_event(&mut self) -> Result<(), RunError> {
        if self.events_emitted >= self.event_budget {
            return Err(RunError::EventBudgetExhausted);
        }
        self.events_emitted += 1;
        Ok(())
    }

    /// Consumes `bytes` from the output byte budget.
    pub fn consume_output_bytes(&mut self, bytes: usize) -> Result<(), RunError> {
        let remaining = self.output_byte_budget_remaining();
        if bytes > remaining {
            return Err(RunError::OutputByteBudgetExhausted);
        }
        self.output_bytes_used += bytes;
        Ok(())
    }

    /// Returns whether the wall-clock deadline has passed.
    #[must_use]
    pub fn is_deadline_exceeded(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Fails with [`RunError::DeadlineExceeded`] when past the deadline.
    pub fn ensure_within_deadline(&self) -> Result<(), RunError> {
        if self.is_deadline_exceeded() {
            Err(RunError::DeadlineExceeded)
        } else {
            Ok(())
        }
    }
}

fn allowed_transition(from: &RunState, to: &RunState) -> bool {
    matches!(
        (from, to),
        (RunState::Draft, RunState::Queued)
            | (RunState::Queued, RunState::Running)
            | (RunState::Running, RunState::Completed)
            | (RunState::Running, RunState::Failed)
            | (RunState::Running, RunState::Cancelled)
            | (RunState::Running, RunState::Interrupted)
            | (RunState::Interrupted, RunState::Recovering)
            | (RunState::Recovering, RunState::Running)
            | (RunState::Recovering, RunState::Failed)
            | (RunState::Recovering, RunState::Cancelled)
            // Task 7: a run pauses exactly when one of its in-flight tool
            // invocations enters `ToolInvocationStatus::AwaitingApproval`
            // (`crate::agent::tool_loop`), and resumes once that invocation
            // is resolved (approved-and-executed, denied, or cancelled).
            // Mirrors `crate::tools::invocation`'s own `(Requested,
            // AwaitingApproval)` / `(AwaitingApproval, Approved | Denied |
            // Cancelled)` shape one level up (per-run instead of
            // per-invocation).
            | (RunState::Running, RunState::AwaitingApproval)
            | (RunState::AwaitingApproval, RunState::Running)
            | (RunState::AwaitingApproval, RunState::Cancelled)
            | (RunState::AwaitingApproval, RunState::Failed)
    )
}

// Required call-ordering contract when composing a `Run` with the
// `crate::agent::tool_loop::ToolLoop` that paused it (documented here
// because `allowed_transition` is exactly what makes the composition
// legal, not because `Run` itself enforces it — it structurally cannot:
// `Run` has no reference to any `ToolLoop`):
//
// `(AwaitingApproval, Cancelled)`/`(AwaitingApproval, Failed)` above are
// legal *at the `Run` level* independent of whatever `ToolInvocation` caused
// the pause. If a caller transitions/cancels a paused `Run` directly (e.g.
// `Run::request_cancel`) without also resolving the owning `ToolLoop`'s
// corresponding pending invocation (`ToolLoop::decline_pending_approval`),
// the two state machines diverge: the invocation is left durably stuck at
// `ToolInvocationStatus::AwaitingApproval` with no caller left holding a
// reference by which to decline or resume it. `crate::agent::engine::run_with_tool_loop`
// closes this for its own re-entry path (cancelling a paused run by
// re-invoking it with an already-cancelled token declines every pending
// invocation via `ToolLoop::pending_invocation_ids`/`decline_pending_approval`
// before transitioning `Run` at all) — but any *other* caller that holds a
// `Run` and a `ToolLoop` independently (e.g. a future direct "cancel this
// run" command handler bypassing `run_with_tool_loop`) MUST do the same
// reconciliation itself before or immediately after cancelling/failing a
// paused `Run`.

fn is_terminal(state: &RunState) -> bool {
    // `AwaitingApproval` is deliberately absent: a paused run is not
    // terminal — see the module doc comment and `allowed_transition`'s
    // `AwaitingApproval -> Running/Cancelled/Failed` arms above.
    matches!(
        state,
        RunState::Completed | RunState::Failed | RunState::Cancelled
    )
}

fn intent_for(from: &RunState, to: &RunState) -> TransitionIntent {
    match (from, to) {
        (RunState::Draft, RunState::Queued) => TransitionIntent::Enqueue,
        (RunState::Queued, RunState::Running) => TransitionIntent::StartInference,
        (RunState::Running, RunState::AwaitingApproval) => TransitionIntent::PauseForApproval,
        (RunState::AwaitingApproval, RunState::Running) => TransitionIntent::ResumeAfterApproval,
        (_, RunState::Completed) => TransitionIntent::Complete,
        (_, RunState::Failed) => TransitionIntent::Fail,
        (_, RunState::Cancelled) => TransitionIntent::Cancel,
        (RunState::Running, RunState::Interrupted) => TransitionIntent::Interrupt,
        (RunState::Interrupted, RunState::Recovering) => TransitionIntent::BeginRecovery,
        (RunState::Recovering, RunState::Running) => TransitionIntent::Resume,
        _ => TransitionIntent::Enqueue,
    }
}

fn state_changed_event(run_id: &RunId, state: RunState) -> RunEvent {
    RunEvent::StateChanged {
        version: CONTRACT_VERSION,
        run_id: run_id.clone(),
        state,
    }
}
