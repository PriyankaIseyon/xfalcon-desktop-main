//! Fixed, run-scoped access to exact tool-result payload ranges.
//!
//! Admission is a frozen set copied from the authoritative DA context before
//! the provider run starts. A model-supplied payload id is never sufficient:
//! the executor first resolves it in that set, then re-reads DA metadata and
//! requires the durable identity to match before any content request.

use std::collections::HashMap;

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::tools::{ToolId, ToolSource};

use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort};
use crate::clients::da::chat_api::{DaChatApiError, DaChatSession};
use crate::clients::da::generated::{
    PayloadStatus, ToolResultPayload, ToolResultPayloadDescriptor,
};
use crate::tools::invocation::ToolResult;
use crate::tools::registry::READ_TOOL_RESULT_TOOL_ID;
use crate::tools::ToolResultBudget;

/// Durable identity admitted by one run's authoritative context snapshot.
///
/// It intentionally omits previews and storage details. Neither is required
/// to identify exact bytes, and carrying them into the executor would widen
/// the data that an error or debug implementation could expose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedToolResultPayload {
    pub(crate) id: Uuid,
    pub(crate) byte_length: usize,
    pub(crate) sha256: String,
    pub(crate) media_type: String,
    pub(crate) serialization_format: String,
    pub(crate) serialization_version: i64,
    pub(crate) content_encoding: String,
}

impl AdmittedToolResultPayload {
    fn matches(&self, payload: &ToolResultPayload) -> bool {
        payload.id == self.id
            && usize::try_from(payload.byte_length).ok() == Some(self.byte_length)
            && payload.sha256 == self.sha256
            && payload.media_type.to_string() == self.media_type
            && payload.serialization_format.to_string() == self.serialization_format
            && payload.serialization_version == self.serialization_version
            && payload.content_encoding == self.content_encoding
            && matches!(payload.status, PayloadStatus::Ready)
            && payload.retrieval_available
    }
}

/// Executes only the exact-result tool against one conversation and one
/// immutable admission set.
pub(crate) struct ExactResultToolExecutor {
    da: DaChatSession,
    conversation_id: Uuid,
    admitted: HashMap<Uuid, AdmittedToolResultPayload>,
    /// The run's range ceiling — the largest range whose base64 fits one of
    /// its tool results.
    max_range_bytes: usize,
}

impl ExactResultToolExecutor {
    #[must_use]
    pub(crate) fn new(
        da: DaChatSession,
        conversation_id: Uuid,
        admitted: impl IntoIterator<Item = AdmittedToolResultPayload>,
        budget: ToolResultBudget,
    ) -> Self {
        Self {
            da,
            conversation_id,
            max_range_bytes: budget.range_bytes(),
            admitted: admitted
                .into_iter()
                .map(|payload| (payload.id, payload))
                .collect(),
        }
    }

    #[must_use]
    pub(crate) fn is_available(&self) -> bool {
        !self.admitted.is_empty()
    }

    async fn read(&self, arguments: &Value) -> Result<ToolResult, ToolExecutionError> {
        let request: ReadRangeRequest = serde_json::from_value(arguments.clone())
            .map_err(|_| ToolExecutionError::Failed("invalid exact-result range".to_owned()))?;
        let Some(admitted) = self.admitted.get(&request.payload_id) else {
            return Err(ToolExecutionError::Failed(
                "exact-result payload is not admitted for this run".to_owned(),
            ));
        };
        let offset = usize::try_from(request.offset)
            .map_err(|_| ToolExecutionError::Failed("invalid exact-result range".to_owned()))?;
        let length = usize::try_from(request.length)
            .map_err(|_| ToolExecutionError::Failed("invalid exact-result range".to_owned()))?;
        validate_range(admitted.byte_length, offset, length, self.max_range_bytes)?;

        // Metadata is read before bytes. This proves the resource still
        // belongs to the path-scoped conversation and has the exact durable
        // identity admitted at context load.
        let payload = self
            .da
            .get_tool_result_payload(self.conversation_id, admitted.id)
            .await
            .map_err(map_da_error)?;
        if !admitted.matches(&payload) {
            return Err(ToolExecutionError::Failed(
                "exact-result payload failed integrity validation".to_owned(),
            ));
        }

        let descriptor = descriptor_from_payload(&payload);
        let range = self
            .da
            .read_tool_result_range(self.conversation_id, &descriptor, offset, length)
            .await
            .map_err(map_da_error)?;
        if range.start != offset
            || range.end_inclusive.checked_add(1) != offset.checked_add(length)
            || range.total_byte_length != admitted.byte_length
            || range.sha256 != admitted.sha256
            || range.bytes.len() != length
        {
            return Err(ToolExecutionError::Failed(
                "exact-result payload failed integrity validation".to_owned(),
            ));
        }
        let next_offset = range.end_inclusive + 1;
        let returned_byte_length = range.bytes.len();
        let structured = json!({
            "trust": "untrusted_tool_data",
            "payload_id": admitted.id,
            "offset": range.start,
            "end_exclusive": next_offset,
            "returned_byte_length": returned_byte_length,
            "total_byte_length": range.total_byte_length,
            "media_type": admitted.media_type,
            "serialization_format": admitted.serialization_format,
            "serialization_version": admitted.serialization_version,
            "content_encoding": admitted.content_encoding,
            "sha256": admitted.sha256,
            "data_encoding": "base64",
            "data": base64::engine::general_purpose::STANDARD.encode(range.bytes),
            "has_more": next_offset < range.total_byte_length,
            "next_offset": (next_offset < range.total_byte_length).then_some(next_offset),
        });
        ToolResult::new(Vec::new(), Some(structured), ToolSource::Internal).map_err(Into::into)
    }
}

