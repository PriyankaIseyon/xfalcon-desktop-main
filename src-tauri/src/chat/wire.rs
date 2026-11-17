//! Maps internal chat domain types to versioned wire contracts for the UI.

use std::collections::{HashMap, HashSet};

use xfalcon_contracts::chat::{
    ChatConversationSummary, ChatConversationView, ChatMessageRole, ChatMessageView,
    ChatSyncAvailability, ChatToolCallStatus, ChatToolCallView, ChatWorkspaceBootstrap,
};
use xfalcon_contracts::{ConversationId, RunId};

use crate::agent::recovery::{
    pick_strongest_recovery_notice, startup_recovery_for_states, StartupRunRef, StartupRunState,
};
use crate::projects::ProjectId;

use super::domain::{
    ChatSnapshot, ConversationSummary, Message, MessageContent, MessageRole, RunRecord,
    ToolCallStatus,
};
use super::sync::SyncAvailability;

pub const STARTUP_RECOVERY_CONVERSATION_LIMIT: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRecoverySnapshot {
    pub snapshot: ChatSnapshot,
    pub active_run_id: Option<RunId>,
    /// Runs THIS process is driving right now.
    ///
    /// Startup recovery answers one question: did a process die holding a run?
    /// A run this process still drives is alive, so it is not evidence of that
    /// and must not raise the banner. Without this the bootstrap that follows a
    /// renderer reload -- a hot reload in development, a re-auth, a reconnect --
    /// reports the run streaming behind it as interrupted, while the backend
    /// goes on and completes it.
    pub live_run_ids: HashSet<String>,
    /// Runs ANOTHER client is driving right now.
    ///
    /// The same question, answered for the other direction. An in-flight run
    /// this device holds no journal entry for was started somewhere else --
    /// xFalcon Web executes its own runs against these conversations -- so it
    /// is not evidence that a process died here either. Kept separate from
    /// [`Self::live_run_ids`] because that set means "this process drives it",
    /// and a run in this one is precisely the opposite.
    pub foreign_run_ids: HashSet<String>,
}

pub fn workspace_bootstrap_from(
    summaries: Vec<ConversationSummary>,
    recovery_notice: Option<String>,
    bindings: &HashMap<String, ProjectId>,
) -> ChatWorkspaceBootstrap {
    ChatWorkspaceBootstrap::with_recovery_notice(
        summaries
            .into_iter()
            .map(|summary| {
                let project_id = bindings
                    .get(summary.id().as_str())
                    .map(|project_id| project_id.to_string().into());
                ChatConversationSummary::new(
                    summary.id().clone(),
                    summary.title().to_owned(),
                    summary.updated_at().to_owned(),
                    summary.pinned_at().map(str::to_owned),
                )
                .with_project_id(project_id)
            })
            .collect(),
        recovery_notice,
    )
}

pub fn startup_recovery_hint_from_snapshot(
    snapshot: &ChatSnapshot,
    active_run_id: Option<&RunId>,
    live_run_ids: &HashSet<String>,
    foreign_run_ids: &HashSet<String>,
) -> Option<String> {
    let runs: Vec<StartupRunRef<'_>> = snapshot
        .runs()
        .iter()
        .filter(|run| !live_run_ids.contains(run.run_id().as_str()))
        // Filtered out of the run set, not merely excluded as the active run:
        // `startup_recovery_for_states` answers "some run is in flight and
        // none is active" with an unconditional notice, so leaving a foreign
        // run in the set raises a crash banner for another client's healthy
        // answer.
        .filter(|run| !foreign_run_ids.contains(run.run_id().as_str()))
        .map(|run| StartupRunRef {
            run_id: run.run_id(),
            state: StartupRunState {
                state: run.state().clone(),
                has_terminal_provider_output: false,
                da_commit_known: false,
            },
        })
        .collect();
    startup_recovery_for_states(&runs, active_run_id)
}

/// Recovery copy belongs only to the conversation currently open.
#[must_use]
pub fn startup_recovery_from_workspace(
    entries: &[WorkspaceRecoverySnapshot],
    active_conversation: Option<&ConversationId>,
) -> Option<String> {
    let active_conversation = active_conversation?;
    pick_strongest_recovery_notice(
        entries
            .iter()
            .filter(|entry| entry.snapshot.conversation().id() == active_conversation)
            .map(|entry| {
                startup_recovery_hint_from_snapshot(
                    &entry.snapshot,
                    entry.active_run_id.as_ref(),
                    &entry.live_run_ids,
                    &entry.foreign_run_ids,
                )
            }),
    )
}

