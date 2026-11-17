//! Local (Path A) run executor — streams an allowlisted provider directly from the
//! Rust desktop core and durably mirrors every delta to DA as it arrives.
//!
//! Uses the SAME `run.*` event-type vocabulary [`super::da_events::map_observed_event`]
//! already reads back for DA-polled (Path B) runs, so a run executed here is
//! resumable/observable exactly like any other run — from this device or any other.
//! Provider delivery and one ordered DA write are polled independently. The
//! synchronous adapter callback enqueues only during a provider poll; a bounded
//! history backlog pauses further provider reads while presentation keeps its
//! own cadence. Accepted records drain before successful turn finalization.
//!
//! # One streaming core, two finalization policies
//!
//! [`stream_turn`] is the whole streaming/mirroring/cancellation core and
//! finalizes nothing. Two callers layer their own finalization on top:
//! [`LocalRunExecutor::run`] (exactly one turn, then finalize — the tool-free
//! path) and [`super::turn_driver::LocalTurnDriver`] (one turn per tool round,
//! finalize only on the turn that asks for no tools). Keeping the core shared
//! is deliberate: the two paths cannot drift apart on delta batching, usage
//! accounting, or provider-stream cancellation, which is exactly the class of
//! divergence a duplicated loop would invite.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;
#[cfg(test)]
use xfalcon_contracts::credentials::ModelId;
use xfalcon_contracts::credentials::ProviderId;
#[cfg(test)]
use xfalcon_contracts::inference::FinishReason;
use xfalcon_contracts::inference::{InferenceErrorCode, InferenceEvent};
use xfalcon_contracts::RunId;
use zeroize::Zeroizing;

use crate::clients::da::chat_api::{DaChatApiError, DaChatSession};
use crate::clients::da::generated::{
    BatchRecord, BatchRecordType, ContextCompaction, ContextCompactionActivate, PayloadMediaType,
    PayloadPreviewStrategy, PayloadSerializationFormat, PayloadStatus, RunFinalize,
    RunFinalizeExpectedFrom, RunFinalizeResult, ToolCallCreate, ToolResultPayloadPrepare,
};
use crate::providers::{
    ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStopReason, ProviderStreamEvent,
    ProviderUsage,
};
use crate::tools::{ToolResult, ToolResultPreviewStrategy};

use super::context::{ContextMessage, ContextToolCall};
use super::output::OutputError;
use super::output::RunOutputBuilder;
#[cfg(test)]
use super::stream::CoalesceConfig;
use super::stream::{InferencePresentationSink, StreamError};
use super::turn_driver::{json_object, tool_output_object};

const MAX_BATCH_RECORDS: usize = 20;
const MAX_BATCH_INTERVAL: Duration = Duration::from_millis(250);
const MAX_PENDING_BATCHES: usize = 64;
const MAX_PENDING_HISTORY_BYTES: usize = 2 * 1024 * 1024;
const NON_TERMINAL_STATUSES: [&str; 2] = ["queued", "running"];

