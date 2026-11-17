use std::sync::{Arc, Mutex};
use std::time::Duration;

use xfalcon_contracts::credentials::ModelId;
use xfalcon_contracts::{ConversationId, RunEvent, RunId, RunState, CONTRACT_VERSION};

use crate::agent::context::{ContextBuilder, ContextError, ContextMessage, ContextRole};
use crate::agent::run::{
    InMemoryRunJournal, Run, RunBudgets, RunError, RunJournal, TransitionIntent,
};

fn budgets() -> RunBudgets {
    RunBudgets {
        event_budget: 64,
        output_byte_budget: 64 * 1024,
        wall_clock: Duration::from_secs(300),
    }
}

fn draft_run() -> Run {
    Run::new(
        RunId::from("run-1"),
        ConversationId::from("conv-1"),
        ModelId::from("model-1"),
        budgets(),
    )
}

fn advance(
    run: &mut Run,
    journal: &mut dyn RunJournal,
    to: RunState,
) -> Result<crate::agent::run::TransitionOutcome, RunError> {
    run.transition(to, journal)
}

fn force_state(run: &mut Run, journal: &mut InMemoryRunJournal, path: &[RunState]) {
    for to in path {
        advance(run, journal, to.clone())
            .unwrap_or_else(|err| panic!("failed to reach {to:?} via {:?}: {err}", path));
    }
}

#[test]
fn happy_path_draft_queued_running_completed() {
    let mut run = draft_run();
    let mut journal = InMemoryRunJournal::new();

    assert_eq!(run.state(), RunState::Draft);
    assert_eq!(run.sequence(), 0);
    assert_eq!(run.attempt(), 0);

    let queued = advance(&mut run, &mut journal, RunState::Queued).expect("draft->queued");
    assert_eq!(run.state(), RunState::Queued);
    assert_eq!(queued.intent, TransitionIntent::Enqueue);
    assert_eq!(queued.sequence, 1);
    assert_eq!(
        queued.event,
        RunEvent::StateChanged {
            version: CONTRACT_VERSION,
            run_id: RunId::from("run-1"),
            state: RunState::Queued,
        }
    );

    let running = advance(&mut run, &mut journal, RunState::Running).expect("queued->running");
    assert_eq!(running.intent, TransitionIntent::StartInference);
    assert_eq!(run.state(), RunState::Running);

    let completed =
        advance(&mut run, &mut journal, RunState::Completed).expect("running->completed");
    assert_eq!(completed.intent, TransitionIntent::Complete);
    assert_eq!(run.state(), RunState::Completed);
    assert_eq!(run.sequence(), 3);
    assert_eq!(journal.entries().len(), 3);
}

#[test]
fn running_may_fail_cancel_or_interrupt() {
    for terminal in [RunState::Failed, RunState::Cancelled, RunState::Interrupted] {
        let mut run = draft_run();
        let mut journal = InMemoryRunJournal::new();
        force_state(
            &mut run,
            &mut journal,
            &[RunState::Queued, RunState::Running],
        );
        let outcome = advance(&mut run, &mut journal, terminal.clone()).expect("running exit");
        assert_eq!(run.state(), terminal);
        let expected_intent = match terminal {
            RunState::Failed => TransitionIntent::Fail,
            RunState::Cancelled => TransitionIntent::Cancel,
            RunState::Interrupted => TransitionIntent::Interrupt,
            _ => unreachable!(),
        };
        assert_eq!(outcome.intent, expected_intent);
    }
}

#[test]
fn interrupted_recovers_to_running_failed_or_cancelled() {
    for next in [RunState::Running, RunState::Failed, RunState::Cancelled] {
        let mut run = draft_run();
        let mut journal = InMemoryRunJournal::new();
        force_state(
            &mut run,
            &mut journal,
            &[
                RunState::Queued,
                RunState::Running,
                RunState::Interrupted,
                RunState::Recovering,
            ],
        );
        assert_eq!(run.state(), RunState::Recovering);
        let before_attempt = run.attempt();
        let outcome = advance(&mut run, &mut journal, next.clone()).expect("recovering exit");
        assert_eq!(run.state(), next);
        if next == RunState::Running {
            assert_eq!(outcome.intent, TransitionIntent::Resume);
            assert_eq!(run.attempt(), before_attempt + 1);
        }
    }

    let mut run = draft_run();
    let mut journal = InMemoryRunJournal::new();
    force_state(
        &mut run,
        &mut journal,
        &[RunState::Queued, RunState::Running, RunState::Interrupted],
    );
    let recovering =
        advance(&mut run, &mut journal, RunState::Recovering).expect("interrupted->recovering");
    assert_eq!(recovering.intent, TransitionIntent::BeginRecovery);
}

