//! Model loop orchestration — journal-first run transitions plus DA observation.
//!
//! **Path B (interim, and a dead end for tool calls in this task):**
//! [`RunEngine::poll_da_run`] observes tenant runs via DA polling; a
//! `tool_call.*`/`approval_required` event it relays still fails the run
//! closed (`RunEngineError::ToolsUnavailable`), unchanged by Task 7 — see
//! `crate::agent::executor`'s module doc comment for the corrected, honest
//! reason (a generic DA tool-call update route exists in the pinned
//! contract but is unused by this crate's client wrapper; whether posting
//! to it resumes a DA-driven run server-side is unverified, not structurally
//! impossible — an earlier draft of this comment overclaimed the latter).
//!
//! **Path A (tool-continuation):** [`run_with_tool_loop`] is the bounded
//! orchestrator: it drives an [`crate::agent::executor::InferenceExecutor`]
//! round by round, and whenever a turn requests tools, hands them to a
//! [`crate::agent::tool_loop::ToolLoop`] for policy-gated dispatch, wiring
//! the explicit `Running -> AwaitingApproval -> Running` transitions
//! (`crate::agent::run::Run`) exactly when a dispatched invocation pauses.
//! `crate::agent::turn_driver::LocalTurnDriver` is the live driver behind it,
//! streaming one real provider turn per round;
//! `crate::agent::tests::tool_loop`'s fake driver exercises the orchestration
//! deterministically without a provider.

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::credentials::ProviderId;
use xfalcon_contracts::inference::{FinishReason, InferenceErrorCode, InferenceEvent};
use xfalcon_contracts::RunState;

use crate::agent::da_events::{map_observed_event, DaEventMapError};
use crate::agent::executor::{
    DaPollingExecutor, DaRunObservation, ExecutorError, InferenceExecutor, ModelTurn, ObservedRun,
    TurnError,
};
use crate::agent::output::{FinalAssistantOutput, OutputError, RunOutputBuilder};
use crate::agent::run::{Run, RunError, RunJournal};
use crate::agent::stream::{CoalesceConfig, InferencePresentationSink, StreamError};
use crate::agent::tool_loop::{
    ModelToolCall, ToolCallOutcome, ToolExecutorPort, ToolLoop, ToolLoopError,
};
use crate::tools::invocation::InvocationJournal;
use crate::tools::policy::PathAuthority;