#[derive(Debug, Error)]
#[cfg(test)]
pub(crate) enum LocalExecutorError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Sink(#[from] RunEventSinkError),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error(transparent)]
    Presentation(#[from] StreamError),
    #[error("local executor cannot satisfy tool calls")]
    ToolsUnavailable,
    #[error("run cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct LocalRunOutcome {
    pub finish_reason: FinishReason,
    pub final_text: String,
    pub presentation: Vec<InferenceEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(crate) enum RunEventSinkError {
    #[error("da run event sink is unavailable")]
    Unavailable,
    #[error("da rejected the finalize — a competing terminal write already landed")]
    Conflict,
    #[error("the durable context changed before activation")]
    ContextConflict,
    #[error("the durable context snapshot is invalid")]
    InvalidContext,
}

pub(crate) struct DurableContextSnapshot {
    pub(crate) active_compaction: Option<ContextCompaction>,
    pub(crate) history_revision: i64,
    pub(crate) snapshot_head_sequence_no: i64,
    pub(crate) boundary_sequence_no: i64,
    pub(crate) compactable_through_sequence_no: i64,
    pub(crate) compactable_source_message_count: i64,
    pub(crate) source_message_ids_sha256: String,
    pub(crate) messages: Vec<ContextMessage>,
}

/// DA transport for durable run-event mirroring — a thin seam so [`LocalRunExecutor`] is
/// unit-testable without a live DA session.
#[async_trait]
pub(crate) trait RunEventSink: Send + Sync {
    async fn apply_batch(
        &self,
        run_id: Uuid,
        records: Vec<BatchRecord>,
    ) -> Result<(), RunEventSinkError>;
    async fn finalize(
        &self,
        run_id: Uuid,
        finalize: RunFinalize,
    ) -> Result<RunFinalizeResult, RunEventSinkError>;
    /// Writes the assistant turn that requested `calls`, and then one message
    /// answering each of them — the durable shape a provider validates, and the
    /// one xfalcon-web writes for the same rounds.
    ///
    /// The DA implementation writes conversation messages, execution-log rows,
    /// and exact payload bindings as one fail-closed operation.
    async fn record_tool_round(
        &self,
        conversation_id: Uuid,
        round: ToolRoundRecord,
        on_progress: &mut (dyn FnMut() + Send),
    ) -> Result<Vec<String>, RunEventSinkError>;

    async fn load_context(
        &self,
        _conversation_id: Uuid,
    ) -> Result<DurableContextSnapshot, RunEventSinkError> {
        Err(RunEventSinkError::Unavailable)
    }

    async fn activate_compaction(
        &self,
        _conversation_id: Uuid,
        _body: ContextCompactionActivate,
    ) -> Result<ContextCompaction, RunEventSinkError> {
        Err(RunEventSinkError::Unavailable)
    }

    async fn reconcile_compaction(
        &self,
        _conversation_id: Uuid,
        _compaction_id: Uuid,
    ) -> Result<Option<ContextCompaction>, RunEventSinkError> {
        Err(RunEventSinkError::Unavailable)
    }
}

/// One completed tool round, as it is written to the conversation.
pub(crate) struct ToolRoundRecord {
    pub(crate) run_id: Uuid,
    pub(crate) round: u32,
    /// Whatever prose the assistant produced alongside its calls. Often empty.
    pub(crate) text: String,
    /// The calls the turn requested, in provider order.
    pub(crate) calls: Vec<ToolRoundCall>,
    /// The run's tool-result budget. The durable preview is cut to the same
    /// budget the rendered answer was, so the two describe one result.
    pub(crate) budget: crate::tools::ToolResultBudget,
}

/// One call, and the answer written for it.
pub(crate) struct ToolRoundCall {
    /// The provider's own call id — what pairs the answer to the request.
    pub(crate) call_id: String,
    pub(crate) tool_name: String,
    pub(crate) arguments: String,
    /// The result body, already bounded and serialized by the caller.
    pub(crate) result: String,
    pub(crate) exact_result: Option<ToolResult>,
}

#[async_trait]
impl RunEventSink for DaChatSession {
    async fn apply_batch(
        &self,
        run_id: Uuid,
        records: Vec<BatchRecord>,
    ) -> Result<(), RunEventSinkError> {
        self.apply_event_batch(run_id, records)
            .await
            .map(|_applied| ())
            .map_err(map_sink_error)
    }

    async fn finalize(
        &self,
        run_id: Uuid,
        finalize: RunFinalize,
    ) -> Result<RunFinalizeResult, RunEventSinkError> {
        self.finalize_run(run_id, finalize)
            .await
            .map_err(map_sink_error)
    }

    async fn record_tool_round(
        &self,
        conversation_id: Uuid,
        round: ToolRoundRecord,
        on_progress: &mut (dyn FnMut() + Send),
    ) -> Result<Vec<String>, RunEventSinkError> {
        let calls: Vec<Value> = round
            .calls
            .iter()
            .map(|call| {
                serde_json::json!({
                    "id": call.call_id,
                    "type": "function",
                    "function": {"name": call.tool_name, "arguments": call.arguments},
                })
            })
            .collect();
        // Order is the contract: the assistant turn advertising the calls must
        // precede the messages answering them, or a literal replay presents a
        // tool result for a call nothing asked for.
        let assistant_id = stable_resource_id(
            "assistant-tool-round",
            conversation_id,
            round.run_id,
            &round.round.to_string(),
        );
        let assistant_write = self
            .create_assistant_tool_call_message(
                conversation_id,
                assistant_id,
                &round.text,
                calls.clone(),
                &assistant_id.to_string(),
            )
            .await;
        match assistant_write {
            Ok(_) => {}
            Err(DaChatApiError::RequestFailed) => {
                self.create_assistant_tool_call_message(
                    conversation_id,
                    assistant_id,
                    &round.text,
                    calls,
                    &assistant_id.to_string(),
                )
                .await
                .map_err(map_sink_error)?;
            }
            Err(error) => return Err(map_sink_error(error)),
        }
        on_progress();
        let mut durable_results = Vec::with_capacity(round.calls.len());
        for (index, call) in round.calls.into_iter().enumerate() {
            let status = durable_tool_status(&call.result).to_owned();
            let tool_call_idempotency =
                stable_resource_id("tool-call", conversation_id, round.run_id, &call.call_id);
            let tool_call_body = ToolCallCreate {
                conversation_id,
                message_id: Some(assistant_id),
                run_id: Some(round.run_id),
                provider_tool_call_id: Some(call.call_id.clone()),
                tool_name: call.tool_name.clone(),
                status: Some(status),
                input: json_object(&call.arguments),
                output: tool_output_object(&call.result),
                sequence_no: Some(i64::try_from(index).unwrap_or(i64::MAX)),
                ..Default::default()
            };
            let tool_call = match self
                .create_tool_call(tool_call_body.clone(), &tool_call_idempotency.to_string())
                .await
            {
                Ok(row) => row,
                Err(DaChatApiError::RequestFailed) => self
                    .create_tool_call(tool_call_body, &tool_call_idempotency.to_string())
                    .await
                    .map_err(map_sink_error)?,
                Err(error) => return Err(map_sink_error(error)),
            };
            on_progress();

            let (durable_result, payload_ids) = match call.exact_result {
                Some(exact_result) => {
                    let exact = exact_result.exact_json_bytes();
                    let preview = exact_result.model_preview(round.budget);
                    let durable_preview = serde_json::to_string(preview.value())
                        .map_err(|_| RunEventSinkError::InvalidContext)?;
                    let payload_id = stable_resource_id(
                        "tool-result-payload",
                        conversation_id,
                        round.run_id,
                        &call.call_id,
                    );
                    let prepare = ToolResultPayloadPrepare {
                        id: payload_id,
                        tool_call_id: tool_call.id,
                        run_id: Some(round.run_id),
                        provider_tool_call_id: call.call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        media_type: PayloadMediaType::ApplicationJson,
                        serialization_format: PayloadSerializationFormat::Json,
                        serialization_version: 1,
                        content_encoding: Some("identity".to_owned()),
                        byte_length: i64::try_from(exact.len()).unwrap_or(i64::MAX),
                        sha256: preview.sha256().to_owned(),
                        preview: Some(durable_preview),
                        preview_strategy: match preview.strategy() {
                            ToolResultPreviewStrategy::Complete => PayloadPreviewStrategy::Complete,
                            ToolResultPreviewStrategy::JsonStructure => {
                                PayloadPreviewStrategy::SchemaRows
                            }
                            ToolResultPreviewStrategy::TextEdges => PayloadPreviewStrategy::Head,
                            ToolResultPreviewStrategy::MetadataOnly => PayloadPreviewStrategy::None,
                        },
                        inline_content_base64: (exact.len() <= 64 * 1024)
                            .then(|| base64::engine::general_purpose::STANDARD.encode(&exact)),
                        metadata: None,
                    };
                    let prepared = match self
                        .prepare_tool_result_payload(
                            conversation_id,
                            &prepare,
                            &payload_id.to_string(),
                        )
                        .await
                    {
                        Ok(prepared) => prepared,
                        Err(DaChatApiError::RequestFailed) => self
                            .prepare_tool_result_payload(
                                conversation_id,
                                &prepare,
                                &payload_id.to_string(),
                            )
                            .await
                            .map_err(map_sink_error)?,
                        Err(error) => return Err(map_sink_error(error)),
                    };
                    on_progress();
                    let finalized = if prepared.payload.status == PayloadStatus::Ready {
                        if prepared.upload.mode
                            != crate::clients::da::generated::UploadInstructionMode::Inline
                        {
                            return Err(RunEventSinkError::InvalidContext);
                        }
                        self.upload_tool_result_payload(
                            conversation_id,
                            &prepared.payload,
                            &prepared.upload,
                            &exact,
                        )
                        .await
                        .map_err(map_sink_error)?;
                        on_progress();
                        prepared.payload
                    } else {
                        if let Err(error) = self
                            .upload_tool_result_payload(
                                conversation_id,
                                &prepared.payload,
                                &prepared.upload,
                                &exact,
                            )
                            .await
                        {
                            if !matches!(
                                error,
                                DaChatApiError::StorageUnavailable | DaChatApiError::RequestFailed
                            ) {
                                return Err(map_sink_error(error));
                            }
                            self.upload_tool_result_payload(
                                conversation_id,
                                &prepared.payload,
                                &prepared.upload,
                                &exact,
                            )
                            .await
                            .map_err(map_sink_error)?;
                        }
                        on_progress();
                        let finalized = self
                            .finalize_tool_result_payload_reconciled(conversation_id, payload_id)
                            .await
                            .map_err(map_sink_error)?;
                        on_progress();
                        finalized
                    };
                    if finalized.status != PayloadStatus::Ready
                        || finalized.sha256 != preview.sha256()
                        || finalized.byte_length != i64::try_from(exact.len()).unwrap_or(i64::MAX)
                    {
                        return Err(RunEventSinkError::InvalidContext);
                    }
                    (
                        attach_payload_reference(&call.result, payload_id),
                        vec![payload_id],
                    )
                }
                None => (call.result, Vec::new()),
            };
            let result_message_id = stable_resource_id(
                "tool-result-message",
                conversation_id,
                round.run_id,
                &call.call_id,
            );
            let result_write = self
                .create_tool_result_message_with_payloads(
                    conversation_id,
                    result_message_id,
                    &call.call_id,
                    &call.tool_name,
                    &durable_result,
                    payload_ids.clone(),
                    &result_message_id.to_string(),
                )
                .await;
            match result_write {
                Ok(_) => {}
                Err(DaChatApiError::RequestFailed) => {
                    self.create_tool_result_message_with_payloads(
                        conversation_id,
                        result_message_id,
                        &call.call_id,
                        &call.tool_name,
                        &durable_result,
                        payload_ids,
                        &result_message_id.to_string(),
                    )
                    .await
                    .map_err(map_sink_error)?;
                }
                Err(error) => return Err(map_sink_error(error)),
            }
            on_progress();
            durable_results.push(durable_result);
        }
        Ok(durable_results)
    }

    async fn load_context(
        &self,
        conversation_id: Uuid,
    ) -> Result<DurableContextSnapshot, RunEventSinkError> {
        let snapshot = self
            .read_context_snapshot(conversation_id)
            .await
            .map_err(map_sink_error)?;
        let messages =
            crate::chat::history::context_from_snapshot(&snapshot.messages).map_err(|error| {
                tracing::error!(
                    conversation = %conversation_id,
                    %error,
                    "durable context snapshot projection failed"
                );
                RunEventSinkError::InvalidContext
            })?;
        Ok(DurableContextSnapshot {
            active_compaction: snapshot.active_compaction,
            history_revision: snapshot.history_revision,
            snapshot_head_sequence_no: snapshot.snapshot_head_sequence_no,
            boundary_sequence_no: snapshot.boundary_sequence_no,
            compactable_through_sequence_no: snapshot.compactable_through_sequence_no,
            compactable_source_message_count: snapshot.compactable_source_message_count,
            source_message_ids_sha256: snapshot.source_message_ids_sha256,
            messages,
        })
    }

    async fn activate_compaction(
        &self,
        conversation_id: Uuid,
        body: ContextCompactionActivate,
    ) -> Result<ContextCompaction, RunEventSinkError> {
        self.activate_context_compaction(conversation_id, &body)
            .await
            .map_err(map_sink_error)
    }

    async fn reconcile_compaction(
        &self,
        conversation_id: Uuid,
        compaction_id: Uuid,
    ) -> Result<Option<ContextCompaction>, RunEventSinkError> {
        self.reconcile_context_compaction(conversation_id, compaction_id)
            .await
            .map_err(map_sink_error)
    }
}

fn map_sink_error(error: DaChatApiError) -> RunEventSinkError {
    if matches!(
        error,
        DaChatApiError::InvalidResponse
            | DaChatApiError::Validation
            | DaChatApiError::IdempotencyConflict
            | DaChatApiError::PayloadIntegrityFailed
            | DaChatApiError::PayloadNotReady
    ) {
        tracing::error!(%error, "durable run-event persistence rejected data");
    }
    match error {
        DaChatApiError::Conflict => RunEventSinkError::Conflict,
        DaChatApiError::ContextConflict | DaChatApiError::ContextSnapshotStale => {
            RunEventSinkError::ContextConflict
        }
        DaChatApiError::InvalidResponse
        | DaChatApiError::Validation
        | DaChatApiError::IdempotencyConflict
        | DaChatApiError::PayloadIntegrityFailed
        | DaChatApiError::PayloadNotReady => RunEventSinkError::InvalidContext,
        _ => RunEventSinkError::Unavailable,
    }
}

/// Maps a rendered tool result onto the statuses `xf_chat.tool_calls` accepts.
///
/// The table's `tool_calls_status_check` admits only `pending`, `running`,
/// `completed`, `error`, `cancelled` and `awaiting_approval`. A tool result's own
/// `status` is wider — `failed`, `denied` and `invalid_arguments` are all normal
/// answers the model reads and recovers from — and writing one verbatim is a 422
/// that ends the run and leaves the round half-saved. The full result still
/// travels in `output`, so nothing the model sees is lost by folding these.
fn durable_tool_status(rendered: &str) -> &'static str {
    let status = serde_json::from_str::<Value>(rendered)
        .ok()
        .and_then(|value| value.get("status")?.as_str().map(str::to_owned));
    match status.as_deref() {
        Some("completed") => "completed",
        Some("cancelled") => "cancelled",
        _ => "error",
    }
}

fn stable_resource_id(kind: &str, conversation_id: Uuid, run_id: Uuid, suffix: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{kind}:{conversation_id}:{run_id}:{suffix}").as_bytes(),
    )
}