/// Task 7: `Running -> AwaitingApproval -> Running` is now the explicit
/// pause/resume path a live [`crate::tools::invocation::ToolInvocation`]
/// entering `AwaitingApproval` drives (`crate::agent::tool_loop`), and a
/// paused run may also be cancelled or fail outright (e.g. the pending
/// invocation was denied/cancelled) without first returning to `Running`.
#[test]
fn running_pauses_for_approval_and_resumes() {
    let mut run = draft_run();
    let mut journal = InMemoryRunJournal::new();
    force_state(
        &mut run,
        &mut journal,
        &[RunState::Queued, RunState::Running],
    );
    let before_attempt = run.attempt();

    let paused =
        advance(&mut run, &mut journal, RunState::AwaitingApproval).expect("running->awaiting");
    assert_eq!(paused.intent, TransitionIntent::PauseForApproval);
    assert_eq!(run.state(), RunState::AwaitingApproval);
    // A paused run is not terminal and must not be silently auto-resumed on
    // restart — the same `AwaitUser` classification already covers it (see
    // `crate::agent::journal_recovery::replay_policy_for_journal`).
    assert_eq!(
        crate::agent::journal_recovery::replay_policy_for_journal(journal.entries()),
        crate::agent::recovery::RecoveryDecision::AwaitUser
    );

    let resumed =
        advance(&mut run, &mut journal, RunState::Running).expect("awaiting->running resume");
    assert_eq!(resumed.intent, TransitionIntent::ResumeAfterApproval);
    assert_eq!(run.state(), RunState::Running);
    // Resuming after an approval pause is not a crash retry: `attempt` must
    // not move, unlike `TransitionIntent::Resume` (crash recovery).
    assert_eq!(run.attempt(), before_attempt);
}

#[test]
fn awaiting_approval_pause_may_end_in_cancellation_or_failure() {
    for terminal in [RunState::Cancelled, RunState::Failed] {
        let mut run = draft_run();
        let mut journal = InMemoryRunJournal::new();
        force_state(
            &mut run,
            &mut journal,
            &[
                RunState::Queued,
                RunState::Running,
                RunState::AwaitingApproval,
            ],
        );
        let outcome = advance(&mut run, &mut journal, terminal.clone()).expect("awaiting exit");
        assert_eq!(run.state(), terminal);
        let expected_intent = match terminal {
            RunState::Cancelled => TransitionIntent::Cancel,
            RunState::Failed => TransitionIntent::Fail,
            _ => unreachable!(),
        };
        assert_eq!(outcome.intent, expected_intent);
        if terminal == RunState::Cancelled {
            assert!(run.cancel_token().is_cancelled());
        }
    }
}

/// `AwaitingApproval` is reachable only from `Running` — every other state
/// (including the terminal ones) still rejects it exactly as before Task 7.
#[test]
fn awaiting_approval_is_unreachable_from_non_running_states() {
    let non_running = [
        RunState::Draft,
        RunState::Queued,
        RunState::Completed,
        RunState::Failed,
        RunState::Cancelled,
        RunState::Interrupted,
        RunState::Recovering,
    ];

    for from in &non_running {
        let mut run = draft_run();
        let mut journal = InMemoryRunJournal::new();
        match from {
            RunState::Draft => {}
            RunState::Queued => force_state(&mut run, &mut journal, &[RunState::Queued]),
            RunState::Interrupted => {
                force_state(
                    &mut run,
                    &mut journal,
                    &[RunState::Queued, RunState::Running, RunState::Interrupted],
                );
            }
            RunState::Recovering => {
                force_state(
                    &mut run,
                    &mut journal,
                    &[
                        RunState::Queued,
                        RunState::Running,
                        RunState::Interrupted,
                        RunState::Recovering,
                    ],
                );
            }
            RunState::Completed => {
                force_state(
                    &mut run,
                    &mut journal,
                    &[RunState::Queued, RunState::Running, RunState::Completed],
                );
            }
            RunState::Failed => {
                force_state(
                    &mut run,
                    &mut journal,
                    &[RunState::Queued, RunState::Running, RunState::Failed],
                );
            }
            RunState::Cancelled => {
                force_state(
                    &mut run,
                    &mut journal,
                    &[RunState::Queued, RunState::Running, RunState::Cancelled],
                );
            }
            RunState::Running | RunState::AwaitingApproval => unreachable!(),
        }
        let err = advance(&mut run, &mut journal, RunState::AwaitingApproval)
            .expect_err("awaiting_approval is reachable only from running");
        assert!(matches!(
            err,
            RunError::IllegalTransition {
                to: RunState::AwaitingApproval,
                ..
            } | RunError::TerminalImmutable(_)
        ));
    }
}

