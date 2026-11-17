//! Pure replay decisions from journaled run transitions (Task 8).

use xfalcon_contracts::RunState;

use super::recovery::{decide_from_run_state, RecoveryDecision};
use super::run::JournalEntry;

/// Inspects the tail of a persisted journal without re-executing side effects.
#[must_use]
pub fn replay_policy_for_journal(entries: &[JournalEntry]) -> RecoveryDecision {
    let Some(last) = entries.last() else {
        return RecoveryDecision::Fail;
    };
    let has_terminal_provider = matches!(last.to, RunState::Completed | RunState::Failed);
    let da_commit_known = matches!(last.to, RunState::Completed);
    decide_from_run_state(last.to.clone(), has_terminal_provider, da_commit_known)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xfalcon_contracts::RunId;

    fn entry(from: RunState, to: RunState) -> JournalEntry {
        JournalEntry {
            run_id: RunId::from("run-replay"),
            sequence: 1,
            from,
            to,
        }
    }

    #[test]
    fn empty_journal_fails_closed() {
        assert_eq!(replay_policy_for_journal(&[]), RecoveryDecision::Fail);
    }

    #[test]
    fn interrupted_tail_awaits_user() {
        assert_eq!(
            replay_policy_for_journal(&[entry(RunState::Running, RunState::Interrupted)]),
            RecoveryDecision::AwaitUser
        );
    }

    #[test]
    fn completed_tail_does_not_resume() {
        assert_eq!(
            replay_policy_for_journal(&[entry(RunState::Running, RunState::Completed)]),
            RecoveryDecision::Fail
        );
    }
}