fn attach_payload_reference(rendered: &str, payload_id: Uuid) -> String {
    let mut value = serde_json::from_str::<Value>(rendered)
        .unwrap_or_else(|_| serde_json::json!({"status": "completed", "result": rendered}));
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "payload_id".to_owned(),
            Value::String(payload_id.to_string()),
        );
        object.insert("payload_ready".to_owned(), Value::Bool(true));
    }
    value.to_string()
}

/// One provider turn's normalized result — everything a caller needs to
/// decide whether the run is finished or must dispatch tools, with none of the
/// finalization policy baked in.
///
/// Deliberately *not* finalizing is what makes multi-round tool calling
/// possible: [`LocalRunExecutor::run`] finalizes after exactly one of these,
/// while [`super::turn_driver::LocalTurnDriver`] finalizes only after the turn
/// that carries no tool calls.
pub(crate) struct StreamedTurn {
    /// Text this turn alone produced — distinct from the run-wide
    /// accumulation in [`RunOutputBuilder`], because the assistant context
    /// message for a tool round must carry only that round's own prose.
    pub text: String,
    pub tool_calls: Vec<ContextToolCall>,
    /// The provider's reasoning blocks for this turn, already merged from
    /// their streamed fragments. Belongs on the assistant context message the
    /// driver builds from this turn, so the next round can echo it back.
    pub reasoning_details: Vec<Value>,
    pub compactions: Vec<StreamedCompaction>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    /// The provider's own count of the prompt it just processed, cache
    /// included. `None` when the wire does not report one.
    pub prompt_tokens: Option<u64>,
    pub saw_completed: bool,
    /// Why the provider ended the turn. [`ProviderStopReason::Natural`] when
    /// the adapter never said.
    pub stop: ProviderStopReason,
    /// Tool calls the provider began and never finished; they were dropped
    /// before reaching [`Self::tool_calls`].
    pub discarded_tool_calls: u32,
}

pub(crate) struct StreamedCompaction {
    pub index: u32,
    pub block: Value,
    pub usage: ProviderUsage,
}

/// Why one turn stopped short of producing a [`StreamedTurn`]. Each variant
/// maps to exactly one [`LocalExecutorError`]; the split exists so
/// [`stream_turn`] can stay free of the finalization policy that differs
/// between a single-turn run and a multi-round driver.
pub(crate) enum TurnStreamError {
    Sink(RunEventSinkError),
    Provider(ProviderError),
    Output(OutputError),
    Presentation(StreamError),
    Cancelled,
}

/// The two cancellation scopes one turn observes.
///
/// They are the same token for a single-turn run, and deliberately *not* the
/// same for a multi-round driver: [`stream_turn`] always cancels `stream` on
/// the way out so a half-drained provider stream is never orphaned, which
/// would end the whole run if that token were also `run`.
pub(crate) struct TurnCancellation<'a> {
    /// The run's own token. User cancellation is read from here, and never
    /// cancelled by [`stream_turn`].
    pub run: &'a tokio_util::sync::CancellationToken,
    /// The token the provider stream is polled under, cancelled once this
    /// turn stops draining it. A driver passes a per-turn child of `run`.
    pub stream: &'a tokio_util::sync::CancellationToken,
    pub activity: Option<&'a super::activity::RunActivity>,
}

#[cfg(test)]
pub(crate) struct LocalRunExecutor<S> {
    sink: S,
}

#[cfg(test)]
impl<S> LocalRunExecutor<S>
where
    S: RunEventSink,
{
    #[must_use]
    pub fn new(sink: S) -> Self {
        Self { sink }
    }

    /// Streams one chat turn from `adapter`, mirroring every delta to DA and emitting
    /// normalized [`InferenceEvent`]s to `on_inference`, then atomically finalizes the run.
    /// Returns the same outcome shape [`super::engine::RunEngine::poll_da_run`] produces so
    /// callers do not need to branch on which path executed.
    ///
    /// This is the **tool-free** Path A entry point: a tool call still fails
    /// the run closed here. A run whose model may call tools goes through
    /// [`super::turn_driver::LocalTurnDriver`] instead, which drives
    /// [`stream_turn`] — the same streaming core this method uses — once per
    /// round. Sharing that core is what keeps the two paths from drifting
    /// apart on delta mirroring, cancellation, or usage accounting.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run(
        &self,
        adapter: Arc<dyn ProviderAdapter>,
        secret: Zeroizing<String>,
        run_id: RunId,
        da_run_id: Uuid,
        model_id: String,
        request: ProviderChatRequest,
        cancel: tokio_util::sync::CancellationToken,
        mut on_inference: impl FnMut(&InferenceEvent) + Send,
    ) -> Result<LocalRunOutcome, LocalExecutorError> {
        let provider = adapter.provider_id();
        let started_at = Instant::now();

        let mut output =
            RunOutputBuilder::with_default_budget(run_id.clone(), ModelId::from(model_id.clone()));
        let mut presentation = InferencePresentationSink::new(
            run_id.clone(),
            CoalesceConfig::for_delta_coalescing(),
            &mut on_inference,
        );
        presentation.emit(InferenceEvent::started(
            run_id.clone(),
            format!("local-{da_run_id}"),
            provider,
            ModelId::from(model_id.clone()),
        ))?;

        if let Some(context) = request.openrouter.as_ref() {
            if let Err(error) = adapter
                .probe_native_compaction(
                    &model_id,
                    &context.route_identity,
                    &context.session_id,
                    secret.as_str(),
                    &cancel,
                )
                .await
            {
                let message = provider_failure_message(provider, error);
                presentation.emit(InferenceEvent::failed(
                    run_id.clone(),
                    inference_error_code_for(error),
                    message.clone(),
                    is_transient_failure(error),
                ))?;
                let _ =
                    finalize_failed(&self.sink, da_run_id, &model_id, started_at, message).await;
                return Err(LocalExecutorError::Provider(error));
            }
        }

        let mut sequence: i64 = 0;
        // One turn, so the run's own token *is* the stream's token: cancelling
        // it on the way out is exactly the intent (nothing else will run).
        let turn = stream_turn(
            &self.sink,
            adapter,
            secret,
            &run_id,
            da_run_id,
            request,
            TurnCancellation {
                run: &cancel,
                stream: &cancel,
                activity: None,
            },
            &mut sequence,
            true,
            &mut presentation,
            &mut output,
        )
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(TurnStreamError::Cancelled) => return Err(LocalExecutorError::Cancelled),
            Err(TurnStreamError::Presentation(error)) => {
                return Err(LocalExecutorError::Presentation(error))
            }
            Err(TurnStreamError::Output(error)) => {
                let _ = finalize_failed(
                    &self.sink,
                    da_run_id,
                    &model_id,
                    started_at,
                    error.to_string(),
                )
                .await;
                return Err(LocalExecutorError::Output(error));
            }
            Err(TurnStreamError::Sink(error)) => {
                let _ = finalize_failed(
                    &self.sink,
                    da_run_id,
                    &model_id,
                    started_at,
                    format!("{error}"),
                )
                .await;
                return Err(LocalExecutorError::Sink(error));
            }
            Err(TurnStreamError::Provider(error)) => {
                let message = provider_failure_message(provider, error);
                presentation.emit(InferenceEvent::failed(
                    run_id.clone(),
                    inference_error_code_for(error),
                    message.clone(),
                    is_transient_failure(error),
                ))?;
                let _ =
                    finalize_failed(&self.sink, da_run_id, &model_id, started_at, message).await;
                return Err(LocalExecutorError::Provider(error));
            }
        };

        if !turn.tool_calls.is_empty() {
            let message = "Tool calls are not available in this chat slice.";
            presentation.emit(InferenceEvent::failed(
                run_id.clone(),
                InferenceErrorCode::InvalidResponse,
                message,
                true,
            ))?;
            let _ = finalize_failed(
                &self.sink,
                da_run_id,
                &model_id,
                started_at,
                message.to_owned(),
            )
            .await;
            return Err(LocalExecutorError::ToolsUnavailable);
        }

        if !turn.saw_completed {
            let message = "The model stream ended without a completion signal.";
            presentation.emit(InferenceEvent::failed(
                run_id.clone(),
                InferenceErrorCode::Internal,
                message,
                true,
            ))?;
            let _ = finalize_failed(
                &self.sink,
                da_run_id,
                &model_id,
                started_at,
                message.to_owned(),
            )
            .await;
            return Err(LocalExecutorError::Provider(ProviderError::InvalidResponse));
        }

        let built = output.finish()?;
        let final_text = built.text().to_owned();

        // A `Conflict` here means a competing terminal write — typically a user cancel
        // racing this same run — already landed; treat it as that cancellation, not as
        // an execution failure the user needs to see an error for.
        match finalize_completed(
            &self.sink,
            da_run_id,
            &model_id,
            started_at,
            &final_text,
            turn.input_tokens,
            turn.output_tokens,
        )
        .await
        {
            // The assistant message id is discarded here and only here: this is
            // the tool-free path, so there is never a tool call to anchor to it.
            Ok(_message_id) => {}
            Err(RunEventSinkError::Conflict) => return Err(LocalExecutorError::Cancelled),
            Err(error) => return Err(LocalExecutorError::Sink(error)),
        }

        Ok(LocalRunOutcome {
            finish_reason: FinishReason::Stop,
            final_text,
            presentation: presentation.finish(),
        })
    }
}

