//! Shared provider-adapter contract for local (Path A) model inference.
//!
//! Each vendor adapter turns a bounded, provider-neutral [`ContextMessage`] list into
//! that vendor's wire request, streams the vendor's SSE response, and normalizes every
//! chunk into a [`ProviderStreamEvent`].
//!
//! # Tool calls
//!
//! Until Task 9a, [`ProviderStreamEvent::ToolCallRequested`] was a unit variant: an
//! adapter could report *that* the model wanted a tool but never *which* one, with
//! what arguments, under what id. That made the whole slice-4 tool subsystem
//! (`crate::tools`, `crate::mcp`, `crate::skills`, `crate::agent::tool_loop`)
//! unreachable from a real model no matter how it was composed — the orchestrator
//! had nothing to dispatch. The variant now carries the full call, and
//! [`ProviderChatRequest::tools`] declares the callable set in the request, so the
//! round trip closes.
//!
//! Two invariants every adapter upholds:
//!
//! - **Emit a call only once it is complete.** OpenAI and Anthropic both stream
//!   arguments as fragments; emitting early hands the loop truncated JSON.
//! - **Never send an empty `tools` array.** Some OpenAI-compatible gateways reject
//!   it outright. Omit the key instead.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;

use crate::agent::context::ContextMessage;

#[derive(Clone, PartialEq)]
pub(crate) enum ProviderStreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    /// New tool arguments or opaque block content arrived. This is progress,
    /// never an executable call, answer text, or durable history record.
    Progress,
    /// One complete tool call the model requested. `arguments` is raw JSON
    /// text (see [`crate::agent::context::ContextToolCall`] for why it is not
    /// pre-parsed); it is normalized to `"{}"` when the provider streamed
    /// nothing, so every consumer can parse unconditionally.
    ToolCallRequested {
        call_id: String,
        name: String,
        arguments: String,
    },
    /// The provider's own reasoning blocks for this assistant turn, merged
    /// from their streamed fragments and ordered by the provider's `index`.
    ///
    /// Opaque on purpose: these are echoed back verbatim on later turns of a
    /// tool loop and are never read, rendered, or rewritten by this app.
    /// Anthropic's rule is that the assistant message must be echoed "exactly
    /// as received" — rebuilding it or dropping a block is a 400 — so anything
    /// this layer understood well enough to reconstruct would be a liability,
    /// not a feature.
    ReasoningDetails(Vec<Value>),
    /// One complete native provider compaction block. The block stays opaque:
    /// callers may persist and replay it, but must not rebuild or summarize it.
    CompactionReady {
        index: u32,
        block: Value,
        usage: ProviderUsage,
    },
    Usage {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    /// How many input tokens the provider counted for this request, cached
    /// tokens included. The one exact measure of the prompt this app ever gets;
    /// the context guard anchors its estimate on it.
    PromptSize {
        tokens: u64,
    },
    /// Why the provider ended this turn, sent once, immediately before
    /// [`Self::Completed`]. An adapter that cannot tell reports
    /// [`ProviderStopReason::Natural`].
    Stopped {
        reason: ProviderStopReason,
        /// Tool calls the provider began and never finished. An adapter drops
        /// them instead of emitting [`Self::ToolCallRequested`], because
        /// half-written arguments are not a call anything may run.
        discarded_tool_calls: u32,
    },
    Completed,
}

/// Why a provider turn ended, normalized across wires.
///
/// Every wire reports this differently (`stop_reason`, `finish_reason`,
/// `finishReason`). Collapsing them here keeps the truncation policy in one
/// place: a turn that ended at a limit is never the same thing as a turn the
/// model finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderStopReason {
    /// The model finished, stopped on a stop sequence, or the wire said nothing.
    Natural,
    /// The model stopped to call tools.
    ToolUse,
    /// The per-turn output ceiling (`max_tokens`) cut the turn short.
    OutputLimit,
    /// The model's whole context window filled before the turn finished.
    ContextLimit,
    /// The connection ended mid-answer with no stop reason: a proxy or the
    /// server closed the stream. Treated like an output limit, because the
    /// remedy is the same: ask for the rest, in smaller pieces.
    StreamCut,
    /// The provider declined to answer.
    Refusal,
    /// A reason this app has no policy for (for example `pause_turn`).
    Other,
}

