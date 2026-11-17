//! Token safety for catalog-backed model context windows.

use std::collections::HashSet;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use xfalcon_contracts::inference::{
    MODEL_CONTEXT_RESERVED_TOKENS, MODEL_MAX_OUTPUT_TOKENS_PER_TURN,
};

use super::context::ContextMessage;
use crate::providers::ProviderToolSchema;

/// Space not advertised as user input or output. This covers request framing,
/// provider-added tokens, and conservative estimator drift.
pub const PROTOCOL_SAFETY_TOKENS: u64 = MODEL_CONTEXT_RESERVED_TOKENS;
/// Conservative fixed overhead for one message or content block that has not
/// yet appeared in provider usage.
pub const UNMEASURED_BLOCK_OVERHEAD_TOKENS: u64 = 32;
/// Fixed request envelope, beta controls, routing, and stream framing.
pub const PROTOCOL_REQUEST_OVERHEAD_TOKENS: u64 = 1_024;
/// Maximum output requested for one provider turn.
///
/// DA supplies each model's provider ceiling. That ceiling remains the source
/// of truth, but it is not the output allocation for every request. Reserving
/// Grok's 450K ceiling on every turn would leave only 33,616 tokens for input.
pub(crate) const MAX_PROVIDER_OUTPUT_TOKENS_PER_TURN: u64 = MODEL_MAX_OUTPUT_TOKENS_PER_TURN;
/// Automatic native compaction threshold for the supported million-token
/// OpenRouter Claude models. The remaining capacity preserves room for the
/// configured output limit, protocol reserve, and large tool-result rounds.
pub(crate) const OPENROUTER_COMPACTION_TRIGGER_TOKENS: u64 = 700_000;
/// The smallest input a provider will compact on request. A request that exists
/// to produce a compaction sets this as its trigger so the provider always
/// compacts it.
pub(crate) const COMPACTION_MIN_TRIGGER_TOKENS: u64 = 50_000;
/// The least history worth sending to be compacted, in estimated tokens. The
/// estimate runs high, so this keeps the true size above
/// [`COMPACTION_MIN_TRIGGER_TOKENS`] with room to spare.
const BOOTSTRAP_MIN_PREFIX_TOKENS: u64 = 80_000;
/// UTF-8 bytes assumed per token for text the provider has not measured.
///
/// Prose runs near four, code and JSON near three, dense identifiers and
/// tables below that. Three stays on the pessimistic side without treating a
/// byte as a token, which overstated every request several times over and made
/// the guard fire long before the provider's own trigger. It is a starting
/// point only: [`UsageAnchor`] replaces it with the provider's count as soon as
/// one exists.
pub(crate) const ESTIMATED_BYTES_PER_TOKEN: u64 = 3;
/// Tokens charged for one image, whatever its file size.
///
/// A provider prices an image by its pixel dimensions, not its bytes: Claude
/// charges one token per 28x28-pixel patch and caps an image at 4,784 tokens on
/// the models that read the largest images (1,568 on the rest). Counting the
/// base64 bytes instead made a 700 KB screenshot look like a quarter-million
/// tokens. The documented ceiling is charged for every image because a
/// few thousand tokens of slack per image costs nothing against a window
/// measured in hundreds of thousands, and an undercount is what overflows one.
pub(crate) const IMAGE_ESTIMATED_TOKENS: u64 = 4_800;

/// Estimated tokens for `bytes` of text.
#[must_use]
pub(crate) const fn text_tokens(bytes: u64) -> u64 {
    bytes.div_ceil(ESTIMATED_BYTES_PER_TOKEN)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ContextCapacityError {
    #[error("model context length must leave room for protocol safety tokens")]
    ContextTooSmall,
    #[error("model output token limit must be positive")]
    OutputLimitMissing,
    #[error("model output and protocol reserves leave no input capacity")]
    NoInputCapacity,
    #[error("context size cannot be represented on this device")]
    PlatformSizeOverflow,
    #[error("durable context history cannot form a compaction boundary")]
    InvalidHistory,
    #[error(
        "fixed request overhead leaves no complete history prefix at the OpenRouter compaction trigger"
    )]
    IndivisibleOverhead,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapPrefix {
    pub(crate) message_count: usize,
    pub(crate) through_sequence_no: i64,
    pub(crate) source_message_count: i64,
    pub(crate) source_message_ids_sha256: String,
}