/// Streams exactly one provider turn: mirrors every delta to DA on the bounded
/// size/time cadence, emits normalized [`InferenceEvent`]s, and reports what
/// the turn produced. Never finalizes the run — see [`StreamedTurn`].
///
/// `sequence` is the run-wide DA event sequence, advanced in place so a
/// multi-round driver's later turns continue the same monotonic series rather
/// than restarting it (DA rejects a replayed sequence number).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_turn<S, F>(
    sink: &S,
    adapter: Arc<dyn ProviderAdapter>,
    secret: Zeroizing<String>,
    run_id: &RunId,
    da_run_id: Uuid,
    request: ProviderChatRequest,
    cancel: TurnCancellation<'_>,
    sequence: &mut i64,
    emit_run_start: bool,
    presentation: &mut InferencePresentationSink<F>,
    output: &mut RunOutputBuilder,
) -> Result<StreamedTurn, TurnStreamError>
where
    S: RunEventSink + ?Sized,
    F: FnMut(&InferenceEvent),
{
    let (tx, mut rx) = mpsc::unbounded_channel::<ProviderStreamEvent>();
    let stream_cancel = cancel.stream.clone();
    // Poll the provider here instead of spawning an independent producer. When
    // the write backlog fills, withholding this poll applies backpressure to
    // the network rather than accumulating deltas behind a slow DA request.
    let provider_stream = async move {
        let mut forward = move |event: ProviderStreamEvent| {
            let _ = tx.send(event);
        };
        adapter
            .stream(&request, secret.as_str(), &stream_cancel, &mut forward)
            .await
    };
    let provider_stream = AssertUnwindSafe(provider_stream).catch_unwind();
    tokio::pin!(provider_stream);
    let mut provider_outcome: Option<Result<(), ProviderError>> = None;
    let mut events_finished = false;

    let mut batch: Vec<BatchRecord> = Vec::new();
    let mut batch_bytes = 0usize;
    if emit_run_start {
        *sequence += 1;
        batch.push(run_event_record(*sequence, "run.start", IndexMap::new()));
        batch_bytes = std::mem::size_of::<BatchRecord>();
    }
    let mut pending: VecDeque<(Vec<BatchRecord>, usize)> = VecDeque::new();
    let mut pending_bytes = batch_bytes;
    let mut write: Option<BoxFuture<'_, Result<(), RunEventSinkError>>> = None;
    let mut write_bytes = 0usize;

    let mut turn_text = String::new();
    let mut tool_calls: Vec<ContextToolCall> = Vec::new();
    let mut reasoning_details: Vec<Value> = Vec::new();
    let mut compactions: Vec<StreamedCompaction> = Vec::new();
    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;
    let mut saw_completed = false;
    let mut prompt_tokens: Option<u64> = None;
    let mut stop = ProviderStopReason::Natural;
    let mut discarded_tool_calls = 0u32;

    let flush_tick = tokio::time::sleep(MAX_BATCH_INTERVAL);
    tokio::pin!(flush_tick);

    // Two independent cadences on one loop. The DA tick above batches durable
    // history on the slow interval a data plane wants; this one hands the reader
    // whatever prose has arrived on the fast interval an eye wants. Sharing one
    // interval would either paint at 250 ms or write DA records at 16 ms.
    //
    // The sink coalesces on its own when the next delta arrives, so this tick
    // exists for the case the sink cannot see: a provider that sends text and
    // then goes quiet. Without it that text waits for the turn to end.
    let paint_interval = presentation.coalesce_interval();
    let paint_tick = tokio::time::sleep(paint_interval);
    tokio::pin!(paint_tick);

    let mut flush_error: Option<RunEventSinkError> = None;
    let mut output_error: Option<OutputError> = None;
    let mut presentation_error: Option<StreamError> = None;
    'stream: loop {
        if (batch.len() >= MAX_BATCH_RECORDS || events_finished)
            && !batch.is_empty()
            && pending.len() < MAX_PENDING_BATCHES
        {
            pending.push_back((std::mem::take(&mut batch), std::mem::take(&mut batch_bytes)));
        }
        if write.is_none() {
            if let Some((records, bytes)) = pending.pop_front() {
                write = Some(sink.apply_batch(da_run_id, records));
                write_bytes = bytes;
            }
        }
        if events_finished && batch.is_empty() && pending.is_empty() && write.is_none() {
            break;
        }
        // One record can cross the byte bound; its size is independently
        // bounded by the provider transport. The count bound also covers
        // streams with many tiny deltas and their per-record allocation cost.
        let capacity_available =
            pending.len() < MAX_PENDING_BATCHES && pending_bytes < MAX_PENDING_HISTORY_BYTES;
        tokio::select! {
            biased;
            () = cancel.run.cancelled() => break 'stream,
            () = &mut paint_tick, if !paint_interval.is_zero() => {
                presentation.flush();
                paint_tick.as_mut().reset(tokio::time::Instant::now() + paint_interval);
            }
            result = async { write.as_mut().expect("guarded pending write").await }, if write.is_some() => {
                write = None;
                pending_bytes = pending_bytes.saturating_sub(std::mem::take(&mut write_bytes));
                if let Err(error) = result {
                    flush_error = Some(error);
                    break 'stream;
                }
                if let Some(activity) = cancel.activity {
                    activity.touch();
                }
            }
            received = rx.recv(), if !events_finished && capacity_available => {
                let Some(event) = received else {
                    events_finished = true;
                    continue;
                };
                if let Some(activity) = cancel.activity {
                    activity.touch();
                }
                if let ProviderStreamEvent::TextDelta(text) = &event {
                    if let Err(error) = output.push_text(text) {
                        output_error = Some(error);
                        break 'stream;
                    }
                }
                *sequence += 1;
                if let Some(record) = batch_record_for(*sequence, &event) {
                    let bytes = std::mem::size_of::<BatchRecord>() + match &event {
                        ProviderStreamEvent::TextDelta(text)
                        | ProviderStreamEvent::ReasoningDelta(text) => text.len(),
                        _ => 0,
                    };
                    batch_bytes = batch_bytes.saturating_add(bytes);
                    pending_bytes = pending_bytes.saturating_add(bytes);
                    batch.push(record);
                }
                // Breaking out rather than propagating immediately: a
                // presentation failure leaves the provider stream running
                // exactly like a DA outage does, and it must be shut down
                // on the way out for the same reason (see below).
                if let Err(error) = present_stream_event(
                    run_id,
                    (*sequence).max(0) as u64,
                    &event,
                    output,
                    presentation,
                ) {
                    presentation_error = Some(error);
                    break 'stream;
                }
                match event {
                    ProviderStreamEvent::TextDelta(text) => turn_text.push_str(&text),
                    ProviderStreamEvent::ToolCallRequested { call_id, name, arguments } => {
                        tool_calls.push(ContextToolCall::new(call_id, name, arguments));
                    }
                    ProviderStreamEvent::Usage { input_tokens: i, output_tokens: o } => {
                        input_tokens = i;
                        output_tokens = o;
                    }
                    ProviderStreamEvent::Completed => saw_completed = true,
                    ProviderStreamEvent::PromptSize { tokens } => prompt_tokens = Some(tokens),
                    ProviderStreamEvent::Stopped { reason, discarded_tool_calls: discarded } => {
                        stop = reason;
                        discarded_tool_calls = discarded;
                    }
                    ProviderStreamEvent::ReasoningDelta(_) | ProviderStreamEvent::Progress => {}
                    // Carried, never inspected: the next round hands these
                    // back verbatim so a reasoning model can continue the
                    // thinking it started before the tool call.
                    ProviderStreamEvent::ReasoningDetails(blocks) => {
                        reasoning_details.extend(blocks);
                    }
                    ProviderStreamEvent::CompactionReady { index, block, usage } => {
                        compactions.push(StreamedCompaction { index, block, usage });
                    }
                }
            }
            () = &mut flush_tick, if pending.len() < MAX_PENDING_BATCHES => {
                if !batch.is_empty() {
                    pending.push_back((std::mem::take(&mut batch), std::mem::take(&mut batch_bytes)));
                }
                flush_tick.as_mut().reset(tokio::time::Instant::now() + MAX_BATCH_INTERVAL);
            }
            result = &mut provider_stream, if provider_outcome.is_none() && rx.is_empty() && capacity_available => {
                provider_outcome = Some(result.unwrap_or(Err(ProviderError::Unavailable)));
            }
        }
    }

    // Capture cancellation intent before we force our own — otherwise a DA outage
    // would be indistinguishable from a user-initiated stop below.
    let user_cancelled = cancel.run.is_cancelled();
    // A DA outage breaks the loop early. Cancel the provider scope before its
    // future is dropped; no independent task or write survives this turn.
    cancel.stream.cancel();

    if user_cancelled {
        return Err(TurnStreamError::Cancelled);
    }
    if let Some(error) = output_error {
        return Err(TurnStreamError::Output(error));
    }
    if let Some(error) = presentation_error {
        return Err(TurnStreamError::Presentation(error));
    }
    if let Some(error) = flush_error {
        return Err(TurnStreamError::Sink(error));
    }
    if let Err(error) = provider_outcome.unwrap_or(Err(ProviderError::Unavailable)) {
        return Err(TurnStreamError::Provider(error));
    }

    Ok(StreamedTurn {
        text: turn_text,
        tool_calls,
        reasoning_details,
        compactions,
        input_tokens,
        output_tokens,
        prompt_tokens,
        saw_completed,
        stop,
        discarded_tool_calls,
    })
}

