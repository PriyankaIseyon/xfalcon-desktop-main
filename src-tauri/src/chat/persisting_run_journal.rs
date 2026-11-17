use tauri::{AppHandle, Runtime};
use xfalcon_contracts::{RunId, RunState};

use crate::agent::run::{RunError, RunJournal};

use super::run_emit::EmittingRunJournal;
use super::run_journal_store::RunJournalStore;

pub struct PersistingRunJournal<J> {
    inner: J,
    store: RunJournalStore,
}

impl<J> PersistingRunJournal<J> {
    pub fn with_inner(inner: J, store: RunJournalStore) -> Self {
        Self { inner, store }
    }
}

pub type PersistingEmittingRunJournal<R> = PersistingRunJournal<EmittingRunJournal<R>>;

impl<R: Runtime> PersistingRunJournal<EmittingRunJournal<R>> {
    pub fn new(app: AppHandle<R>, store: RunJournalStore, run_id: RunId) -> Self {
        Self::with_inner(EmittingRunJournal::new(app, run_id), store)
    }
}

impl<J: RunJournal> RunJournal for PersistingRunJournal<J> {
    fn record_transition(
        &mut self,
        run_id: &RunId,
        sequence: u64,
        from: RunState,
        to: RunState,
    ) -> Result<(), RunError> {
        if self
            .store
            .append_transition(run_id, sequence, from.clone(), to.clone())
            .is_err()
        {
            return Err(RunError::Journal("disk journal rejected the transition"));
        }
        self.inner.record_transition(run_id, sequence, from, to)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};

    use tempfile::tempdir;

    use super::*;
    use crate::security::account_cleanup::ValidatedAccountId;

    struct RecordingJournal {
        transitions: Arc<Mutex<Vec<(RunState, RunState)>>>,
    }

    impl RunJournal for RecordingJournal {
        fn record_transition(
            &mut self,
            _run_id: &RunId,
            _sequence: u64,
            from: RunState,
            to: RunState,
        ) -> Result<(), RunError> {
            self.transitions
                .lock()
                .expect("recording journal lock")
                .push((from, to));
            Ok(())
        }
    }

    #[test]
    fn persistence_failure_prevents_downstream_side_effects() {
        let directory = tempdir().expect("tempdir");
        let invalid_root = directory.path().join("not-a-directory");
        fs::write(&invalid_root, b"file blocks journal directory").expect("sentinel");
        let account = ValidatedAccountId::parse("journal-ordering").expect("account");
        let transitions = Arc::new(Mutex::new(Vec::new()));
        let inner = RecordingJournal {
            transitions: transitions.clone(),
        };
        let store = RunJournalStore::new(invalid_root, account);
        let mut journal = PersistingRunJournal::with_inner(inner, store);

        let result = journal.record_transition(
            &RunId::from("run-ordering"),
            1,
            RunState::Draft,
            RunState::Queued,
        );

        assert!(matches!(result, Err(RunError::Journal(_))));
        assert!(transitions
            .lock()
            .expect("recording journal lock")
            .is_empty());
    }
}
