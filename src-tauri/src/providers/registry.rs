//! Resolves the [`ProviderAdapter`] implementation for an allowlisted provider.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;
use xfalcon_contracts::custom_models::CustomModelSampling;

use super::adapter::{ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent};
use super::anthropic::AnthropicAdapter;
use super::custom_openai::CustomOpenAiAdapter;
use super::google::GoogleAdapter;
use super::openai::OpenAiAdapter;
use super::openrouter::OpenRouterAdapter;
use super::retry::RetryingAdapter;

/// The adapter for `provider`, wrapped so a transient failure is retried while
/// nothing has been emitted yet.
///
/// Wrapping here rather than at each call site is what makes the retry
/// universal: every path that reaches a model goes through this function, so
/// none of them can forget it, and no adapter carries its own loop.
#[must_use]
pub(crate) fn adapter_for(provider: ProviderId) -> Arc<dyn ProviderAdapter> {
    if provider == ProviderId::CustomOpenAi {
        // Custom endpoints require their DA-bound URL. Keep the old entry point
        // fail-closed for a missed call site rather than guessing an origin.
        return Arc::new(CustomEndpointRequiredAdapter);
    }
    with_retry(vendor_adapter(provider))
}

/// Wraps an adapter that is built outside this module, so it gets the same retry
/// as every other path to a model.
#[must_use]
pub(crate) fn with_retry(adapter: Arc<dyn ProviderAdapter>) -> Arc<dyn ProviderAdapter> {
    Arc::new(RetryingAdapter::new(adapter))
}

/// Builds the guarded, single-attempt adapter for a DA-validated custom model URL.
pub(crate) fn custom_openai_adapter_with_sampling(
    base_url: &str,
    upstream_model_id: &str,
    sampling: CustomModelSampling,
) -> Result<Arc<dyn ProviderAdapter>, ProviderError> {
    Ok(Arc::new(CustomOpenAiAdapter::with_sampling(
        base_url,
        upstream_model_id,
        sampling,
    )?))
}

fn vendor_adapter(provider: ProviderId) -> Arc<dyn ProviderAdapter> {
    match provider {
        ProviderId::OpenAi => Arc::new(OpenAiAdapter::new()),
        ProviderId::Anthropic => Arc::new(AnthropicAdapter::new()),
        ProviderId::Google => Arc::new(GoogleAdapter::new()),
        ProviderId::OpenRouter => Arc::new(OpenRouterAdapter::new()),
        ProviderId::CustomOpenAi => Arc::new(CustomEndpointRequiredAdapter),
        // The plan runtimes run through their own constructors, which carry what
        // they need. A call site that reaches here by mistake fails closed.
        ProviderId::ClaudeCode | ProviderId::ChatgptPlan => {
            Arc::new(PlanRuntimeRequiredAdapter(provider))
        }
    }
}

struct PlanRuntimeRequiredAdapter(ProviderId);

#[async_trait]
impl ProviderAdapter for PlanRuntimeRequiredAdapter {
    fn provider_id(&self) -> ProviderId {
        self.0
    }

    async fn stream(
        &self,
        _request: &ProviderChatRequest,
        _secret: &str,
        _cancel: &CancellationToken,
        _on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        Err(ProviderError::InvalidResponse)
    }
}

struct CustomEndpointRequiredAdapter;

#[async_trait]
impl ProviderAdapter for CustomEndpointRequiredAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::CustomOpenAi
    }

    async fn stream(
        &self,
        _request: &ProviderChatRequest,
        _secret: &str,
        _cancel: &CancellationToken,
        _on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        Err(ProviderError::InvalidResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_matching_adapter_for_every_provider() {
        for provider in [
            ProviderId::OpenAi,
            ProviderId::Anthropic,
            ProviderId::Google,
            ProviderId::OpenRouter,
            ProviderId::CustomOpenAi,
            ProviderId::ClaudeCode,
            ProviderId::ChatgptPlan,
        ] {
            assert_eq!(adapter_for(provider).provider_id(), provider);
        }
    }

    #[test]
    fn custom_adapter_requires_a_valid_bound_endpoint() {
        assert!(custom_openai_adapter_with_sampling(
            "https://gateway.example.org/v1",
            "model",
            CustomModelSampling::default(),
        )
        .is_ok());
        assert!(custom_openai_adapter_with_sampling(
            "ftp://127.0.0.1/v1",
            "model",
            CustomModelSampling::default(),
        )
        .is_err());
        assert!(custom_openai_adapter_with_sampling(
            "http://127.0.0.1:11434/v1",
            "model",
            CustomModelSampling::default(),
        )
        .is_ok());
        assert!(custom_openai_adapter_with_sampling(
            "http://localhost:8000/v1",
            "model",
            CustomModelSampling::default(),
        )
        .is_ok());
    }
}
