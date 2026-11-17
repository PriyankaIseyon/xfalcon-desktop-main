//! Apply journaled run transitions after process restart (Task 8 / slice exit gate #4).

use xfalcon_contracts::RunState;

use crate::agent::journal_recovery::replay_policy_for_journal;
use crate::agent::recovery::{is_in_flight_run_state, RecoveryDecision};
use crate::agent::run::JournalEntry;

use super::domain::RunRecord;

/// When a persisted transition log exists and policy says the run cannot resume silently,
/// mark the projection run interrupted so retry does not duplicate assistant output.
#[must_use]
pub fn interrupted_run_patch_from_journal(
    entries: &[JournalEntry],
    run: &RunRecord,
) -> Option<RunRecord> {
    if entries.is_empty() || !is_in_flight_run_state(run.state().clone()) {
        return None;
    }
    if matches!(run.state(), RunState::Interrupted) {
        return None;
    }
    let decision = replay_policy_for_journal(entries);
    let should_interrupt = matches!(
        decision,
        RecoveryDecision::AwaitUser | RecoveryDecision::Resume
    );
    if !should_interrupt {
        return None;
    }
    RunRecord::try_new(
        run.run_id().clone(),
        run.conversation_id().clone(),
        RunState::Interrupted,
        run.sequence().saturating_add(1),
    )
    .ok()
}

/// Recovery captured before zombie finalization, scoped to this boot and account.
#[derive(Default)]
pub(crate) struct RecoveredRuns {
    runs: std::collections::HashMap<(String, String), xfalcon_contracts::RunId>,
}

impl RecoveredRuns {
    pub(crate) fn remember(
        &mut self,
        account: &str,
        conversation: &xfalcon_contracts::ConversationId,
        run: xfalcon_contracts::RunId,
    ) {
        self.runs
            .insert((account.to_owned(), conversation.as_str().to_owned()), run);
    }

    pub(crate) fn for_snapshot(
        &mut self,
        account: &str,
        snapshot: &super::domain::ChatSnapshot,
    ) -> Option<xfalcon_contracts::RunId> {
        let key = (
            account.to_owned(),
            snapshot.conversation().id().as_str().to_owned(),
        );
        let recovered = self.runs.get(&key)?;
        if snapshot.runs().last().is_some_and(|run| {
            run.run_id() == recovered
                && !matches!(run.state(), RunState::Completed | RunState::Failed)
        }) {
            return Some(recovered.clone());
        }
        // A newer turn or authoritative completion retires the old notice.
        self.runs.remove(&key);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use xfalcon_contracts::{ConversationId, RunId};

    fn run_record(state: RunState) -> RunRecord {
        RunRecord::try_new(
            RunId::from("run-cold"),
            ConversationId::from(Uuid::new_v4().to_string()),
            state,
            1,
        )
        .expect("run")
    }

    fn entry(from: RunState, to: RunState) -> JournalEntry {
        JournalEntry {
            run_id: RunId::from("run-cold"),
            sequence: 1,
            from,
            to,
        }
    }

    #[test]
    fn running_tail_after_crash_becomes_interrupted_patch() {
        let run = run_record(RunState::Running);
        let patch =
            interrupted_run_patch_from_journal(&[entry(RunState::Queued, RunState::Running)], &run)
                .expect("patch");
        assert_eq!(patch.state(), &RunState::Interrupted);
    }

    #[test]
    fn completed_journal_does_not_patch() {
        let run = run_record(RunState::Running);
        assert!(interrupted_run_patch_from_journal(
            &[entry(RunState::Running, RunState::Completed)],
            &run,
        )
        .is_none());
    }

    /// Spec: chat-run-recovery — "kill between DA finalize and local ack".
    /// Boot reconcile adopts DA's terminal state into the run record, so a
    /// stale non-terminal journal tail must never re-interrupt it (an
    /// Interrupted patch here is what used to invite the duplicate-writing
    /// retry).
    #[test]
    fn da_terminal_run_is_never_patched_despite_stale_journal_tail() {
        for state in [RunState::Completed, RunState::Failed, RunState::Cancelled] {
            let run = run_record(state);
            assert!(interrupted_run_patch_from_journal(
                &[entry(RunState::Queued, RunState::Running)],
                &run,
            )
            .is_none());
        }
    }
}