impl ProviderStopReason {
    /// Whether the turn ended before the model chose to stop.
    #[must_use]
    pub(crate) const fn is_truncation(self) -> bool {
        matches!(
            self,
            Self::OutputLimit | Self::ContextLimit | Self::StreamCut
        )
    }
}

impl fmt::Debug for ProviderStreamEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Progress => formatter.write_str("Progress"),
            Self::TextDelta(value) => formatter
                .debug_struct("TextDelta")
                .field("bytes", &value.len())
                .finish(),
            Self::ReasoningDelta(value) => formatter
                .debug_struct("ReasoningDelta")
                .field("bytes", &value.len())
                .finish(),
            Self::ToolCallRequested {
                call_id,
                name,
                arguments,
            } => formatter
                .debug_struct("ToolCallRequested")
                .field("call_id", call_id)
                .field("name", name)
                .field("argument_bytes", &arguments.len())
                .finish(),
            Self::ReasoningDetails(blocks) => formatter
                .debug_struct("ReasoningDetails")
                .field("block_count", &blocks.len())
                .finish(),
            Self::CompactionReady {
                index,
                block: _,
                usage,
            } => formatter
                .debug_struct("CompactionReady")
                .field("index", index)
                .field("block", &"<redacted>")
                .field("usage", usage)
                .finish(),
            Self::Usage {
                input_tokens,
                output_tokens,
            } => formatter
                .debug_struct("Usage")
                .field("input_tokens", input_tokens)
                .field("output_tokens", output_tokens)
                .finish(),
            Self::PromptSize { tokens } => formatter
                .debug_struct("PromptSize")
                .field("tokens", tokens)
                .finish(),
            Self::Stopped {
                reason,
                discarded_tool_calls,
            } => formatter
                .debug_struct("Stopped")
                .field("reason", reason)
                .field("discarded_tool_calls", discarded_tool_calls)
                .finish(),
            Self::Completed => formatter.write_str("Completed"),
        }
    }
}

/// Detailed usage for one Anthropic Messages response.
///
/// With native compaction enabled, the top-level counts exclude compaction
/// sampling. `iterations` is therefore the billing authority when present;
/// consumers must not add it to the top-level counts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProviderUsage {
    pub provider_request_id: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
    pub reasoning_output_tokens: Option<u32>,
    pub iterations: Vec<ProviderUsageIteration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderUsageIteration {
    pub kind: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// OpenRouter-only state that primary agent calls must carry.
///
/// Utility calls may leave this absent. The OpenRouter primary-agent path must
/// set it from the canonical conversation identity before adapter dispatch.
#[derive(Clone, PartialEq)]
pub(crate) struct OpenRouterRequestContext {
    /// Non-secret credential identity used only for capability-cache scope.
    pub route_identity: String,
    /// Canonical DA conversation UUID, used as OpenRouter's sticky route key.
    pub session_id: String,
    /// The active native compaction blocks in provider order. Each value must
    /// be an exact `compaction` content block returned by the provider.
    pub compaction_blocks: Vec<Value>,
    /// Tool-call turns whose thinking was produced against the prefix that an
    /// active compaction replaced. The Messages serializer omits thinking only
    /// for these turns. Thinking produced after the active block stays intact
    /// for the next tool round.
    pub invalidated_thinking_call_ids: BTreeSet<String>,
    /// Input tokens at which the provider compacts by itself.
    ///
    /// The ordinary trigger keeps a long run inside the window. A request
    /// made to *produce* a compaction over history too long to send whole sets
    /// the smallest legal trigger instead, so the provider compacts it whatever
    /// this app's own estimate says the prompt weighs.
    pub compaction_trigger_tokens: u64,
}

impl fmt::Debug for OpenRouterRequestContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenRouterRequestContext")
            .field("route_identity", &self.route_identity)
            .field("session_id", &self.session_id)
            .field("compaction_block_count", &self.compaction_blocks.len())
            .field(
                "invalidated_thinking_call_count",
                &self.invalidated_thinking_call_ids.len(),
            )
            .field("compaction_trigger_tokens", &self.compaction_trigger_tokens)
            .finish()
    }
}