/// Every (from, to) pair except the documented allow-list must fail.
#[test]
fn exhaustive_illegal_transitions_are_rejected() {
    let states = [
        RunState::Draft,
        RunState::Queued,
        RunState::Running,
        RunState::AwaitingApproval,
        RunState::Completed,
        RunState::Failed,
        RunState::Cancelled,
        RunState::Interrupted,
        RunState::Recovering,
    ];

    let allowed: &[(RunState, RunState)] = &[
        (RunState::Draft, RunState::Queued),
        (RunState::Queued, RunState::Running),
        (RunState::Running, RunState::Completed),
        (RunState::Running, RunState::Failed),
        (RunState::Running, RunState::Cancelled),
        (RunState::Running, RunState::Interrupted),
        (RunState::Interrupted, RunState::Recovering),
        (RunState::Recovering, RunState::Running),
        (RunState::Recovering, RunState::Failed),
        (RunState::Recovering, RunState::Cancelled),
        (RunState::Running, RunState::AwaitingApproval),
        (RunState::AwaitingApproval, RunState::Running),
        (RunState::AwaitingApproval, RunState::Cancelled),
        (RunState::AwaitingApproval, RunState::Failed),
    ];

    for from in &states {
        for to in &states {
            if from == to {
                continue;
            }
            if allowed.contains(&(from.clone(), to.clone())) {
                continue;
            }

            let mut run = draft_run();
            let mut journal = InMemoryRunJournal::new();
            if !reach_state(&mut run, &mut journal, from) {
                continue;
            }
            let before = journal.entries().len();
            let err = advance(&mut run, &mut journal, to.clone())
                .expect_err(&format!("expected illegal {from:?} -> {to:?}"));
            assert!(
                matches!(
                    err,
                    RunError::IllegalTransition { .. } | RunError::TerminalImmutable(_)
                ),
                "unexpected error for {from:?}->{to:?}: {err:?}"
            );
            assert_eq!(
                journal.entries().len(),
                before,
                "illegal transition must not journal"
            );
            assert_eq!(run.state(), *from);
        }
    }
}

fn reach_state(run: &mut Run, journal: &mut InMemoryRunJournal, target: &RunState) -> bool {
    match target {
        RunState::Draft => true,
        RunState::Queued => {
            force_state(run, journal, &[RunState::Queued]);
            true
        }
        RunState::Running => {
            force_state(run, journal, &[RunState::Queued, RunState::Running]);
            true
        }
        RunState::Interrupted => {
            force_state(
                run,
                journal,
                &[RunState::Queued, RunState::Running, RunState::Interrupted],
            );
            true
        }
        RunState::Recovering => {
            force_state(
                run,
                journal,
                &[
                    RunState::Queued,
                    RunState::Running,
                    RunState::Interrupted,
                    RunState::Recovering,
                ],
            );
            true
        }
        RunState::Completed => {
            force_state(
                run,
                journal,
                &[RunState::Queued, RunState::Running, RunState::Completed],
            );
            true
        }
        RunState::Failed => {
            force_state(
                run,
                journal,
                &[RunState::Queued, RunState::Running, RunState::Failed],
            );
            true
        }
        RunState::Cancelled => {
            force_state(
                run,
                journal,
                &[RunState::Queued, RunState::Running, RunState::Cancelled],
            );
            true
        }
        RunState::AwaitingApproval => {
            force_state(
                run,
                journal,
                &[
                    RunState::Queued,
                    RunState::Running,
                    RunState::AwaitingApproval,
                ],
            );
            true
        }
    }
}

