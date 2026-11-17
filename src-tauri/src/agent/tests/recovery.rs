use xfalcon_contracts::{RunId, RunState};

use crate::agent::recovery::{
    decide, decide_from_run_state, pick_strongest_recovery_notice, policy_for_interrupted,
    startup_recovery_for_states, RecoveryDecision, RecoveryScenario, StartupRunRef,
    StartupRunState, STARTUP_RECOVERY_NOTICE_AWAIT_USER, STARTUP_RECOVERY_NOTICE_RECONCILE,
};

#[test]
fn recovery_matrix_matches_plan_scenarios() {
    assert_eq!(
        decide(RecoveryScenario::QueuedNotStarted),
        RecoveryDecision::Resume
    );
    assert_eq!(
        decide(RecoveryScenario::RunningAwaitingProvider),
        RecoveryDecision::AwaitUser
    );
    assert_eq!(
        decide(RecoveryScenario::ProviderTerminalUncertainDa),
        RecoveryDecision::AwaitUser
    );
    assert_eq!(
        decide(RecoveryScenario::DaCommittedLocalPending),
        RecoveryDecision::Resume
    );
    assert_eq!(
        decide(RecoveryScenario::UserCancelled),
        RecoveryDecision::Fail
    );
}

#[test]
fn interrupted_and_recovering_runs_await_user_action() {
    assert_eq!(
        policy_for_interrupted(RunState::Interrupted),
        RecoveryDecision::AwaitUser
    );
    assert_eq!(
        decide_from_run_state(RunState::Recovering, false, false),
        RecoveryDecision::AwaitUser
    );
}

#[test]
fn terminal_provider_without_da_commit_awaits_reconcile() {
    assert_eq!(
        decide_from_run_state(RunState::Running, true, false),
        RecoveryDecision::AwaitUser
    );
}

#[test]
fn completed_runs_do_not_resume() {
    assert_eq!(
        decide_from_run_state(RunState::Completed, true, true),
        RecoveryDecision::Fail
    );
}

#[test]
fn crash_scenarios_startup_recovery_table() {
    let run_queued = RunId::from("run-queued");
    let run_running = RunId::from("run-running");
    let run_reconcile = RunId::from("run-reconcile");
    let run_cancelled = RunId::from("run-cancelled");

    let queued = [StartupRunRef {
        run_id: &run_queued,
        state: StartupRunState {
            state: RunState::Queued,
            has_terminal_provider_output: false,
            da_commit_known: false,
        },
    }];
    assert_eq!(
        startup_recovery_for_states(&queued, Some(&run_queued)).as_deref(),
        None,
        "queued safe resume"
    );

    let running = [StartupRunRef {
        run_id: &run_running,
        state: StartupRunState {
            state: RunState::Running,
            has_terminal_provider_output: false,
            da_commit_known: false,
        },
    }];
    assert_eq!(
        startup_recovery_for_states(&running, Some(&run_running)).as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER),
        "running awaiting provider"
    );

    let reconcile = [StartupRunRef {
        run_id: &run_reconcile,
        state: StartupRunState {
            state: RunState::Running,
            has_terminal_provider_output: true,
            da_commit_known: false,
        },
    }];
    assert_eq!(
        startup_recovery_for_states(&reconcile, Some(&run_reconcile)).as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_RECONCILE),
        "running with terminal provider"
    );

    let cancelled = [StartupRunRef {
        run_id: &run_cancelled,
        state: StartupRunState {
            state: RunState::Cancelled,
            has_terminal_provider_output: false,
            da_commit_known: false,
        },
    }];
    assert_eq!(
        startup_recovery_for_states(&cancelled, Some(&run_cancelled)).as_deref(),
        None,
        "cancelled is not in flight"
    );
}

#[test]
fn startup_recovery_without_active_run_id_uses_conservative_notice() {
    let run_running = RunId::from("run-running");
    let runs = [StartupRunRef {
        run_id: &run_running,
        state: StartupRunState {
            state: RunState::Running,
            has_terminal_provider_output: false,
            da_commit_known: false,
        },
    }];
    assert_eq!(
        startup_recovery_for_states(&runs, None).as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
    );
}

#[test]
fn pick_strongest_recovery_notice_prefers_reconcile_copy() {
    assert_eq!(
        pick_strongest_recovery_notice([
            Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER.to_owned()),
            Some(STARTUP_RECOVERY_NOTICE_RECONCILE.to_owned()),
        ])
        .as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_RECONCILE)
    );
}
