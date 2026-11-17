use std::sync::Arc;

use tauri::Manager;
use uuid::Uuid;

use crate::app::commands::AppRuntimeState;
use crate::audit::{emit, AuditAction, AuditOutcome, AuditResourceType};
use crate::uploads::service::UploadService;
use crate::work_barrier::WorkBarrier;
use xfalcon_contracts::chat::{
    ChatConversationView, ChatErrorCode, ChatMcpSelectionView, ChatModelCatalogView,
    ChatOperationError, ChatSubmitResult, ChatSubmitStatus, ChatWorkspaceBootstrap,
};
use xfalcon_contracts::custom_models::{
    CustomModelCreateInput, CustomModelDiscoveryInput, CustomModelDiscoveryResult,
    CustomModelUpdateInput, CustomModelView,
};
use xfalcon_contracts::mcp::McpServerId;
use xfalcon_contracts::ConversationId;

#[tauri::command]
pub async fn chat_workspace_bootstrap(
    app: tauri::AppHandle,
) -> Result<ChatWorkspaceBootstrap, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .workspace_bootstrap(Some(app.clone()))
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_model_catalog(
    app: tauri::AppHandle,
    conversation_id: Option<ConversationId>,
) -> Result<ChatModelCatalogView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .model_catalog(conversation_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_custom_model_list(
    app: tauri::AppHandle,
) -> Result<Vec<CustomModelView>, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .custom_model_list()
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_custom_model_discover(
    state: tauri::State<'_, AppRuntimeState>,
    input: CustomModelDiscoveryInput,
) -> Result<CustomModelDiscoveryResult, ChatOperationError> {
    state
        .chat()
        .custom_model_discover(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn chat_custom_model_create(
    app: tauri::AppHandle,
    input: CustomModelCreateInput,
) -> Result<CustomModelView, ChatOperationError> {
    let result = app
        .state::<AppRuntimeState>()
        .chat()
        .custom_model_create(input)
        .await;
    let model_id = result
        .as_ref()
        .map_or("custom_model", |view| custom_model_resource_id(&view.id));
    record_custom_model(
        &app,
        AuditAction::CredentialCreate,
        model_id,
        emit::outcome_for(&result),
    );
    result.map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_custom_model_update(
    app: tauri::AppHandle,
    id: String,
    input: CustomModelUpdateInput,
) -> Result<CustomModelView, ChatOperationError> {
    let result = app
        .state::<AppRuntimeState>()
        .chat()
        .custom_model_update(id.clone(), input)
        .await;
    record_custom_model(
        &app,
        AuditAction::CredentialReplace,
        custom_model_resource_id(&id),
        emit::outcome_for(&result),
    );
    result.map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_custom_model_delete(
    app: tauri::AppHandle,
    id: String,
) -> Result<(), ChatOperationError> {
    let result = app
        .state::<AppRuntimeState>()
        .chat()
        .custom_model_delete(id.clone())
        .await;
    record_custom_model(
        &app,
        AuditAction::CredentialDelete,
        custom_model_resource_id(&id),
        emit::outcome_for(&result),
    );
    result.map_err(ChatOperationError::from)
}

/// Records a custom-model write as a credential event: the model's own id,
/// or `custom_model` when the id is not a well-formed one.
fn record_custom_model<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: AuditAction,
    resource_id: &str,
    outcome: AuditOutcome,
) {
    emit::record_with_detail(
        app,
        action,
        AuditResourceType::Credential,
        resource_id,
        outcome,
        "custom_model",
    );
}

fn custom_model_resource_id(id: &str) -> &str {
    if Uuid::parse_str(id).is_ok() {
        id
    } else {
        "custom_model"
    }
}

#[tauri::command]
pub async fn chat_default_model_set(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ChatModelCatalogView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .default_model_set(model_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_create(
    app: tauri::AppHandle,
    opening_message: Option<String>,
    mcp_server_id: Option<McpServerId>,
) -> Result<ChatConversationView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_create(app.clone(), opening_message, mcp_server_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_mcp_selection_get(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
) -> Result<ChatMcpSelectionView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .mcp_selection_get(conversation_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_open(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
) -> Result<ChatConversationView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_open(app.clone(), conversation_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_preview(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
) -> Result<Option<ChatConversationView>, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_preview(conversation_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_search(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<ConversationId>, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_search(query)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_rename(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    title: String,
) -> Result<ChatConversationView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_rename(app.clone(), conversation_id, title)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_archive(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    archived: bool,
) -> Result<ChatConversationView, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_archive(app.clone(), conversation_id, archived)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_pin(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    pinned: bool,
) -> Result<ChatWorkspaceBootstrap, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_set_pinned(app.clone(), conversation_id, pinned)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_delete(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
) -> Result<(), ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .conversation_delete(conversation_id, Some(app.clone()))
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_conversation_reset_active(app: tauri::AppHandle) {
    app.state::<AppRuntimeState>()
        .chat()
        .reset_active_conversation();
}

#[tauri::command]
pub async fn chat_message_send(
    app: tauri::AppHandle,
    conversation_id: Option<ConversationId>,
    prompt: String,
    attachment_ids: Option<Vec<String>>,
    mcp_server_id: Option<McpServerId>,
) -> ChatSubmitResult {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let _admission = match barrier.admit().await {
        Ok(guard) => guard,
        Err(message) => return ChatSubmitResult::failed(message),
    };
    let parsed = match parse_attachment_ids(attachment_ids) {
        Ok(ids) => ids,
        Err(message) => return ChatSubmitResult::failed(message),
    };
    let result = app
        .state::<AppRuntimeState>()
        .chat()
        .message_send(
            conversation_id,
            prompt,
            Some(app.clone()),
            parsed.clone(),
            mcp_server_id,
        )
        .await;
    if result.status != ChatSubmitStatus::Failed && !parsed.is_empty() {
        let _ = app
            .state::<Arc<UploadService>>()
            .forget_by_resource_ids(&parsed)
            .await;
    }
    result
}

fn parse_attachment_ids(raw: Option<Vec<String>>) -> Result<Vec<Uuid>, &'static str> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if raw.len() > 64 {
        return Err("Too many attachments.");
    }
    let mut ids = Vec::with_capacity(raw.len());
    for value in raw {
        ids.push(Uuid::parse_str(value.trim()).map_err(|_| "An attachment could not be linked.")?);
    }
    Ok(ids)
}

#[tauri::command]
pub async fn chat_run_cancel(
    app: tauri::AppHandle,
    run_id: String,
) -> Result<(), ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .run_cancel(run_id, Some(&app))
        .await
        .map_err(ChatOperationError::from)
}

/// Approves or declines one tool call a run paused on.
///
/// `approve = true` mints an `crate::tools::policy::ApprovalToken` scoped to
/// exactly this invocation and tool inside the run's own `ToolLoop`, which
/// `PolicyEngine::authorize` then re-checks from scratch. There is no way to
/// approve anything else from here, and no way to approve at all for a run
/// that is not currently paused on this invocation.
#[tauri::command]
pub async fn chat_tool_approval_resolve(
    app: tauri::AppHandle,
    run_id: String,
    invocation_id: String,
    approve: bool,
) -> Result<(), ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .resolve_tool_approval(app.clone(), run_id, invocation_id, approve)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_run_retry(
    app: tauri::AppHandle,
    run_id: String,
    mcp_server_id: Option<McpServerId>,
) -> Result<xfalcon_contracts::chat::ChatSubmitResult, ChatOperationError> {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let _admission = barrier
        .admit()
        .await
        .map_err(|message| ChatOperationError::new(ChatErrorCode::Failed, message))?;
    app.state::<AppRuntimeState>()
        .chat()
        .run_retry(Some(app.clone()), run_id, mcp_server_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_message_edit(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    message_id: String,
    text: String,
    mcp_server_id: Option<McpServerId>,
) -> Result<xfalcon_contracts::chat::ChatSubmitResult, ChatOperationError> {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let _admission = barrier
        .admit()
        .await
        .map_err(|message| ChatOperationError::new(ChatErrorCode::Failed, message))?;
    app.state::<AppRuntimeState>()
        .chat()
        .message_edit(
            Some(app.clone()),
            conversation_id,
            message_id,
            text,
            mcp_server_id,
        )
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_message_regenerate(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    message_id: String,
    mcp_server_id: Option<McpServerId>,
) -> Result<xfalcon_contracts::chat::ChatSubmitResult, ChatOperationError> {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let _admission = barrier
        .admit()
        .await
        .map_err(|message| ChatOperationError::new(ChatErrorCode::Failed, message))?;
    app.state::<AppRuntimeState>()
        .chat()
        .message_regenerate(
            Some(app.clone()),
            conversation_id,
            message_id,
            mcp_server_id,
        )
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_message_feedback_list(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
) -> Result<Vec<xfalcon_contracts::chat::ChatMessageFeedback>, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .message_feedback_list(conversation_id)
        .await
        .map_err(ChatOperationError::from)
}

#[tauri::command]
pub async fn chat_message_feedback_set(
    app: tauri::AppHandle,
    conversation_id: ConversationId,
    message_id: Uuid,
    rating: Option<i32>,
) -> Result<Option<xfalcon_contracts::chat::ChatMessageFeedback>, ChatOperationError> {
    app.state::<AppRuntimeState>()
        .chat()
        .message_feedback_set(conversation_id, message_id, rating)
        .await
        .map_err(ChatOperationError::from)
}