pub(crate) async fn finalize_completed<S>(
    sink: &S,
    da_run_id: Uuid,
    model_id: &str,
    started_at: Instant,
    final_text: &str,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
) -> Result<Option<Uuid>, RunEventSinkError>
where
    S: RunEventSink + ?Sized,
{
    let duration_ms = i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX);
    let finalize = RunFinalize {
        status: "completed".to_owned(),
        expected_from: Some(non_terminal_expected_from()),
        model: model_id.to_owned(),
        content: Some(final_text.to_owned()),
        token_input: input_tokens.map(i64::from),
        token_output: output_tokens.map(i64::from),
        completed_at: Some(chrono::Utc::now()),
        duration_ms: Some(duration_ms),
        ..Default::default()
    };
    // The assistant message's durable id is the anchor a recorded tool call
    // attaches to; it is null when `content` was empty and no message was
    // written.
    sink.finalize(da_run_id, finalize)
        .await
        .map(|result| result.message_id)
}

pub(crate) async fn finalize_failed<S>(
    sink: &S,
    da_run_id: Uuid,
    model_id: &str,
    started_at: Instant,
    message: String,
) -> Result<(), RunEventSinkError>
where
    S: RunEventSink + ?Sized,
{
    let duration_ms = i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX);
    let finalize = RunFinalize {
        status: "error".to_owned(),
        expected_from: Some(non_terminal_expected_from()),
        model: model_id.to_owned(),
        error: Some(message),
        completed_at: Some(chrono::Utc::now()),
        duration_ms: Some(duration_ms),
        ..Default::default()
    };
    sink.finalize(da_run_id, finalize).await.map(|_| ())
}

fn non_terminal_expected_from() -> RunFinalizeExpectedFrom {
    RunFinalizeExpectedFrom::Array(
        NON_TERMINAL_STATUSES
            .iter()
            .map(|status| (*status).to_owned())
            .collect(),
    )
}

pub(crate) const fn inference_error_code_for(error: ProviderError) -> InferenceErrorCode {
    match error {
        ProviderError::AuthenticationFailed
        | ProviderError::CustomAuthenticationFailed
        | ProviderError::PlanSignInRequired
        | ProviderError::PlanNotEligible => InferenceErrorCode::AuthenticationFailed,
        // A plan limit is like an account that cannot pay: the sign-in works
        // and waiting does not help, so it must not read as a rate limit.
        ProviderError::PaymentRequired | ProviderError::PlanUsageLimit => {
            InferenceErrorCode::PaymentRequired
        }
        ProviderError::RateLimited { .. } => InferenceErrorCode::RateLimited,
        ProviderError::Unavailable
        | ProviderError::TimedOut
        | ProviderError::EndpointUnavailable
        | ProviderError::RuntimeUnavailable => InferenceErrorCode::ProviderUnavailable,
        ProviderError::AddressRefused => InferenceErrorCode::InvalidResponse,
        ProviderError::PayloadTooLarge | ProviderError::ContextOverflow => {
            InferenceErrorCode::PayloadTooLarge
        }
        ProviderError::NativeCompactionUnsupported | ProviderError::MalformedCompaction => {
            InferenceErrorCode::InvalidResponse
        }
        ProviderError::TlsVerificationFailed => InferenceErrorCode::TlsVerificationFailed,
        ProviderError::InvalidResponse => InferenceErrorCode::InvalidResponse,
        ProviderError::Cancelled => InferenceErrorCode::Cancelled,
    }
}

/// Whether trying the same request again later can reasonably succeed. The
/// reader is offered a retry for these; a rejected key or a refused address
/// would fail the same way again.
pub(crate) const fn is_transient_failure(error: ProviderError) -> bool {
    matches!(
        error,
        ProviderError::RateLimited { .. }
            | ProviderError::Unavailable
            | ProviderError::TimedOut
            | ProviderError::EndpointUnavailable
    )
}