#[async_trait]
impl ToolExecutorPort for ExactResultToolExecutor {
    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        if tool_id.as_str() != READ_TOOL_RESULT_TOOL_ID {
            return Err(ToolExecutionError::Failed(
                "tool is not the exact-result reader".to_owned(),
            ));
        }
        if cancel.is_cancelled() {
            return Err(ToolExecutionError::Cancelled);
        }
        tokio::select! {
            () = cancel.cancelled() => Err(ToolExecutionError::Cancelled),
            result = self.read(arguments) => result,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRangeRequest {
    payload_id: Uuid,
    offset: u64,
    length: u64,
}

fn validate_range(
    total: usize,
    offset: usize,
    length: usize,
    max_length: usize,
) -> Result<(), ToolExecutionError> {
    if length > max_length {
        return Err(ToolExecutionError::Failed(format!(
            "length must be at most {max_length} bytes for this run"
        )));
    }
    if length == 0 || offset >= total {
        return Err(ToolExecutionError::Failed(
            "invalid exact-result range".to_owned(),
        ));
    }
    let Some(end) = offset.checked_add(length) else {
        return Err(ToolExecutionError::Failed(
            "invalid exact-result range".to_owned(),
        ));
    };
    if end > total {
        return Err(ToolExecutionError::Failed(
            "invalid exact-result range".to_owned(),
        ));
    }
    Ok(())
}

fn descriptor_from_payload(payload: &ToolResultPayload) -> ToolResultPayloadDescriptor {
    ToolResultPayloadDescriptor {
        byte_length: payload.byte_length,
        content_encoding: payload.content_encoding.clone(),
        id: payload.id,
        media_type: payload.media_type.clone(),
        preview: payload.preview.clone(),
        preview_byte_length: payload.preview_byte_length,
        preview_strategy: payload.preview_strategy.clone(),
        retrieval_available: payload.retrieval_available,
        serialization_format: payload.serialization_format.clone(),
        serialization_version: payload.serialization_version,
        sha256: payload.sha256.clone(),
        status: payload.status.clone(),
        storage_mode: payload.storage_mode.clone(),
    }
}

fn map_da_error(error: DaChatApiError) -> ToolExecutionError {
    match error {
        DaChatApiError::Unauthorized => ToolExecutionError::AuthorizationRefused {
            label: "Data Analyst".to_owned(),
        },
        DaChatApiError::Timeout => ToolExecutionError::Timeout,
        // A rate limit is not a slow query: narrowing the call will not help,
        // waiting will.
        DaChatApiError::RateLimited => ToolExecutionError::RateLimited { retry_after: None },
        DaChatApiError::PayloadIntegrityFailed | DaChatApiError::InvalidResponse => {
            ToolExecutionError::Failed(
                "exact-result payload failed integrity validation".to_owned(),
            )
        }
        DaChatApiError::NotFound | DaChatApiError::PayloadNotReady => ToolExecutionError::Failed(
            "exact-result payload is not available for this run".to_owned(),
        ),
        _ => ToolExecutionError::Failed("exact-result range retrieval failed".to_owned()),
    }
}
