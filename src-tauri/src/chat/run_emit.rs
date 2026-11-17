use tauri::{AppHandle, Runtime};
use xfalcon_contracts::{RunEvent, RunId, RunState, CONTRACT_VERSION};

use crate::agent::run::{InMemoryRunJournal, RunError, RunJournal};
use crate::app::events::emit_chat_run_event;

pub struct EmittingRunJournal<R: Runtime> {
    inner: InMemoryRunJournal,
    app: AppHandle<R>,
    run_id: RunId,
}

impl<R: Runtime> EmittingRunJournal<R> {
    pub fn new(app: AppHandle<R>, run_id: RunId) -> Self {
        Self {
            inner: InMemoryRunJournal::new(),
            app,
            run_id,
        }
    }
}

impl<R: Runtime> RunJournal for EmittingRunJournal<R> {
    fn record_transition(
        &mut self,
        run_id: &RunId,
        sequence: u64,
        from: RunState,
        to: RunState,
    ) -> Result<(), RunError> {
        self.inner
            .record_transition(run_id, sequence, from, to.clone())?;
        let event = RunEvent::StateChanged {
            version: CONTRACT_VERSION,
            run_id: self.run_id.clone(),
            state: to,
        };
        let _ = emit_chat_run_event(&self.app, &event);
        Ok(())
    }
}
