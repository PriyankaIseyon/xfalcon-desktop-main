use async_trait::async_trait;
use xfalcon_contracts::ConversationId;

use crate::security::account_cleanup::ValidatedAccountId;

use super::domain::{
    ChatBootstrap, ChatError, ChatProjectionEvent, ChatSnapshot, Conversation, Message,
    NewConversation, NewMessage, PersistedRunEvent, RunRecord, WorkspaceChatBootstrap,
};

#[async_trait]
pub trait CloudChatRepository: Send + Sync {
    async fn workspace_bootstrap(&self) -> Result<WorkspaceChatBootstrap, ChatError>;

    async fn conversation_bootstrap(&self, id: ConversationId) -> Result<ChatBootstrap, ChatError>;

    async fn create_conversation(&self, input: NewConversation) -> Result<Conversation, ChatError>;

    async fn append_message(&self, input: NewMessage) -> Result<Message, ChatError>;

    async fn record_run_event(&self, event: PersistedRunEvent) -> Result<(), ChatError>;
}

#[async_trait]
pub trait LocalChatProjection: Send + Sync {
    fn account_id(&self) -> &ValidatedAccountId;

    async fn load_snapshot(&self, id: ConversationId) -> Result<Option<ChatSnapshot>, ChatError>;

    /// Returns `None` for a deleted or missing conversation, and for a cache row
    /// that cannot be decrypted, as `load_snapshot` does. Otherwise returns the
    /// runs `load_snapshot` would carry, in the same order. Unlike
    /// `load_snapshot`, it never opens the messages or the draft, so a
    /// conversation whose messages or draft fail to decrypt still gets its runs
    /// here. That is accepted: such a conversation is reconciled from the cloud,
    /// which replaces the broken cache row.
    async fn load_runs(&self, id: ConversationId) -> Result<Option<Vec<RunRecord>>, ChatError>;

    async fn deleted_conversations(
        &self,
    ) -> Result<std::collections::HashSet<ConversationId>, ChatError>;

    async fn apply(&self, event: ChatProjectionEvent) -> Result<(), ChatError>;

    async fn purge_account(&self, account: ValidatedAccountId) -> Result<(), ChatError>;
}