/// One tool declared to the model, in provider-neutral form.
///
/// `input_schema` is a JSON Schema object. It originates from
/// `crate::tools::registry::ToolDescriptor` and is echoed to the provider
/// verbatim — the registry, not this layer, is the authority on what a tool
/// accepts.
#[derive(Clone, Debug)]
pub(crate) struct ProviderToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// What an adapter sends when its API *requires* a per-turn ceiling and the
/// caller asked for none.
///
/// Not a policy choice — a floor for the two wires that will not accept the
/// field's absence. Inflating it would be worse than omitting: Anthropic
/// rejects a `max_tokens` above the model's own maximum, and that maximum
/// differs per model, so any large constant is a 400 waiting for a model bump.
/// The adapters that reach this value do not request reasoning, so a modest
/// ceiling costs them nothing.
pub(crate) const REQUIRED_MAX_OUTPUT_TOKENS: u32 = 4_096;

#[derive(Clone)]
pub(crate) struct ProviderChatRequest {
    pub model_id: String,
    pub messages: Vec<ContextMessage>,
    /// Per-turn output ceiling, or `None` to let the provider apply its own.
    ///
    /// `None` is the normal case. A ceiling has to cover the model's thinking
    /// as well as its answer — they share this budget — so a number picked
    /// here is a number that truncates an answer the moment a model thinks
    /// more than expected. The run stays bounded regardless by
    /// `crate::agent::output::RunOutputBuilder`'s run-wide byte budget, which
    /// is the limit that actually protects the user.
    pub max_output_tokens: Option<u32>,
    /// Whether the adapter may ask the selected model for a reasoning trace.
    /// Small utility calls disable this so their output budget remains output.
    pub allow_reasoning: bool,
    /// The tools the model may call this turn. Empty means "no tools" and
    /// every adapter must then omit its tool key entirely rather than send an
    /// empty array.
    pub tools: Vec<ProviderToolSchema>,
    /// OpenRouter Anthropic Messages state for the primary agent path.
    pub openrouter: Option<OpenRouterRequestContext>,
}

