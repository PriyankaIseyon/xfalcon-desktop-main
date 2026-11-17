mod feedback;

use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use futures_util::StreamExt as _;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;
use xfalcon_contracts::custom_models::{CustomModelCreateInput, CustomModelUpdateInput};
use zeroize::Zeroizing;

use crate::agent::context_budget::ContextCapacity;
use crate::agent::da_events::is_terminal_run_status;
use crate::tools::ToolResultBudget;

use super::generated::{
    BatchRecord, ChatAttachment, ChatAttachmentSignGetResponse, ChatErrorCode, ChatErrorEnvelope,
    ChatMessage, ContextCompaction, ContextCompactionActivate, ContextMessage, ContextMessageRole,
    ContextPage, ConversationBootstrap, ConversationCreate, ConversationCreateResult,
    CursorPageChatAttachment, CursorPageChatMessage, EventBatchRequest, EventBatchResult,
    MessageCreate, MessageInsertResult, ModelEntity, PayloadStatus, RunCreate, RunFinalize,
    RunFinalizeResult, ToolCall, ToolCallApproval, ToolCallCreate, ToolResultContentHandle,
    ToolResultPayload, ToolResultPayloadDescriptor, ToolResultPayloadFinalize,
    ToolResultPayloadPrepare, ToolResultPayloadPrepareResult, UploadInstruction,
    UploadInstructionMode, WorkspaceBootstrap,
};
use super::mapping::TenantBaseUrl;

/// `Copy` and comparable: every variant is a unit, and the datasets surface
/// asserts on the exact refusal it got.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(crate) enum DaChatApiError {
    #[error("not signed in")]
    Unauthorized,
    #[error("tenant chat API request failed")]
    RequestFailed,
    #[error("tenant chat API returned an unexpected response")]
    InvalidResponse,
    #[error("chat resource was not found")]
    NotFound,
    #[error("a competing terminal write already landed on this run")]
    Conflict,
    #[error("context changed before compaction activation")]
    ContextConflict,
    #[error("context snapshot became stale while paging")]
    ContextSnapshotStale,
    #[error("idempotency identity was reused with different content")]
    IdempotencyConflict,
    #[error("payload exceeds the supported exact-result size")]
    PayloadTooLarge,
    #[error("tool-result storage quota is exhausted")]
    StorageQuotaExceeded,
    #[error("tool-result storage is unavailable")]
    StorageUnavailable,
    #[error("tool-result payload is not ready")]
    PayloadNotReady,
    #[error("tool-result payload failed integrity validation")]
    PayloadIntegrityFailed,
    #[error("chat request failed validation")]
    Validation,
    #[error("chat request was rate limited")]
    RateLimited,
    #[error("chat request timed out")]
    Timeout,
    #[error("provider compaction format is unsupported")]
    UnsupportedCompactionFormat,
    #[error("no chat model is available for this tenant")]
    NoModel,
    #[error("the selected private model is no longer available")]
    CustomModelUnavailable,
    #[error("the selected private model has insufficient input capacity")]
    CustomModelCapacityUnsupported,
}

#[derive(Debug, Error)]
pub(crate) enum CustomModelApiError {
    #[error("Sign in again to manage private models.")]
    Unauthorized,
    #[error("Private models are disabled on this deployment.")]
    Forbidden,
    #[error("This private model was removed. Add it again if you still need it.")]
    Removed,
    #[error("The private model limit was reached, or this model can no longer be changed.")]
    Conflict,
    #[error("{0}")]
    Validation(String),
    #[error("Private model management is rate limited. Try again shortly.")]
    RateLimited,
    #[error("Could not contact the model catalog. Try again shortly.")]
    Unavailable,
    #[error("The model catalog returned an unexpected response.")]
    InvalidResponse,
}

#[derive(Serialize)]
struct CustomModelCreateBody<'a> {
    base_url: &'a str,
    upstream_model_id: &'a str,
    secret: &'a str,
    display_name: &'a str,
    context_length: i64,
    max_output_tokens: i64,
    supports_tools: bool,
    supports_vision: bool,
}

#[derive(Serialize)]
struct CustomModelUpdateBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_model_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_length: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_vision: Option<bool>,
}

const CONTEXT_PAGE_LIMIT: usize = 200;
/// DA's own page ceiling for chat attachments.
const ATTACHMENT_PAGE_LIMIT: usize = 200;
/// Pages read before a conversation's attachment list is called too long to
/// load: 2,000 files, far past anything a person attaches to one chat.
const MAX_ATTACHMENT_PAGES: usize = 10;
const MAX_CONTEXT_STALE_RESTARTS: usize = 2;
const MAX_MODEL_PAGES: usize = 32;
const MODEL_PAGE_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const MAX_TOOL_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// The largest range any run's reader asks for: the ceiling budget's range.
/// DA itself streams ranges of any length.
pub(crate) const MAX_TOOL_RESULT_RANGE_BYTES: usize = ToolResultBudget::ceiling().range_bytes();

#[derive(Clone)]
pub(crate) struct ContextSnapshot {
    pub(crate) active_compaction: Option<ContextCompaction>,
    pub(crate) history_revision: i64,
    pub(crate) snapshot_head_sequence_no: i64,
    pub(crate) boundary_sequence_no: i64,
    pub(crate) compactable_through_sequence_no: i64,
    pub(crate) compactable_source_message_count: i64,
    pub(crate) source_message_ids_sha256: String,
    pub(crate) messages: Vec<ContextMessage>,
}

pub(crate) struct ToolResultRange {
    pub(crate) bytes: Vec<u8>,
    pub(crate) start: usize,
    pub(crate) end_inclusive: usize,
    pub(crate) total_byte_length: usize,
    pub(crate) sha256: String,
}

#[derive(Clone)]
pub(crate) struct DaChatSession {
    base: TenantBaseUrl,
    client: reqwest::Client,
    bearer: Zeroizing<String>,
}

impl DaChatSession {
    /// Builds a session on a client the caller owns. A `reqwest::Client` pools
    /// its connections, so the service that owns `client` passes a clone of it
    /// to every session it builds and those sessions share one warm connection
    /// to the tenant. The client carries no credentials; the bearer is applied
    /// per request.
    pub(crate) fn with_client(
        da_origin: &str,
        bearer: impl Into<String>,
        client: reqwest::Client,
    ) -> Result<Self, DaChatApiError> {
        let base = TenantBaseUrl::parse(da_origin).map_err(|_| DaChatApiError::RequestFailed)?;
        Ok(Self {
            base,
            client,
            bearer: Zeroizing::new(bearer.into()),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        loopback_origin: &str,
        bearer: impl Into<String>,
    ) -> Result<Self, DaChatApiError> {
        let client =
            super::mapping::build_hardened_client().map_err(|_| DaChatApiError::RequestFailed)?;
        Self::for_test_with_client(loopback_origin, bearer, client)
    }

    #[cfg(test)]
    pub(crate) fn for_test_with_client(
        loopback_origin: &str,
        bearer: impl Into<String>,
        client: reqwest::Client,
    ) -> Result<Self, DaChatApiError> {
        let base = TenantBaseUrl::parse_loopback(loopback_origin)
            .map_err(|_| DaChatApiError::RequestFailed)?;
        Ok(Self {
            base,
            client,
            bearer: Zeroizing::new(bearer.into()),
        })
    }

    fn url(&self, path: &str) -> Url {
        self.base.url_with_path(path)
    }

    pub(crate) async fn workspace_bootstrap(&self) -> Result<WorkspaceBootstrap, DaChatApiError> {
        let response = self
            .client
            .get(self.url("/api/v1/chat/bootstrap/workspace"))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if !response.status().is_success() {
            if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
                return Err(DaChatApiError::Unauthorized);
            }
            return Err(DaChatApiError::RequestFailed);
        }
        response
            .json()
            .await
            .map_err(|_| DaChatApiError::InvalidResponse)
    }

    pub(crate) async fn list_models(&self) -> Result<Vec<ModelEntity>, DaChatApiError> {
        self.list_model_rows(None, true, true).await
    }

    /// Settings needs disabled rows with a live key too. DA's picker route
    /// defaults both filters to true, so query each state explicitly and leave
    /// removed rows (whose key was revoked) out of the management surface.
    pub(crate) async fn list_custom_models_for_management(
        &self,
    ) -> Result<Vec<ModelEntity>, CustomModelApiError> {
        let mut models = Vec::new();
        let mut by_id = HashMap::new();
        for enabled in [true, false] {
            for active in [true, false] {
                let rows = self
                    .list_model_rows(Some("custom_openai"), enabled, active)
                    .await
                    .map_err(|error| match error {
                        DaChatApiError::Unauthorized => CustomModelApiError::Unauthorized,
                        DaChatApiError::InvalidResponse => CustomModelApiError::InvalidResponse,
                        _ => CustomModelApiError::Unavailable,
                    })?;
                for row in rows.into_iter().filter(|row| row.provider_key_id.is_some()) {
                    if let Some(&index) = by_id.get(&row.id) {
                        // A model may change state between the four queries.
                        // Keep the later snapshot and one management card.
                        models[index] = row;
                    } else {
                        by_id.insert(row.id, models.len());
                        models.push(row);
                    }
                }
            }
        }
        Ok(models)
    }

    async fn list_model_rows(
        &self,
        provider: Option<&str>,
        enabled: bool,
        active: bool,
    ) -> Result<Vec<ModelEntity>, DaChatApiError> {
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut seen_models = HashSet::new();
        for _ in 0..MAX_MODEL_PAGES {
            let mut url = self.url("/api/v1/chat/models");
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("limit", "200");
                query.append_pair("is_enabled", if enabled { "true" } else { "false" });
                query.append_pair("is_active", if active { "true" } else { "false" });
                if let Some(provider) = provider {
                    query.append_pair("provider", provider);
                }
                if let Some(cursor) = cursor.as_deref() {
                    query.append_pair("cursor", cursor);
                }
            }
            let response = self
                .client
                .get(url)
                .bearer_auth(self.bearer.as_str())
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            if !response.status().is_success() {
                return Err(match response.status().as_u16() {
                    401 | 403 => DaChatApiError::Unauthorized,
                    _ => DaChatApiError::RequestFailed,
                });
            }
            let body = read_bounded_body(response, MODEL_PAGE_BYTES)
                .await
                .map_err(|_| DaChatApiError::InvalidResponse)?;
            let page: CursorPage<ModelEntity> =
                serde_json::from_slice(&body).map_err(|_| DaChatApiError::InvalidResponse)?;
            for model in page.items.ok_or(DaChatApiError::InvalidResponse)? {
                if !seen_models.insert(model.id) {
                    return Err(DaChatApiError::InvalidResponse);
                }
                // A plan model is built on this device and never sent by DA. A
                // row that names one is dropped here, before any selection reads
                // it, so DA cannot start a local runtime or spend a plan.
                if is_plan_model_id(&model.model_id) {
                    tracing::warn!(
                        model_id = %model.model_id.trim(),
                        "DA model row names a device-local plan model; dropped"
                    );
                    continue;
                }
                models.push(model);
            }
            match page.next_cursor {
                Some(next) if !next.is_empty() && seen_cursors.insert(next.clone()) => {
                    cursor = Some(next);
                }
                Some(_) => return Err(DaChatApiError::InvalidResponse),
                None => return Ok(models),
            }
        }
        Err(DaChatApiError::InvalidResponse)
    }

