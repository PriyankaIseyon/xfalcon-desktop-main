//! Map DA run-event replay rows into normalized [`InferenceEvent`] values.

use indexmap::IndexMap;
use serde_json::Value;
use xfalcon_contracts::credentials::{ModelId, ProviderId};
use xfalcon_contracts::inference::{
    FinishReason, InferenceErrorCode, InferenceEvent, ReasoningSummary,
};
use xfalcon_contracts::{RunId, CONTRACT_VERSION};

use crate::agent::executor::{ObservedRunEvent, TOOL_EVENT_PREFIX};
use crate::agent::output::RunOutputBuilder;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaEventMapError {
    UnknownEventType,
    ToolsUnavailable,
}

/// Resolves DA's `provider` string onto a provider this build can dial itself.
///
/// `None` means "not one of ours" — a provider DA knows about that this desktop
/// build has no adapter for. It must stay `None` rather than falling back to a
/// default: the previous default was [`ProviderId::OpenAi`], which sent an
/// unrecognized provider's key to `api.openai.com`. Every such run failed at
/// authentication, blaming the user's key for a routing bug. The caller's
/// correct response to `None` is to run the turn server-side, where the
/// provider *is* known — never to guess.
pub fn map_provider_id(provider: &str) -> Option<ProviderId> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" => Some(ProviderId::OpenAi),
        "anthropic" => Some(ProviderId::Anthropic),
        "google" => Some(ProviderId::Google),
        "openrouter" => Some(ProviderId::OpenRouter),
        "custom_openai" => Some(ProviderId::CustomOpenAi),
        // Device-local runtimes. Their ids come from this device's own model
        // list, never from DA: `clients::da::mapping::map_provider` still
        // refuses both, so DA cannot select a runtime that runs a program or
        // spends a plan on this device.
        "claude_code" => Some(ProviderId::ClaudeCode),
        "chatgpt_plan" => Some(ProviderId::ChatgptPlan),
        _ => None,
    }
}

/// Whether DA considers this run finished.
///
/// `"failed"` belongs here because DA emits it: `crate::chat::da_repository`
/// has always parsed `"failed" | "error"` onto the same `RunState::Failed`.
/// Leaving it out made a terminal run look live — Stop then asked DA to cancel
/// a run it had already closed, DA fenced the write, and the reader was told
/// their Stop had failed when there was nothing left to stop.
///
/// The single source of truth for "is this run over": `super::executor`'s poll
/// loop, `crate::chat::service`'s Stop reconcile, and the DA client's
/// terminal-conflict check all read it, so the three cannot drift apart again.
pub fn is_terminal_run_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "completed" | "error" | "failed" | "cancelled"
    )
}

pub fn map_observed_event(
    run_id: &RunId,
    provider: ProviderId,
    model_id: &ModelId,
    event: &ObservedRunEvent,
    output: &mut RunOutputBuilder,
    started: &mut bool,
) -> Result<Vec<InferenceEvent>, DaEventMapError> {
    if event.event_type.starts_with(TOOL_EVENT_PREFIX) {
        output.note_tool_request();
        return Err(DaEventMapError::ToolsUnavailable);
    }

    match event.event_type.as_str() {
        "run.start" => {
            if *started {
                return Ok(Vec::new());
            }
            *started = true;
            Ok(vec![InferenceEvent::Started {
                version: CONTRACT_VERSION,
                run_id: run_id.clone(),
                request_id: format!("da-{}", event.sequence_no).into(),
                provider,
                model_id: model_id.clone(),
            }])
        }
        "run.stream_delta" => {
            let Some(text) = extract_text_payload(&event.payload) else {
                return Ok(Vec::new());
            };
            let _ = output.push_text(&text);
            Ok(vec![InferenceEvent::text_delta(
                run_id.clone(),
                event.sequence_no.max(0) as u64,
                text,
            )])
        }
        "run.stream_retract" => Ok(Vec::new()),
        "run.thinking_step" => {
            let Some(text) = extract_text_payload(&event.payload) else {
                return Ok(Vec::new());
            };
            let summary = ReasoningSummary::provider_designated(text)
                .map_err(|_| DaEventMapError::UnknownEventType)?;
            Ok(vec![InferenceEvent::reasoning_summary(
                run_id.clone(),
                event.sequence_no.max(0) as u64,
                summary,
            )])
        }
        "run.complete" => Ok(vec![InferenceEvent::completed(
            run_id.clone(),
            FinishReason::Stop,
        )]),
        "run.cancel" => Ok(vec![InferenceEvent::failed(
            run_id.clone(),
            InferenceErrorCode::Cancelled,
            "run cancelled",
            false,
        )]),
        "run.error" => {
            let message =
                extract_text_payload(&event.payload).unwrap_or_else(|| "run failed".into());
            Ok(vec![InferenceEvent::failed(
                run_id.clone(),
                InferenceErrorCode::ProviderUnavailable,
                message,
                true,
            )])
        }
        "approval_required" => {
            output.note_tool_request();
            Err(DaEventMapError::ToolsUnavailable)
        }
        _ => Ok(Vec::new()),
    }
}

fn extract_text_payload(payload: &IndexMap<String, Value>) -> Option<String> {
    for key in ["delta", "text", "content", "message"] {
        if let Some(value) = payload.get(key).and_then(Value::as_str) {
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_every_provider_this_build_can_dial() {
        for (name, expected) in [
            ("openai", ProviderId::OpenAi),
            ("anthropic", ProviderId::Anthropic),
            ("google", ProviderId::Google),
            ("openrouter", ProviderId::OpenRouter),
            ("claude_code", ProviderId::ClaudeCode),
            ("chatgpt_plan", ProviderId::ChatgptPlan),
        ] {
            assert_eq!(map_provider_id(name), Some(expected));
            assert_eq!(
                map_provider_id(&format!("  {} ", name.to_ascii_uppercase())),
                Some(expected),
                "DA's spelling of a provider is not case- or whitespace-normalized"
            );
        }
    }

    /// Regression: an unrecognized provider used to resolve to
    /// [`ProviderId::OpenAi`], which sent that provider's key to
    /// `api.openai.com` and surfaced the resulting 401 as the user's key being
    /// bad. It must resolve to nothing, so the caller runs the turn server-side.
    #[test]
    fn an_unrecognized_provider_resolves_to_nothing_rather_than_a_default() {
        for name in ["azure-openai", "mistral", "openai-compatible", "", "  "] {
            assert_eq!(
                map_provider_id(name),
                None,
                "{name:?} must not be silently routed to another provider"
            );
        }
    }
}
