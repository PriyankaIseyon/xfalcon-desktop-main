//! Tool-invocation journal that also streams every entry to the frontend.
//!
//! Mirrors [`super::run_emit::EmittingRunJournal`] exactly one layer down:
//! record into the real journal first, emit only if that succeeded. A UI that
//! showed a step the journal rejected would be describing history that does
//! not exist.
//!
//! # Durability
//!
//! The inner journal is in-memory, so a run's invocation history does not
//! survive process restart. That is sufficient *today* and stops being
//! sufficient the moment a `xfalcon_contracts::tools::ToolRisk::Mutating`
//! tool enters the registry: `crate::agent::tool_loop::ToolLoop` pauses a run
//! at `AwaitingApproval` only for such a tool, and resuming that pause after
//! a restart is what `crate::agent::tool_loop::last_journaled_status` exists
//! to reconstruct. Every internal tool
//! (`crate::tools::registry::project_read_tool_descriptors` and
//! `project_propose_patch_tool_descriptor`) is `ReadOnly` — `propose_patch`
//! deliberately so, because it only *plans* a mutation and the real gate is
//! `crate::approvals::ApprovalsService::resolve`, whose state *is* persisted
//! and encrypted. So no run can currently reach a pause that a restart would
//! strand. Adding the first mutating tool must come with a persistent
//! invocation store.

use tauri::{AppHandle, Runtime};
use xfalcon_contracts::tools::{InvocationId, ToolInvocationEvent};

use crate::app::events::emit_tool_invocation_event;
use crate::tools::invocation::{
    InMemoryInvocationJournal, InvocationJournal, InvocationJournalError,
};

pub struct EmittingInvocationJournal<R: Runtime> {
    inner: InMemoryInvocationJournal,
    app: AppHandle<R>,
}

impl<R: Runtime> EmittingInvocationJournal<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            inner: InMemoryInvocationJournal::new(),
            app,
        }
    }
}

impl<R: Runtime> InvocationJournal for EmittingInvocationJournal<R> {
    fn record(
        &mut self,
        invocation_id: &InvocationId,
        sequence: u64,
        event: ToolInvocationEvent,
    ) -> Result<(), InvocationJournalError> {
        self.inner.record(invocation_id, sequence, event.clone())?;
        let _ = emit_tool_invocation_event(&self.app, &event);
        Ok(())
    }
}
