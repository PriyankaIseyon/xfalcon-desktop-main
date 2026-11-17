use std::{error::Error, fmt};

use schemars::JsonSchema;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

use crate::credentials::{CredentialId, ModelId, ProviderId};
use crate::{ContractVersion, ConversationId, RunId, CONTRACT_VERSION};

const MAX_REASONING_SUMMARY_BYTES: usize = 4_096;

/// Tokens reserved outside model input and output for protocol framing and
/// conservative estimator drift.
pub const MODEL_CONTEXT_RESERVED_TOKENS: u64 = 16_384;
/// Maximum output tokens allocated to one inference turn, even when a model's
/// catalog ceiling is higher.
pub const MODEL_MAX_OUTPUT_TOKENS_PER_TURN: u64 = 128_000;

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct InferenceRequestId(String);

impl From<&str> for InferenceRequestId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for InferenceRequestId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct ToolCallId(String);

impl From<&str> for ToolCallId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ToolCallId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, TS)]
#[schemars(transparent)]
#[ts(type = "string")]
pub struct ReasoningSummary(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReasoningSummaryTooLong {
    actual_bytes: usize,
}

impl fmt::Display for ReasoningSummaryTooLong {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "reasoning summary is {} bytes; maximum is {}",
            self.actual_bytes, MAX_REASONING_SUMMARY_BYTES
        )
    }
}

impl Error for ReasoningSummaryTooLong {}

impl ReasoningSummary {
    pub fn provider_designated(
        summary: impl Into<String>,
    ) -> Result<Self, ReasoningSummaryTooLong> {
        let summary = summary.into();
        if summary.len() > MAX_REASONING_SUMMARY_BYTES {
            return Err(ReasoningSummaryTooLong {
                actual_bytes: summary.len(),
            });
        }
        Ok(Self(summary))
    }
}

impl Serialize for ReasoningSummary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ReasoningSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let summary = String::deserialize(deserializer)?;
        Self::provider_designated(summary).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct InferenceRequest {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub run_id: RunId,
    pub conversation_id: ConversationId,
    pub credential_id: CredentialId,
    pub model_id: ModelId,
    pub input: String,
}

impl InferenceRequest {
    pub fn new(
        run_id: RunId,
        conversation_id: ConversationId,
        credential_id: impl Into<CredentialId>,
        model_id: impl Into<ModelId>,
        input: impl Into<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            run_id,
            conversation_id,
            credential_id: credential_id.into(),
            model_id: model_id.into(),
            input: input.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum InferenceErrorCode {
    AuthenticationFailed,
    /// The credential is valid; the account behind it cannot pay for the call.
    /// Distinct from [`Self::AuthenticationFailed`] because the remedy is the
    /// account, not the key, and distinct from [`Self::RateLimited`] because
    /// waiting does not help.
    PaymentRequired,
    RateLimited,
    ProviderUnavailable,
    /// The request was too large for the provider to accept. Deterministic: the
    /// same request fails identically every time.
    PayloadTooLarge,
    /// The TLS handshake failed certificate verification. Deterministic for the
    /// host, and on a managed fleet usually caused by an inspecting proxy
    /// rather than by the provider.
    TlsVerificationFailed,
    InvalidResponse,
    Cancelled,
    Internal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum InferenceEvent {
    Started {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        request_id: InferenceRequestId,
        provider: ProviderId,
        model_id: ModelId,
    },
    TextDelta {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        #[ts(type = "number")]
        sequence: u64,
        text: String,
    },
    ReasoningSummary {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        #[ts(type = "number")]
        sequence: u64,
        summary: ReasoningSummary,
    },
    ToolCallDelta {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        #[ts(type = "number")]
        sequence: u64,
        call_id: ToolCallId,
        tool_name: Option<String>,
        arguments_delta: String,
    },
    Usage {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        #[ts(type = "number")]
        input_tokens: u64,
        #[ts(type = "number")]
        cached_input_tokens: u64,
        #[ts(type = "number")]
        output_tokens: u64,
        #[ts(type = "number")]
        reasoning_output_tokens: u64,
    },
    /// The run is shortening its conversation history to fit the model's
    /// context window (`active`), or has finished doing so. Sent around the
    /// one slow step a person would otherwise mistake for a stall.
    ContextCompaction {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        active: bool,
    },
    Completed {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        finish_reason: FinishReason,
    },
    Failed {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        code: InferenceErrorCode,
        message: String,
        retryable: bool,
    },
}

impl InferenceEvent {
    pub fn started(
        run_id: RunId,
        request_id: impl Into<InferenceRequestId>,
        provider: ProviderId,
        model_id: impl Into<ModelId>,
    ) -> Self {
        Self::Started {
            version: CONTRACT_VERSION,
            run_id,
            request_id: request_id.into(),
            provider,
            model_id: model_id.into(),
        }
    }

    pub fn text_delta(run_id: RunId, sequence: u64, text: impl Into<String>) -> Self {
        Self::TextDelta {
            version: CONTRACT_VERSION,
            run_id,
            sequence,
            text: text.into(),
        }
    }

    pub fn reasoning_summary(run_id: RunId, sequence: u64, summary: ReasoningSummary) -> Self {
        Self::ReasoningSummary {
            version: CONTRACT_VERSION,
            run_id,
            sequence,
            summary,
        }
    }

    pub fn tool_call_delta(
        run_id: RunId,
        sequence: u64,
        call_id: impl Into<ToolCallId>,
        tool_name: Option<String>,
        arguments_delta: impl Into<String>,
    ) -> Self {
        Self::ToolCallDelta {
            version: CONTRACT_VERSION,
            run_id,
            sequence,
            call_id: call_id.into(),
            tool_name,
            arguments_delta: arguments_delta.into(),
        }
    }

    pub fn usage(
        run_id: RunId,
        input_tokens: u64,
        cached_input_tokens: u64,
        output_tokens: u64,
        reasoning_output_tokens: u64,
    ) -> Self {
        Self::Usage {
            version: CONTRACT_VERSION,
            run_id,
            input_tokens,
            cached_input_tokens,
            output_tokens,
            reasoning_output_tokens,
        }
    }

    pub fn context_compaction(run_id: RunId, active: bool) -> Self {
        Self::ContextCompaction {
            version: CONTRACT_VERSION,
            run_id,
            active,
        }
    }

    pub fn completed(run_id: RunId, finish_reason: FinishReason) -> Self {
        Self::Completed {
            version: CONTRACT_VERSION,
            run_id,
            finish_reason,
        }
    }

    pub fn failed(
        run_id: RunId,
        code: InferenceErrorCode,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self::Failed {
            version: CONTRACT_VERSION,
            run_id,
            code,
            message: message.into(),
            retryable,
        }
    }
}