pub(crate) fn provider_failure_message(provider: ProviderId, error: ProviderError) -> String {
    match error {
        ProviderError::AuthenticationFailed => "The provider rejected this credential.".to_owned(),
        ProviderError::CustomAuthenticationFailed => {
            "The private model endpoint rejected this key. Update the key in Settings → Models."
                .to_owned()
        }
        // Names the account, not the request rate: the key works and waiting
        // will not help, so "rate limited" would send the user to the wrong fix.
        ProviderError::PaymentRequired => {
            "This provider account cannot pay for the request. Check its billing or credit balance."
                .to_owned()
        }
        ProviderError::RateLimited { .. } => "The provider rate limited this request.".to_owned(),
        ProviderError::Unavailable => "The provider is temporarily unavailable.".to_owned(),
        // The endpoint answered the connection, so the host is fine. Naming the
        // slow model keeps the reader from checking settings that are correct.
        ProviderError::TimedOut => {
            "The model took too long to respond, so the request was stopped. A slow or busy model \
             is the usual cause. Try again, or choose a faster model."
                .to_owned()
        }
        ProviderError::EndpointUnavailable => {
            "The private model endpoint could not be reached. Check its host in Settings → Models."
                .to_owned()
        }
        ProviderError::AddressRefused => {
            "The private model's API base URL is not valid. Check the URL in Settings → Models."
                .to_owned()
        }
        ProviderError::PayloadTooLarge => {
            "The request was larger than the provider accepts. Send fewer or smaller attachments."
                .to_owned()
        }
        ProviderError::ContextOverflow => {
            "The conversation exceeded the model context window before it could be compacted."
                .to_owned()
        }
        ProviderError::NativeCompactionUnsupported => {
            "The selected OpenRouter route does not support durable context compaction.".to_owned()
        }
        ProviderError::MalformedCompaction => {
            "OpenRouter returned an invalid context compaction block.".to_owned()
        }
        // The likeliest cause on a managed machine is a TLS-inspecting proxy,
        // and the fix is local — so this must never read as a provider outage,
        // which sends the user to a status page for a problem on their own
        // machine. The message is the only diagnostic a desktop user has.
        ProviderError::TlsVerificationFailed => {
            "The provider's TLS certificate could not be verified. A network proxy or a missing \
             certificate authority on this machine is the usual cause."
                .to_owned()
        }
        ProviderError::InvalidResponse => {
            "The provider returned an unexpected response.".to_owned()
        }
        ProviderError::PlanSignInRequired => {
            "Your plan sign-in is missing or has expired. Sign in again in Settings → Models & connections, then retry."
                .to_owned()
        }
        // No estimate and no reset time: the vendor may apply a limit specific
        // to this app, so the plan can be non-empty when this fires.
        // "Manage usage" is a link on the ChatGPT card only, so Claude's reader
        // is sent to the vendor instead of to a control that does not exist.
        ProviderError::PlanUsageLimit => match provider {
            ProviderId::ClaudeCode => {
                "Your Claude plan's usage limit was reached. Check your usage with Claude, then try again later."
                    .to_owned()
            }
            _ => {
                "Usage limit reached. Open Manage usage in Settings → Models & connections to see your plan's usage, then try again later."
                    .to_owned()
            }
        },
        ProviderError::PlanNotEligible => {
            "This plan cannot be used with xFalcon. Check your plan in Settings → Models & connections."
                .to_owned()
        }
        ProviderError::RuntimeUnavailable => {
            "Claude did not start or did not answer in time. In Settings → Models & connections, check that Claude Code is installed, signed in and switched on, then try again."
                .to_owned()
        }
        ProviderError::Cancelled => "The request was cancelled.".to_owned(),
    }
}

fn run_event_record(
    sequence_no: i64,
    event_type: &str,
    payload: IndexMap<String, Value>,
) -> BatchRecord {
    BatchRecord {
        r#type: BatchRecordType::RunEvent,
        event_type: Some(event_type.to_owned()),
        sequence_no: Some(sequence_no),
        payload: Some(payload),
        ..Default::default()
    }
}

fn batch_record_for(sequence: i64, event: &ProviderStreamEvent) -> Option<BatchRecord> {
    match event {
        ProviderStreamEvent::TextDelta(text) => Some(run_event_record(
            sequence,
            "run.stream_delta",
            IndexMap::from([("delta".to_owned(), Value::String(text.clone()))]),
        )),
        ProviderStreamEvent::ReasoningDelta(text) => Some(run_event_record(
            sequence,
            "run.thinking_step",
            IndexMap::from([("text".to_owned(), Value::String(text.clone()))]),
        )),
        ProviderStreamEvent::Completed => {
            Some(run_event_record(sequence, "run.complete", IndexMap::new()))
        }
        // The replay copy of the same thinking `run.thinking_step` above
        // already recorded. Journaling it a second time would put the model's
        // reasoning into the durable run record twice, in two shapes.
        ProviderStreamEvent::Progress
        | ProviderStreamEvent::ReasoningDetails(_)
        | ProviderStreamEvent::CompactionReady { .. }
        | ProviderStreamEvent::ToolCallRequested { .. }
        | ProviderStreamEvent::Stopped { .. }
        | ProviderStreamEvent::PromptSize { .. }
        | ProviderStreamEvent::Usage { .. } => None,
    }
}

