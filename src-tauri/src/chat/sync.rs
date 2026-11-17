use std::sync::Arc;

use futures_util::stream::{self, StreamExt};
use xfalcon_contracts::ConversationId;

use crate::security::account_cleanup::ValidatedAccountId;

use super::domain::{
    ChatError, ChatProjectionEvent, ChatSnapshot, RunRecord, WorkspaceChatBootstrap,
};
use super::repository::{CloudChatRepository, LocalChatProjection};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncAvailability {
    Live,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciledConversation {
    pub snapshot: ChatSnapshot,
    pub availability: SyncAvailability,
}

pub struct ChatSyncService<C, L> {
    cloud: Arc<C>,
    local: L,
    account: ValidatedAccountId,
}

impl<C, L> ChatSyncService<C, L>
where
    C: CloudChatRepository,
    L: LocalChatProjection,
{
    pub fn new(cloud: Arc<C>, local: L, account: ValidatedAccountId) -> Self {
        Self {
            cloud,
            local,
            account,
        }
    }

    pub fn account(&self) -> &ValidatedAccountId {
        &self.account
    }

    pub async fn peek_local(&self, id: ConversationId) -> Result<Option<ChatSnapshot>, ChatError> {
        self.ensure_account_bound()?;
        self.local.load_snapshot(id).await
    }

    pub async fn peek_runs(&self, id: ConversationId) -> Result<Option<Vec<RunRecord>>, ChatError> {
        self.ensure_account_bound()?;
        self.local.load_runs(id).await
    }

    pub async fn workspace_bootstrap(&self) -> Result<WorkspaceChatBootstrap, ChatError> {
        self.ensure_account_bound()?;
        let workspace = self.cloud.workspace_bootstrap().await?;
        let deleted = self.local.deleted_conversations().await?;
        Ok(WorkspaceChatBootstrap::new(
            workspace
                .conversations()
                .iter()
                .filter(|row| !deleted.contains(row.id()))
                .cloned()
                .collect(),
            workspace.cursor().clone(),
        ))
    }

    pub async fn reconcile_conversation(
        &self,
        id: ConversationId,
    ) -> Result<ReconciledConversation, ChatError> {
        self.ensure_account_bound()?;
        let local_snapshot = self.local.load_snapshot(id.clone()).await?;
        let preserved_draft = local_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.draft().cloned());

        match self.cloud.conversation_bootstrap(id.clone()).await {
            Ok(bootstrap) => {
                let snapshot = ChatSnapshot::new(
                    bootstrap.conversation().clone(),
                    bootstrap.messages().to_vec(),
                    preserved_draft,
                    bootstrap.runs().to_vec(),
                    bootstrap.cursor().clone(),
                );
                self.local
                    .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot.clone()))
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, conversation = %id.as_str(), "reconcile: local snapshot write failed");
                        error
                    })?;
                Ok(ReconciledConversation {
                    snapshot,
                    availability: SyncAvailability::Live,
                })
            }
            Err(ChatError::Unauthorized) => {
                tracing::warn!(conversation = %id.as_str(), "reconcile: cloud bootstrap unauthorized");
                Err(ChatError::Unauthorized)
            }
            Err(error @ (ChatError::Unavailable | ChatError::NotFound)) => {
                tracing::warn!(%error, conversation = %id.as_str(), had_local_copy = local_snapshot.is_some(), "reconcile: cloud bootstrap failed; falling back to local copy");
                // No local copy: surface the ORIGINAL error class. NotFound is
                // the server's authoritative "this conversation does not
                // exist" and drives the caller's self-heal; collapsing it into
                // Unavailable made deletion indistinguishable from a network
                // blip (xfalcon-web keeps the same distinction in
                // `loadViaBootstrap`).
                if self.local.deleted_conversations().await?.contains(&id) {
                    return Err(ChatError::NotFound);
                }
                let snapshot = local_snapshot.ok_or(error)?;
                Ok(ReconciledConversation {
                    snapshot,
                    availability: SyncAvailability::Stale,
                })
            }
            Err(error) => {
                tracing::error!(%error, conversation = %id.as_str(), "reconcile: cloud bootstrap failed");
                Err(error)
            }
        }
    }

    /// Reconciles conversations with bounded overlapping cloud reads while
    /// preserving input order in the returned results.
    pub async fn reconcile_conversations(
        &self,
        ids: Vec<ConversationId>,
        concurrency: usize,
    ) -> Vec<(ConversationId, Result<ReconciledConversation, ChatError>)> {
        stream::iter(ids)
            .map(|id| async move {
                let result = self.reconcile_conversation(id.clone()).await;
                (id, result)
            })
            .buffered(concurrency.max(1))
            .collect()
            .await
    }

    pub async fn deleted_conversations(
        &self,
    ) -> Result<std::collections::HashSet<ConversationId>, ChatError> {
        self.ensure_account_bound()?;
        self.local.deleted_conversations().await
    }

    pub async fn apply_projection_event(
        &self,
        event: ChatProjectionEvent,
    ) -> Result<(), ChatError> {
        self.ensure_account_bound()?;
        match self.local.apply(event.clone()).await {
            Ok(()) => Ok(()),
            Err(ChatError::SequenceNotMonotonic) => {
                let conversation_id =
                    conversation_id_for_event(&event).ok_or(ChatError::SequenceNotMonotonic)?;
                self.reconcile_conversation(conversation_id).await?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn ensure_account_bound(&self) -> Result<(), ChatError> {
        if self.local.account_id().as_str() != self.account.as_str() {
            return Err(ChatError::Validation("account mismatch"));
        }
        Ok(())
    }
}

fn conversation_id_for_event(event: &ChatProjectionEvent) -> Option<ConversationId> {
    match event {
        ChatProjectionEvent::UpsertConversation(conversation) => Some(conversation.id().clone()),
        ChatProjectionEvent::AppendMessage(message) => Some(message.conversation_id().clone()),
        ChatProjectionEvent::UpsertDraft(draft) => Some(draft.conversation_id().clone()),
        ChatProjectionEvent::ClearDraft { conversation_id } => Some(conversation_id.clone()),
        ChatProjectionEvent::UpsertRun(run) => Some(run.conversation_id().clone()),
        ChatProjectionEvent::ReplaceSnapshot(snapshot) => {
            Some(snapshot.conversation().id().clone())
        }
        ChatProjectionEvent::AdvanceCursor(_) => None,
        ChatProjectionEvent::RemoveConversation { conversation_id } => {
            Some(conversation_id.clone())
        }
    }
}
