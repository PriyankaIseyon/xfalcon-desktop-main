//! Run executor port — DA polling is the interim implementation until local inference lands.
//!
//! # The `InferenceExecutor` port, and why Path A only
//!
//! [`InferenceExecutor`] is the seam [`crate::agent::engine::run_with_tool_loop`]
//! drives every round: "give me the model's next turn, having already seen
//! whatever tool results the previous round produced." It is deliberately
//! decoupled from any concrete provider wire format.
//!
//! Task 7 shipped this port plus the bounded, policy-gated, journal-first
//! orchestrator behind it ([`crate::agent::tool_loop::ToolLoop`]) at a time
//! when **no** [`crate::providers::ProviderAdapter`] could implement it:
//! every adapter turned a model's tool-calling intent into a bare
//! `ToolCallRequested` marker carrying no `call_id`, tool name, or arguments,
//! so there was nothing structured to dispatch. Task 9 closed that gap —
//! `ProviderStreamEvent::ToolCallRequested` now carries the full call,
//! `ProviderChatRequest::tools` declares the callable set, and
//! [`crate::agent::turn_driver::LocalTurnDriver`] is the live Path A
//! implementation of this port, streaming one real provider turn per round.
//! `crate::agent::tests::tool_loop`'s fake implementation remains as the
//! deterministic, provider-free way to exercise the orchestrator itself.
//!
//! [`crate::agent::engine::run_local`] (and
//! [`crate::agent::local_executor::LocalRunExecutor::run`] underneath it)
//! still fails a tool-call request closed. That is now a *routing* statement,
//! not a capability one: it is the tool-free entry point, kept because a run
//! with no tools has no reason to pay for round bookkeeping. Both share one
//! streaming core (`crate::agent::local_executor::stream_turn`), so they
//! cannot drift.
//!
//! Path B ([`RunEngine::poll_da_run`](super::engine::RunEngine::poll_da_run))
//! stays a dead end too, but on a narrower and more honest claim than an
//! earlier draft of this comment made. Corrected after a review finding
//! caught the overclaim (see
//! `docs/decisions.md`'s
//! "Path A/B scoping decision" section for the full trace): the pinned DA
//! OpenAPI contract (`contracts/da/openapi.json`) does define a *generic*
//! tool-call update route — `PATCH /api/v1/chat/tool-calls/{tool_call_id}`
//! and `PATCH /api/v1/chat/tool-calls/by-provider-call` (keyed by `run_id` +
//! `provider_tool_call_id`), both accepting the generated `ToolCallUpdate`
//! body with `output`/`status`/`error`/`duration_ms` fields — exactly the
//! shape a locally-executed tool result would need to report back onto a
//! DA-tracked `tool_calls` row. `crate::clients::da::chat_api::DaChatSession`
//! (this crate's hand-written wrapper over that contract) simply never
//! surfaces those two generated request types as callable methods; it only
//! wraps the narrower `.../approval` endpoint (used by
//! `crate::chat::ApprovalDaSync`). So "no route exists" is true only of this
//! crate's own hand-rolled client subset, not of the pinned contract itself.
//! The question that comment left open — whether writing to that generic
//! update route causes a DA-side run to resume generation — has since been
//! answered, and the answer is no: **DA runs no inference at all.**
//! `/api/v1/chat/*` is a data plane. Nothing in `mcp_server/` calls a model
//! provider, and no worker advances a `queued` run (its `background/` module
//! holds only the metadata-sync jobs and the orphan reaper). DA's own
//! `RunFinalize.tool_calls` field documents the division explicitly: it exists
//! for "a client that executes tools locally."
//!
//! So Path B is not a dead end awaiting a client-side gap — it is an observer
//! for a producer that does not exist. `crate::chat::service` therefore routes
//! every run through Path A and refuses a send it cannot execute, rather than
//! creating a run that would sit queued until its deadline.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::inference::FinishReason;
use xfalcon_contracts::tools::{InvocationId, ToolId};

use super::tool_loop::ToolCallOutcome;

pub const TOOL_EVENT_PREFIX: &str = "tool_call.";

/// One structured tool call a model turn asked for, decoupled from any
/// concrete provider wire format — see this module's doc comment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestedToolCall {
    pub invocation_id: InvocationId,
    pub tool_id: ToolId,
    pub arguments: Value,
}