/// Routes one provider event into the presentation sink.
///
/// One function rather than a builder plus a caller that decides what to do with
/// what it built: reasoning cannot be coalesced as a finished
/// [`InferenceEvent`], because its payload type is opaque and capped, so the
/// sink has to receive the raw fragment. Having the caller branch on the event
/// kind to choose a method is how the two paths drift.
fn present_stream_event<F>(
    run_id: &RunId,
    sequence: u64,
    event: &ProviderStreamEvent,
    output: &mut RunOutputBuilder,
    presentation: &mut InferencePresentationSink<F>,
) -> Result<(), StreamError>
where
    F: FnMut(&InferenceEvent),
{
    match event {
        ProviderStreamEvent::TextDelta(text) => presentation.emit(InferenceEvent::text_delta(
            run_id.clone(),
            sequence,
            text.clone(),
        )),
        // Reasoning must never be able to become answer bytes on the durable
        // record: it is untrusted chain-of-thought on its own event, and
        // `output` is what the durable message is built from.
        ProviderStreamEvent::ReasoningDelta(text) => presentation.emit_reasoning(text, sequence),
        ProviderStreamEvent::ToolCallRequested { .. } => {
            output.note_tool_request();
            Ok(())
        }
        // Never presented: these are the opaque blocks the next request echoes
        // back, not something a reader can be shown. `ReasoningDelta` above is
        // the display form.
        ProviderStreamEvent::Progress
        | ProviderStreamEvent::ReasoningDetails(_)
        | ProviderStreamEvent::CompactionReady { .. }
        | ProviderStreamEvent::Usage { .. }
        | ProviderStreamEvent::PromptSize { .. }
        | ProviderStreamEvent::Stopped { .. }
        | ProviderStreamEvent::Completed => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use xfalcon_contracts::{ConversationId, RunId};

    use super::*;
    use crate::agent::context::{ContextMessage, ContextRole};

    /// The table's check constraint admits only these. A status outside it is a
    /// 422 from DA that fails the run and strands the round half-saved.
    const DURABLE_STATUSES: [&str; 6] = [
        "pending",
        "running",
        "completed",
        "error",
        "cancelled",
        "awaiting_approval",
    ];

    #[test]
    fn every_status_a_tool_can_return_is_one_the_table_accepts() {
        for rendered in [
            r#"{"status":"completed","result":{}}"#,
            r#"{"status":"failed","error":"x","retryable":true}"#,
            r#"{"status":"denied","reason":"project.not_active"}"#,
            r#"{"status":"invalid_arguments","error":"x"}"#,
            r#"{"status":"something_new"}"#,
            r#"{"status":7}"#,
            r#"{"result":"no status at all"}"#,
            "plain text, not JSON",
        ] {
            let status = durable_tool_status(rendered);
            assert!(DURABLE_STATUSES.contains(&status), "{rendered} -> {status}");
        }
        assert_eq!(
            durable_tool_status(r#"{"status":"completed"}"#),
            "completed"
        );
        assert_eq!(
            durable_tool_status(r#"{"status":"cancelled"}"#),
            "cancelled"
        );
        assert_eq!(durable_tool_status(r#"{"status":"denied"}"#), "error");
        assert_eq!(durable_tool_status(r#"{"status":"failed"}"#), "error");
    }

    struct StubAdapter {
        events: Mutex<Vec<ProviderStreamEvent>>,
        error: Mutex<Option<ProviderError>>,
    }

    #[async_trait]
    impl ProviderAdapter for StubAdapter {
        fn provider_id(&self) -> xfalcon_contracts::credentials::ProviderId {
            xfalcon_contracts::credentials::ProviderId::OpenAi
        }

        async fn probe_native_compaction(
            &self,
            _model_id: &str,
            _route_identity: &str,
            _session_id: &str,
            _secret: &str,
            _cancel: &tokio_util::sync::CancellationToken,
        ) -> Result<(), ProviderError> {
            Ok(())
        }

        async fn stream(
            &self,
            _request: &ProviderChatRequest,
            _secret: &str,
            _cancel: &tokio_util::sync::CancellationToken,
            on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
        ) -> Result<(), ProviderError> {
            for event in self.events.lock().expect("lock").drain(..) {
                on_event(event);
            }
            match self.error.lock().expect("lock").take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        batches: Mutex<Vec<Vec<BatchRecord>>>,
        finalized: Mutex<Vec<RunFinalize>>,
        fail_finalize_with: Mutex<Option<RunEventSinkError>>,
        fail_apply_batch_with: Mutex<Option<RunEventSinkError>>,
    }

    #[async_trait]
    impl RunEventSink for RecordingSink {
        async fn apply_batch(
            &self,
            _run_id: Uuid,
            records: Vec<BatchRecord>,
        ) -> Result<(), RunEventSinkError> {
            if let Some(error) = *self.fail_apply_batch_with.lock().expect("lock") {
                return Err(error);
            }
            self.batches.lock().expect("lock").push(records);
            Ok(())
        }

        async fn finalize(
            &self,
            _run_id: Uuid,
            finalize: RunFinalize,
        ) -> Result<RunFinalizeResult, RunEventSinkError> {
            self.finalized.lock().expect("lock").push(finalize.clone());
            if let Some(error) = *self.fail_finalize_with.lock().expect("lock") {
                return Err(error);
            }
            Ok(RunFinalizeResult {
                run: crate::clients::da::generated::Run::default(),
                message_id: None,
                message_sequence_no: None,
            })
        }

        /// Unreachable for the same reason: a run with no tool call has no
        /// round to write to the conversation.
        async fn record_tool_round(
            &self,
            _conversation_id: Uuid,
            _round: ToolRoundRecord,
            _on_progress: &mut (dyn FnMut() + Send),
        ) -> Result<Vec<String>, RunEventSinkError> {
            unreachable!("the tool-free executor never writes a tool round")
        }
    }

    #[derive(Default)]
    struct GatedSink {
        inner: RecordingSink,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
        writes: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl RunEventSink for GatedSink {
        async fn apply_batch(
            &self,
            run_id: Uuid,
            records: Vec<BatchRecord>,
        ) -> Result<(), RunEventSinkError> {
            if self
                .writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0
            {
                self.started.notify_one();
                self.release.notified().await;
            }
            self.inner.apply_batch(run_id, records).await
        }

        async fn finalize(
            &self,
            run_id: Uuid,
            finalize: RunFinalize,
        ) -> Result<RunFinalizeResult, RunEventSinkError> {
            self.inner.finalize(run_id, finalize).await
        }

        async fn record_tool_round(
            &self,
            conversation_id: Uuid,
            round: ToolRoundRecord,
            on_progress: &mut (dyn FnMut() + Send),
        ) -> Result<Vec<String>, RunEventSinkError> {
            self.inner
                .record_tool_round(conversation_id, round, on_progress)
                .await
        }
    }

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: "gpt-example".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hello")],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        }
    }

    #[tokio::test]
    async fn slow_history_write_does_not_block_paint_and_finalization_waits_for_all_records() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(
                (0..19)
                    .map(|_| ProviderStreamEvent::TextDelta("a".to_owned()))
                    .chain([
                        ProviderStreamEvent::TextDelta("later".to_owned()),
                        ProviderStreamEvent::Completed,
                    ])
                    .collect(),
            ),
            error: Mutex::new(None),
        });
        let executor = LocalRunExecutor::new(GatedSink::default());
        let painted = tokio::sync::Notify::new();
        let inference = |event: &InferenceEvent| {
            if let InferenceEvent::TextDelta { text, .. } = event {
                if text.contains("later") {
                    painted.notify_one();
                }
            }
        };
        let observe = async {
            executor.sink.started.notified().await;
            let visible =
                tokio::time::timeout(Duration::from_millis(500), painted.notified()).await;
            assert!(executor
                .sink
                .inner
                .finalized
                .lock()
                .expect("lock")
                .is_empty());
            executor.sink.release.notify_one();
            visible
        };
        let (result, visible) = tokio::join!(
            executor.run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                inference,
            ),
            observe,
        );
        let outcome = result.expect("run succeeds");
        assert!(
            visible.is_ok(),
            "the answer must paint while its history write is blocked"
        );
        assert_eq!(outcome.final_text, format!("{}later", "a".repeat(19)));
        let batches = executor.sink.inner.batches.lock().expect("lock");
        let sequences: Vec<_> = batches
            .iter()
            .flatten()
            .map(|record| record.sequence_no)
            .collect();
        assert_eq!(sequences, (1..=22).map(Some).collect::<Vec<_>>());
        assert_eq!(executor.sink.inner.finalized.lock().expect("lock").len(), 1);
    }

    struct YieldingAdapter {
        produced: std::sync::atomic::AtomicUsize,
        count: usize,
        delta: ProviderStreamEvent,
    }

    #[async_trait]
    impl ProviderAdapter for YieldingAdapter {
        fn provider_id(&self) -> xfalcon_contracts::credentials::ProviderId {
            xfalcon_contracts::credentials::ProviderId::OpenAi
        }

        async fn stream(
            &self,
            _request: &ProviderChatRequest,
            _secret: &str,
            cancel: &tokio_util::sync::CancellationToken,
            on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
        ) -> Result<(), ProviderError> {
            for _ in 0..self.count {
                if cancel.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                on_event(self.delta.clone());
                self.produced
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
            if matches!(self.delta, ProviderStreamEvent::ReasoningDelta(_)) {
                on_event(ProviderStreamEvent::TextDelta("done".to_owned()));
            }
            on_event(ProviderStreamEvent::Completed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn blocked_history_backpressures_provider_then_drains_without_losing_text() {
        use std::sync::atomic::Ordering;

        let count = MAX_PENDING_BATCHES * MAX_BATCH_RECORDS * 4;
        let adapter = Arc::new(YieldingAdapter {
            produced: std::sync::atomic::AtomicUsize::new(0),
            count,
            delta: ProviderStreamEvent::TextDelta("a".to_owned()),
        });
        let executor = LocalRunExecutor::new(GatedSink::default());
        let observe = async {
            executor.sink.started.notified().await;
            tokio::time::timeout(Duration::from_secs(1), async {
                while adapter.produced.load(Ordering::SeqCst)
                    < MAX_PENDING_BATCHES * MAX_BATCH_RECORDS
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("provider fills the backlog");
            tokio::time::sleep(Duration::from_millis(20)).await;
            let paused = adapter.produced.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            let still_paused = adapter.produced.load(Ordering::SeqCst);
            executor.sink.release.notify_one();
            (paused, still_paused)
        };
        let (result, (paused, still_paused)) = tokio::join!(
            executor.run(
                adapter.clone(),
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            ),
            observe,
        );
        assert!(
            paused < count,
            "provider must pause before consuming the whole stream"
        );
        assert_eq!(
            paused, still_paused,
            "blocked persistence must stop further reads"
        );
        assert_eq!(result.expect("run succeeds").final_text, "a".repeat(count));
        let batches = executor.sink.inner.batches.lock().expect("lock");
        let sequences: Vec<_> = batches
            .iter()
            .flatten()
            .map(|record| record.sequence_no)
            .collect();
        assert_eq!(
            sequences,
            (1..=count as i64 + 2).map(Some).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn large_reasoning_backlog_is_bounded_by_bytes_before_the_record_limit() {
        use std::sync::atomic::Ordering;

        let delta_bytes = 64 * 1024;
        let count = MAX_PENDING_HISTORY_BYTES / delta_bytes * 4;
        let adapter = Arc::new(YieldingAdapter {
            produced: std::sync::atomic::AtomicUsize::new(0),
            count,
            delta: ProviderStreamEvent::ReasoningDelta("r".repeat(delta_bytes)),
        });
        let executor = LocalRunExecutor::new(GatedSink::default());
        let observe = async {
            executor.sink.started.notified().await;
            tokio::time::timeout(Duration::from_secs(1), async {
                while adapter.produced.load(Ordering::SeqCst) < 30 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("reasoning fills the byte backlog");
            tokio::time::sleep(Duration::from_millis(20)).await;
            let paused = adapter.produced.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            let still_paused = adapter.produced.load(Ordering::SeqCst);
            executor.sink.release.notify_one();
            (paused, still_paused)
        };
        let (result, (paused, still_paused)) = tokio::join!(
            executor.run(
                adapter.clone(),
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            ),
            observe,
        );
        assert_eq!(paused, still_paused);
        assert!(paused <= MAX_PENDING_HISTORY_BYTES / delta_bytes + 1);
        assert_eq!(result.expect("run succeeds").final_text, "done");
        let batches = executor.sink.inner.batches.lock().expect("lock");
        let records: Vec<_> = batches.iter().flatten().collect();
        assert_eq!(records.len(), count + 3);
        assert_eq!(
            records
                .iter()
                .filter(|record| record.event_type.as_deref() == Some("run.thinking_step"))
                .count(),
            count
        );
    }

    #[tokio::test]
    async fn cancellation_does_not_wait_for_a_blocked_history_write() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(
                (0..20)
                    .map(|_| ProviderStreamEvent::TextDelta("a".to_owned()))
                    .collect(),
            ),
            error: Mutex::new(None),
        });
        let executor = LocalRunExecutor::new(GatedSink::default());
        let cancel = tokio_util::sync::CancellationToken::new();
        let returned = tokio::sync::Notify::new();
        let run = async {
            let result = executor
                .run(
                    adapter,
                    Zeroizing::new("sk-test".to_owned()),
                    RunId::from("run-1"),
                    Uuid::nil(),
                    "gpt-example".to_owned(),
                    request(),
                    cancel.clone(),
                    |_| {},
                )
                .await;
            returned.notify_one();
            result
        };
        let observe = async {
            executor.sink.started.notified().await;
            cancel.cancel();
            let stopped =
                tokio::time::timeout(Duration::from_millis(500), returned.notified()).await;
            executor.sink.release.notify_one();
            stopped
        };
        let (result, stopped) = tokio::join!(run, observe);
        assert!(
            stopped.is_ok(),
            "cancellation must release the stream before the history sink"
        );
        assert!(matches!(result, Err(LocalExecutorError::Cancelled)));
        assert!(executor
            .sink
            .inner
            .finalized
            .lock()
            .expect("lock")
            .is_empty());
    }

    #[tokio::test]
    async fn successful_stream_finalizes_completed_with_full_text() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(vec![
                ProviderStreamEvent::TextDelta("Hel".to_owned()),
                ProviderStreamEvent::TextDelta("lo".to_owned()),
                ProviderStreamEvent::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(2),
                },
                ProviderStreamEvent::Completed,
            ]),
            error: Mutex::new(None),
        });
        let sink = RecordingSink::default();
        let executor = LocalRunExecutor::new(sink);

        let mut collected = Vec::new();
        let outcome = executor
            .run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |event| collected.push(event.clone()),
            )
            .await
            .expect("run succeeds");

        assert_eq!(outcome.final_text, "Hello");
        assert_eq!(outcome.finish_reason, FinishReason::Stop);
        let finalized = executor.sink.finalized.lock().expect("lock");
        assert_eq!(finalized.len(), 1);
        assert_eq!(finalized[0].status, "completed");
        assert_eq!(finalized[0].content, Some("Hello".to_owned()));
        assert_eq!(finalized[0].token_input, Some(10));
        assert_eq!(finalized[0].token_output, Some(2));

        let _ = ConversationId::from("unused");
    }

    #[tokio::test]
    async fn tool_call_request_fails_closed_and_finalizes_as_error() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(vec![
                ProviderStreamEvent::TextDelta("partial".to_owned()),
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_1".to_owned(),
                    name: "x".to_owned(),
                    arguments: "{}".to_owned(),
                },
                ProviderStreamEvent::Completed,
            ]),
            error: Mutex::new(None),
        });
        let sink = RecordingSink::default();
        let executor = LocalRunExecutor::new(sink);

        let result = executor
            .run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            )
            .await;

        assert!(matches!(result, Err(LocalExecutorError::ToolsUnavailable)));
        let finalized = executor.sink.finalized.lock().expect("lock");
        assert_eq!(finalized[0].status, "error");
    }

    #[tokio::test]
    async fn provider_failure_finalizes_as_error_without_marking_completed() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(Vec::new()),
            error: Mutex::new(Some(ProviderError::RateLimited { retry_after: None })),
        });
        let sink = RecordingSink::default();
        let executor = LocalRunExecutor::new(sink);

        let result = executor
            .run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            )
            .await;

        assert!(matches!(
            result,
            Err(LocalExecutorError::Provider(ProviderError::RateLimited {
                retry_after: None
            }))
        ));
        let finalized = executor.sink.finalized.lock().expect("lock");
        assert_eq!(finalized[0].status, "error");
    }

    #[tokio::test]
    async fn finalize_conflict_is_treated_as_a_clean_cancel_not_a_failure() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(vec![
                ProviderStreamEvent::TextDelta("done".to_owned()),
                ProviderStreamEvent::Completed,
            ]),
            error: Mutex::new(None),
        });
        let sink = RecordingSink::default();
        *sink.fail_finalize_with.lock().expect("lock") = Some(RunEventSinkError::Conflict);
        let executor = LocalRunExecutor::new(sink);

        let result = executor
            .run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            )
            .await;

        assert!(
            matches!(result, Err(LocalExecutorError::Cancelled)),
            "a competing terminal write (e.g. a user cancel) must surface as Cancelled, not a generic failure: {result:?}"
        );
    }

    #[tokio::test]
    async fn mid_stream_da_outage_cancels_the_provider_stream_instead_of_orphaning_it() {
        let events = (0..19)
            .map(|i| ProviderStreamEvent::TextDelta(i.to_string()))
            .chain(std::iter::once(ProviderStreamEvent::Completed))
            .collect();
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(events),
            error: Mutex::new(None),
        });
        let sink = RecordingSink::default();
        *sink.fail_apply_batch_with.lock().expect("lock") = Some(RunEventSinkError::Unavailable);
        let executor = LocalRunExecutor::new(sink);
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = executor
            .run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                cancel.clone(),
                |_| {},
            )
            .await;

        assert!(matches!(
            result,
            Err(LocalExecutorError::Sink(RunEventSinkError::Unavailable))
        ));
        assert!(
            cancel.is_cancelled(),
            "a mid-stream DA outage must cancel the in-flight provider call, not leave it \
             running unattended in the background"
        );
    }

    #[tokio::test]
    async fn stream_ending_without_completion_is_treated_as_a_failure() {
        let adapter = Arc::new(StubAdapter {
            events: Mutex::new(vec![ProviderStreamEvent::TextDelta("partial".to_owned())]),
            error: Mutex::new(None),
        });
        let sink = RecordingSink::default();
        let executor = LocalRunExecutor::new(sink);

        let result = executor
            .run(
                adapter,
                Zeroizing::new("sk-test".to_owned()),
                RunId::from("run-1"),
                Uuid::nil(),
                "gpt-example".to_owned(),
                request(),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            )
            .await;

        assert!(matches!(
            result,
            Err(LocalExecutorError::Provider(ProviderError::InvalidResponse))
        ));
    }

    /// Reasoning must never be able to become answer bytes on the durable
    /// record. It is untrusted chain-of-thought on its own event; the answer
    /// is what the model chose to emit. `RunOutputBuilder` is what the durable
    /// message is built from, so this asserts on the builder rather than on
    /// the event alone.
    #[test]
    fn a_reasoning_delta_becomes_its_own_event_and_never_answer_text() {
        let run_id = RunId::from("run-1");
        let mut output = crate::agent::output::RunOutputBuilder::with_default_budget(
            run_id.clone(),
            ModelId::from("gpt-example"),
        );

        let mut seen: Vec<InferenceEvent> = Vec::new();
        let mut presentation = InferencePresentationSink::new(
            run_id.clone(),
            CoalesceConfig::for_tests(),
            |event: &InferenceEvent| seen.push(event.clone()),
        );

        present_stream_event(
            &run_id,
            7,
            &ProviderStreamEvent::ReasoningDelta("weighing the options".to_owned()),
            &mut output,
            &mut presentation,
        )
        .expect("presented");
        presentation.flush();
        drop(presentation);

        assert!(matches!(
            seen.as_slice(),
            [InferenceEvent::ReasoningSummary { .. }]
        ));
        assert_eq!(output.text_snapshot(), "");
    }

    #[test]
    fn only_failures_that_can_pass_later_are_offered_a_retry() {
        for error in [
            ProviderError::RateLimited { retry_after: None },
            ProviderError::Unavailable,
            ProviderError::EndpointUnavailable,
        ] {
            assert!(is_transient_failure(error), "{error:?}");
        }
        for error in [
            ProviderError::AuthenticationFailed,
            ProviderError::CustomAuthenticationFailed,
            ProviderError::AddressRefused,
            ProviderError::ContextOverflow,
            ProviderError::InvalidResponse,
            ProviderError::TlsVerificationFailed,
        ] {
            assert!(!is_transient_failure(error), "{error:?}");
        }
    }

    #[test]
    fn a_plan_usage_limit_names_only_places_that_exist_for_that_provider() {
        let chatgpt_text = "Usage limit reached. Open Manage usage in Settings → Models & connections to see your plan's usage, then try again later.";

        assert_eq!(
            provider_failure_message(ProviderId::ClaudeCode, ProviderError::PlanUsageLimit),
            "Your Claude plan's usage limit was reached. Check your usage with Claude, then try again later."
        );
        assert_eq!(
            provider_failure_message(ProviderId::ChatgptPlan, ProviderError::PlanUsageLimit),
            chatgpt_text
        );
        assert_eq!(
            provider_failure_message(ProviderId::OpenAi, ProviderError::PlanUsageLimit),
            chatgpt_text
        );
    }
}