pub(crate) fn request_input_tokens(
    messages: &[ContextMessage],
    tools: &[ProviderToolSchema],
    compaction_blocks: &[Value],
) -> Result<u64, ContextCapacityError> {
    let overflow = || ContextCapacityError::PlatformSizeOverflow;
    let mut tokens = PROTOCOL_REQUEST_OVERHEAD_TOKENS;
    let mut blocks = 0u64;
    for message in messages {
        let text = u64::try_from(message.text_bytes()).map_err(|_| overflow())?;
        let images = u64::try_from(message.attachment_count()).map_err(|_| overflow())?;
        tokens = tokens
            .checked_add(text_tokens(text))
            .and_then(|tokens| tokens.checked_add(images.checked_mul(IMAGE_ESTIMATED_TOKENS)?))
            .ok_or_else(overflow)?;
        let message_blocks = 1usize
            .saturating_add(usize::from(!message.content.is_empty()))
            .saturating_add(message.tool_calls.len())
            .saturating_add(message.attachments.len())
            .saturating_add(message.reasoning_details.len())
            .saturating_add(usize::from(message.responds_to.is_some()));
        blocks = blocks
            .checked_add(u64::try_from(message_blocks).map_err(|_| overflow())?)
            .ok_or_else(overflow)?;
    }
    for tool in tools {
        let wire = json!({
            "name": tool.name,
            "description": tool.description,
            "input_schema": tool.input_schema,
        });
        let bytes = serde_json::to_vec(&wire).map_err(|_| overflow())?.len();
        tokens = tokens
            .checked_add(text_tokens(u64::try_from(bytes).map_err(|_| overflow())?))
            .ok_or_else(overflow)?;
        blocks = blocks.checked_add(1).ok_or_else(overflow)?;
    }
    for block in compaction_blocks {
        let bytes = serde_json::to_vec(block).map_err(|_| overflow())?.len();
        tokens = tokens
            .checked_add(text_tokens(u64::try_from(bytes).map_err(|_| overflow())?))
            .ok_or_else(overflow)?;
        blocks = blocks.checked_add(1).ok_or_else(overflow)?;
    }
    tokens
        .checked_add(
            blocks
                .checked_mul(UNMEASURED_BLOCK_OVERHEAD_TOKENS)
                .ok_or_else(overflow)?,
        )
        .ok_or_else(overflow)
}

/// The least a provider's prompt count may be, as a percent of this app's
/// estimate for the same request, to be believed. See [`UsageAnchor::new`].
const MIN_PLAUSIBLE_ANCHOR_PERCENT: u128 = 40;

/// Ties this app's own estimate to the provider's count.
///
/// After any request the provider reports exactly how many input tokens it
/// read. The next request is that plus the growth since, so the estimate only
/// has to be right about a few new messages instead of the whole history. The
/// growth is measured in the estimator's own units, which keeps it consistent
/// with the request the anchor was taken from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UsageAnchor {
    prompt_tokens: u64,
    local_estimate: u64,
}

impl UsageAnchor {
    /// `prompt_tokens` is what the provider counted for a request that this
    /// app's estimator scored at `local_estimate`. A request scored at zero
    /// carries nothing to scale from.
    ///
    /// A count far below the estimate is not trusted. The estimator charges
    /// three bytes per token, which runs high but not by half: real text is
    /// four or five. A server that counts only the tokens it newly read (one
    /// that reuses a cached prefix, such as a local Ollama) reports a small
    /// fraction of the real prompt, and anchoring on it would make a full
    /// context look nearly empty, so compaction would never start. Without an
    /// anchor the estimate stays on the cautious side.
    #[must_use]
    pub(crate) fn new(prompt_tokens: u64, local_estimate: u64) -> Option<Self> {
        (local_estimate > 0
            && prompt_tokens > 0
            && u128::from(prompt_tokens) * 100
                >= u128::from(local_estimate) * MIN_PLAUSIBLE_ANCHOR_PERCENT)
            .then_some(Self {
                prompt_tokens,
                local_estimate,
            })
    }

    /// The estimate for a request the estimator scores at `local_now`.
    #[must_use]
    pub(crate) fn estimate(self, local_now: u64) -> u64 {
        if local_now >= self.local_estimate {
            self.prompt_tokens
                .saturating_add(local_now - self.local_estimate)
        } else {
            // The context shrank: keep the provider's ratio rather than the
            // difference, which would understate a request that is mostly the
            // part that stayed.
            let scaled = u128::from(self.prompt_tokens) * u128::from(local_now)
                / u128::from(self.local_estimate);
            u64::try_from(scaled).unwrap_or(u64::MAX)
        }
    }
}

