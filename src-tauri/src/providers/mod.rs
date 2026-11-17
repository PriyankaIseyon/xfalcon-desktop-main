use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{
    header::AUTHORIZATION, redirect::Policy, Client, RequestBuilder, Response, StatusCode, Url,
};
use xfalcon_contracts::credentials::ProviderId;
use zeroize::Zeroizing;

use crate::credentials::verifier::{CredentialVerifier, VerificationError, VerifiedCredential};

mod adapter;
mod anthropic;
pub(crate) mod chatgpt_plan;
pub(crate) mod claude_runtime;
mod custom_openai;
pub(crate) mod gateway;
mod google;
mod http;
mod openai;
mod openai_compat;
mod openrouter;
mod registry;
mod retry;

pub(crate) use adapter::{
    OpenRouterRequestContext, ProviderAdapter, ProviderChatRequest, ProviderError,
    ProviderStopReason, ProviderStreamEvent, ProviderToolSchema, ProviderUsage,
};
#[cfg(test)]
pub(crate) use custom_openai::prompt_scan_fragments as custom_openai_prompt_scan_fragments;
#[cfg(test)]
pub(crate) use custom_openai::request_body_for_test as custom_openai_request_body_for_test;
pub(crate) use custom_openai::{
    discover_models, normalize_base_url_input, verify_custom_model_connection,
    CustomModelConnectionError,
};
#[cfg(test)]
pub(crate) use openrouter::prompt_scan_fragments as openrouter_prompt_scan_fragments;
pub(crate) use openrouter::requires_strict_thinking_prefix;
#[cfg(test)]
pub(crate) use openrouter::{openrouter_compat_request_body, openrouter_request_body};
pub(crate) use registry::{adapter_for, custom_openai_adapter_with_sampling, with_retry};

use http::sensitive_header;

pub(crate) const OPENAI_MODELS_ENDPOINT: &str = "https://api.openai.com/v1/models";
pub(crate) const ANTHROPIC_MODELS_ENDPOINT: &str = "https://api.anthropic.com/v1/models";
pub(crate) const GOOGLE_MODELS_ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/models";
/// Deliberately **not** `openrouter.ai/api/v1/models`: that endpoint is
/// unauthenticated and answers 200 to any bearer, so verifying against it would
/// accept a typo'd or revoked key. `/api/v1/key` is the authenticated one — it
/// describes the presented key and answers 401 when there isn't a valid one.
pub(crate) const OPENROUTER_KEY_ENDPOINT: &str = "https://openrouter.ai/api/v1/key";
pub(crate) const MAX_VERIFICATION_RESPONSE_BYTES: usize = 64 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(15);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(crate) struct FixedOriginCredentialVerifier {
    client: Client,
    endpoints: ProviderEndpoints,
}

impl FixedOriginCredentialVerifier {
    pub(crate) fn new() -> Result<Self, VerifierBuildError> {
        Self::with_configuration(
            ProviderEndpoints::production()?,
            CONNECT_TIMEOUT,
            READ_TIMEOUT,
            TOTAL_TIMEOUT,
        )
    }

    fn with_configuration(
        endpoints: ProviderEndpoints,
        connect_timeout: Duration,
        read_timeout: Duration,
        total_timeout: Duration,
    ) -> Result<Self, VerifierBuildError> {
        let client = Client::builder()
            .use_rustls_tls()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(connect_timeout)
            .read_timeout(read_timeout)
            .timeout(total_timeout)
            .build()
            .map_err(|_| VerifierBuildError)?;
        Ok(Self { client, endpoints })
    }