pub fn conversation_view_from(
    snapshot: ChatSnapshot,
    availability: SyncAvailability,
) -> ChatConversationView {
    let availability = match availability {
        SyncAvailability::Live => ChatSyncAvailability::Live,
        SyncAvailability::Stale => ChatSyncAvailability::Stale,
    };
    let active_run_id = snapshot
        .runs()
        .iter()
        .find(|run| is_active_run(run))
        .map(|run| run.run_id().clone());
    ChatConversationView::new(
        snapshot.conversation().id().clone(),
        snapshot.conversation().title().to_owned(),
        availability,
        snapshot.messages().iter().map(message_view_from).collect(),
        active_run_id,
    )
}

/// Drops an active-run pointer that names a run another client is driving.
///
/// [`conversation_view_from`] picks the first in-flight run as the view's
/// active run, which is right when this device executes it. This window has no
/// follower for another client's stream, so it will never receive an event for
/// that run: left in place the pointer holds the composer on a run that can
/// never report progress here. The DA row is untouched — the client that owns
/// the run finishes it, and its answer arrives on a later reconcile.
#[must_use]
pub fn without_foreign_active_run(
    mut view: ChatConversationView,
    foreign_run_ids: &HashSet<String>,
) -> ChatConversationView {
    if view
        .active_run_id
        .as_ref()
        .is_some_and(|run_id| foreign_run_ids.contains(run_id.as_str()))
    {
        view.active_run_id = None;
    }
    view
}

/// The run whose state this window may report: the most recent one another
/// client is not driving.
///
/// Reporting a foreign run's `Running` would start a spinner nothing on this
/// side ever ends.
#[must_use]
pub fn latest_owned_run_state(
    snapshot: &ChatSnapshot,
    foreign_run_ids: &HashSet<String>,
) -> Option<(RunId, xfalcon_contracts::RunState)> {
    snapshot
        .runs()
        .iter()
        .rev()
        .find(|run| !foreign_run_ids.contains(run.run_id().as_str()))
        .map(|run| (run.run_id().clone(), run.state().clone()))
}

pub(crate) fn is_active_run(run: &RunRecord) -> bool {
    matches!(
        run.state(),
        xfalcon_contracts::RunState::Draft
            | xfalcon_contracts::RunState::Queued
            | xfalcon_contracts::RunState::Running
            | xfalcon_contracts::RunState::Recovering
            | xfalcon_contracts::RunState::AwaitingApproval
            | xfalcon_contracts::RunState::Interrupted
    )
}

fn message_view_from(message: &Message) -> ChatMessageView {
    ChatMessageView::new(
        message.id().as_uuid().to_string(),
        match message.role() {
            MessageRole::User => ChatMessageRole::User,
            MessageRole::Assistant => ChatMessageRole::Assistant,
            MessageRole::System => ChatMessageRole::System,
            MessageRole::Tool => ChatMessageRole::Tool,
        },
        message_text(message),
        message.sequence(),
        message
            .tool_calls()
            .iter()
            .map(|call| {
                ChatToolCallView::new(
                    call.id.as_str(),
                    call.name.as_str(),
                    match call.status {
                        ToolCallStatus::Answered => ChatToolCallStatus::Answered,
                        ToolCallStatus::Unanswered => ChatToolCallStatus::Unanswered,
                    },
                )
                .with_step(call.step.clone())
                .with_arguments(call.arguments.clone(), call.arguments_truncated)
                .with_result(call.result.clone(), call.result_truncated)
            })
            .collect(),
    )
    .with_attachments(
        message
            .content()
            .iter()
            .filter_map(|part| match part {
                MessageContent::AttachmentRef {
                    attachment_id,
                    file_name,
                    mime_type,
                } => Some(xfalcon_contracts::chat::ChatAttachmentView {
                    attachment_id: attachment_id.as_uuid().to_string(),
                    file_name: file_name.clone(),
                    mime_type: mime_type.clone(),
                }),
                _ => None,
            })
            .collect(),
    )
}

fn message_text(message: &Message) -> String {
    message
        .content()
        .iter()
        .filter_map(|part| match part {
            MessageContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn projection_run_state_changed(
    conversation_id: ConversationId,
    run_id: RunId,
    state: xfalcon_contracts::RunState,
) -> xfalcon_contracts::chat::ChatProjectionEvent {
    xfalcon_contracts::chat::ChatProjectionEvent::RunStateChanged {
        version: xfalcon_contracts::CONTRACT_VERSION,
        conversation_id,
        run_id,
        state,
    }
}

pub fn projection_snapshot_replaced(
    view: ChatConversationView,
) -> xfalcon_contracts::chat::ChatProjectionEvent {
    xfalcon_contracts::chat::ChatProjectionEvent::SnapshotReplaced {
        version: xfalcon_contracts::CONTRACT_VERSION,
        conversation: view,
    }
}