/// One round's outcome from whatever drives model turns for a tool-capable
/// run: either the model is done (final text), or it wants to run tools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelTurn {
    Final {
        finish_reason: FinishReason,
        final_text: String,
    },
    ToolCalls(Vec<RequestedToolCall>),
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum TurnError {
    #[error("model turn cancelled")]
    Cancelled,
    /// The turn failed and the implementation **already emitted** a terminal
    /// `xfalcon_contracts::inference::InferenceEvent::Failed` carrying a more
    /// accurate error code than the orchestrator could reconstruct (a
    /// provider rate limit, an auth rejection). Callers must transition the
    /// run to a terminal state exactly as for [`Self::Failed`] but must not
    /// emit a second terminal event — the presentation stream would then
    /// carry two, and the UI would render the run as failing twice.
    #[error("model turn failed: {0}")]
    Reported(String),
    #[error("model turn failed: {0}")]
    Failed(String),
}

/// The port [`crate::agent::engine::run_with_tool_loop`] calls every round.
///
/// `crate::agent::turn_driver::LocalTurnDriver` is the live Path A
/// implementation; `crate::agent::tests::tool_loop`'s fake drives the
/// orchestrator deterministically without a provider.
#[async_trait]
pub trait InferenceExecutor: Send + Sync {
    /// Produces the model's next turn. `tool_outcomes` carries every
    /// dispatched call's outcome from the previous round (empty on the very
    /// first call) — including denials/failures/cancellations, not only
    /// completions — so an implementation can fold them into whatever
    /// context it maintains before deciding the next turn.
    async fn next_turn(
        &mut self,
        tool_outcomes: Vec<ToolCallOutcome>,
        cancel: &CancellationToken,
    ) -> Result<ModelTurn, TurnError>;

    /// Records outcomes the orchestrator produced but will never hand to a
    /// [`Self::next_turn`] call: when a round pauses for approval it returns
    /// to its caller immediately, so the *other* calls in that same round —
    /// which may have completed normally — would otherwise be dropped on the
    /// floor. An implementation that maintains a message context must fold
    /// these in, or the assistant turn that requested them ends up with
    /// unanswered tool calls, which OpenAI and Anthropic both reject
    /// outright.
    ///
    /// Never called with an outcome that is still pending
    /// ([`ToolCallOutcome::AwaitingApproval`]) — that one arrives later,
    /// through [`Self::next_turn`], once it resolves.
    ///
    /// Default: drop them, which is correct only for an implementation that
    /// keeps no cross-round context.
    fn record_outcomes(&mut self, _outcomes: Vec<ToolCallOutcome>) {}
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedRun {
    pub status: String,
    pub output_message: Option<String>,
    pub error: Option<String>,
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedRunEvent {
    pub sequence_no: i64,
    pub event_type: String,
    pub payload: IndexMap<String, Value>,
}

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("tenant run observation failed")]
    ObservationFailed,
    #[error("run observation unauthorized")]
    Unauthorized,
    #[error("run was not found")]
    NotFound,
}

#[async_trait]
pub trait DaRunObservation: Send + Sync {
    async fn fetch_run(&self, run_id: Uuid) -> Result<ObservedRun, ExecutorError>;
    async fn fetch_new_events(
        &self,
        run_id: Uuid,
        after_sequence: i64,
    ) -> Result<Vec<ObservedRunEvent>, ExecutorError>;
}

pub struct DaPollingExecutor<O> {
    observer: Arc<O>,
}

impl<O> DaPollingExecutor<O>
where
    O: DaRunObservation,
{
    #[must_use]
    pub fn new(observer: Arc<O>) -> Self {
        Self { observer }
    }

    pub async fn poll_until_terminal(
        &self,
        run_id: Uuid,
        cancel: CancellationToken,
        poll_interval: Duration,
        mut on_events: impl FnMut(Vec<ObservedRunEvent>) -> Result<(), ExecutorError>,
    ) -> Result<ObservedRun, ExecutorError> {
        let mut last_sequence = 0_i64;
        loop {
            if cancel.is_cancelled() {
                return self.observer.fetch_run(run_id).await;
            }

            let observed = self.observer.fetch_run(run_id).await?;
            let events = self
                .observer
                .fetch_new_events(run_id, last_sequence)
                .await?;
            if !events.is_empty() {
                last_sequence = events
                    .iter()
                    .map(|event| event.sequence_no)
                    .max()
                    .unwrap_or(last_sequence);
                on_events(events)?;
            }

            if crate::agent::da_events::is_terminal_run_status(&observed.status) {
                return Ok(observed);
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