/// The longest run of oldest history that can be sent alone to be compacted.
///
/// "Can be sent" means it fits `hard_input_tokens` with the tools and the
/// active compaction block the request will carry. It must also be big enough
/// for a provider to compact at all, so that the request this prefix feeds
/// produces a compaction instead of an ordinary answer.
/// Replay-only messages preceding saved turns stay on the wire, while source
/// coverage counts only durable identities. No cut can end on replay alone.
pub(crate) fn largest_bootstrap_prefix(
    hard_input_tokens: u64,
    stable_prefix: &[ContextMessage],
    history: &[ContextMessage],
    tools: &[ProviderToolSchema],
    active_blocks: &[Value],
    parent_boundary: i64,
) -> Result<Option<BootstrapPrefix>, ContextCapacityError> {
    let mut largest = None;
    let mut saw_complete_boundary = false;
    for end in 1..=history.len() {
        if history[end - 1].durable_identity.is_none()
            || !is_complete_tool_boundary(&history[..end])
        {
            continue;
        }
        saw_complete_boundary = true;
        let mut request = Vec::with_capacity(stable_prefix.len() + end);
        request.extend_from_slice(stable_prefix);
        request.extend_from_slice(&history[..end]);
        let tokens = request_input_tokens(&request, tools, active_blocks)?;
        if tokens > hard_input_tokens {
            break;
        }
        if tokens < BOOTSTRAP_MIN_PREFIX_TOKENS {
            continue;
        }
        largest = Some(prefix_metadata(&history[..end], parent_boundary)?);
    }
    if largest.is_none() && saw_complete_boundary {
        return Err(ContextCapacityError::IndivisibleOverhead);
    }
    Ok(largest)
}

pub(crate) fn is_complete_tool_boundary(messages: &[ContextMessage]) -> bool {
    let mut offered = HashSet::new();
    let mut answered = HashSet::new();
    for message in messages {
        for call in &message.tool_calls {
            if !offered.insert(call.call_id.as_str()) {
                return false;
            }
        }
        if let Some(result) = message.responds_to.as_ref() {
            if !offered.contains(result.call_id.as_str())
                || !answered.insert(result.call_id.as_str())
            {
                return false;
            }
        }
    }
    offered == answered
}

fn prefix_metadata(
    messages: &[ContextMessage],
    parent_boundary: i64,
) -> Result<BootstrapPrefix, ContextCapacityError> {
    let identities = messages
        .iter()
        .filter_map(|message| message.durable_identity.as_ref())
        .collect::<Vec<_>>();
    if identities.is_empty()
        || identities
            .windows(2)
            .any(|pair| pair[0].sequence_no >= pair[1].sequence_no)
        || identities[0].sequence_no <= parent_boundary
    {
        return Err(ContextCapacityError::InvalidHistory);
    }
    let manifest = identities
        .iter()
        .map(|identity| {
            json!({
                "active": true,
                "id": identity.id,
                "role": identity.role,
                "sequence_no": identity.sequence_no,
            })
        })
        .collect::<Vec<_>>();
    let encoded =
        serde_json::to_vec(&manifest).map_err(|_| ContextCapacityError::PlatformSizeOverflow)?;
    Ok(BootstrapPrefix {
        message_count: messages.len(),
        through_sequence_no: identities.last().expect("non-empty identities").sequence_no,
        source_message_count: i64::try_from(identities.len())
            .map_err(|_| ContextCapacityError::PlatformSizeOverflow)?,
        source_message_ids_sha256: format!("{:x}", Sha256::digest(encoded)),
    })
}

/// Checked capacity values used by context admission and provider request
/// serialization. One value owns both calculations so they cannot drift.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextCapacity {
    context_tokens: u64,
    max_output_tokens: u64,
    hard_input_tokens: u64,
}

