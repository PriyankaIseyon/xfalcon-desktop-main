//! Interrupted-run recovery policy (Task 8).

use xfalcon_contracts::RunState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryDecision {
    /// Safe to restart from journal without duplicating external side effects.
    Resume,
    /// Terminal or unsafe — surface failure, do not auto-restart.
    Fail,
    /// Needs user confirmation or reconcile before continuing.
    AwaitUser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryScenario {
    QueuedNotStarted,
    RunningAwaitingProvider,
    ProviderTerminalUncertainDa,
    DaCommittedLocalPending,
    UserCancelled,
}

pub fn decide(scenario: RecoveryScenario) -> RecoveryDecision {
    match scenario {
        RecoveryScenario::QueuedNotStarted => RecoveryDecision::Resume,
        RecoveryScenario::RunningAwaitingProvider => RecoveryDecision::AwaitUser,
        RecoveryScenario::ProviderTerminalUncertainDa => RecoveryDecision::AwaitUser,
        RecoveryScenario::DaCommittedLocalPending => RecoveryDecision::Resume,
        RecoveryScenario::UserCancelled => RecoveryDecision::Fail,
    }
}

pub fn decide_from_run_state(
    state: RunState,
    has_terminal_provider_output: bool,
    da_commit_known: bool,
) -> RecoveryDecision {
    match state {
        RunState::Queued | RunState::Draft => decide(RecoveryScenario::QueuedNotStarted),
        RunState::Running | RunState::Recovering => {
            if has_terminal_provider_output && !da_commit_known {
                decide(RecoveryScenario::ProviderTerminalUncertainDa)
            } else {
                decide(RecoveryScenario::RunningAwaitingProvider)
            }
        }
        RunState::Interrupted => RecoveryDecision::AwaitUser,
        RunState::Cancelled => decide(RecoveryScenario::UserCancelled),
        RunState::Completed => RecoveryDecision::Fail,
        RunState::Failed => RecoveryDecision::Fail,
        RunState::AwaitingApproval => RecoveryDecision::AwaitUser,
    }
}

pub fn policy_for_interrupted(state: RunState) -> RecoveryDecision {
    decide_from_run_state(state, false, false)
}

/// User-facing copy aligned with the chat reducer interrupted notice.
pub const STARTUP_RECOVERY_NOTICE_AWAIT_USER: &str =
    "This response was interrupted before it finished. You can retry when ready.";

pub const STARTUP_RECOVERY_NOTICE_RECONCILE: &str =
    "A previous response may still be syncing with the server. Open this conversation to reconcile before retrying.";

/// Centralized UI copy for interrupted runs (import in the web layer via `recoveryPolicy.ts`).
pub mod recover_interrupted_run_policy {
    use super::{policy_for_interrupted, RecoveryDecision, STARTUP_RECOVERY_NOTICE_AWAIT_USER};
    use xfalcon_contracts::RunState;

    pub use super::STARTUP_RECOVERY_NOTICE_AWAIT_USER as AWAIT_USER;
    pub use super::STARTUP_RECOVERY_NOTICE_RECONCILE as RECONCILE;

    #[must_use]
    pub fn notice_for_interrupted_run() -> &'static str {
        STARTUP_RECOVERY_NOTICE_AWAIT_USER
    }

    #[must_use]
    pub fn notice_for_run_state(state: RunState) -> Option<&'static str> {
        match policy_for_interrupted(state) {
            RecoveryDecision::AwaitUser => Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER),
            _ => None,
        }
    }

    #[must_use]
    pub fn notice_for_stream_terminal_state(state: RunState) -> Option<&'static str> {
        match state {
            RunState::Interrupted | RunState::Failed => Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER),
            _ => None,
        }
    }
}

#[must_use]
pub fn recovery_notice_strength(notice: &str) -> u8 {
    if notice == STARTUP_RECOVERY_NOTICE_RECONCILE {
        2
    } else if notice == STARTUP_RECOVERY_NOTICE_AWAIT_USER {
        1
    } else {
        0
    }
}

#[must_use]
pub fn pick_strongest_recovery_notice(
    notices: impl IntoIterator<Item = Option<String>>,
) -> Option<String> {
    notices.into_iter().flatten().max_by(|left, right| {
        recovery_notice_strength(left)
            .cmp(&recovery_notice_strength(right))
            .then_with(|| left.cmp(right))
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupRunState {
    pub state: RunState,
    pub has_terminal_provider_output: bool,
    pub da_commit_known: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupRunRef<'a> {
    pub run_id: &'a xfalcon_contracts::RunId,
    pub state: StartupRunState,
}

#[must_use]
pub fn is_in_flight_run_state(state: RunState) -> bool {
    matches!(
        state,
        RunState::Draft
            | RunState::Queued
            | RunState::Running
            | RunState::Recovering
            | RunState::AwaitingApproval
            | RunState::Interrupted
    )
}

fn startup_notice_for_run(run: &StartupRunState) -> Option<&'static str> {
    match decide_from_run_state(
        run.state.clone(),
        run.has_terminal_provider_output,
        run.da_commit_known,
    ) {
        RecoveryDecision::AwaitUser => {
            if matches!(run.state, RunState::Running | RunState::Recovering)
                && run.has_terminal_provider_output
                && !run.da_commit_known
            {
                Some(STARTUP_RECOVERY_NOTICE_RECONCILE)
            } else {
                Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
            }
        }
        _ => None,
    }
}

/// Returns a recovery banner hint when any in-flight run needs user attention.
#[must_use]
pub fn startup_recovery_for_states(
    runs: &[StartupRunRef<'_>],
    active_run_id: Option<&xfalcon_contracts::RunId>,
) -> Option<String> {
    let in_flight: Vec<&StartupRunRef<'_>> = runs
        .iter()
        .filter(|run| is_in_flight_run_state(run.state.state.clone()))
        .collect();
    if in_flight.is_empty() {
        return None;
    }

    if let Some(active) = active_run_id {
        if let Some(run) = in_flight
            .iter()
            .find(|run| run.run_id.as_str() == active.as_str())
        {
            return startup_notice_for_run(&run.state).map(str::to_owned);
        }
        return in_flight
            .iter()
            .find_map(|run| startup_notice_for_run(&run.state).map(str::to_owned));
    }

    Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER.to_owned())
}
