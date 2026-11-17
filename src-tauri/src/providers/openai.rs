//! OpenAI Chat Completions adapter.
//!
//! The wire itself lives in [`super::openai_compat`]; this file is only
//! OpenAI's half of it — the endpoint, the bearer credential, and the fact that
//! OpenAI needs `stream_options` asked for explicitly to report usage.

use async_trait::async_trait;
use reqwest::header::AUTHORIZATION;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;
use zeroize::Zeroizing;

use super::adapter::{ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent};
use super::http::{sensitive_header, shared_streaming_client};
use super::openai_compat::{build_request_body, stream_completion, CompatDialect};

const CHAT_COMPLETIONS_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

const DIALECT: CompatDialect = CompatDialect {
    request_usage_via_stream_options: true,
};

pub(crate) struct OpenAiAdapter;

impl OpenAiAdapter {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::OpenAi
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let client = shared_streaming_client()?;
        let bearer = Zeroizing::new(format!("Bearer {secret}"));
        let body = build_request_body(request, &DIALECT);
        let builder = client
            .post(CHAT_COMPLETIONS_ENDPOINT)
            .header(AUTHORIZATION, sensitive_header(bearer.as_str())?);
        stream_completion(builder, &body, cancel, on_event).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::{ContextMessage, ContextRole};

    #[test]
    fn asks_openai_for_the_final_usage_chunk() {
        let request = ProviderChatRequest {
            model_id: "gpt-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request, &DIALECT);

        assert_eq!(
            body["stream_options"]["include_usage"], true,
            "OpenAI reports no usage at all unless it is asked for"
        );
    }

    #[test]
    fn dials_only_the_compiled_in_openai_origin() {
        assert_eq!(
            CHAT_COMPLETIONS_ENDPOINT,
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn reports_its_own_provider_id() {
        assert_eq!(OpenAiAdapter::new().provider_id(), ProviderId::OpenAi);
    }
}