#[test]
fn terminal_states_are_immutable() {
    for terminal in [RunState::Completed, RunState::Failed, RunState::Cancelled] {
        let mut run = draft_run();
        let mut journal = InMemoryRunJournal::new();
        force_state(
            &mut run,
            &mut journal,
            &[RunState::Queued, RunState::Running, terminal.clone()],
        );
        for to in [
            RunState::Draft,
            RunState::Queued,
            RunState::Running,
            RunState::AwaitingApproval,
            RunState::Completed,
            RunState::Failed,
            RunState::Cancelled,
            RunState::Interrupted,
            RunState::Recovering,
        ] {
            if to == terminal {
                continue;
            }
            let err = advance(&mut run, &mut journal, to).expect_err("terminal immutable");
            assert_eq!(err, RunError::TerminalImmutable(terminal.clone()));
            assert_eq!(run.state(), terminal);
        }
    }
}

#[test]
fn cancel_is_idempotent() {
    let mut run = draft_run();
    let mut journal = InMemoryRunJournal::new();
    force_state(
        &mut run,
        &mut journal,
        &[RunState::Queued, RunState::Running],
    );

    let first = run.request_cancel(&mut journal).expect("cancel");
    assert_eq!(first.intent, TransitionIntent::Cancel);
    assert_eq!(run.state(), RunState::Cancelled);
    let journaled = journal.entries().len();

    let second = run.request_cancel(&mut journal).expect("idempotent cancel");
    assert_eq!(second.intent, TransitionIntent::AlreadyCancelled);
    assert_eq!(run.state(), RunState::Cancelled);
    assert_eq!(
        journal.entries().len(),
        journaled,
        "idempotent cancel must not journal again"
    );
}

/// Journal entry must exist before the caller can observe / dispatch the side-effect intent.
#[test]
fn transition_is_journaled_before_side_effect_intent_is_returned() {
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let order_for_journal = order.clone();

    struct OrderJournal {
        order: Arc<Mutex<Vec<&'static str>>>,
        inner: InMemoryRunJournal,
    }

    impl RunJournal for OrderJournal {
        fn record_transition(
            &mut self,
            run_id: &RunId,
            sequence: u64,
            from: RunState,
            to: RunState,
        ) -> Result<(), RunError> {
            self.order.lock().expect("lock").push("journal");
            self.inner.record_transition(run_id, sequence, from, to)
        }
    }

    let mut run = draft_run();
    let mut journal = OrderJournal {
        order: order_for_journal,
        inner: InMemoryRunJournal::new(),
    };

    let outcome = advance(&mut run, &mut journal, RunState::Queued).expect("queue");
    // Side effect is only "dispatched" after the outcome is in hand:
    order.lock().expect("lock").push("side_effect");
    let _ = outcome.intent;

    assert_eq!(
        order.lock().expect("lock").as_slice(),
        ["journal", "side_effect"]
    );
    assert_eq!(journal.inner.entries().len(), 1);
}

#[test]
fn journal_failure_rolls_back_state() {
    struct FailingJournal;

    impl RunJournal for FailingJournal {
        fn record_transition(
            &mut self,
            _run_id: &RunId,
            _sequence: u64,
            _from: RunState,
            _to: RunState,
        ) -> Result<(), RunError> {
            Err(RunError::Journal("disk full"))
        }
    }

    let mut run = draft_run();
    let mut journal = FailingJournal;
    let err = advance(&mut run, &mut journal, RunState::Queued).expect_err("journal fails");
    assert_eq!(err, RunError::Journal("disk full"));
    assert_eq!(run.state(), RunState::Draft);
    assert_eq!(run.sequence(), 0);
}

#[test]
fn event_and_output_budgets_are_enforced() {
    let mut run = Run::new(
        RunId::from("run-budget"),
        ConversationId::from("conv-1"),
        ModelId::from("model-1"),
        RunBudgets {
            event_budget: 2,
            output_byte_budget: 8,
            wall_clock: Duration::from_secs(60),
        },
    );

    run.consume_event().expect("1");
    run.consume_event().expect("2");
    assert_eq!(run.consume_event(), Err(RunError::EventBudgetExhausted));
    assert_eq!(run.event_budget_remaining(), 0);

    run.consume_output_bytes(5).expect("5 bytes");
    assert_eq!(
        run.consume_output_bytes(4),
        Err(RunError::OutputByteBudgetExhausted)
    );
    run.consume_output_bytes(3).expect("exact remainder");
    assert_eq!(run.output_byte_budget_remaining(), 0);
}