impl fmt::Debug for ProviderChatRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderChatRequest")
            .field("model_id", &self.model_id)
            .field("message_count", &self.messages.len())
            .field("max_output_tokens", &self.max_output_tokens)
            .field("allow_reasoning", &self.allow_reasoning)
            .field("tool_count", &self.tools.len())
            .field("openrouter", &self.openrouter)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ProviderError {
    #[error("provider rejected the credential")]
    AuthenticationFailed,
    /// The credential is accepted and the account cannot pay for the call.
    ///
    /// Kept apart from [`Self::AuthenticationFailed`] and [`Self::RateLimited`]
    /// because it shares a remedy with neither: the key is fine, and waiting
    /// changes nothing.
    #[error("provider account cannot pay for this request")]
    PaymentRequired,
    #[error("provider rate limited the request")]
    RateLimited {
        /// What the provider's `Retry-After` header asked for, when it sent one
        /// and spelled it as delay-seconds. Carried on the error because the
        /// response it came from is consumed inside the adapter, so nothing
        /// above this layer could read it otherwise.
        retry_after: Option<Duration>,
    },
    #[error("provider is temporarily unavailable")]
    Unavailable,
    /// The provider stopped sending bytes for longer than the transport's idle
    /// bound, before or during the response.
    ///
    /// Kept apart from [`Self::Unavailable`] because the remedy differs: the
    /// endpoint was reached, and a slow or busy model is the usual cause, so
    /// "check the host" and "the provider is down" both mislead.
    #[error("provider did not send data in time")]
    TimedOut,
    /// A custom endpoint rejected its model-bound credential. Kept separate
    /// so the UI can identify the affected private model key.
    #[error("custom provider rejected the credential")]
    CustomAuthenticationFailed,
    /// A custom endpoint could not be resolved or reached. This carries no URL
    /// because custom routes are user data and may contain sensitive topology.
    #[error("custom provider endpoint is unreachable")]
    EndpointUnavailable,
    /// A custom endpoint's URL failed its syntax check. Deterministic and never
    /// retried.
    #[error("custom provider endpoint was refused")]
    AddressRefused,
    /// The request exceeded what the provider will accept. Deterministic — the
    /// same bytes are refused every time — so it is reported, never retried.
    #[error("request is larger than the provider accepts")]
    PayloadTooLarge,
    /// The provider rejected the native-compaction beta contract or could not
    /// route every required parameter.
    #[error("the selected provider route does not support native compaction")]
    NativeCompactionUnsupported,
    /// The provider returned an incomplete or invalid compaction block.
    #[error("the provider returned a malformed native compaction block")]
    MalformedCompaction,
    /// The request crossed the provider's hard context window.
    #[error("the request exceeds the model context window")]
    ContextOverflow,
    /// The TLS handshake failed certificate verification.
    ///
    /// Deterministic for the host: the next handshake presents the same chain
    /// to the same trust store. This is the expected failure on a fleet behind
    /// a TLS-inspecting proxy, where it is the local machine that needs fixing
    /// and not the provider — so it must never be reported as an outage.
    #[error("could not verify the provider's TLS certificate")]
    TlsVerificationFailed,
    #[error("provider returned a response this adapter could not parse")]
    InvalidResponse,
    #[error("provider request was cancelled")]
    Cancelled,
    /// A plan sign-in (Claude or ChatGPT) is missing, expired, or was revoked.
    /// The remedy is signing in again, not retrying.
    #[error("the plan sign-in is missing or no longer valid")]
    PlanSignInRequired,
    /// The plan's own usage limit stopped the request. Waiting does not help
    /// and no reset time is known, so it is never retried automatically.
    #[error("the plan usage limit was reached")]
    PlanUsageLimit,
    /// The vendor says this user, workspace or plan cannot use the route.
    #[error("the plan cannot be used with this app")]
    PlanNotEligible,
    /// The plan runtime (the `claude` program) is missing, too old, or failed
    /// to start.
    #[error("the plan runtime is unavailable")]
    RuntimeUnavailable,
}

/// Streams one chat turn from a single allowlisted model provider.
///
/// Fixed-vendor implementations use compiled origins. The custom OpenAI-compatible
/// implementation accepts only the model-bound URL returned by DA, then revalidates,
/// resolves, filters, and pins it for each request. `secret` is the caller's decrypted
/// provider-key plaintext, borrowed for the duration of one call; it must never be
/// logged, cloned into a longer-lived value, or echoed in any error variant.
#[async_trait]
pub(crate) trait ProviderAdapter: Send + Sync {
    fn provider_id(&self) -> ProviderId;

    /// Probes the route-specific native-compaction contract.
    ///
    /// `route_identity` is a non-secret credential identity supplied by the
    /// credential store. It keys the bounded capability cache without hashing
    /// or retaining the provider secret. Non-OpenRouter adapters fail closed.
    async fn probe_native_compaction(
        &self,
        _model_id: &str,
        _route_identity: &str,
        _session_id: &str,
        _secret: &str,
        _cancel: &CancellationToken,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::NativeCompactionUnsupported)
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError>;
}