    fn request(
        &self,
        provider: ProviderId,
        secret: &str,
    ) -> Result<RequestBuilder, VerificationError> {
        let endpoint = self
            .endpoints
            .for_provider(provider)
            .ok_or(VerificationError::ProviderUnavailable)?;
        let request = self.client.get(endpoint.clone());
        let to_verification_error = |_: ProviderError| VerificationError::InvalidCredential;
        match provider {
            ProviderId::OpenAi | ProviderId::OpenRouter => {
                let bearer = Zeroizing::new(format!("Bearer {secret}"));
                Ok(request.header(
                    AUTHORIZATION,
                    sensitive_header(bearer.as_str()).map_err(to_verification_error)?,
                ))
            }
            ProviderId::Anthropic => Ok(request
                .header(
                    "x-api-key",
                    sensitive_header(secret).map_err(to_verification_error)?,
                )
                .header("anthropic-version", ANTHROPIC_VERSION)),
            ProviderId::Google => Ok(request.header(
                "x-goog-api-key",
                sensitive_header(secret).map_err(to_verification_error)?,
            )),
            // None of these has a provider key to verify.
            ProviderId::CustomOpenAi | ProviderId::ClaudeCode | ProviderId::ChatgptPlan => {
                Err(VerificationError::ProviderUnavailable)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        openai: Url,
        anthropic: Url,
        google: Url,
        openrouter: Url,
        connect_timeout: Duration,
        read_timeout: Duration,
        total_timeout: Duration,
    ) -> Result<Self, VerifierBuildError> {
        let endpoints = ProviderEndpoints {
            openai,
            anthropic,
            google,
            openrouter,
        };
        if !endpoints.all_test_endpoints_are_loopback() {
            return Err(VerifierBuildError);
        }
        Self::with_configuration(endpoints, connect_timeout, read_timeout, total_timeout)
    }
}

#[async_trait]
impl CredentialVerifier for FixedOriginCredentialVerifier {
    async fn verify(
        &self,
        provider: ProviderId,
        secret: &str,
    ) -> Result<VerifiedCredential, VerificationError> {
        let response = self
            .request(provider, secret)?
            .send()
            .await
            .map_err(|_| VerificationError::ProviderUnavailable)?;
        classify_bounded_response(response).await
    }
}

async fn classify_bounded_response(
    response: Response,
) -> Result<VerifiedCredential, VerificationError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_VERIFICATION_RESPONSE_BYTES as u64)
    {
        return Err(VerificationError::ProviderUnavailable);
    }

    let status = response.status();
    let mut bytes_received = 0_usize;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| VerificationError::ProviderUnavailable)?;
        bytes_received = bytes_received
            .checked_add(chunk.len())
            .filter(|length| *length <= MAX_VERIFICATION_RESPONSE_BYTES)
            .ok_or(VerificationError::ProviderUnavailable)?;
    }

    match status {
        status if status.is_success() => Ok(VerifiedCredential),
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED => {
            Err(VerificationError::InvalidCredential)
        }
        StatusCode::FORBIDDEN => Err(VerificationError::InsufficientPermissions),
        StatusCode::TOO_MANY_REQUESTS => Err(VerificationError::RateLimited),
        _ => Err(VerificationError::ProviderUnavailable),
    }
}

struct ProviderEndpoints {
    openai: Url,
    anthropic: Url,
    google: Url,
    openrouter: Url,
}

impl ProviderEndpoints {
    fn production() -> Result<Self, VerifierBuildError> {
        Ok(Self {
            openai: parse_endpoint(OPENAI_MODELS_ENDPOINT)?,
            anthropic: parse_endpoint(ANTHROPIC_MODELS_ENDPOINT)?,
            google: parse_endpoint(GOOGLE_MODELS_ENDPOINT)?,
            openrouter: parse_endpoint(OPENROUTER_KEY_ENDPOINT)?,
        })
    }

    const fn for_provider(&self, provider: ProviderId) -> Option<&Url> {
        match provider {
            ProviderId::OpenAi => Some(&self.openai),
            ProviderId::Anthropic => Some(&self.anthropic),
            ProviderId::Google => Some(&self.google),
            ProviderId::OpenRouter => Some(&self.openrouter),
            ProviderId::CustomOpenAi | ProviderId::ClaudeCode | ProviderId::ChatgptPlan => None,
        }
    }

    #[cfg(test)]
    fn all_test_endpoints_are_loopback(&self) -> bool {
        [
            &self.openai,
            &self.anthropic,
            &self.google,
            &self.openrouter,
        ]
        .into_iter()
        .all(is_loopback_test_endpoint)
    }
}

fn parse_endpoint(value: &str) -> Result<Url, VerifierBuildError> {
    Url::parse(value).map_err(|_| VerifierBuildError)
}

#[cfg(test)]
fn is_loopback_test_endpoint(url: &Url) -> bool {
    let is_loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    matches!(url.scheme(), "http" | "https")
        && is_loopback
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the fixed-origin provider verifier could not be constructed")]
pub(crate) struct VerifierBuildError;

#[cfg(test)]
mod tests;