#[test]
fn context_builder_respects_message_and_byte_budgets() {
    let mut ctx = ContextBuilder::new(2, 20);
    ctx.push(ContextMessage::new(ContextRole::System, "sys"))
        .expect("system");
    ctx.push(ContextMessage::new(ContextRole::User, "hello"))
        .expect("user");
    let before = ctx.clone().build();
    assert_eq!(
        ctx.push(ContextMessage::new(ContextRole::Assistant, "world!!")),
        Err(ContextError::MessageLimitExceeded)
    );
    let messages = ctx.build();
    assert_eq!(messages, before, "capacity failure may not evict history");
    assert!(messages.iter().map(|m| m.byte_len()).sum::<usize>() <= 20);

    let mut tight = ContextBuilder::new(4, 4);
    assert_eq!(
        tight.push(ContextMessage::new(ContextRole::User, "toolong")),
        Err(ContextError::ByteLimitExceeded)
    );
    assert_eq!(
        tight.push(ContextMessage::new(ContextRole::User, "")),
        Err(ContextError::EmptyContent)
    );
}

/// Deterministic exhaustive/random-seed walk: no escape from terminal; journal precedes intent.
#[test]
fn property_transition_sequences_never_escape_terminal_and_journal_first() {
    const SEED: u64 = 0x2026_0728;
    let mut rng = Lcg::new(SEED);

    let candidates = [
        RunState::Draft,
        RunState::Queued,
        RunState::Running,
        RunState::AwaitingApproval,
        RunState::Completed,
        RunState::Failed,
        RunState::Cancelled,
        RunState::Interrupted,
        RunState::Recovering,
    ];

    for _ in 0..500 {
        let mut run = draft_run();
        let mut journal = InMemoryRunJournal::new();
        let mut reached_terminal = false;

        for _ in 0..24 {
            let to = candidates[rng.next_usize(candidates.len())].clone();
            let before_entries = journal.entries().len();
            let before_state = run.state();

            match run.transition(to.clone(), &mut journal) {
                Ok(outcome) => {
                    assert_eq!(
                        journal.entries().len(),
                        before_entries + 1,
                        "successful transition must journal exactly once before intent use"
                    );
                    let _dispatch = outcome.intent;
                    if matches!(
                        outcome.state,
                        RunState::Completed | RunState::Failed | RunState::Cancelled
                    ) {
                        reached_terminal = true;
                    }
                }
                Err(RunError::TerminalImmutable(state)) => {
                    assert!(
                        reached_terminal
                            || matches!(
                                before_state,
                                RunState::Completed | RunState::Failed | RunState::Cancelled
                            )
                    );
                    assert_eq!(state, before_state);
                    assert_eq!(journal.entries().len(), before_entries);
                    assert_eq!(run.state(), before_state);
                }
                Err(RunError::IllegalTransition { .. }) => {
                    assert_eq!(journal.entries().len(), before_entries);
                    assert_eq!(run.state(), before_state);
                }
                Err(other) => panic!("unexpected error: {other:?}"),
            }

            if reached_terminal {
                // Further transitions must not escape.
                let escape = run.transition(RunState::Running, &mut journal);
                assert!(
                    matches!(
                        escape,
                        Err(RunError::TerminalImmutable(_))
                            | Err(RunError::IllegalTransition { .. })
                    ),
                    "escaped terminal: {escape:?}"
                );
                assert!(matches!(
                    run.state(),
                    RunState::Completed | RunState::Failed | RunState::Cancelled
                ));
                break;
            }
        }
    }
}

/// Tiny deterministic LCG — no new test deps.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self {
            state: seed | 1, // avoid zero period
        }
    }

    fn next_u64(&mut self) -> u64 {
        // Numerical Recipes LCG
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        self.state
    }

    fn next_usize(&mut self, modulo: usize) -> usize {
        (self.next_u64() as usize) % modulo.max(1)
    }
}