#[derive(Debug, Error)]
pub enum RunEngineError {
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(transparent)]
    Executor(#[from] ExecutorError),
    #[error("model-only executor cannot satisfy tool calls")]
    ToolsUnavailable,
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error("run cancelled")]
    Cancelled,
    #[error(transparent)]
    Presentation(#[from] StreamError),
    #[error(transparent)]
    ToolLoop(#[from] ToolLoopError),
    #[error("model turn failed: {0}")]
    Turn(String),
}

/// Outcome of [`run_with_tool_loop`] — distinct from [`RunEngineOutcome`]
/// because a paused run has not finished: there is no `final_text`/
/// `finish_reason` yet, only the fact that the caller must wait for
/// [`ToolLoop::resume`]/[`ToolLoop::decline_pending_approval`] before this
/// function can usefully be called again for the same run.
#[derive(Debug)]
pub enum ToolLoopRunOutcome {
    Completed(RunEngineOutcome),
    AwaitingApproval,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEngineOutcome {
    pub finish_reason: FinishReason,
    pub final_text: String,
    pub presentation: Vec<InferenceEvent>,
}

pub struct RunEngine<O> {
    observer: Arc<O>,
}

impl<O> RunEngine<O>
where
    O: DaRunObservation,
{
    #[must_use]
    pub fn new(observer: Arc<O>) -> Self {
        Self { observer }
    }

    pub async fn poll_da_run<F>(
        &self,
        run: &mut Run,
        journal: &mut dyn RunJournal,
        da_run_id: Uuid,
        cancel: CancellationToken,
        poll_interval: Duration,
        mut on_inference: F,
    ) -> Result<RunEngineOutcome, RunEngineError>
    where
        F: FnMut(&InferenceEvent) + Send,
    {
        run.transition(RunState::Queued, journal)?;
        run.transition(RunState::Running, journal)?;

        let run_id = run.run_id().clone();
        let model_id = run.model_id().clone();
        let provider = ProviderId::OpenAi;
        let mut output = RunOutputBuilder::with_default_budget(run_id.clone(), model_id.clone());
        let mut started = true;
        let mut tool_requested = false;
        let mut presentation = InferencePresentationSink::new(
            run_id.clone(),
            CoalesceConfig::for_delta_coalescing(),
            &mut on_inference,
        );
        let started_event = InferenceEvent::started(
            run_id.clone(),
            format!("da-{da_run_id}"),
            provider,
            model_id.clone(),
        );
        presentation.emit(started_event)?;

        let executor = DaPollingExecutor::new(self.observer.clone());
        let observed = match executor
            .poll_until_terminal(da_run_id, cancel.clone(), poll_interval, |events| {
                for event in events {
                    let provider = ProviderId::OpenAi;
                    match map_observed_event(
                        &run_id,
                        provider,
                        &model_id,
                        &event,
                        &mut output,
                        &mut started,
                    ) {
                        Ok(batch) => {
                            for event in batch {
                                presentation
                                    .emit(event)
                                    .map_err(|_| ExecutorError::ObservationFailed)?;
                            }
                        }
                        Err(DaEventMapError::ToolsUnavailable) => {
                            tool_requested = true;
                        }
                        Err(DaEventMapError::UnknownEventType) => {}
                    }
                }
                // One poll is a complete unit of observed progress, so its prose
                // goes out with it. Path A has a timer for this (`stream_turn`'s
                // paint tick); here the poll IS the timer, and without this the
                // batch would wait for the first delta of the NEXT poll — every
                // answer a whole poll interval behind what DA already knows.
                presentation.flush();
                Ok(())
            })
            .await
        {
            Ok(observed) => observed,
            Err(error) => {
                let _ = run.transition(RunState::Interrupted, journal);
                return Err(RunEngineError::Executor(error));
            }
        };

        if cancel.is_cancelled() {
            let _ = run.request_cancel(journal);
            return Err(RunEngineError::Cancelled);
        }

        if tool_requested || output.had_tool_request() {
            let _ = run.transition(RunState::Failed, journal);
            presentation.emit(InferenceEvent::failed(
                run_id.clone(),
                InferenceErrorCode::InvalidResponse,
                "Tool calls are not available in this chat slice.",
                true,
            ))?;
            return Err(RunEngineError::ToolsUnavailable);
        }

        let final_text = finalize_text(&observed, &output)?;
        let finish_reason = finish_reason_for(&observed);

        match observed.status.as_str() {
            "cancelled" => {
                let _ = run.request_cancel(journal);
                return Err(RunEngineError::Cancelled);
            }
            "error" => {
                run.transition(RunState::Failed, journal)?;
                return Err(RunEngineError::Output(OutputError::EmptyOutput));
            }
            _ => {
                run.transition(RunState::Completed, journal)?;
            }
        }

        let mut built = RunOutputBuilder::with_default_budget(run_id.clone(), model_id.clone());
        built.push_text(&final_text)?;
        let _ = FinalAssistantOutput::from_builder(built.finish()?)?;

        Ok(RunEngineOutcome {
            finish_reason,
            final_text,
            presentation: presentation.finish(),
        })
    }
}

/// Path A — the bounded tool-continuation orchestrator Task 7 adds. Drives
/// `turn_driver` round by round; whenever a turn requests tools, dispatches
/// them through `tool_loop` and, unless the round paused for approval,
/// feeds every outcome back into the next `next_turn` call. Wires the
/// explicit `Running -> AwaitingApproval` pause exactly when
/// [`RoundOutcome::paused`](crate::agent::tool_loop::RoundOutcome::paused)
/// is set.
///
/// # Presentation events
///
/// [`RunEngineOutcome::presentation`] collects only what *this* function
/// emits — terminal failures it decides on. A turn driver streams its own
/// deltas straight to the same `on_inference` callback, so the caller sees
/// everything; the returned vector is not a transcript of the run.
///
/// # Resuming a paused run
///
/// This function never resolves a pending approval itself. A caller resumes
/// a run left in `RunState::AwaitingApproval` by first calling
/// [`ToolLoop::resume`] (or [`ToolLoop::decline_pending_approval`]) directly
/// against the *same* `tool_loop` to resolve the specific invocation that
/// paused it, then re-invoking this function with that resolution's
/// [`ToolCallOutcome`] as `resumed_tool_outcomes` — which seeds the first
/// `next_turn` call exactly as if it were this round's own output. Passing
/// `AwaitingApproval -> Running` here (rather than leaving it implicit)
/// mirrors how [`Run`] itself only ever transitions in response to
/// something that already happened, never speculatively: the transition
/// happens in this same call, right before the model turn that consumes the
/// resolution.
#[allow(clippy::too_many_arguments)]
pub async fn run_with_tool_loop(
    run: &mut Run,
    run_journal: &mut dyn RunJournal,
    invocation_journal: &mut dyn InvocationJournal,
    turn_driver: &mut dyn InferenceExecutor,
    tool_loop: &mut ToolLoop,
    tool_executor: &dyn ToolExecutorPort,
    path_authority: Option<&dyn PathAuthority>,
    resumed_tool_outcomes: Vec<ToolCallOutcome>,
    cancel: CancellationToken,
    mut on_inference: impl FnMut(&InferenceEvent) + Send,
) -> Result<ToolLoopRunOutcome, RunEngineError> {
    match run.state() {
        RunState::Draft => {
            run.transition(RunState::Queued, run_journal)?;
            run.transition(RunState::Running, run_journal)?;
        }
        RunState::Queued => {
            run.transition(RunState::Running, run_journal)?;
        }
        // Re-entering after a resolved approval pause: this is a resume,
        // never a fresh queue/start.
        RunState::AwaitingApproval => {
            if cancel.is_cancelled() {
                turn_driver.record_outcomes(resumed_tool_outcomes);
                // The run was cancelled while paused for approval. Left
                // alone, the invocation(s) that caused the pause would stay
                // stuck at `ToolInvocationStatus::AwaitingApproval` in
                // `tool_loop`'s own `pending_approval` map forever — nothing
                // else holds a reference by which to resolve them (a review
                // finding named this exact "two state machines can diverge"
                // failure class). Resolve every currently-pending invocation
                // (declining it, never executing it) before transitioning
                // `run` at all, rather than transitioning to `Running` first
                // and catching the cancellation one loop iteration later
                // (which would leave this reconciliation undone).
                for invocation_id in tool_loop.pending_invocation_ids() {
                    match tool_loop
                        .decline_pending_approval(&invocation_id, invocation_journal)
                        .await
                    {
                        Ok(outcome) => turn_driver.record_outcomes(vec![outcome]),
                        Err(error) => tracing::error!(
                            run_id = run.run_id().as_str(),
                            %error,
                            "cancelled run: pending approval could not be declined"
                        ),
                    }
                }
                run.request_cancel(run_journal)?;
                return Err(RunEngineError::Cancelled);
            }
            run.transition(RunState::Running, run_journal)?;
        }
        _ => {}
    }

    let run_id = run.run_id().clone();
    let mut presentation = InferencePresentationSink::new(
        run_id.clone(),
        CoalesceConfig::for_delta_coalescing(),
        &mut on_inference,
    );

    let mut tool_outcomes: Vec<ToolCallOutcome> = resumed_tool_outcomes;
    loop {
        if cancel.is_cancelled() {
            turn_driver.record_outcomes(tool_outcomes);
            let _ = run.request_cancel(run_journal);
            return Err(RunEngineError::Cancelled);
        }
        if let Err(error) = run.ensure_within_deadline() {
            turn_driver.record_outcomes(tool_outcomes);
            // Mirrors every other terminal error path in this function
            // (`TurnError::Failed` below, every branch of the sibling
            // `poll_da_run`): reach a terminal, journaled `Run` state before
            // returning, rather than leaving `run` stuck non-terminal in
            // `RunState::Running` after the caller has already stopped
            // driving it. A review finding caught this arm propagating via
            // `?` with no transition at all.
            let _ = run.transition(RunState::Failed, run_journal);
            return Err(RunEngineError::Run(error));
        }
        if tool_loop.authorization_refused() {
            turn_driver.record_outcomes(tool_outcomes);
            run.transition(RunState::Failed, run_journal)?;
            return Err(RunEngineError::ToolLoop(
                ToolLoopError::AuthorizationRefused,
            ));
        }

        let (turn, deadline_elapsed) = drain_on_deadline(
            turn_driver.next_turn(std::mem::take(&mut tool_outcomes), &cancel),
            &cancel,
            run.deadline(),
        )
        .await;
        match turn {
            Ok(ModelTurn::Final {
                finish_reason,
                final_text,
            }) => {
                run.transition(RunState::Completed, run_journal)?;
                return Ok(ToolLoopRunOutcome::Completed(RunEngineOutcome {
                    finish_reason,
                    final_text,
                    presentation: presentation.finish(),
                }));
            }
            Ok(ModelTurn::ToolCalls(calls)) => {
                if deadline_elapsed
                    || (!cancel.is_cancelled() && run.ensure_within_deadline().is_err())
                {
                    run.transition(RunState::Failed, run_journal)?;
                    return Err(RunEngineError::Run(RunError::DeadlineExceeded));
                }
                if cancel.is_cancelled() {
                    run.request_cancel(run_journal)?;
                    return Err(RunEngineError::Cancelled);
                }
                let model_calls: Vec<ModelToolCall> = calls
                    .into_iter()
                    .map(|call| ModelToolCall {
                        invocation_id: call.invocation_id,
                        tool_id: call.tool_id,
                        arguments: call.arguments,
                    })
                    .collect();
                let (round, deadline_elapsed) = drain_on_deadline(
                    tool_loop.run_round(
                        model_calls,
                        tool_executor,
                        path_authority,
                        invocation_journal,
                        &cancel,
                    ),
                    &cancel,
                    run.deadline(),
                )
                .await;
                let round = match round {
                    Ok(round) => round,
                    Err(error) => {
                        // `ToolLoopError` (round-cap hit, round timeout,
                        // too-many-parallel-calls — all reachable through
                        // ordinary model behavior, not only adversarial
                        // input) used to bubble out via `?` here with no
                        // `Run` transition at all, permanently stranding the
                        // run in the non-terminal `RunState::Running` with
                        // nothing left able to resume, fail, or clean it up
                        // — three independent reviewers flagged this same
                        // gap. Mirror the `TurnError::Failed` arm below:
                        // reach a terminal, journaled state before
                        // returning.
                        let _ = run.transition(RunState::Failed, run_journal);
                        return Err(RunEngineError::ToolLoop(error));
                    }
                };
                if tool_loop.authorization_refused()
                    || deadline_elapsed
                    || tool_loop.round_timed_out()
                {
                    turn_driver.record_outcomes(round.outcomes);
                    // A failed round can also contain approval-gated calls.
                    // Close every sibling before failing so none remains
                    // durably awaiting approval.
                    for invocation_id in tool_loop.pending_invocation_ids() {
                        match tool_loop
                            .decline_pending_approval(&invocation_id, invocation_journal)
                            .await
                        {
                            Ok(outcome) => turn_driver.record_outcomes(vec![outcome]),
                            Err(error) => {
                                run.transition(RunState::Failed, run_journal)?;
                                return Err(RunEngineError::ToolLoop(error));
                            }
                        }
                    }
                    run.transition(RunState::Failed, run_journal)?;
                    if deadline_elapsed {
                        return Err(RunEngineError::Run(RunError::DeadlineExceeded));
                    }
                    if tool_loop.round_timed_out() {
                        return Err(RunEngineError::ToolLoop(ToolLoopError::RoundTimeout));
                    }
                    return Err(RunEngineError::ToolLoop(
                        ToolLoopError::AuthorizationRefused,
                    ));
                }
                if round.paused {
                    // The round's *other* outcomes would be dropped here: this
                    // function returns to its caller now, and only the
                    // resolution of the paused invocation comes back through
                    // `resumed_tool_outcomes`. Hand them to the turn driver so
                    // a driver maintaining a message context can still answer
                    // every call the model made — an assistant turn left with
                    // unanswered tool calls is rejected outright by OpenAI and
                    // Anthropic on the next request.
                    turn_driver.record_outcomes(round.outcomes);
                    run.transition(RunState::AwaitingApproval, run_journal)?;
                    return Ok(ToolLoopRunOutcome::AwaitingApproval);
                }
                tool_outcomes = round.outcomes;
            }
            Err(TurnError::Cancelled) => {
                if deadline_elapsed {
                    run.transition(RunState::Failed, run_journal)?;
                    return Err(RunEngineError::Run(RunError::DeadlineExceeded));
                }
                let _ = run.request_cancel(run_journal);
                return Err(RunEngineError::Cancelled);
            }
            // The driver already emitted a terminal `Failed` carrying a more
            // accurate error code than this layer can reconstruct. Emitting a
            // second one here would make the UI render the run as failing
            // twice.
            Err(TurnError::Reported(message)) => {
                let _ = run.transition(RunState::Failed, run_journal);
                return Err(RunEngineError::Turn(message));
            }
            Err(TurnError::Failed(message)) => {
                let _ = run.transition(RunState::Failed, run_journal);
                let _ = presentation.emit(InferenceEvent::failed(
                    run_id.clone(),
                    InferenceErrorCode::Internal,
                    message.clone(),
                    false,
                ));
                return Err(RunEngineError::Turn(message));
            }
        }
    }
}

/// Stop execution at the deadline without dropping an in-flight journal or
/// DA write. An explicit Stop observed first retains its cancellation result.
async fn drain_on_deadline<T>(
    step: impl std::future::Future<Output = T>,
    cancel: &CancellationToken,
    deadline: std::time::Instant,
) -> (T, bool) {
    tokio::pin!(step);
    tokio::select! {
        biased;
        result = &mut step => (result, false),
        () = cancel.cancelled() => (step.await, false),
        () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            cancel.cancel();
            (step.await, true)
        }
    }
}

fn finalize_text(
    observed: &ObservedRun,
    output: &RunOutputBuilder,
) -> Result<String, RunEngineError> {
    if let Some(message) = observed.output_message.as_deref() {
        if !message.is_empty() {
            return Ok(message.to_owned());
        }
    }
    if !output.text_snapshot().is_empty() {
        return Ok(output.text_snapshot().to_owned());
    }
    Err(RunEngineError::Output(OutputError::EmptyOutput))
}

fn finish_reason_for(observed: &ObservedRun) -> FinishReason {
    match observed.status.as_str() {
        "cancelled" => FinishReason::Cancelled,
        _ => FinishReason::Stop,
    }
}
