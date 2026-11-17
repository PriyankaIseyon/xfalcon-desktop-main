pub(crate) mod attachments;
#[cfg(test)]
pub(crate) use attachments::prompt_scan_attachment_text;
mod cold_start_recovery;
mod commands;
pub(crate) mod custom_model_settings;
pub mod da_repository;
pub mod domain;
pub(crate) mod follow_ups;
#[cfg(test)]
pub(crate) use follow_ups::prompt_scan_request as prompt_scan_follow_up_request;
pub(crate) mod history;
mod invocation_emit;
pub mod local_projection;
pub(crate) mod mcp_selection;
pub(crate) mod model_preference;
mod persisting_run_journal;
pub mod repository;
mod run_emit;
mod run_journal_store;
mod service;
pub mod sync;
mod title;
#[cfg(test)]
pub(crate) use title::model_title_request as prompt_scan_title_request;
mod tool_run;
mod wire;

pub use commands::{
    chat_conversation_archive, chat_conversation_create, chat_conversation_delete,
    chat_conversation_open, chat_conversation_pin, chat_conversation_preview,
    chat_conversation_rename, chat_conversation_reset_active, chat_conversation_search,
    chat_custom_model_create, chat_custom_model_delete, chat_custom_model_discover,
    chat_custom_model_list, chat_custom_model_update, chat_default_model_set,
    chat_mcp_selection_get, chat_message_edit, chat_message_feedback_list,
    chat_message_feedback_set, chat_message_regenerate, chat_message_send, chat_model_catalog,
    chat_run_cancel, chat_run_retry, chat_tool_approval_resolve, chat_workspace_bootstrap,
};
pub use da_repository::{ApprovalDaSync, DaChatRepository};
pub use follow_ups::chat_follow_ups;
pub use local_projection::{
    EncryptedChatProjection, CHAT_PROJECTION_DB_FILE, LOCAL_PROJECTIONS_DIR,
};
pub use service::ChatService;
pub use sync::{ChatSyncService, ReconciledConversation, SyncAvailability};
/// The run's wall-clock budget; transport ceilings are held to it.
pub(crate) use tool_run::RUN_WALL_CLOCK;

#[cfg(test)]
mod tests;