impl ContextCapacity {
    pub fn new(
        context_tokens: i64,
        catalog_max_output_tokens: i64,
    ) -> Result<Self, ContextCapacityError> {
        let context_tokens =
            u64::try_from(context_tokens).map_err(|_| ContextCapacityError::ContextTooSmall)?;
        if context_tokens <= PROTOCOL_SAFETY_TOKENS {
            return Err(ContextCapacityError::ContextTooSmall);
        }
        let catalog_max_output_tokens = u64::try_from(catalog_max_output_tokens)
            .map_err(|_| ContextCapacityError::OutputLimitMissing)?;
        if catalog_max_output_tokens == 0 {
            return Err(ContextCapacityError::OutputLimitMissing);
        }
        // Validate the raw DA ceiling before applying the request cap. A
        // provider cannot advertise more output than its whole context.
        if catalog_max_output_tokens >= context_tokens {
            return Err(ContextCapacityError::NoInputCapacity);
        }
        let max_output_tokens = catalog_max_output_tokens.min(MAX_PROVIDER_OUTPUT_TOKENS_PER_TURN);
        let hard_input_tokens = context_tokens
            .checked_sub(max_output_tokens)
            .and_then(|remaining| remaining.checked_sub(PROTOCOL_SAFETY_TOKENS))
            .filter(|remaining| *remaining > 0)
            .ok_or(ContextCapacityError::NoInputCapacity)?;
        Ok(Self {
            context_tokens,
            max_output_tokens,
            hard_input_tokens,
        })
    }

    #[must_use]
    pub const fn context_tokens(self) -> u64 {
        self.context_tokens
    }

    #[must_use]
    pub const fn max_output_tokens(self) -> u64 {
        self.max_output_tokens
    }

    #[must_use]
    pub const fn hard_input_tokens(self) -> u64 {
        self.hard_input_tokens
    }

    /// Estimate for new UTF-8 bytes not covered by the last exact provider
    /// usage value: [`ESTIMATED_BYTES_PER_TOKEN`] per token, then each wire
    /// block pays a fixed framing cost.
    pub fn unmeasured_input_tokens(
        self,
        utf8_bytes: usize,
        content_blocks: usize,
    ) -> Result<u64, ContextCapacityError> {
        let utf8_bytes =
            u64::try_from(utf8_bytes).map_err(|_| ContextCapacityError::PlatformSizeOverflow)?;
        let content_blocks = u64::try_from(content_blocks)
            .map_err(|_| ContextCapacityError::PlatformSizeOverflow)?;
        text_tokens(utf8_bytes)
            .checked_add(
                content_blocks
                    .checked_mul(UNMEASURED_BLOCK_OVERHEAD_TOKENS)
                    .ok_or(ContextCapacityError::PlatformSizeOverflow)?,
            )
            .ok_or(ContextCapacityError::PlatformSizeOverflow)
    }

    #[must_use]
    pub const fn admits(self, input_tokens: u64) -> bool {
        input_tokens <= self.hard_input_tokens
    }
}

#[cfg(test)]
mod shared_policy_tests {
    use super::*;

    #[test]
    fn context_capacity_enforces_the_shared_policy_at_exact_boundaries() {
        assert_eq!(
            ContextCapacity::new(200_000, 200_000),
            Err(ContextCapacityError::NoInputCapacity),
            "the raw output ceiling must be below the context before capping"
        );

        let capped_boundary = MODEL_MAX_OUTPUT_TOKENS_PER_TURN
            .checked_add(MODEL_CONTEXT_RESERVED_TOKENS)
            .expect("shared policy fits u64");
        assert_eq!(
            ContextCapacity::new(capped_boundary as i64, 128_001),
            Err(ContextCapacityError::NoInputCapacity),
            "context must exceed capped output plus the reserve"
        );
        let capped = ContextCapacity::new(capped_boundary as i64 + 1, 128_001)
            .expect("one token above the shared boundary is valid");
        assert_eq!(capped.max_output_tokens(), MODEL_MAX_OUTPUT_TOKENS_PER_TURN);
        assert_eq!(capped.hard_input_tokens(), 1);

        let model_output = 64_000_u64;
        let model_boundary = model_output
            .checked_add(MODEL_CONTEXT_RESERVED_TOKENS)
            .expect("model policy fits u64");
        assert_eq!(
            ContextCapacity::new(model_boundary as i64, model_output as i64),
            Err(ContextCapacityError::NoInputCapacity),
            "context must exceed the model output ceiling plus the reserve"
        );
        let model = ContextCapacity::new(model_boundary as i64 + 1, model_output as i64)
            .expect("one token above the model boundary is valid");
        assert_eq!(model.max_output_tokens(), model_output);
        assert_eq!(model.hard_input_tokens(), 1);
    }
}