    pub(crate) async fn create_custom_model(
        &self,
        input: CustomModelCreateInput,
    ) -> Result<ModelEntity, CustomModelApiError> {
        let secret = Zeroizing::new(input.secret);
        let body = CustomModelCreateBody {
            base_url: &input.base_url,
            upstream_model_id: &input.upstream_model_id,
            secret: secret.as_str(),
            display_name: &input.display_name,
            context_length: input.context_length,
            max_output_tokens: input.max_output_tokens,
            supports_tools: input.supports_tools,
            supports_vision: input.supports_vision,
        };
        let response = self
            .client
            .post(self.url("/api/v1/chat/custom-models"))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| CustomModelApiError::Unavailable)?;
        read_custom_model_response(response, 201, Some(secret.as_str()), Some(&input.base_url))
            .await
    }

    pub(crate) async fn update_custom_model(
        &self,
        id: Uuid,
        input: CustomModelUpdateInput,
    ) -> Result<ModelEntity, CustomModelApiError> {
        let secret = input.secret.map(Zeroizing::new);
        let body = CustomModelUpdateBody {
            base_url: input.base_url.as_deref(),
            secret: secret.as_deref().map(String::as_str),
            is_enabled: input.is_enabled,
            is_active: input.is_active,
            upstream_model_id: input.upstream_model_id.as_deref(),
            display_name: input.display_name.as_deref(),
            context_length: input.context_length,
            max_output_tokens: input.max_output_tokens,
            supports_tools: input.supports_tools,
            supports_vision: input.supports_vision,
        };
        let response = self
            .client
            .patch(self.url(&format!("/api/v1/chat/custom-models/{id}")))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| CustomModelApiError::Unavailable)?;
        read_custom_model_response(
            response,
            200,
            secret.as_deref().map(String::as_str),
            input.base_url.as_deref(),
        )
        .await
    }

    pub(crate) async fn delete_custom_model(&self, id: Uuid) -> Result<(), CustomModelApiError> {
        let response = self
            .client
            .delete(self.url(&format!("/api/v1/chat/custom-models/{id}")))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| CustomModelApiError::Unavailable)?;
        let status = response.status().as_u16();
        let body = read_bounded_body(response, 64 * 1024)
            .await
            .map_err(|_| CustomModelApiError::InvalidResponse)?;
        if status == 200 {
            let result: super::generated::DeletionResult =
                serde_json::from_slice(&body).map_err(|_| CustomModelApiError::InvalidResponse)?;
            return (result.id == id)
                .then_some(())
                .ok_or(CustomModelApiError::InvalidResponse);
        }
        Err(custom_model_error(status, &body, None, None))
    }

    pub(crate) async fn create_conversation(
        &self,
        title: Option<&str>,
        model_id: Option<&str>,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
        auto_attach: bool,
    ) -> Result<ConversationCreateResult, DaChatApiError> {
        let body = ConversationCreate {
            title: title.map(str::to_owned),
            model: model_id.map(str::to_owned),
            system_prompt: None,
            agent_id: None,
            metadata: metadata.map(|metadata| metadata.into_iter().collect()),
            auto_attach: Some(auto_attach),
        };
        let response = self
            .client
            .post(self.url("/api/v1/chat/conversations"))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if response.status().as_u16() == 201 {
            return response
                .json()
                .await
                .map_err(|_| DaChatApiError::InvalidResponse);
        }
        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(DaChatApiError::Unauthorized);
        }
        Err(DaChatApiError::RequestFailed)
    }

    pub(crate) async fn create_user_message(
        &self,
        conversation_id: uuid::Uuid,
        content: &str,
    ) -> Result<(), DaChatApiError> {
        let body = MessageCreate {
            id: None,
            role: ContextMessageRole::User,
            content: Some(content.to_owned()),
            metadata: None,
            token_input: None,
            token_output: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            attachment_ids: None,
            tool_result_payload_ids: None,
        };
        self.insert_message(conversation_id, body).await
    }

    pub(crate) async fn create_run(
        &self,
        conversation_id: uuid::Uuid,
        model_id: &str,
        provider: &str,
        input_message: &str,
    ) -> Result<uuid::Uuid, DaChatApiError> {
        let body = RunCreate {
            id: None,
            conversation_id,
            model: model_id.to_owned(),
            provider: provider.to_owned(),
            input_message: input_message.to_owned(),
            metadata: None,
        };
        let response = self
            .client
            .post(self.url("/api/v1/chat/runs"))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if !response.status().is_success() {
            if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
                return Err(DaChatApiError::Unauthorized);
            }
            return Err(DaChatApiError::RequestFailed);
        }
        let created: RunCreated = response
            .json()
            .await
            .map_err(|_| DaChatApiError::InvalidResponse)?;
        Ok(created.id)
    }

    pub(crate) async fn conversation_bootstrap(
        &self,
        conversation_id: Uuid,
    ) -> Result<ConversationBootstrap, DaChatApiError> {
        let path = format!("/api/v1/chat/bootstrap/conversations/{conversation_id}");
        let mut url = self.url(&path);
        url.set_query(Some("max_chats=200"));
        let response = self
            .client
            .get(url)
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    pub(crate) async fn get_conversation(
        &self,
        conversation_id: Uuid,
    ) -> Result<super::generated::Conversation, DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}");
        let response = self
            .client
            .get(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    /// The conversation's durable messages — the run's model context, including
    /// the tool rounds any client wrote. See [`crate::chat::history`].
    ///
    /// `active_only` drops rows a checkpoint superseded, so a rewound
    /// conversation replays as the user now sees it. `sort=sequence_no` is the
    /// server-allocated total order; `created_at` ties inside one round.
    pub(crate) async fn list_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ChatMessage>, DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}/messages");
        let mut url = self.url(&path);
        url.set_query(Some("sort=sequence_no&active_only=true&limit=200"));
        let response = self
            .client
            .get(url)
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let page: CursorPageChatMessage = map_response(response).await?;
        Ok(page.items.unwrap_or_default())
    }

    /// Revision targets can have fallen outside the bootstrap's latest 200 rows.
    pub(crate) async fn revision_message(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        tenant_id: &str,
    ) -> Result<ChatMessage, DaChatApiError> {
        let response = self
            .client
            .get(self.url(&format!("/api/v1/chat/messages/{message_id}")))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let message: ChatMessage = map_response(response).await?;
        if message.id != message_id
            || message.conversation_id != conversation_id
            || message.tenant_id != tenant_id
            || message.superseded_by_checkpoint_id.is_some()
        {
            return Err(DaChatApiError::NotFound);
        }
        Ok(message)
    }

    /// Read through the target, stopping as soon as its turn can be revised.
    /// The bootstrap is a display window, not an authoritative membership check.
    pub(crate) async fn revision_history(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<ChatMessage>, DaChatApiError> {
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut previous_sequence = None;
        loop {
            let mut url = self.url(&format!(
                "/api/v1/chat/conversations/{conversation_id}/messages"
            ));
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair("sort", "sequence_no")
                    .append_pair("active_only", "true")
                    .append_pair("limit", "200");
                if let Some(cursor) = &cursor {
                    query.append_pair("cursor", cursor);
                }
            }
            let response = self
                .client
                .get(url)
                .bearer_auth(self.bearer.as_str())
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            let page: CursorPageChatMessage = map_response(response).await?;
            for message in page.items.unwrap_or_default() {
                if message.conversation_id != conversation_id
                    || message.tenant_id != tenant_id
                    || message.superseded_by_checkpoint_id.is_some()
                    || previous_sequence.is_some_and(|previous| message.sequence_no <= previous)
                {
                    return Err(DaChatApiError::InvalidResponse);
                }
                previous_sequence = Some(message.sequence_no);
                let found = message.id == message_id;
                // Earlier turns cannot supply this target's prompt or cutoff.
                if message.role.eq_ignore_ascii_case("user") {
                    messages.clear();
                }
                messages.push(message);
                if found {
                    return Ok(messages);
                }
            }
            match page.next_cursor {
                Some(next) if !next.is_empty() && seen_cursors.insert(next.clone()) => {
                    cursor = Some(next)
                }
                None if !page.has_more.unwrap_or(false) => return Err(DaChatApiError::NotFound),
                _ => return Err(DaChatApiError::InvalidResponse),
            }
        }
    }

    /// Every ready attachment in `conversation_id`, oldest first.
    ///
    /// Only `ready` rows: a `pending` or `failed` upload has no bytes a run
    /// could read. A list longer than [`MAX_ATTACHMENT_PAGES`] pages fails
    /// rather than returning part of it, because the part left out would be
    /// silently missing from the model's view of the chat.
    pub(crate) async fn list_conversation_attachments(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ChatAttachment>, DaChatApiError> {
        let mut cursor: Option<String> = None;
        let mut attachments = Vec::new();
        for _ in 0..MAX_ATTACHMENT_PAGES {
            let mut url = self.url("/api/v1/chat/chat-attachments");
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("conversation_id", &conversation_id.to_string());
                query.append_pair("status", "ready");
                query.append_pair("sort", "created_at");
                query.append_pair("limit", &ATTACHMENT_PAGE_LIMIT.to_string());
                if let Some(value) = cursor.as_deref() {
                    query.append_pair("cursor", value);
                }
            }
            let response = self
                .client
                .get(url)
                .bearer_auth(self.bearer.as_str())
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            let page: CursorPageChatAttachment = map_response(response).await?;
            for attachment in page.items.unwrap_or_default() {
                // RLS scopes the rows; this checks the filter was honoured.
                if attachment.conversation_id != conversation_id {
                    return Err(DaChatApiError::InvalidResponse);
                }
                attachments.push(attachment);
            }
            match (page.has_more, page.next_cursor) {
                (Some(true), Some(next)) if !next.is_empty() => cursor = Some(next),
                (Some(true), _) => return Err(DaChatApiError::InvalidResponse),
                _ => return Ok(attachments),
            }
        }
        Err(DaChatApiError::PayloadTooLarge)
    }

    /// One attachment's bytes, at most `maximum`.
    ///
    /// Object storage serves them through a short-lived presigned GET, which
    /// never receives the DA bearer. An on-prem deployment with local storage
    /// cannot presign (`sign-get` answers 501) and streams them from its own
    /// authenticated `content` route instead.
    pub(crate) async fn read_attachment_bytes(
        &self,
        storage_key: &str,
        maximum: usize,
    ) -> Result<Vec<u8>, DaChatApiError> {
        let mut sign_url = self.url("/api/v1/user/chat-attachments/sign-get");
        sign_url.query_pairs_mut().append_pair("key", storage_key);
        let signed = self
            .client
            .get(sign_url)
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if signed.status() == reqwest::StatusCode::NOT_IMPLEMENTED {
            let mut content_url = self.url("/api/v1/user/chat-attachments/content");
            content_url
                .query_pairs_mut()
                .append_pair("key", storage_key);
            let response = self
                .client
                .get(content_url)
                .bearer_auth(self.bearer.as_str())
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            if !response.status().is_success() {
                return Err(map_error_response(response).await);
            }
            return read_bounded_body(response, maximum).await;
        }
        let handle: ChatAttachmentSignGetResponse = map_response(signed).await?;
        let target = validate_signed_url(&handle.url, cfg!(test))?;
        let response = self
            .client
            .get(target)
            .send()
            .await
            .map_err(|_| DaChatApiError::StorageUnavailable)?;
        if !response.status().is_success() {
            return Err(DaChatApiError::StorageUnavailable);
        }
        read_bounded_body(response, maximum).await
    }

    /// Reads the complete frozen inference snapshot. A stale cursor restarts
    /// from page one, never from the rejected cursor, and only within a fixed
    /// retry budget so a rapidly changing conversation fails visibly.
    pub(crate) async fn read_context_snapshot(
        &self,
        conversation_id: Uuid,
    ) -> Result<ContextSnapshot, DaChatApiError> {
        for restart in 0..=MAX_CONTEXT_STALE_RESTARTS {
            match self.read_context_snapshot_once(conversation_id).await {
                Err(DaChatApiError::ContextSnapshotStale)
                    if restart < MAX_CONTEXT_STALE_RESTARTS =>
                {
                    continue;
                }
                result => return result,
            }
        }
        Err(DaChatApiError::ContextSnapshotStale)
    }

    async fn read_context_snapshot_once(
        &self,
        conversation_id: Uuid,
    ) -> Result<ContextSnapshot, DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}/context");
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut seen_messages = HashSet::new();
        let mut snapshot: Option<ContextSnapshot> = None;
        let mut last_sequence = None;

        loop {
            let mut url = self.url(&path);
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("limit", &CONTEXT_PAGE_LIMIT.to_string());
                if let Some(value) = cursor.as_deref() {
                    query.append_pair("cursor", value);
                }
            }
            let response = self
                .client
                .get(url)
                .bearer_auth(self.bearer.as_str())
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            let page: ContextPage = map_response(response).await?;

            validate_context_page(conversation_id, &page, snapshot.as_ref(), last_sequence)?;
            let messages = page.messages.clone().unwrap_or_default();
            for message in &messages {
                if !seen_messages.insert(message.id) {
                    return Err(DaChatApiError::InvalidResponse);
                }
                last_sequence = Some(message.sequence_no);
            }

            if let Some(existing) = snapshot.as_mut() {
                existing.messages.extend(messages);
            } else {
                snapshot = Some(ContextSnapshot {
                    active_compaction: page.active_compaction.clone(),
                    history_revision: page.history_revision,
                    snapshot_head_sequence_no: page.snapshot_head_sequence_no,
                    boundary_sequence_no: page.boundary_sequence_no,
                    compactable_through_sequence_no: page.compactable_through_sequence_no,
                    compactable_source_message_count: page.compactable_source_message_count,
                    source_message_ids_sha256: page.source_message_ids_sha256.clone(),
                    messages,
                });
            }

            match (page.has_more, page.next_cursor) {
                (false, None) => return snapshot.ok_or(DaChatApiError::InvalidResponse),
                (true, Some(next)) if !next.is_empty() && seen_cursors.insert(next.clone()) => {
                    cursor = Some(next);
                }
                _ => return Err(DaChatApiError::InvalidResponse),
            }
        }
    }

    pub(crate) async fn get_context_compaction(
        &self,
        conversation_id: Uuid,
        compaction_id: Uuid,
    ) -> Result<ContextCompaction, DaChatApiError> {
        let path = format!(
            "/api/v1/chat/conversations/{conversation_id}/context-compactions/{compaction_id}"
        );
        let response = self
            .client
            .get(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    pub(crate) async fn activate_context_compaction(
        &self,
        conversation_id: Uuid,
        body: &ContextCompactionActivate,
    ) -> Result<ContextCompaction, DaChatApiError> {
        let path =
            format!("/api/v1/chat/conversations/{conversation_id}/context-compactions:activate");
        let response = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    /// Reconciles an activation whose response may have been lost. `NotFound`
    /// proves the client-supplied identity did not commit; any returned row is
    /// DA's authoritative immutable record.
    pub(crate) async fn reconcile_context_compaction(
        &self,
        conversation_id: Uuid,
        compaction_id: Uuid,
    ) -> Result<Option<ContextCompaction>, DaChatApiError> {
        match self
            .get_context_compaction(conversation_id, compaction_id)
            .await
        {
            Ok(value) => Ok(Some(value)),
            Err(DaChatApiError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn prepare_tool_result_payload(
        &self,
        conversation_id: Uuid,
        body: &ToolResultPayloadPrepare,
        idempotency_key: &str,
    ) -> Result<ToolResultPayloadPrepareResult, DaChatApiError> {
        validate_prepare_request(body, idempotency_key)?;
        let path =
            format!("/api/v1/chat/conversations/{conversation_id}/tool-result-payloads:prepare");
        let response = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .header(super::generated::IDEMPOTENCY_KEY.as_str(), idempotency_key)
            .json(body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let result: ToolResultPayloadPrepareResult = map_response(response).await?;
        validate_prepare_result(conversation_id, body, &result)?;
        Ok(result)
    }

    /// Uploads exact bytes according to DA's prepared instruction. Signed
    /// object-store requests never receive the DA bearer token.
    pub(crate) async fn upload_tool_result_payload(
        &self,
        conversation_id: Uuid,
        payload: &ToolResultPayload,
        upload: &UploadInstruction,
        exact: &[u8],
    ) -> Result<(), DaChatApiError> {
        match upload.mode {
            UploadInstructionMode::Inline => {
                let expected_status = if matches!(payload.status, PayloadStatus::Ready) {
                    PayloadStatus::Ready
                } else if matches!(payload.status, PayloadStatus::Pending) {
                    PayloadStatus::Pending
                } else {
                    return Err(DaChatApiError::InvalidResponse);
                };
                validate_exact_payload(payload, exact, expected_status)?;
                return Ok(());
            }
            UploadInstructionMode::Local => {
                validate_exact_payload(payload, exact, PayloadStatus::Pending)?;
                let expected_path = format!(
                    "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{}/content",
                    payload.id
                );
                if upload.method.as_deref() != Some("PUT")
                    || upload.url.as_deref() != Some(expected_path.as_str())
                {
                    return Err(DaChatApiError::InvalidResponse);
                }
                let response = self
                    .client
                    .put(self.url(&expected_path))
                    .bearer_auth(self.bearer.as_str())
                    .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                    .body(exact.to_vec())
                    .send()
                    .await
                    .map_err(|_| DaChatApiError::StorageUnavailable)?;
                return map_empty_response(response).await;
            }
            UploadInstructionMode::Presigned => {}
        }

        validate_exact_payload(payload, exact, PayloadStatus::Pending)?;

        if upload.method.as_deref() != Some("PUT") {
            return Err(DaChatApiError::InvalidResponse);
        }
        if upload
            .expires_at
            .is_none_or(|value| value <= chrono::Utc::now())
        {
            return Err(DaChatApiError::InvalidResponse);
        }
        let target = upload
            .url
            .as_deref()
            .ok_or(DaChatApiError::InvalidResponse)
            .and_then(|value| validate_signed_url(value, cfg!(test)))?;
        let mut request = self.client.put(target).body(exact.to_vec());
        for (name, value) in upload.required_headers.as_ref().into_iter().flatten() {
            if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("cookie") {
                return Err(DaChatApiError::InvalidResponse);
            }
            let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| DaChatApiError::InvalidResponse)?;
            let header_value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| DaChatApiError::InvalidResponse)?;
            request = request.header(header_name, header_value);
        }
        let response = request
            .send()
            .await
            .map_err(|_| DaChatApiError::StorageUnavailable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(DaChatApiError::StorageUnavailable)
        }
    }

    pub(crate) async fn finalize_tool_result_payload(
        &self,
        conversation_id: Uuid,
        payload_id: Uuid,
    ) -> Result<ToolResultPayload, DaChatApiError> {
        let path = format!(
            "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}:finalize"
        );
        let response = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&ToolResultPayloadFinalize {})
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let payload: ToolResultPayload = map_response(response).await?;
        if payload.id != payload_id
            || payload.conversation_id != conversation_id
            || !matches!(payload.status, PayloadStatus::Ready)
        {
            return Err(DaChatApiError::InvalidResponse);
        }
        validate_payload_record(&payload)?;
        Ok(payload)
    }

    /// Completes a prepared payload across ambiguous finalize responses.
    ///
    /// A transport failure does not prove that DA rolled the transaction back.
    /// The client therefore reads the stable, client-supplied payload identity
    /// before it retries the mutation. The retry is bounded, and a second
    /// ambiguous response is reconciled once more instead of creating a new
    /// payload or binding a pending one.
    pub(crate) async fn finalize_tool_result_payload_reconciled(
        &self,
        conversation_id: Uuid,
        payload_id: Uuid,
    ) -> Result<ToolResultPayload, DaChatApiError> {
        match self
            .finalize_tool_result_payload(conversation_id, payload_id)
            .await
        {
            Ok(payload) => Ok(payload),
            Err(DaChatApiError::RequestFailed) => {
                let reconciled = self
                    .reconcile_tool_result_payload(conversation_id, payload_id)
                    .await?;
                match reconciled {
                    Some(payload) if payload.status == PayloadStatus::Ready => Ok(payload),
                    Some(payload) if payload.status == PayloadStatus::Pending => {
                        match self
                            .finalize_tool_result_payload(conversation_id, payload_id)
                            .await
                        {
                            Ok(payload) => Ok(payload),
                            Err(DaChatApiError::RequestFailed) => self
                                .reconcile_tool_result_payload(conversation_id, payload_id)
                                .await?
                                .filter(|payload| payload.status == PayloadStatus::Ready)
                                .ok_or(DaChatApiError::PayloadNotReady),
                            Err(error) => Err(error),
                        }
                    }
                    Some(_) => Err(DaChatApiError::PayloadNotReady),
                    None => Err(DaChatApiError::InvalidResponse),
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn get_tool_result_payload(
        &self,
        conversation_id: Uuid,
        payload_id: Uuid,
    ) -> Result<ToolResultPayload, DaChatApiError> {
        let path = format!(
            "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}"
        );
        let response = self
            .client
            .get(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let payload: ToolResultPayload = map_response(response).await?;
        if payload.id != payload_id || payload.conversation_id != conversation_id {
            return Err(DaChatApiError::InvalidResponse);
        }
        validate_payload_record(&payload)?;
        Ok(payload)
    }

    pub(crate) async fn reconcile_tool_result_payload(
        &self,
        conversation_id: Uuid,
        payload_id: Uuid,
    ) -> Result<Option<ToolResultPayload>, DaChatApiError> {
        match self
            .get_tool_result_payload(conversation_id, payload_id)
            .await
        {
            Ok(value) => Ok(Some(value)),
            Err(DaChatApiError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn read_tool_result_range(
        &self,
        conversation_id: Uuid,
        descriptor: &ToolResultPayloadDescriptor,
        offset: usize,
        length: usize,
    ) -> Result<ToolResultRange, DaChatApiError> {
        validate_range_request(descriptor, offset, length)?;
        let end = offset
            .checked_add(length - 1)
            .ok_or(DaChatApiError::Validation)?;
        let path = format!(
            "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{}/content",
            descriptor.id
        );
        let range = format!("bytes={offset}-{end}");
        let response = self
            .client
            .get(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .header(reqwest::header::RANGE, &range)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;

        if response.status().as_u16() == 206 {
            return decode_range_response(response, descriptor, offset, end).await;
        }
        if response.status().is_success() {
            return Err(DaChatApiError::InvalidResponse);
        }
        let first_error = map_error_response(response).await;
        if first_error != DaChatApiError::Validation {
            return Err(first_error);
        }

        // Non-local object storage cannot proxy a range. Ask once without a
        // range for its short-lived handle, then apply the range directly.
        let handle_response = self
            .client
            .get(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if !handle_response.status().is_success() {
            return Err(map_error_response(handle_response).await);
        }
        let content_type = handle_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type.starts_with("application/json") {
            return Err(DaChatApiError::InvalidResponse);
        }
        let handle_bytes = read_bounded_body(handle_response, 16 * 1024).await?;
        let handle: ToolResultContentHandle =
            serde_json::from_slice(&handle_bytes).map_err(|_| DaChatApiError::InvalidResponse)?;
        validate_content_handle(&handle, descriptor)?;
        let target = validate_signed_url(&handle.url, cfg!(test))?;
        let signed_response = self
            .client
            .get(target)
            .header(reqwest::header::RANGE, &range)
            .send()
            .await
            .map_err(|_| DaChatApiError::StorageUnavailable)?;
        if signed_response.status().as_u16() != 206 {
            return Err(DaChatApiError::StorageUnavailable);
        }
        decode_range_response(signed_response, descriptor, offset, end).await
    }

    /// Writes the assistant turn that *requested* tool calls, in the shape a
    /// provider validates on the next request.
    ///
    /// This is the durable half of a tool round, and it is why a conversation
    /// this device ran can be continued anywhere. `xf_chat.chats.tool_calls`
    /// carries the OpenAI pairing (`{id, type, function: {name, arguments}}`)
    /// beside the `role="tool"` messages that answer it — the same two writes
    /// xfalcon-web makes (`persistAssistantToolCallMessage` /
    /// `persistToolResultMessage`), so either product can replay the other's
    /// conversations without reconstructing anything.
    ///
    /// The `xf_chat.tool_calls` rows written by [`Self::create_tool_call`] stay
    /// what they always were: the execution log (arguments, output, timing,
    /// approval), not the conversation.
    pub(crate) async fn create_assistant_tool_call_message(
        &self,
        conversation_id: uuid::Uuid,
        message_id: Uuid,
        text: &str,
        calls: Vec<serde_json::Value>,
        idempotency_key: &str,
    ) -> Result<MessageInsertResult, DaChatApiError> {
        let body = MessageCreate {
            id: Some(message_id),
            role: ContextMessageRole::Assistant,
            // Empty rather than absent when the turn was calls-only: a provider
            // rejects an assistant message with neither content nor tool calls,
            // and web sends `''` here for the same reason.
            content: Some(text.to_owned()),
            metadata: None,
            token_input: None,
            token_output: None,
            tool_calls: Some(super::generated::ChatMessageToolCalls::Array(calls)),
            tool_call_id: None,
            name: None,
            attachment_ids: None,
            tool_result_payload_ids: None,
        };
        self.append_message(conversation_id, body, Some(idempotency_key))
            .await
    }

    /// Writes one tool result, answering exactly one call the assistant turn
    /// above advertised.
    ///
    /// `tool_call_id` is the provider's own id, not this app's invocation id:
    /// it is what the provider matches against the assistant turn's
    /// `tool_calls`, and a mismatch there fails the whole next request rather
    /// than just this result.
    pub(crate) async fn create_tool_result_message(
        &self,
        conversation_id: uuid::Uuid,
        tool_call_id: &str,
        tool_name: &str,
        content: &str,
    ) -> Result<(), DaChatApiError> {
        let body = MessageCreate {
            id: None,
            role: ContextMessageRole::Tool,
            content: Some(content.to_owned()),
            metadata: None,
            token_input: None,
            token_output: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_owned()),
            name: Some(tool_name.to_owned()),
            attachment_ids: None,
            tool_result_payload_ids: None,
        };
        self.insert_message(conversation_id, body).await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the fields mirror one durable message boundary without an extra request type"
    )]
    pub(crate) async fn create_tool_result_message_with_payloads(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        provider_tool_call_id: &str,
        tool_name: &str,
        content: &str,
        payload_ids: Vec<Uuid>,
        idempotency_key: &str,
    ) -> Result<MessageInsertResult, DaChatApiError> {
        let body = MessageCreate {
            id: Some(message_id),
            role: ContextMessageRole::Tool,
            content: Some(content.to_owned()),
            metadata: None,
            token_input: None,
            token_output: None,
            tool_calls: None,
            tool_call_id: Some(provider_tool_call_id.to_owned()),
            name: Some(tool_name.to_owned()),
            attachment_ids: None,
            tool_result_payload_ids: Some(payload_ids),
        };
        self.append_message(conversation_id, body, Some(idempotency_key))
            .await
    }

    pub(crate) async fn append_message(
        &self,
        conversation_id: Uuid,
        body: MessageCreate,
        idempotency_key: Option<&str>,
    ) -> Result<MessageInsertResult, DaChatApiError> {
        if let Some(key) = idempotency_key {
            validate_idempotency_key(key)?;
        }
        let path = format!("/api/v1/chat/conversations/{conversation_id}/messages");
        let mut request = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&body);
        if let Some(key) = idempotency_key {
            request = request.header(super::generated::IDEMPOTENCY_KEY.as_str(), key);
        }
        let response = request
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let result: MessageInsertResult = map_response(response).await?;
        validate_insert_result(&body, &result)?;
        Ok(result)
    }

    async fn insert_message(
        &self,
        conversation_id: uuid::Uuid,
        body: MessageCreate,
    ) -> Result<(), DaChatApiError> {
        self.append_message(conversation_id, body, None)
            .await
            .map(|_| ())
    }

    /// Records one executed tool call against the assistant message it belongs
    /// to.
    ///
    /// This is what makes a locally-executed round replayable in a later turn.
    /// `xf_chat.tool_calls` is the table DA built for it: `provider_tool_call_id`
    /// and `tool_name` reconstruct the assistant turn's calls, `output`/`error`
    /// reconstruct the results answering them, and `sequence_no` orders a round
    /// of parallel calls deterministically.
    ///
    /// `message_id` comes from the finalize that wrote the assistant message
    /// ([`RunFinalizeResult::message_id`]), so a row is attached to the exact
    /// turn that made the call rather than merely to the conversation.
    pub(crate) async fn create_tool_call(
        &self,
        body: ToolCallCreate,
        idempotency_key: &str,
    ) -> Result<ToolCall, DaChatApiError> {
        validate_idempotency_key(idempotency_key)?;
        let response = self
            .client
            .post(self.url("/api/v1/chat/tool-calls"))
            .bearer_auth(self.bearer.as_str())
            .header(super::generated::IDEMPOTENCY_KEY.as_str(), idempotency_key)
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let result: ToolCall = map_response(response).await?;
        if result.id.is_nil()
            || result.conversation_id != body.conversation_id
            || result.message_id != body.message_id
            || result.run_id != body.run_id
            || result.provider_tool_call_id != body.provider_tool_call_id
            || result.tool_name != body.tool_name
        {
            return Err(DaChatApiError::InvalidResponse);
        }
        Ok(result)
    }

    pub(crate) async fn create_user_message_idempotent(
        &self,
        conversation_id: Uuid,
        content: &str,
        idempotency_key: Option<&str>,
        attachment_ids: Option<Vec<Uuid>>,
    ) -> Result<MessageInsertResult, DaChatApiError> {
        let body = MessageCreate {
            id: None,
            role: ContextMessageRole::User,
            content: Some(content.to_owned()),
            metadata: None,
            token_input: None,
            token_output: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            attachment_ids,
            tool_result_payload_ids: None,
        };
        self.append_message(conversation_id, body, idempotency_key)
            .await
    }

    pub(crate) async fn get_run(
        &self,
        run_id: uuid::Uuid,
    ) -> Result<super::generated::Run, DaChatApiError> {
        let path = format!("/api/v1/chat/runs/{run_id}");
        let response = self
            .client
            .get(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    pub(crate) async fn list_run_events(
        &self,
        run_id: uuid::Uuid,
        after_sequence: i64,
    ) -> Result<Vec<super::generated::RunEvent>, DaChatApiError> {
        let path = format!("/api/v1/chat/runs/{run_id}/events");
        let mut url = self.url(&path);
        url.set_query(Some("limit=200&sort=sequence_no"));
        let response = self
            .client
            .get(url)
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let page: super::generated::CursorPageRunEvent = map_response(response).await?;
        let events = page.items.unwrap_or_default();
        Ok(events
            .into_iter()
            .filter(|event| event.sequence_no > after_sequence)
            .collect())
    }

    pub(crate) async fn cancel_run(&self, run_id: uuid::Uuid) -> Result<(), DaChatApiError> {
        match self.update_run_status(run_id, "cancelled").await {
            Err(DaChatApiError::Conflict) => {
                // The status endpoint fences terminal writes. A run that is
                // already terminal satisfies Stop, without rewriting its answer
                // or status.
                let run = self.get_run(run_id).await?;
                if is_terminal_run_status(&run.status) {
                    Ok(())
                } else {
                    Err(DaChatApiError::Conflict)
                }
            }
            outcome => outcome,
        }
    }

    async fn patch_conversation(
        &self,
        conversation_id: uuid::Uuid,
        update: serde_json::Value,
    ) -> Result<super::generated::Conversation, DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}");
        let response = self
            .client
            .patch(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&update)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    pub(crate) async fn set_conversation_model(
        &self,
        conversation_id: uuid::Uuid,
        model_id: &str,
    ) -> Result<super::generated::Conversation, DaChatApiError> {
        self.patch_conversation(conversation_id, serde_json::json!({ "model": model_id }))
            .await
    }

    pub(crate) async fn set_conversation_title(
        &self,
        conversation_id: uuid::Uuid,
        title: &str,
    ) -> Result<super::generated::Conversation, DaChatApiError> {
        self.patch_conversation(conversation_id, serde_json::json!({ "title": title }))
            .await
    }

    pub(crate) async fn set_conversation_archived(
        &self,
        conversation_id: uuid::Uuid,
        archived: bool,
    ) -> Result<super::generated::Conversation, DaChatApiError> {
        self.patch_conversation(
            conversation_id,
            serde_json::json!({ "is_archived": archived }),
        )
        .await
    }

    /// Pins or unpins one conversation.
    ///
    /// A raw JSON body rather than [`super::generated::ConversationUpdate`]:
    /// the endpoint is a `model_fields_set` partial update where an explicit
    /// `pinned_at: null` clears the pin and an *omitted* field is left
    /// untouched — a distinction the generated struct's
    /// `skip_serializing_none` cannot express.
    pub(crate) async fn set_conversation_pinned(
        &self,
        conversation_id: uuid::Uuid,
        pinned: bool,
    ) -> Result<super::generated::Conversation, DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}");
        let pinned_at = pinned.then(|| chrono::Utc::now().to_rfc3339());
        let response = self
            .client
            .patch(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&serde_json::json!({ "pinned_at": pinned_at }))
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    pub(crate) async fn delete_conversation(
        &self,
        conversation_id: uuid::Uuid,
    ) -> Result<(), DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}");
        let response = self
            .client
            .delete(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if response.status().is_success() || response.status().as_u16() == 204 {
            return Ok(());
        }
        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(DaChatApiError::Unauthorized);
        }
        if response.status().as_u16() == 404 {
            return Err(DaChatApiError::NotFound);
        }
        Err(DaChatApiError::RequestFailed)
    }

    pub(crate) async fn truncate_conversation(
        &self,
        conversation_id: uuid::Uuid,
        from_message_id: uuid::Uuid,
    ) -> Result<super::generated::TruncateResult, DaChatApiError> {
        let path = format!("/api/v1/chat/conversations/{conversation_id}:truncate");
        let body = super::generated::TruncateRequest { from_message_id };
        let response = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        map_response(response).await
    }

    /// Applies an ordered batch of run-event records in one DA transaction. Used by the
    /// local (Path A) executor to durably mirror streamed provider deltas as they arrive,
    /// using the same `run.*` event-type vocabulary the DA-poll observer already reads
    /// back (see `agent::da_events::map_observed_event`), so a run started locally stays
    /// resumable/observable from any device.
    pub(crate) async fn apply_event_batch(
        &self,
        run_id: Uuid,
        records: Vec<BatchRecord>,
    ) -> Result<u32, DaChatApiError> {
        if records.is_empty() {
            return Ok(0);
        }
        let path = format!("/api/v1/chat/runs/{run_id}/events:batch");
        let body = EventBatchRequest { records };
        let response = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        let result: EventBatchResult = map_response(response).await?;
        Ok(u32::try_from(result.applied).unwrap_or(u32::MAX))
    }

    /// Atomically persists the assistant message and terminal status for a run in ONE
    /// DA transaction — the local-execution counterpart of the DA-side `finalizeRunCompletion`
    /// xFalcon Web performs for its own runs.
    pub(crate) async fn finalize_run(
        &self,
        run_id: Uuid,
        finalize: RunFinalize,
    ) -> Result<RunFinalizeResult, DaChatApiError> {
        let path = format!("/api/v1/chat/runs/{run_id}:finalize");
        let response = self
            .client
            .post(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&finalize)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        // A 409 here means a competing terminal write (typically a user cancel) already
        // fenced this run — the docstring on `RunFinalize` is explicit that callers must
        // treat it as "already terminated," not as a failed finalize to retry.
        if response.status().as_u16() == 409 {
            return Err(DaChatApiError::Conflict);
        }
        map_response(response).await
    }

    pub(crate) async fn update_run_status(
        &self,
        run_id: Uuid,
        status: &str,
    ) -> Result<(), DaChatApiError> {
        let path = format!("/api/v1/chat/runs/{run_id}/status");
        let body = serde_json::json!({ "status": status });
        let response = self
            .client
            .patch(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if response.status().is_success() {
            return Ok(());
        }
        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(DaChatApiError::Unauthorized);
        }
        if response.status().as_u16() == 409 {
            return Err(DaChatApiError::Conflict);
        }
        Err(DaChatApiError::RequestFailed)
    }

    /// Records an approval decision for a DA-tracked tool call, over the
    /// same pinned, authenticated DA/MCP-audience channel every other method
    /// on this type already uses (`self.client`/`self.bearer`) -- there is
    /// no separate provider/control-token path for this. Mirrors the pinned
    /// `PATCH /api/v1/chat/tool_calls/{tool_call_id}/approval` contract
    /// (`super::generated::ApproveToolCallApiV1ChatToolCallsToolCallIdApprovalPatchRequest`/
    /// `ToolCallApproval`), built by hand exactly like every sibling method
    /// in this file (`update_run_status`, `update_conversation`, ...) rather
    /// than invoking the generated request wrapper's own `parse_response` --
    /// see this file's existing methods for that established convention.
    /// `status` is passed through verbatim (e.g. `"approved"`/`"denied"`);
    /// DA is the schema authority for which values it accepts, exactly like
    /// `update_run_status`'s own `status` parameter.
    pub(crate) async fn update_tool_call_approval(
        &self,
        tool_call_id: Uuid,
        status: &str,
    ) -> Result<(), DaChatApiError> {
        let path = format!("/api/v1/chat/tool_calls/{tool_call_id}/approval");
        let body = ToolCallApproval {
            status: Some(status.to_owned()),
        };
        let response = self
            .client
            .patch(self.url(&path))
            .bearer_auth(self.bearer.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        if response.status().is_success() {
            return Ok(());
        }
        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(DaChatApiError::Unauthorized);
        }
        if response.status().as_u16() == 404 {
            return Err(DaChatApiError::NotFound);
        }
        Err(DaChatApiError::RequestFailed)
    }
}

async fn map_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, DaChatApiError> {
    let status = response.status().as_u16();
    let path = response.url().path().to_owned();
    if response.status().is_success() {
        return response.json().await.map_err(|error| {
            let error_kind = if error.is_decode() {
                "decode"
            } else if error.is_timeout() {
                "timeout"
            } else {
                "body"
            };
            tracing::error!(
                error_kind,
                status,
                path,
                "DA response body did not match the pinned schema"
            );
            DaChatApiError::InvalidResponse
        });
    }
    tracing::warn!(status, path, "DA request rejected");
    Err(map_error_response(response).await)
}

async fn map_empty_response(response: reqwest::Response) -> Result<(), DaChatApiError> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(map_error_response(response).await)
    }
}

async fn map_error_response(response: reqwest::Response) -> DaChatApiError {
    let status = response.status().as_u16();
    let envelope = match read_bounded_body(response, 64 * 1024).await {
        Ok(body) => serde_json::from_slice::<ChatErrorEnvelope>(&body).ok(),
        Err(_) => None,
    };
    if let Some(envelope) = envelope {
        return match envelope.error.code {
            ChatErrorCode::Unauthorized
            | ChatErrorCode::CrossTenant
            | ChatErrorCode::InsufficientScope => DaChatApiError::Unauthorized,
            ChatErrorCode::NotFound | ChatErrorCode::IdentityNotFound => DaChatApiError::NotFound,
            ChatErrorCode::Conflict => DaChatApiError::Conflict,
            ChatErrorCode::ContextConflict => DaChatApiError::ContextConflict,
            ChatErrorCode::ContextSnapshotStale => DaChatApiError::ContextSnapshotStale,
            ChatErrorCode::IdempotencyConflict => DaChatApiError::IdempotencyConflict,
            ChatErrorCode::PayloadTooLarge => DaChatApiError::PayloadTooLarge,
            ChatErrorCode::StorageQuotaExceeded => DaChatApiError::StorageQuotaExceeded,
            ChatErrorCode::StorageUnavailable => DaChatApiError::StorageUnavailable,
            ChatErrorCode::PayloadNotReady => DaChatApiError::PayloadNotReady,
            ChatErrorCode::PayloadIntegrityFailed => DaChatApiError::PayloadIntegrityFailed,
            ChatErrorCode::ValidationError => DaChatApiError::Validation,
            ChatErrorCode::RateLimited => DaChatApiError::RateLimited,
            ChatErrorCode::Timeout => DaChatApiError::Timeout,
            ChatErrorCode::UnsupportedCompactionFormat => {
                DaChatApiError::UnsupportedCompactionFormat
            }
            ChatErrorCode::Internal => DaChatApiError::RequestFailed,
        };
    }
    match status {
        401 | 403 => DaChatApiError::Unauthorized,
        404 => DaChatApiError::NotFound,
        409 => DaChatApiError::Conflict,
        413 => DaChatApiError::PayloadTooLarge,
        422 => DaChatApiError::Validation,
        429 => DaChatApiError::RateLimited,
        504 => DaChatApiError::Timeout,
        _ => DaChatApiError::RequestFailed,
    }
}

async fn read_custom_model_response(
    response: reqwest::Response,
    expected_status: u16,
    submitted_secret: Option<&str>,
    submitted_url: Option<&str>,
) -> Result<ModelEntity, CustomModelApiError> {
    let status = response.status().as_u16();
    let body = read_bounded_body(response, 64 * 1024)
        .await
        .map_err(|_| CustomModelApiError::InvalidResponse)?;
    if status != expected_status {
        return Err(custom_model_error(
            status,
            &body,
            submitted_secret,
            submitted_url,
        ));
    }
    let model: ModelEntity =
        serde_json::from_slice(&body).map_err(|_| CustomModelApiError::InvalidResponse)?;
    if model.provider != "custom_openai"
        || model.provider_parent != "custom_openai"
        || model.model_id != format!("custom/{}", model.id)
    {
        return Err(CustomModelApiError::InvalidResponse);
    }
    Ok(model)
}

fn custom_model_error(
    status: u16,
    body: &[u8],
    submitted_secret: Option<&str>,
    submitted_url: Option<&str>,
) -> CustomModelApiError {
    match status {
        401 => CustomModelApiError::Unauthorized,
        403 => CustomModelApiError::Forbidden,
        404 => CustomModelApiError::Removed,
        409 => CustomModelApiError::Conflict,
        422 => {
            let message = serde_json::from_slice::<ChatErrorEnvelope>(body)
                .ok()
                .filter(|envelope| envelope.error.code == ChatErrorCode::ValidationError)
                .map(|envelope| envelope.error.message)
                .map(|message| {
                    let fields = message
                        .strip_prefix("request failed validation: ")
                        .unwrap_or(&message);
                    fields.strip_prefix("body.").unwrap_or(fields).to_owned()
                })
                .filter(|message| {
                    message.len() <= 256
                        && !message.chars().any(char::is_control)
                        && !message.contains("://")
                        && submitted_secret.is_none_or(|secret| !message.contains(secret))
                        && submitted_url.is_none_or(|url| !message.contains(url))
                        && [
                            "base_url",
                            "upstream_model_id",
                            "secret",
                            "display_name",
                            "context_length",
                            "max_output_tokens",
                            "supports_tools",
                            "supports_vision",
                            "is_enabled",
                            "is_active",
                        ]
                        .iter()
                        .any(|field| message.starts_with(field))
                })
                .unwrap_or_else(|| "Check the private model fields and try again.".to_owned());
            CustomModelApiError::Validation(message)
        }
        429 => CustomModelApiError::RateLimited,
        _ => CustomModelApiError::Unavailable,
    }
}

fn validate_context_page(
    conversation_id: Uuid,
    page: &ContextPage,
    prior: Option<&ContextSnapshot>,
    last_sequence: Option<i64>,
) -> Result<(), DaChatApiError> {
    if page.history_revision < 0
        || page.boundary_sequence_no < 0
        || page.snapshot_head_sequence_no < page.boundary_sequence_no
        || page.compactable_through_sequence_no < page.boundary_sequence_no
        || page.compactable_through_sequence_no > page.snapshot_head_sequence_no
        || page.compactable_source_message_count < 0
        || !is_sha256(&page.source_message_ids_sha256)
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    if let Some(active) = page.active_compaction.as_ref() {
        if prior.is_some()
            || active.conversation_id != conversation_id
            || active.invalidated_at.is_some()
            || active.through_sequence_no != page.boundary_sequence_no
            || !is_sha256(&active.provider_block_sha256)
            || !is_sha256(&active.source_message_ids_sha256)
            || !is_sha256(&active.context_fingerprint)
        {
            return Err(DaChatApiError::InvalidResponse);
        }
    } else if prior.is_none() && page.boundary_sequence_no != 0 {
        return Err(DaChatApiError::InvalidResponse);
    }
    if let Some(expected) = prior {
        if page.active_compaction.is_some()
            || page.history_revision != expected.history_revision
            || page.snapshot_head_sequence_no != expected.snapshot_head_sequence_no
            || page.boundary_sequence_no != expected.boundary_sequence_no
            || page.compactable_through_sequence_no != expected.compactable_through_sequence_no
            || page.compactable_source_message_count != expected.compactable_source_message_count
            || page.source_message_ids_sha256 != expected.source_message_ids_sha256
        {
            return Err(DaChatApiError::InvalidResponse);
        }
    }
    let messages = page.messages.as_deref().unwrap_or_default();
    if page.has_more && messages.is_empty() {
        return Err(DaChatApiError::InvalidResponse);
    }
    let mut sequence = last_sequence.unwrap_or(page.boundary_sequence_no);
    for message in messages {
        if message.sequence_no <= sequence || message.sequence_no > page.snapshot_head_sequence_no {
            return Err(DaChatApiError::InvalidResponse);
        }
        for payload in message.tool_result_payloads.as_deref().unwrap_or_default() {
            validate_payload_descriptor(payload)?;
        }
        sequence = message.sequence_no;
    }
    Ok(())
}

fn validate_payload_descriptor(
    payload: &ToolResultPayloadDescriptor,
) -> Result<(), DaChatApiError> {
    if payload.byte_length <= 0
        || usize::try_from(payload.byte_length).map_or(true, |size| size > MAX_TOOL_RESULT_BYTES)
        || !is_sha256(&payload.sha256)
        || !matches!(payload.status, PayloadStatus::Ready)
        || !payload.retrieval_available
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    validate_preview(
        payload.preview.as_deref(),
        payload.preview_byte_length,
        &payload.preview_strategy,
    )?;
    Ok(())
}

fn validate_payload_record(payload: &ToolResultPayload) -> Result<(), DaChatApiError> {
    if payload.id.is_nil()
        || payload.conversation_id.is_nil()
        || payload.tool_call_id.is_nil()
        || payload.byte_length <= 0
        || usize::try_from(payload.byte_length).map_or(true, |size| size > MAX_TOOL_RESULT_BYTES)
        || !is_sha256(&payload.sha256)
        || payload.serialization_version <= 0
        || payload.provider_tool_call_id.trim().is_empty()
        || payload.tool_name.trim().is_empty()
        || (matches!(payload.status, PayloadStatus::Ready) != payload.retrieval_available)
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    validate_preview(
        payload.preview.as_deref(),
        payload.preview_byte_length,
        &payload.preview_strategy,
    )
}

fn validate_prepare_request(
    body: &ToolResultPayloadPrepare,
    idempotency_key: &str,
) -> Result<(), DaChatApiError> {
    let byte_length = usize::try_from(body.byte_length).map_err(|_| DaChatApiError::Validation)?;
    if body.id.is_nil()
        || body.tool_call_id.is_nil()
        || byte_length == 0
        || byte_length > MAX_TOOL_RESULT_BYTES
        || !is_sha256(&body.sha256)
        || body.serialization_version <= 0
        || body.provider_tool_call_id.trim().is_empty()
        || body.tool_name.trim().is_empty()
    {
        return Err(if byte_length > MAX_TOOL_RESULT_BYTES {
            DaChatApiError::PayloadTooLarge
        } else {
            DaChatApiError::Validation
        });
    }
    validate_idempotency_key(idempotency_key)?;
    validate_preview(body.preview.as_deref(), None, &body.preview_strategy)
        .map_err(|_| DaChatApiError::Validation)?;
    if let Some(encoded) = body.inline_content_base64.as_deref() {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| DaChatApiError::PayloadIntegrityFailed)?;
        if decoded.len() != byte_length || sha256_hex(&decoded) != body.sha256 {
            return Err(DaChatApiError::PayloadIntegrityFailed);
        }
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<(), DaChatApiError> {
    if value.is_empty() || value.len() > 255 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(DaChatApiError::Validation);
    }
    Ok(())
}

fn validate_prepare_result(
    conversation_id: Uuid,
    request: &ToolResultPayloadPrepare,
    result: &ToolResultPayloadPrepareResult,
) -> Result<(), DaChatApiError> {
    let payload = &result.payload;
    validate_payload_record(payload)?;
    let expected_inline = request.inline_content_base64.is_some();
    if payload.id != request.id
        || payload.conversation_id != conversation_id
        || payload.tool_call_id != request.tool_call_id
        || payload.run_id != request.run_id
        || payload.provider_tool_call_id != request.provider_tool_call_id
        || payload.tool_name != request.tool_name
        || payload.byte_length != request.byte_length
        || payload.sha256 != request.sha256
        || payload.content_encoding != request.content_encoding.as_deref().unwrap_or("identity")
        || payload.media_type != request.media_type
        || payload.preview != request.preview
        || payload.preview_strategy != request.preview_strategy
        || payload.serialization_format != request.serialization_format
        || payload.serialization_version != request.serialization_version
        || result.quota_limit_bytes < 0
        || result.quota_used_bytes < 0
        || result.quota_used_bytes > result.quota_limit_bytes
        || expected_inline != matches!(result.upload.mode, UploadInstructionMode::Inline)
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    if expected_inline {
        if !matches!(
            payload.status,
            PayloadStatus::Pending | PayloadStatus::Ready
        ) {
            return Err(DaChatApiError::InvalidResponse);
        }
    } else if !matches!(payload.status, PayloadStatus::Pending) {
        return Err(DaChatApiError::InvalidResponse);
    }
    validate_preview(
        payload.preview.as_deref(),
        payload.preview_byte_length,
        &payload.preview_strategy,
    )?;
    Ok(())
}

fn validate_insert_result(
    request: &MessageCreate,
    result: &MessageInsertResult,
) -> Result<(), DaChatApiError> {
    if result.sequence_no <= 0 || request.id.is_some_and(|id| id != result.id) {
        return Err(DaChatApiError::InvalidResponse);
    }
    let requested = request
        .tool_result_payload_ids
        .as_deref()
        .unwrap_or_default();
    let returned = result.tool_result_payloads.as_deref().unwrap_or_default();
    let requested_ids: HashSet<_> = requested.iter().copied().collect();
    let returned_ids: HashSet<_> = returned.iter().map(|payload| payload.id).collect();
    if requested_ids.len() != requested.len()
        || returned_ids.len() != returned.len()
        || requested_ids != returned_ids
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    for payload in returned {
        validate_payload_descriptor(payload)?;
    }
    Ok(())
}

fn validate_exact_payload(
    payload: &ToolResultPayload,
    exact: &[u8],
    expected_status: PayloadStatus,
) -> Result<(), DaChatApiError> {
    if exact.is_empty() || exact.len() > MAX_TOOL_RESULT_BYTES {
        return Err(DaChatApiError::PayloadTooLarge);
    }
    if usize::try_from(payload.byte_length).ok() != Some(exact.len())
        || !is_sha256(&payload.sha256)
        || sha256_hex(exact) != payload.sha256
        || payload.status != expected_status
    {
        return Err(DaChatApiError::PayloadIntegrityFailed);
    }
    Ok(())
}

fn validate_preview(
    preview: Option<&str>,
    reported_length: Option<i64>,
    strategy: &super::generated::PayloadPreviewStrategy,
) -> Result<(), DaChatApiError> {
    let actual = preview.map_or(0, |value| value.len());
    // The ceiling, not a run's budget: a stored preview was cut to the budget
    // of whichever run produced it, and any of them is valid to read back.
    if actual > ToolResultBudget::CEILING_BYTES
        || reported_length.is_some_and(|value| usize::try_from(value).ok() != Some(actual))
        || (matches!(strategy, super::generated::PayloadPreviewStrategy::None) && preview.is_some())
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    Ok(())
}

fn validate_range_request(
    descriptor: &ToolResultPayloadDescriptor,
    offset: usize,
    length: usize,
) -> Result<(), DaChatApiError> {
    validate_payload_descriptor(descriptor)?;
    let total =
        usize::try_from(descriptor.byte_length).map_err(|_| DaChatApiError::InvalidResponse)?;
    if length == 0 || length > MAX_TOOL_RESULT_RANGE_BYTES || offset >= total {
        return Err(DaChatApiError::Validation);
    }
    let requested_end = offset
        .checked_add(length)
        .ok_or(DaChatApiError::Validation)?;
    if requested_end > total {
        return Err(DaChatApiError::Validation);
    }
    Ok(())
}

fn validate_content_handle(
    handle: &ToolResultContentHandle,
    descriptor: &ToolResultPayloadDescriptor,
) -> Result<(), DaChatApiError> {
    if handle.mode != "presigned"
        || handle.byte_length != descriptor.byte_length
        || handle.sha256 != descriptor.sha256
        || handle.expires_at <= chrono::Utc::now()
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    Ok(())
}

fn validate_signed_url(value: &str, allow_test_loopback: bool) -> Result<Url, DaChatApiError> {
    let url = Url::parse(value).map_err(|_| DaChatApiError::InvalidResponse)?;
    let test_loopback = allow_test_loopback
        && url.scheme() == "http"
        && matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        );
    if (url.scheme() != "https" && !test_loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    Ok(url)
}

async fn read_bounded_body(
    response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, DaChatApiError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(DaChatApiError::PayloadTooLarge);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| DaChatApiError::StorageUnavailable)?;
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return Err(DaChatApiError::PayloadTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn decode_range_response(
    response: reqwest::Response,
    descriptor: &ToolResultPayloadDescriptor,
    expected_start: usize,
    expected_end: usize,
) -> Result<ToolResultRange, DaChatApiError> {
    let total =
        usize::try_from(descriptor.byte_length).map_err(|_| DaChatApiError::InvalidResponse)?;
    let header = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or(DaChatApiError::InvalidResponse)?;
    let expected_header = format!("bytes {expected_start}-{expected_end}/{total}");
    if header != expected_header {
        return Err(DaChatApiError::InvalidResponse);
    }
    let expected_length = expected_end - expected_start + 1;
    let bytes = read_bounded_body(response, expected_length).await?;
    if bytes.len() != expected_length {
        return Err(DaChatApiError::PayloadIntegrityFailed);
    }
    if expected_start == 0 && expected_end + 1 == total && sha256_hex(&bytes) != descriptor.sha256 {
        return Err(DaChatApiError::PayloadIntegrityFailed);
    }
    Ok(ToolResultRange {
        bytes,
        start: expected_start,
        end_inclusive: expected_end,
        total_byte_length: total,
        sha256: descriptor.sha256.clone(),
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Deserialize)]
struct CursorPage<T> {
    items: Option<Vec<T>>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunCreated {
    id: uuid::Uuid,
}

/// How the selected catalog model manages its conversation context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelExecutionMode {
    NativeCompaction,
    BoundedContext,
}

pub(crate) const GROK_MODEL_ID: &str = "~x-ai/grok-latest";
pub(crate) const SONNET_MODEL_ID: &str = "~anthropic/claude-sonnet-latest";
pub(crate) const LUNA_MODEL_ID: &str = "~openai/gpt-luna-latest";
pub(crate) const SOL_MODEL_ID: &str = "~openai/gpt-sol-latest";
pub(crate) const HAIKU_MODEL_ID: &str = "~anthropic/claude-haiku-latest";

/// The pinned GPT-5.6 id Luna had before it moved to OpenRouter's latest-family
/// alias.
///
/// Never offered, and never a default: a saved preference for it resolves to
/// [`LUNA_MODEL_ID`]. It still runs a conversation already locked to it, while
/// the tenant catalog serves the row, because a conversation's model is fixed
/// for its lifetime and dropping the id would strand every Luna conversation.
pub(crate) const RETIRED_LUNA_MODEL_ID: &str = "openai/gpt-5.6-luna";

/// The offered product models in display order. The first eligible row is the
/// default for a person with no saved preference, so Haiku leads.
const SUPPORTED_MODEL_IDS: [&str; 5] = [
    HAIKU_MODEL_ID,
    SONNET_MODEL_ID,
    GROK_MODEL_ID,
    LUNA_MODEL_ID,
    SOL_MODEL_ID,
];

/// A catalog-backed model that satisfies one supported execution contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelSelection {
    pub(crate) model_id: String,
    pub(crate) provider: String,
    pub(crate) context_length: i64,
    pub(crate) max_output_tokens: i64,
    pub(crate) execution_mode: ModelExecutionMode,
    /// Present only for a caller-owned OpenAI-compatible model. The inference
    /// path uses this exact catalog binding rather than resolving a credential
    /// by provider name.
    pub(crate) custom: Option<CustomModelRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CustomModelRoute {
    pub(crate) model_id: Uuid,
    pub(crate) base_url: String,
    pub(crate) upstream_model_id: String,
    pub(crate) provider_key_id: Uuid,
}

pub(crate) const MIN_NATIVE_CONTEXT_TOKENS: i64 = 1_000_000;

/// Resolves the default supported model: the first eligible row in
/// [`SUPPORTED_MODEL_IDS`], which is Haiku. A saved preference overrides it.
/// A conversation made before models were locked stays on Sonnet.
pub(crate) fn select_model(models: &[ModelEntity]) -> Result<ModelSelection, DaChatApiError> {
    select_model_by_id(models, None)
}

/// Resolves a model a person may choose for new chats: an offered product or
/// caller-owned custom model, with the retired Luna id read as its successor.
///
/// For a saved preference and a new default. A conversation's locked model goes
/// through [`select_model_by_id`] instead, which also runs the retired id.
pub(crate) fn select_offered_model(
    models: &[ModelEntity],
    requested_model_id: &str,
) -> Result<ModelSelection, DaChatApiError> {
    let requested = match requested_model_id.trim() {
        RETIRED_LUNA_MODEL_ID => LUNA_MODEL_ID,
        model_id => model_id,
    };
    if !SUPPORTED_MODEL_IDS.contains(&requested)
        && custom_model_uuid(requested).is_none()
        && plan_model_provider(requested).is_none()
    {
        return Err(DaChatApiError::NoModel);
    }
    select_model_by_id(models, Some(requested))
}

/// Resolves one exact model after validating every catalog field that affects
/// routing, tools, or context capacity.
///
/// Product rows carry no custom route. A custom row carries the exact endpoint,
/// upstream model, and key identity that the transport must use. Selection only
/// validates the stored endpoint structure. The transport repeats the check and
/// resolves the host when it connects.
pub(crate) fn select_model_by_id(
    models: &[ModelEntity],
    requested_model_id: Option<&str>,
) -> Result<ModelSelection, DaChatApiError> {
    let requested_model_id = requested_model_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model = match requested_model_id {
        Some(model_id) => models
            .iter()
            .find(|model| model.model_id.trim() == model_id && eligible_model(model)),
        None => SUPPORTED_MODEL_IDS.iter().find_map(|model_id| {
            models
                .iter()
                .find(|model| model.model_id.trim() == *model_id && eligible_model(model))
        }),
    }
    .ok_or_else(|| selection_error(models, requested_model_id))?;
    let custom = custom_model_route(model);
    let execution_mode = if custom.is_some() {
        ModelExecutionMode::BoundedContext
    } else {
        execution_mode(model.model_id.trim()).ok_or(DaChatApiError::NoModel)?
    };
    Ok(ModelSelection {
        model_id: model.model_id.trim().to_owned(),
        provider: model.provider.trim().to_owned(),
        context_length: model.context_length,
        max_output_tokens: model
            .max_output_tokens
            .expect("eligible models have a positive output limit"),
        execution_mode,
        custom,
    })
}

/// Returns the first catalog row that satisfies the full execution contract.
pub(crate) fn pick_default_model(models: &[ModelEntity]) -> Option<&ModelEntity> {
    SUPPORTED_MODEL_IDS.iter().find_map(|model_id| {
        models
            .iter()
            .find(|model| model.model_id.trim() == *model_id && eligible_model(model))
    })
}

pub(crate) fn supported_models(models: &[ModelEntity]) -> Vec<&ModelEntity> {
    let mut supported: Vec<_> = SUPPORTED_MODEL_IDS
        .iter()
        .filter_map(|model_id| {
            models
                .iter()
                .find(|model| model.model_id.trim() == *model_id && eligible_model(model))
        })
        .collect();
    supported.extend(models.iter().filter(|model| {
        custom_model_uuid(model.model_id.trim()).is_some() && eligible_model(model)
    }));
    supported.extend(models.iter().filter(|model| {
        plan_model_provider(model.model_id.trim()).is_some() && eligible_model(model)
    }));
    supported
}

fn execution_mode(model_id: &str) -> Option<ModelExecutionMode> {
    match model_id {
        SONNET_MODEL_ID => Some(ModelExecutionMode::NativeCompaction),
        GROK_MODEL_ID | LUNA_MODEL_ID | SOL_MODEL_ID | HAIKU_MODEL_ID | RETIRED_LUNA_MODEL_ID => {
            Some(ModelExecutionMode::BoundedContext)
        }
        // A plan runtime has no native compaction contract, so Desktop bounds
        // the context itself, as it does for a private model.
        _ => plan_model_provider(model_id).map(|_| ModelExecutionMode::BoundedContext),
    }
}

fn eligible_model(model: &ModelEntity) -> bool {
    let Some(max_output_tokens) = model.max_output_tokens else {
        return false;
    };
    let valid_capacity = model.context_length > 0
        && max_output_tokens > 0
        && max_output_tokens < model.context_length;
    let display_name = model.display_name.trim();
    let common_contract = !display_name.is_empty()
        && display_name.chars().count() <= 128
        && model.is_chat
        && model.is_enabled
        && model.is_active
        && model.supports_streaming
        && model.supports_tools
        && valid_capacity;
    if !common_contract {
        return false;
    }

    if custom_model_uuid(model.model_id.trim()).is_some() {
        return custom_model_route(model).is_some()
            && ContextCapacity::new(model.context_length, max_output_tokens).is_ok();
    }

    if let Some(provider) = plan_model_provider(model.model_id.trim()) {
        // The provider fields must name the same runtime as the id, so a row
        // cannot borrow another provider's route. Only the device source reaches
        // this branch with a plan id: `list_model_rows` drops DA's rows.
        return model.provider.trim() == provider
            && model.provider_parent.trim() == provider
            && ContextCapacity::new(model.context_length, max_output_tokens).is_ok();
    }

    let Some(mode) = execution_mode(model.model_id.trim()) else {
        return false;
    };
    model.provider.trim().eq_ignore_ascii_case("openrouter")
        && model
            .provider_parent
            .trim()
            .eq_ignore_ascii_case("openrouter")
        && (mode != ModelExecutionMode::NativeCompaction
            || model.context_length >= MIN_NATIVE_CONTEXT_TOKENS)
}

fn selection_error(models: &[ModelEntity], requested_model_id: Option<&str>) -> DaChatApiError {
    let Some(model_id) = requested_model_id.filter(|id| custom_model_uuid(id).is_some()) else {
        return DaChatApiError::NoModel;
    };
    if models.iter().any(|model| {
        model.model_id == model_id
            && model.is_chat
            && model.is_enabled
            && model.is_active
            && model.supports_streaming
            && model.supports_tools
            && custom_model_route(model).is_some()
            && model.max_output_tokens.is_some_and(|output| {
                output > 0
                    && output < model.context_length
                    && ContextCapacity::new(model.context_length, output).is_err()
            })
    }) {
        DaChatApiError::CustomModelCapacityUnsupported
    } else {
        DaChatApiError::CustomModelUnavailable
    }
}

fn custom_model_uuid(model_id: &str) -> Option<Uuid> {
    Uuid::parse_str(model_id.strip_prefix("custom/")?).ok()
}

/// Model ids of the device-local plan runtimes are `claude_code/<id>` and
/// `chatgpt_plan/<id>`. Their rows are built on this device from the runtime's
/// own state and merged into the catalog; DA never sends one.
pub(crate) const CLAUDE_MODEL_PREFIX: &str = "claude_code/";
pub(crate) const CHATGPT_MODEL_PREFIX: &str = "chatgpt_plan/";

/// Whether an id names a device-local plan model. Matched on the prefix alone,
/// so a DA row with such an id is recognised whatever its provider fields say.
pub(crate) fn is_plan_model_id(model_id: &str) -> bool {
    let model_id = model_id.trim();
    model_id.starts_with(CLAUDE_MODEL_PREFIX) || model_id.starts_with(CHATGPT_MODEL_PREFIX)
}

/// The provider a plan model id belongs to, or `None` for every other id.
pub(crate) fn plan_model_provider(model_id: &str) -> Option<&'static str> {
    let model_id = model_id.trim();
    if model_id.len() > CLAUDE_MODEL_PREFIX.len() && model_id.starts_with(CLAUDE_MODEL_PREFIX) {
        Some("claude_code")
    } else if model_id.len() > CHATGPT_MODEL_PREFIX.len()
        && model_id.starts_with(CHATGPT_MODEL_PREFIX)
    {
        Some("chatgpt_plan")
    } else {
        None
    }
}

fn custom_model_route(model: &ModelEntity) -> Option<CustomModelRoute> {
    let custom_id = custom_model_uuid(model.model_id.trim())?;
    let owner_user_id = model.owner_user_id.as_deref()?;
    if custom_id != model.id
        || model.model_id != format!("custom/{}", model.id)
        || model.provider != "custom_openai"
        || model.provider_parent != "custom_openai"
        || model.tenant_id.trim().is_empty()
        || owner_user_id.is_empty()
        || owner_user_id.trim() != owner_user_id
        || owner_user_id.bytes().any(|byte| byte < 32 || byte == 127)
    {
        return None;
    }

    let base_url = model.base_url.as_deref()?;
    let upstream_model_id = model.upstream_model_id.as_deref()?;
    let provider_key_id = model.provider_key_id.filter(|id| !id.is_nil())?;
    if !valid_custom_base_url(base_url) || !valid_custom_upstream_model_id(upstream_model_id) {
        return None;
    }

    Some(CustomModelRoute {
        model_id: model.id,
        base_url: base_url.to_owned(),
        upstream_model_id: upstream_model_id.to_owned(),
        provider_key_id,
    })
}

fn valid_custom_base_url(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 2_048
        || !value.is_ascii()
        || value.trim() != value
        || value.bytes().any(|byte| byte < 33 || byte == 127)
        || value.contains('\\')
    {
        return false;
    }
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.port() != Some(0)
        && url.query().is_none()
        && url.fragment().is_none()
}

fn valid_custom_upstream_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 256
        && value.trim() == value
        && !value.bytes().any(|byte| byte < 32 || byte == 127)
}

pub(crate) fn summarize_mcp_servers(
    bootstrap: &WorkspaceBootstrap,
    bundled_endpoint: &Url,
) -> Vec<(String, String, String, bool)> {
    let Some(servers) = bootstrap.mcp_servers.as_ref() else {
        return Vec::new();
    };
    servers
        .iter()
        // Workspace search and composer mentions consume this summary rather
        // than the native MCP registry view. Apply the same endpoint identity
        // here so a tenant alias cannot disappear from Settings yet survive as
        // a second, unusable mention target elsewhere in the app.
        .filter(|value| !workspace_server_matches_endpoint(value, bundled_endpoint))
        .filter_map(|value| {
            let object = value.as_object()?;
            let id = object
                .get("id")
                .and_then(|v| v.as_str())
                .or_else(|| object.get("server_id").and_then(|v| v.as_str()))
                .unwrap_or("unknown")
                .to_owned();
            let name = object
                .get("name")
                .or_else(|| object.get("display_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("Tenant MCP server")
                .to_owned();
            let transport = object
                .get("transport")
                .or_else(|| object.get("connection_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("streamable_http")
                .to_owned();
            let enabled = object
                .get("is_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Some((id, name, transport, enabled))
        })
        .collect()
}

fn workspace_server_matches_endpoint(value: &serde_json::Value, endpoint: &Url) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("url"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| Url::parse(raw).ok())
        .is_some_and(|candidate| candidate == *endpoint)
}
