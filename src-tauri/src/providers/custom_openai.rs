//! Guarded transport for private OpenAI-compatible model endpoints.
//!
//! DA stores the endpoint text. This client repeats the syntax validation
//! because stored data is still a network trust boundary, then resolves the
//! hostname for every request and pins the answered addresses into that
//! request's client. Keeping the URL hostname preserves TLS certificate and SNI
//! checks while preventing a second DNS lookup from rebinding the connection.
//! Any `http` or `https` host is allowed, including `localhost` and IP literals
//! on public, private and loopback addresses.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{redirect::Policy, Client, Response, Url};
use serde_json::{json, Value};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;
use xfalcon_contracts::custom_models::{
    CustomModelDiscoveryResult, CustomModelDiscoveryUnavailableReason, CustomModelSampling,
    DiscoveredCustomModel,
};
use xfalcon_contracts::CONTRACT_VERSION;
use zeroize::Zeroizing;

use crate::agent::context::{ContextMessage, ContextRole};
use crate::credentials::service::MAX_PROVIDER_KEY_BYTES;

use super::adapter::{
    ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStopReason, ProviderStreamEvent,
};
use super::gateway;
use super::http::{
    classify_response, classify_transport_error, sensitive_header, DEFAULT_STREAM_BYTES,
};
use super::openai_compat::{
    build_request_body, consume_completion_body, consume_completion_stream, CompatDialect,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest silence before or between bytes from a private model.
///
/// Longer than the 90 s the hosted providers get: those fill a slow start with
/// keep-alive framing, and a self-hosted server usually sends nothing at all
/// while it reads a long prompt. Under the run's 300 s stall limit, so a silent
/// model is reported as a timeout and not as a stalled run.
const READ_TIMEOUT: Duration = Duration::from_secs(240);
const TOTAL_TIMEOUT: Duration = super::http::TOTAL_TIMEOUT;
const MAX_BASE_URL_BYTES: usize = 2_048;
const MAX_UPSTREAM_MODEL_ID_CHARS: usize = 256;
const MAX_DISCOVERY_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCOVERED_MODELS: usize = 2_048;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(45);
const CONNECTION_TEST_MAX_OUTPUT_TOKENS: u32 = 64;
const CONNECTION_TEST_PROMPT: &str = "Reply with a brief greeting.";
const CHAT_COMPLETIONS_SEGMENTS: [&str; 2] = ["chat", "completions"];
const GATEWAY_TOOL_RESPONSE_GUIDANCE: &str = "Keep private planning and tool-step narration out of response content. \
Use the reasoning channel for thinking. When a tool is needed, call it directly without a textual prelude. \
Reserve response content for the user's answer or a necessary clarification.";
const DIALECT: CompatDialect = CompatDialect {
    // Many OpenAI-compatible gateways accept streaming Chat Completions but
    // reject OpenAI's optional usage extension. The company gateway accepts it
    // and is asked for usage in `apply_gateway_parameters`.
    request_usage_via_stream_options: false,
};
pub(crate) struct CustomOpenAiAdapter {
    base_url: String,
    upstream_model_id: String,
    resolver: Arc<dyn AddressResolver>,
    sampling: CustomModelSampling,
    /// Set only for the company gateway, which is single-engine and slow.
    gateway: Option<GatewayTransport>,
}

/// How requests to the company gateway differ from any other private endpoint:
/// one request at a time per app, and a retry for the failures the gateway
/// documents as transient.
struct GatewayTransport {
    queue: Arc<Semaphore>,
    retry_delays: Vec<Duration>,
    /// When a cut stream was last seen. Requests are sent whole for a while
    /// after (see [`gateway::NON_STREAMING_AFTER_CUT`]).
    last_stream_cut: Arc<std::sync::Mutex<Option<Instant>>>,
}

impl GatewayTransport {
    fn shared() -> Self {
        // One slot for the whole app, not per adapter: every chat, title and
        // follow-up request shares the same engine behind the gateway.
        static QUEUE: OnceLock<Arc<Semaphore>> = OnceLock::new();
        static LAST_CUT: OnceLock<Arc<std::sync::Mutex<Option<Instant>>>> = OnceLock::new();
        Self {
            queue: QUEUE.get_or_init(|| Arc::new(Semaphore::new(1))).clone(),
            retry_delays: gateway::RETRY_DELAYS.to_vec(),
            last_stream_cut: LAST_CUT.get_or_init(Arc::default).clone(),
        }
    }

    fn streams_recently_cut(&self) -> bool {
        self.last_stream_cut
            .lock()
            .ok()
            .and_then(|cut| *cut)
            .is_some_and(|at| at.elapsed() < gateway::NON_STREAMING_AFTER_CUT)
    }

    fn note_stream_cut(&self) {
        if let Ok(mut cut) = self.last_stream_cut.lock() {
            *cut = Some(Instant::now());
        }
    }

    async fn enter(
        &self,
        cancel: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit, ProviderError> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ProviderError::Cancelled),
            permit = self.queue.clone().acquire_owned() => {
                permit.map_err(|_| ProviderError::EndpointUnavailable)
            }
        }
    }
}

/// One attempt that did not produce a response to stream.
struct AttemptFailure {
    error: ProviderError,
    /// Whether the gateway documents this failure as worth another attempt.
    transient: bool,
}

impl AttemptFailure {
    const fn permanent(error: ProviderError) -> Self {
        Self {
            error,
            transient: false,
        }
    }
}

impl CustomOpenAiAdapter {
    pub(crate) fn new(base_url: &str, upstream_model_id: &str) -> Result<Self, ProviderError> {
        Self::with_sampling(base_url, upstream_model_id, CustomModelSampling::default())
    }

    pub(crate) fn with_sampling(
        base_url: &str,
        upstream_model_id: &str,
        sampling: CustomModelSampling,
    ) -> Result<Self, ProviderError> {
        validate_base_url(base_url)?;
        validate_upstream_model_id(upstream_model_id)?;
        validate_sampling(&sampling)?;
        Ok(Self {
            base_url: base_url.to_owned(),
            upstream_model_id: upstream_model_id.to_owned(),
            resolver: Arc::new(SystemResolver),
            sampling,
            gateway: gateway::is_gateway_model(base_url, upstream_model_id)
                .then(GatewayTransport::shared),
        })
    }

    #[cfg(test)]
    fn with_test_resolver(
        base_url: &str,
        upstream_model_id: &str,
        resolver: Arc<dyn AddressResolver>,
    ) -> Result<Self, ProviderError> {
        validate_base_url(base_url)?;
        validate_upstream_model_id(upstream_model_id)?;
        Ok(Self {
            base_url: base_url.to_owned(),
            upstream_model_id: upstream_model_id.to_owned(),
            resolver,
            sampling: CustomModelSampling::default(),
            gateway: None,
        })
    }

    async fn request_target(
        &self,
        cancel: &CancellationToken,
    ) -> Result<RequestTarget, ProviderError> {
        self.request_target_with_segments(cancel, &CHAT_COMPLETIONS_SEGMENTS)
            .await
    }

    async fn request_target_with_segments(
        &self,
        cancel: &CancellationToken,
        segments: &[&str],
    ) -> Result<RequestTarget, ProviderError> {
        let parsed = validate_base_url(&self.base_url)?;
        let resolved = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            resolved = self.resolver.resolve(&parsed.host, parsed.port) => resolved?,
        };
        let addresses = dedupe_addresses(resolved)?;
        let mut endpoint = parsed.url;
        {
            let mut path = endpoint
                .path_segments_mut()
                .map_err(|_| ProviderError::AddressRefused)?;
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(RequestTarget {
            endpoint,
            host: parsed.host,
            addresses,
        })
    }

    fn client_for(&self, target: &RequestTarget) -> Result<Client, ProviderError> {
        let (read_timeout, total_timeout) = if self.gateway.is_some() {
            (gateway::HTTP_TIMEOUT, gateway::HTTP_TIMEOUT)
        } else {
            (READ_TIMEOUT, TOTAL_TIMEOUT)
        };
        Client::builder()
            .use_rustls_tls()
            // A proxy would resolve the target again and defeat address pinning.
            .no_proxy()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(read_timeout)
            .timeout(total_timeout)
            .resolve_to_addrs(&target.host, &target.addresses)
            .build()
            .map_err(|_| ProviderError::EndpointUnavailable)
    }

    /// Sends the request once and returns the response only if it succeeded.
    async fn send_once(
        &self,
        body: &Value,
        authorization: &HeaderValue,
        cancel: &CancellationToken,
    ) -> Result<Response, AttemptFailure> {
        let target = self
            .request_target(cancel)
            .await
            .map_err(|error| AttemptFailure {
                transient: error == ProviderError::EndpointUnavailable,
                error,
            })?;
        let client = self
            .client_for(&target)
            .map_err(AttemptFailure::permanent)?;
        let builder = client
            .post(target.endpoint)
            .header(AUTHORIZATION, authorization.clone());
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Err(AttemptFailure::permanent(ProviderError::Cancelled));
            }
            response = builder.json(body).send() => response,
        }
        .map_err(|error| match classify_transport_error(&error) {
            ProviderError::TlsVerificationFailed => {
                AttemptFailure::permanent(ProviderError::TlsVerificationFailed)
            }
            // The model was reached and went quiet. A timeout means the
            // request may still be running behind the gateway, and sending it
            // again would queue a second copy.
            ProviderError::TimedOut => AttemptFailure::permanent(ProviderError::TimedOut),
            _ => AttemptFailure {
                error: ProviderError::EndpointUnavailable,
                transient: !error.is_timeout(),
            },
        })?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            return Err(AttemptFailure::permanent(
                ProviderError::CustomAuthenticationFailed,
            ));
        }
        if !response.status().is_success() {
            return Err(AttemptFailure {
                // 504 is the gateway's 60-minute limit and is never retried.
                transient: matches!(status, 502 | 503),
                error: classify_response(response).await,
            });
        }
        Ok(response)
    }

    fn request_body(&self, request: &ProviderChatRequest) -> Value {
        let mut body = build_request_body(request, &DIALECT);
        body["model"] = Value::String(self.upstream_model_id.clone());
        if self.gateway.is_some() {
            apply_gateway_parameters(&mut body, request, &self.sampling);
        } else {
            apply_sampling_parameters(&mut body, &self.sampling);
        }
        body
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CustomModelDiscoveryError {
    #[error("Enter a valid API base URL, such as http://localhost:8000/v1.")]
    AddressRefused,
    #[error("Enter a valid API key.")]
    InvalidInput,
    #[error("The provider rejected this API key.")]
    AuthenticationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CustomModelConnectionError {
    #[error("Enter a valid API base URL, such as http://localhost:8000/v1.")]
    AddressRefused,
    #[error("The provider rejected this API key.")]
    AuthenticationFailed,
    #[error("The provider could not use this model configuration. Check the model ID, API base URL, and model settings.")]
    ModelRejected,
    #[error("Could not reach the provider. Check the API base URL and network connection.")]
    Unreachable,
    #[error("The provider connection test timed out. Check the API base URL and try again.")]
    TimedOut,
    #[error("The provider could not complete the connection test. Check the API key, model ID, and provider account.")]
    ProviderRejected,
}

pub(crate) async fn verify_custom_model_connection(
    base_url: &str,
    upstream_model_id: &str,
    secret: &str,
    sampling: CustomModelSampling,
    max_output_tokens: i64,
) -> Result<(), CustomModelConnectionError> {
    let adapter = CustomOpenAiAdapter::with_sampling(base_url, upstream_model_id, sampling)
        .map_err(map_connection_error)?;
    let max_output_tokens = u32::try_from(max_output_tokens)
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.min(CONNECTION_TEST_MAX_OUTPUT_TOKENS))
        .ok_or(CustomModelConnectionError::ModelRejected)?;
    verify_connection_with_adapter(&adapter, secret, max_output_tokens, CONNECTION_TEST_TIMEOUT)
        .await
}

async fn verify_connection_with_adapter(
    adapter: &CustomOpenAiAdapter,
    secret: &str,
    max_output_tokens: u32,
    timeout: Duration,
) -> Result<(), CustomModelConnectionError> {
    let request = ProviderChatRequest {
        model_id: adapter.upstream_model_id.clone(),
        messages: vec![ContextMessage::new(
            ContextRole::User,
            CONNECTION_TEST_PROMPT,
        )],
        max_output_tokens: Some(max_output_tokens),
        allow_reasoning: false,
        tools: Vec::new(),
        openrouter: None,
    };
    let cancel = CancellationToken::new();
    let mut has_answer = false;
    let mut completed = false;
    let mut truncated = false;
    let mut collect = |event| match event {
        ProviderStreamEvent::TextDelta(text) => {
            has_answer |= text.chars().any(|character| !character.is_whitespace());
        }
        ProviderStreamEvent::Stopped { reason, .. } if reason.is_truncation() => truncated = true,
        ProviderStreamEvent::Completed => completed = true,
        _ => {}
    };
    let outcome = tokio::time::timeout(
        timeout,
        adapter.stream(&request, secret, &cancel, &mut collect),
    )
    .await
    .map_err(|_| CustomModelConnectionError::TimedOut)?
    .map_err(map_connection_error);
    match outcome {
        Ok(()) if has_answer && completed && !truncated => Ok(()),
        Ok(()) => Err(CustomModelConnectionError::ModelRejected),
        Err(error) => Err(error),
    }
}

fn map_connection_error(error: ProviderError) -> CustomModelConnectionError {
    match error {
        ProviderError::AuthenticationFailed
        | ProviderError::CustomAuthenticationFailed
        | ProviderError::PlanSignInRequired
        | ProviderError::PlanNotEligible => CustomModelConnectionError::AuthenticationFailed,
        ProviderError::AddressRefused => CustomModelConnectionError::AddressRefused,
        ProviderError::InvalidResponse
        | ProviderError::PayloadTooLarge
        | ProviderError::ContextOverflow
        | ProviderError::NativeCompactionUnsupported
        | ProviderError::MalformedCompaction => CustomModelConnectionError::ModelRejected,
        ProviderError::PaymentRequired
        | ProviderError::RateLimited { .. }
        | ProviderError::PlanUsageLimit => CustomModelConnectionError::ProviderRejected,
        ProviderError::TimedOut => CustomModelConnectionError::TimedOut,
        ProviderError::Unavailable
        | ProviderError::RuntimeUnavailable
        | ProviderError::EndpointUnavailable
        | ProviderError::TlsVerificationFailed
        | ProviderError::Cancelled => CustomModelConnectionError::Unreachable,
    }
}

pub(crate) async fn discover_models(
    base_url: String,
    secret: String,
) -> Result<CustomModelDiscoveryResult, CustomModelDiscoveryError> {
    if secret.is_empty()
        || secret.len() > MAX_PROVIDER_KEY_BYTES
        || secret.trim() != secret
        || secret.bytes().any(|byte| byte < 33 || byte == 127)
    {
        return Err(CustomModelDiscoveryError::InvalidInput);
    }
    let base_url = normalize_base_url_input(&base_url)
        .map_err(|_| CustomModelDiscoveryError::AddressRefused)?;
    let secret = Zeroizing::new(secret);
    let adapter = CustomOpenAiAdapter::new(&base_url, "discovery")
        .map_err(|_| CustomModelDiscoveryError::AddressRefused)?;
    discover_models_with_adapter(&adapter, secret.as_str()).await
}

async fn discover_models_with_adapter(
    adapter: &CustomOpenAiAdapter,
    secret: &str,
) -> Result<CustomModelDiscoveryResult, CustomModelDiscoveryError> {
    match tokio::time::timeout(DISCOVERY_TIMEOUT, discover_models_once(adapter, secret)).await {
        Ok(result) => result,
        Err(_) => Ok(discovery_unavailable(
            CustomModelDiscoveryUnavailableReason::Unreachable,
        )),
    }
}

async fn discover_models_once(
    adapter: &CustomOpenAiAdapter,
    secret: &str,
) -> Result<CustomModelDiscoveryResult, CustomModelDiscoveryError> {
    let cancel = CancellationToken::new();
    let target = match adapter
        .request_target_with_segments(&cancel, &["models"])
        .await
    {
        Ok(target) => target,
        Err(ProviderError::AddressRefused) => {
            return Err(CustomModelDiscoveryError::AddressRefused)
        }
        Err(_) => {
            return Ok(discovery_unavailable(
                CustomModelDiscoveryUnavailableReason::Unreachable,
            ))
        }
    };
    let client = match Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(DISCOVERY_TIMEOUT)
        .timeout(DISCOVERY_TIMEOUT)
        .resolve_to_addrs(&target.host, &target.addresses)
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return Ok(discovery_unavailable(
                CustomModelDiscoveryUnavailableReason::Unreachable,
            ))
        }
    };
    let bearer = Zeroizing::new(format!("Bearer {secret}"));
    let authorization = sensitive_header(bearer.as_str())
        .map_err(|_| CustomModelDiscoveryError::AuthenticationFailed)?;
    let response = match client
        .get(target.endpoint)
        .header(AUTHORIZATION, authorization)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => {
            return Ok(discovery_unavailable(
                CustomModelDiscoveryUnavailableReason::Unreachable,
            ))
        }
    };
    match response.status().as_u16() {
        401 | 403 => return Err(CustomModelDiscoveryError::AuthenticationFailed),
        404 | 405 | 501 => {
            return Ok(discovery_unavailable(
                CustomModelDiscoveryUnavailableReason::Unsupported,
            ))
        }
        status if !(200..300).contains(&status) => {
            return Ok(discovery_unavailable(
                CustomModelDiscoveryUnavailableReason::Unreachable,
            ))
        }
        _ => {}
    }
    let body = match read_discovery_body(response).await {
        Ok(body) => body,
        Err(()) => {
            return Ok(discovery_unavailable(
                CustomModelDiscoveryUnavailableReason::InvalidResponse,
            ))
        }
    };
    match parse_discovered_models(&body, secret, &adapter.base_url) {
        Ok(models) if models.is_empty() => Ok(discovery_unavailable(
            CustomModelDiscoveryUnavailableReason::Empty,
        )),
        Ok(models) => Ok(CustomModelDiscoveryResult {
            version: CONTRACT_VERSION,
            models,
            existing_models: Vec::new(),
            unavailable_reason: None,
        }),
        Err(()) => Ok(discovery_unavailable(
            CustomModelDiscoveryUnavailableReason::InvalidResponse,
        )),
    }
}

fn discovery_unavailable(
    reason: CustomModelDiscoveryUnavailableReason,
) -> CustomModelDiscoveryResult {
    CustomModelDiscoveryResult {
        version: CONTRACT_VERSION,
        models: Vec::new(),
        existing_models: Vec::new(),
        unavailable_reason: Some(reason),
    }
}

async fn read_discovery_body(response: Response) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DISCOVERY_RESPONSE_BYTES as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_DISCOVERY_RESPONSE_BYTES {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse_discovered_models(
    body: &[u8],
    secret: &str,
    base_url: &str,
) -> Result<Vec<DiscoveredCustomModel>, ()> {
    let value: Value = serde_json::from_slice(body).map_err(|_| ())?;
    let data = value.get("data").and_then(Value::as_array).ok_or(())?;
    if data.len() > MAX_DISCOVERED_MODELS {
        return Err(());
    }
    let mut seen = HashSet::new();
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let object = item.as_object().ok_or(())?;
        let id = object.get("id").and_then(Value::as_str).ok_or(())?;
        if !valid_discovered_text(id, MAX_UPSTREAM_MODEL_ID_CHARS)
            || (!secret.is_empty() && id.contains(secret))
        {
            return Err(());
        }
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let name = object.get("name").and_then(Value::as_str);
        if name.is_some_and(|name| !secret.is_empty() && name.contains(secret)) {
            return Err(());
        }
        let discovered_display_name = name.filter(|name| valid_discovered_text(name, 128));
        let mut display_name = discovered_display_name.unwrap_or(id).to_owned();
        let top_provider = object.get("top_provider");
        let model_info = object.get("model_info");
        let context_length = first_positive_i64(&[
            object.get("context_length"),
            object.get("max_model_len"),
            top_provider.and_then(|value| value.get("context_length")),
            model_info.and_then(|value| value.get("context_length")),
            model_info.and_then(|value| value.get("max_model_len")),
        ]);
        let max_output_tokens = first_positive_i64(&[
            object.get("max_output_tokens"),
            top_provider.and_then(|value| value.get("max_completion_tokens")),
            model_info.and_then(|value| value.get("max_output_tokens")),
            model_info.and_then(|value| value.get("max_completion_tokens")),
        ]);
        let supported = object
            .get("supported_parameters")
            .and_then(valid_string_array);
        let supports_tools = supported.as_deref().map(|values| values.contains(&"tools"));
        let supported_sampling_parameters: Option<Vec<String>> =
            supported.as_deref().map(|values| {
                values
                    .iter()
                    .filter(|parameter| matches!(**parameter, "temperature" | "top_p"))
                    .map(|parameter| (*parameter).to_owned())
                    .collect()
            });
        let supports_vision = object
            .get("architecture")
            .and_then(|value| value.get("input_modalities"))
            .and_then(valid_string_array)
            .map(|modalities| modalities.contains(&"image"));
        let defaults = object.get("default_parameters");
        let sampling = CustomModelSampling {
            temperature: valid_bounded_f64(
                defaults.and_then(|value| value.get("temperature")),
                2.0,
            ),
            top_p: valid_bounded_f64(defaults.and_then(|value| value.get("top_p")), 1.0),
        };
        if gateway::is_gateway_model(base_url, id) && discovered_display_name.is_none() {
            display_name = gateway::DISPLAY_NAME.to_owned();
        }
        models.push(DiscoveredCustomModel {
            upstream_model_id: id.to_owned(),
            display_name,
            context_length,
            max_output_tokens,
            supports_tools,
            supports_vision,
            sampling,
            supported_sampling_parameters,
        });
    }
    Ok(models)
}

fn valid_discovered_text(value: &str, maximum_chars: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= maximum_chars
        && !value.bytes().any(|byte| byte < 32 || byte == 127)
}

fn valid_string_array(value: &Value) -> Option<Vec<&str>> {
    value
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
}

fn first_positive_i64(values: &[Option<&Value>]) -> Option<i64> {
    values.iter().flatten().find_map(|value| {
        value
            .as_i64()
            .filter(|value| *value > 0 && *value <= i64::from(i32::MAX))
    })
}

fn valid_bounded_f64(value: Option<&Value>, maximum: f64) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=maximum).contains(value))
}

/// The gateway-specific request parameters. Thinking stays on for the answer to
/// a new message. A request that does not want reasoning (a title, a follow-up
/// chip, a context summary) turns it off, which takes about 5 seconds instead of
/// 65. So does one that only reads tool results: the plan was made when the
/// message arrived, and thinking again after every tool call turned a run of
/// fourteen calls into eleven minutes.
fn apply_sampling_parameters(body: &mut Value, sampling: &CustomModelSampling) {
    if let Some(temperature) = sampling.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = sampling.top_p {
        body["top_p"] = json!(top_p);
    }
}

fn apply_gateway_parameters(
    body: &mut Value,
    request: &ProviderChatRequest,
    sampling: &CustomModelSampling,
) {
    apply_sampling_parameters(body, sampling);
    body["stream_options"] = json!({ "include_usage": true });
    // Qwen can narrate a tool round in ordinary content, which is streamed as
    // an answer until the tool request identifies it as intermediate prose.
    // Keep the instruction on the gateway wire; do not buffer genuine answers
    // or change the saved conversation prompt or other providers.
    if !request.tools.is_empty() {
        let messages = body["messages"]
            .as_array_mut()
            .expect("completion requests contain a message array");
        if let Some(system) = messages
            .iter_mut()
            .find(|message| message["role"] == "system")
        {
            let content = system["content"]
                .as_str()
                .expect("system messages contain text");
            system["content"] =
                Value::String(format!("{content}\n\n{GATEWAY_TOOL_RESPONSE_GUIDANCE}"));
        } else {
            messages.insert(
                0,
                json!({ "role": "system", "content": GATEWAY_TOOL_RESPONSE_GUIDANCE }),
            );
        }
    }
    let reading_tool_results = request
        .messages
        .last()
        .is_some_and(|message| message.role == ContextRole::Tool);
    if !request.allow_reasoning || reading_tool_results {
        body["reasoning_effort"] = json!("none");
    }
}

#[async_trait]
impl ProviderAdapter for CustomOpenAiAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::CustomOpenAi
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        // Held until the stream ends. The gateway serves one request at a time,
        // so a second one would only wait in its queue with the clock running.
        let _slot = match &self.gateway {
            Some(gateway) => Some(gateway.enter(cancel).await?),
            None => None,
        };
        let bearer = Zeroizing::new(format!("Bearer {secret}"));
        let authorization = sensitive_header(bearer.as_str())?;
        // The request's model id is the durable DA catalogue identity. Custom
        // gateways need the model's separately bound upstream id on the wire.
        // Change only the wire field so a large conversation is not cloned.
        let mut body = self.request_body(request);
        // After a cut stream the gateway is asked for the whole answer instead.
        let whole = self
            .gateway
            .as_ref()
            .is_some_and(GatewayTransport::streams_recently_cut);
        if whole {
            body["stream"] = Value::Bool(false);
            if let Some(object) = body.as_object_mut() {
                object.remove("stream_options");
            }
        }
        // Retrying is safe here: no event has reached the caller before a
        // response arrives.
        let mut retry_delays = self
            .gateway
            .iter()
            .flat_map(|gateway| gateway.retry_delays.iter().copied());
        let response = loop {
            let failure = match self.send_once(&body, &authorization, cancel).await {
                Ok(response) => break response,
                Err(failure) => failure,
            };
            let Some(delay) = failure.transient.then(|| retry_delays.next()).flatten() else {
                return Err(failure.error);
            };
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ProviderError::Cancelled),
                () = tokio::time::sleep(delay) => {}
            }
        };
        let mut cut = false;
        let mut thinking_bytes = 0usize;
        let mut answering = false;
        let mut thinking_ran_away = false;
        // A child token, so that giving up on endless thinking stops this read
        // without looking like the person pressed Stop.
        let stop = cancel.child_token();
        let mut forward = |event: ProviderStreamEvent| {
            cut |= matches!(
                event,
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::StreamCut,
                    ..
                }
            );
            match &event {
                ProviderStreamEvent::ReasoningDelta(text) => {
                    thinking_bytes = thinking_bytes.saturating_add(text.len());
                    if self.gateway.is_some()
                        && !answering
                        && thinking_bytes > gateway::MAX_THINKING_BYTES
                    {
                        thinking_ran_away = true;
                        stop.cancel();
                    }
                }
                ProviderStreamEvent::TextDelta(_)
                | ProviderStreamEvent::ToolCallRequested { .. } => answering = true,
                _ => {}
            }
            if !thinking_ran_away {
                on_event(event);
            }
        };
        let outcome = if whole {
            consume_completion_body(response, cancel, &mut forward).await
        } else {
            let limit = if self.gateway.is_some() {
                gateway::STREAM_BYTE_LIMIT
            } else {
                DEFAULT_STREAM_BYTES
            };
            consume_completion_stream(response, &stop, limit, &mut forward).await
        };
        if thinking_ran_away && !cancel.is_cancelled() {
            // The model has thought for minutes and written nothing. End the
            // turn as one that spent its output budget: the run asks again
            // with thinking off.
            tracing::warn!(
                thinking_bytes,
                "the gateway model kept thinking without answering; ending the turn"
            );
            on_event(ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::OutputLimit,
                discarded_tool_calls: 0,
            });
            on_event(ProviderStreamEvent::Completed);
            return Ok(());
        }
        if cut {
            if let Some(gateway) = &self.gateway {
                gateway.note_stream_cut();
            }
        }
        // A failure here came after the endpoint answered, so its host is
        // fine: `Unavailable` and `TimedOut` stay as they are rather than
        // becoming `EndpointUnavailable`, which tells the reader to check it.
        outcome.map_err(|error| match error {
            ProviderError::AuthenticationFailed => ProviderError::CustomAuthenticationFailed,
            other => other,
        })
    }
}

#[cfg(test)]
pub(crate) fn request_body_for_test(
    base_url: &str,
    upstream_model_id: &str,
    sampling: CustomModelSampling,
    request: &ProviderChatRequest,
) -> Result<Value, ProviderError> {
    Ok(
        CustomOpenAiAdapter::with_sampling(base_url, upstream_model_id, sampling)?
            .request_body(request),
    )
}

#[cfg(test)]
pub(crate) fn prompt_scan_fragments() -> Vec<(&'static str, String)> {
    let request = ProviderChatRequest {
        model_id: "synthetic-model".to_owned(),
        messages: vec![ContextMessage::new(
            ContextRole::User,
            CONNECTION_TEST_PROMPT,
        )],
        max_output_tokens: Some(CONNECTION_TEST_MAX_OUTPUT_TOKENS),
        allow_reasoning: false,
        tools: Vec::new(),
        openrouter: None,
    };
    let body = request_body_for_test(
        "https://api.example.com",
        "synthetic-model",
        CustomModelSampling::default(),
        &request,
    )
    .expect("synthetic gateway request is valid");
    let mut request_with_tools = request.clone();
    request_with_tools.tools = vec![super::adapter::ProviderToolSchema {
        name: "synthetic.query".to_owned(),
        description: "Query synthetic business data".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
    }];
    let guided_body = request_body_for_test(
        "https://api.example.com",
        "synthetic-model",
        CustomModelSampling::default(),
        &request_with_tools,
    )
    .expect("synthetic gateway request with a tool is valid");
    vec![
        (
            "custom_gateway_tool_response_guidance",
            GATEWAY_TOOL_RESPONSE_GUIDANCE.to_owned(),
        ),
        ("custom_openai_probe_request", body.to_string()),
        (
            "custom_openai_tool_guidance_request",
            guided_body.to_string(),
        ),
    ]
}

fn validate_sampling(sampling: &CustomModelSampling) -> Result<(), ProviderError> {
    let valid = |value: Option<f64>, maximum: f64| {
        value.is_none_or(|value| value.is_finite() && (0.0..=maximum).contains(&value))
    };
    if valid(sampling.temperature, 2.0) && valid(sampling.top_p, 1.0) {
        Ok(())
    } else {
        Err(ProviderError::InvalidResponse)
    }
}

struct RequestTarget {
    endpoint: Url,
    host: String,
    addresses: Vec<SocketAddr>,
}

struct ParsedBaseUrl {
    url: Url,
    host: String,
    port: u16,
}

fn validate_base_url(base_url: &str) -> Result<ParsedBaseUrl, ProviderError> {
    if base_url.is_empty()
        || base_url.len() > MAX_BASE_URL_BYTES
        || !base_url.is_ascii()
        || base_url.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        || base_url.contains('\\')
        || base_url.contains('?')
        || base_url.contains('#')
    {
        return Err(ProviderError::AddressRefused);
    }

    let url = Url::parse(base_url).map_err(|_| ProviderError::AddressRefused)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || authority_contains_percent(base_url)
    {
        return Err(ProviderError::AddressRefused);
    }

    // `host_str` keeps the brackets of an IPv6 literal. The resolver and the
    // client's address override take the bare address.
    let authority_host = url.host_str().ok_or(ProviderError::AddressRefused)?;
    let host = authority_host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    if url.domain().is_some_and(|domain| !valid_dns_name(domain)) {
        return Err(ProviderError::AddressRefused);
    }
    let port = url
        .port_or_known_default()
        .filter(|port| *port != 0)
        .ok_or(ProviderError::AddressRefused)?;
    if has_dot_path_segment(url.path()) {
        return Err(ProviderError::AddressRefused);
    }

    // DA stores one normalized spelling. Accepting any other spelling here
    // would mean the value did not pass that contract or was altered later.
    if normalized_base_url(&url, authority_host, port) != base_url {
        return Err(ProviderError::AddressRefused);
    }

    Ok(ParsedBaseUrl { url, host, port })
}

/// The one spelling this client stores and compares: lower-case scheme and
/// host, the scheme's default port dropped, no trailing slash.
fn normalized_base_url(url: &Url, authority_host: &str, port: u16) -> String {
    let default_port = if url.scheme() == "https" { 443 } else { 80 };
    let authority = if port == default_port {
        authority_host.to_owned()
    } else {
        format!("{authority_host}:{port}")
    };
    format!(
        "{}://{authority}{}",
        url.scheme(),
        url.path().trim_end_matches('/')
    )
}

pub(crate) fn normalize_base_url_input(base_url: &str) -> Result<String, ProviderError> {
    if base_url.is_empty()
        || base_url.len() > MAX_BASE_URL_BYTES
        || !base_url.is_ascii()
        || base_url.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        || base_url.contains('\\')
        || base_url.contains('?')
        || base_url.contains('#')
    {
        return Err(ProviderError::AddressRefused);
    }
    let raw_path = base_url
        .split_once("://")
        .and_then(|(_, remainder)| remainder.find('/').map(|index| &remainder[index..]))
        .unwrap_or("");
    if has_dot_path_segment(raw_path) {
        return Err(ProviderError::AddressRefused);
    }
    let url = Url::parse(base_url).map_err(|_| ProviderError::AddressRefused)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || authority_contains_percent(base_url)
        || authority_is_empty(base_url)
        || has_dot_path_segment(url.path())
    {
        return Err(ProviderError::AddressRefused);
    }
    let authority_host = url.host_str().ok_or(ProviderError::AddressRefused)?;
    // `url` rewrites `127.1`, `0x7f.0.0.1` and reads `010.0.0.1` as octal 8.0.0.1.
    // Sending the key to an address the person did not type is worse than a refusal.
    if authority_host.parse::<std::net::Ipv4Addr>().is_ok()
        && !typed_host_is(base_url, authority_host)
    {
        return Err(ProviderError::AddressRefused);
    }
    let port = url
        .port_or_known_default()
        .filter(|port| *port != 0)
        .ok_or(ProviderError::AddressRefused)?;
    let normalized = normalized_base_url(&url, authority_host, port);
    validate_base_url(&normalized)?;
    Ok(normalized)
}

fn validate_upstream_model_id(value: &str) -> Result<(), ProviderError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().count() > MAX_UPSTREAM_MODEL_ID_CHARS
        || value
            .chars()
            .any(|character| character < ' ' || character == '\u{7f}')
    {
        return Err(ProviderError::InvalidResponse);
    }
    Ok(())
}

fn authority_contains_percent(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .is_none_or(|authority| authority.contains('%') || authority.contains('@'))
}

fn has_dot_path_segment(path: &str) -> bool {
    path.split('/').any(|segment| {
        let decoded_dots = segment.to_ascii_lowercase().replace("%2e", ".");
        matches!(decoded_dots.as_str(), "." | "..")
    })
}

/// A DNS name in the one spelling DA stores: lower-case labels of letters, digits
/// and hyphens (a hyphen never first or last), none empty, at most 63 bytes each,
/// at most 253 in all, and a last label that starts with a letter so that no
/// numeric spelling of an IP address passes as a name.
fn valid_dns_name(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
        && host
            .rsplit('.')
            .next()
            .is_some_and(|last| last.starts_with(|first: char| first.is_ascii_lowercase()))
}

/// Whether the text between `://` and the next `/` is empty. The `url` crate
/// skips extra slashes, so `http:///v1` would otherwise parse as host `v1`.
fn authority_is_empty(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .is_none_or(str::is_empty)
}

/// Whether the host as typed (the authority without its port) equals `host`,
/// ignoring case. Used to refuse a numeric IPv4 spelling the `url` crate rewrites.
fn typed_host_is(value: &str, host: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .is_some_and(|authority| {
            let typed = authority
                .rsplit_once(':')
                .map_or(authority, |(name, _)| name);
            typed.eq_ignore_ascii_case(host)
        })
}

#[async_trait]
trait AddressResolver: Send + Sync {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, ProviderError>;
}

struct SystemResolver;

#[async_trait]
impl AddressResolver for SystemResolver {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, ProviderError> {
        tokio::net::lookup_host((host, port))
            .await
            .map(|addresses| addresses.collect())
            .map_err(|_| ProviderError::EndpointUnavailable)
    }
}

/// Drops repeated answers and refuses an empty one. Every address is allowed:
/// a private model may be on loopback, on the local network or on the internet.
fn dedupe_addresses(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, ProviderError> {
    if addresses.is_empty() {
        return Err(ProviderError::EndpointUnavailable);
    }
    let mut seen = HashSet::new();
    Ok(addresses
        .into_iter()
        .filter(|address| seen.insert(*address))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::net::Ipv4Addr;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::agent::context::{ContextMessage, ContextRole};

    struct SequenceResolver(Mutex<VecDeque<Vec<SocketAddr>>>);
    struct HangingResolver;

    impl SequenceResolver {
        fn new(results: impl IntoIterator<Item = Vec<SocketAddr>>) -> Self {
            Self(Mutex::new(results.into_iter().collect()))
        }
    }

    #[async_trait]
    impl AddressResolver for SequenceResolver {
        async fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<SocketAddr>, ProviderError> {
            self.0
                .lock()
                .expect("resolver lock")
                .pop_front()
                .ok_or(ProviderError::EndpointUnavailable)
        }
    }

    #[async_trait]
    impl AddressResolver for HangingResolver {
        async fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<SocketAddr>, ProviderError> {
            std::future::pending().await
        }
    }

    fn request(model_id: &str) -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: model_id.to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hello")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        }
    }

    fn test_adapter(
        server: &MockServer,
        resolver: Arc<dyn AddressResolver>,
    ) -> CustomOpenAiAdapter {
        let address = *server.address();
        CustomOpenAiAdapter::with_test_resolver(
            &format!("http://gateway.example.org:{}/v1", address.port()),
            "configured/upstream-model",
            resolver,
        )
        .expect("test adapter")
    }

    #[test]
    fn stored_base_url_is_revalidated_in_normal_form() {
        for invalid in [
            "ftp://api.example.org/v1",
            "https://api.example.org/v1/",
            "https://API.example.org/v1",
            "https://api.example.org:443/v1",
            "http://api.example.org:0/v1",
            "http://127.0.0.1:8000/v1/",
            "http://LOCALHOST:8000/v1",
            "http://127.1:8000/v1",
            "http://[::1]:80/v1",
            "http://user:pw@localhost:8000/v1",
            "https://user@api.example.org/v1",
            "https://api.example.org/v1?key=x",
            "https://api.example.org/v1#fragment",
            "https://api.example.org/v1/%2e%2e/admin",
            "http://my_host:8000/v1",
            "http://-bad.example.com/v1",
            "http://bad-.example.com/v1",
            "http://example..com/v1",
            "http://x.4a/v1",
            "http://localhost./v1",
            "https://api.example.org./v1",
        ] {
            assert_eq!(
                CustomOpenAiAdapter::new(invalid, "model").map(|_| ()),
                Err(ProviderError::AddressRefused),
                "{invalid}"
            );
        }
        for valid in [
            "https://api.example.org/openai/v1",
            "http://localhost:8000/v1",
            "http://127.0.0.1:11434/v1",
            "http://192.168.1.20:8000/v1",
            "http://10.0.0.5/v1",
            "http://[::1]:8080/v1",
            "http://ollama:11434/v1",
            "https://models.internal/v1",
            "http://169.254.169.254/v1",
            "http://xn--bcher-kva.example.com/v1",
            "http://3d-printer.local/v1",
            "http://a-b.c-d.example/v1",
        ] {
            assert!(CustomOpenAiAdapter::new(valid, "model").is_ok(), "{valid}");
        }
    }

    #[test]
    fn user_base_urls_are_normalized_before_the_strict_stored_route_boundary() {
        for (input, expected) in [
            ("https://API.example.org/v1", "https://api.example.org/v1"),
            (
                "https://api.example.org:443/v1",
                "https://api.example.org/v1",
            ),
            ("https://api.example.org/v1/", "https://api.example.org/v1"),
            (
                "https://API.example.org:8443/openai/v1/",
                "https://api.example.org:8443/openai/v1",
            ),
            ("http://LOCALHOST:8000/v1/", "http://localhost:8000/v1"),
            ("http://127.0.0.1:11434/v1/", "http://127.0.0.1:11434/v1"),
            ("http://10.0.0.5:80/v1", "http://10.0.0.5/v1"),
            ("http://[::1]:8080/v1/", "http://[::1]:8080/v1"),
            ("http://api.example.org:80/v1", "http://api.example.org/v1"),
            ("http://192.168.1.20:8000/v1", "http://192.168.1.20:8000/v1"),
        ] {
            let normalized = normalize_base_url_input(input).expect("safe input normalizes");
            assert_eq!(normalized, expected);
            assert!(CustomOpenAiAdapter::new(&normalized, "model").is_ok());
        }
        for invalid in [
            "ftp://api.example.org/v1",
            "127.0.0.1:8000/v1",
            "localhost:8000/v1",
            "api.example.org/v1",
            "https://user@api.example.org/v1/",
            "http://user:pw@localhost:8000/v1",
            "http://localhost:8000/v1?x=1",
            "https://api.example.org/v1/../admin",
            "https://api.example.org/v1/%2e%2e/admin",
            "http:///v1",
            "http:///localhost:8000/v1",
            "https://./v1",
            "http://my_host:8000/v1",
            "http://-bad.example.com/v1",
            "http://example..com/v1",
            "http://x.4a/v1",
            "http://localhost./v1",
            "http://127.1:11434/v1",
            "http://127.0.1/v1",
            "http://010.0.0.1/v1",
            "http://192.168.001.010:8000/v1",
            "http://0x7f.0.0.1/v1",
            "http://2130706433/v1",
        ] {
            assert!(normalize_base_url_input(invalid).is_err(), "{invalid}");
        }
        let long_label = format!("http://{}.example.com/v1", "a".repeat(64));
        assert!(normalize_base_url_input(&long_label).is_err());
    }

    #[test]
    fn dns_names_follow_the_stored_grammar() {
        for valid in [
            "localhost",
            "ollama",
            "models.internal",
            "api.example.com",
            "xn--bcher-kva.example.com",
            "a-b.c-d",
            "3d-printer.local",
        ] {
            assert!(valid_dns_name(valid), "{valid}");
        }
        let long_label = "a".repeat(64);
        let long_name = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(62)
        );
        assert_eq!(long_name.len(), 254);
        for invalid in [
            "",
            "my_host",
            "-a.com",
            "a-.com",
            "a..b",
            "a.",
            "x.4a",
            "x.1e3",
            "1.2.3.4",
            "A.com",
            long_label.as_str(),
            long_name.as_str(),
        ] {
            assert!(!valid_dns_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn upstream_model_id_is_bounded_and_nonblank() {
        for invalid in ["", " model", "model ", "model\n"] {
            assert_eq!(
                CustomOpenAiAdapter::new("https://api.example.org/v1", invalid).map(|_| ()),
                Err(ProviderError::InvalidResponse),
                "{invalid:?}"
            );
        }
        assert!(CustomOpenAiAdapter::new(
            "https://api.example.org/v1",
            &"x".repeat(MAX_UPSTREAM_MODEL_ID_CHARS + 1)
        )
        .is_err());
    }

    #[tokio::test]
    async fn discovery_rejects_keys_above_the_shared_credential_bound() {
        assert!(matches!(
            discover_models(
                "https://api.example.org/v1".to_owned(),
                "x".repeat(MAX_PROVIDER_KEY_BYTES + 1),
            )
            .await,
            Err(CustomModelDiscoveryError::InvalidInput)
        ));
    }

    #[test]
    fn every_resolved_address_is_allowed_and_repeats_are_dropped() {
        let loopback: SocketAddr = "127.0.0.1:8000".parse().expect("loopback address");
        let private: SocketAddr = "192.168.1.20:8000".parse().expect("private address");
        let link_local: SocketAddr = "169.254.169.254:8000".parse().expect("link-local address");
        let loopback_v6: SocketAddr = "[::1]:8000".parse().expect("loopback IPv6 address");

        assert_eq!(
            dedupe_addresses(vec![loopback, private, loopback, link_local, loopback_v6]),
            Ok(vec![loopback, private, link_local, loopback_v6])
        );
        assert_eq!(
            dedupe_addresses(Vec::new()),
            Err(ProviderError::EndpointUnavailable)
        );
    }

    #[tokio::test]
    async fn stored_url_is_checked_again_before_each_request() {
        let server = MockServer::start().await;
        let address = *server.address();
        let resolver = Arc::new(SequenceResolver::new([vec![address]]));
        let mut adapter = test_adapter(&server, resolver);
        adapter.base_url = "ftp://127.0.0.1/v1".to_owned();

        assert_eq!(
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .await,
            Err(ProviderError::AddressRefused)
        );
    }

    #[tokio::test]
    async fn pinned_loopback_transport_sends_exact_endpoint_model_and_sensitive_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer top-secret"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "model": "configured/upstream-model",
                "stream": true,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
                "text/event-stream",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let address = *server.address();
        let adapter = test_adapter(
            &server,
            Arc::new(SequenceResolver::new([vec![address, address]])),
        );
        let mut events = Vec::new();
        adapter
            .stream(
                &request("da-custom-model-uuid"),
                "top-secret",
                &CancellationToken::new(),
                &mut |event| events.push(event),
            )
            .await
            .expect("stream succeeds");
        assert!(events.contains(&ProviderStreamEvent::TextDelta("ok".to_owned())));
        assert!(events.contains(&ProviderStreamEvent::Completed));
    }

    #[tokio::test]
    async fn discovery_preserves_the_base_path_and_parses_only_documented_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer discovery-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {
                        "id": "organization/model-a",
                        "name": "Model A",
                        "context_length": 131072,
                        "top_provider": {"max_completion_tokens": 8192},
                        "default_parameters": {"temperature": 0.4, "top_p": 0.8},
                        "architecture": {"input_modalities": ["text", "image"]},
                        "supported_parameters": ["tools", "temperature", "top_p", "frequency_penalty"]
                    },
                    {"id": "organization/model-a", "context_length": 1},
                    {"id": "organization/model-b", "model_info": {"max_model_len": 65536}}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let address = *server.address();
        let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));

        let result = discover_models_with_adapter(&adapter, "discovery-key")
            .await
            .expect("discovery succeeds");

        assert_eq!(result.unavailable_reason, None);
        assert_eq!(result.models.len(), 2, "duplicate IDs are removed");
        let model = &result.models[0];
        assert_eq!(model.upstream_model_id, "organization/model-a");
        assert_eq!(model.display_name, "Model A");
        assert_eq!(model.context_length, Some(131_072));
        assert_eq!(model.max_output_tokens, Some(8_192));
        assert_eq!(model.supports_tools, Some(true));
        assert_eq!(model.supports_vision, Some(true));
        assert_eq!(model.sampling.temperature, Some(0.4));
        assert_eq!(model.sampling.top_p, Some(0.8));
        assert_eq!(
            model.supported_sampling_parameters,
            Some(vec!["temperature".to_owned(), "top_p".to_owned()])
        );
        assert_eq!(result.models[1].context_length, Some(65_536));
        assert_eq!(result.models[1].supports_tools, None);
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_timeout_includes_dns_resolution() {
        let server = MockServer::start().await;
        let adapter = test_adapter(&server, Arc::new(HangingResolver));

        let result = discover_models_with_adapter(&adapter, "key")
            .await
            .expect("timeout is a fallback result");

        assert_eq!(
            result.unavailable_reason,
            Some(CustomModelDiscoveryUnavailableReason::Unreachable)
        );
        assert!(server
            .received_requests()
            .await
            .expect("requests")
            .is_empty());
    }

    #[tokio::test]
    async fn discovery_rejects_a_chunked_body_over_the_limit() {
        let payload = format!(
            r#"{{"data":[{{"id":"bounded-model","padding":"{}"}}]}}"#,
            "x".repeat(MAX_DISCOVERY_RESPONSE_BYTES)
        )
        .into_bytes();
        let parsed = parse_discovered_models(&payload, "key", "https://gateway.example.org/v1")
            .expect("the complete oversized payload is valid discovery JSON");
        assert_eq!(parsed[0].upstream_model_id, "bounded-model");

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("connection");
            let mut request_method = [0_u8; 4];
            stream
                .read_exact(&mut request_method)
                .await
                .expect("request method");
            assert_eq!(&request_method, b"GET ");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("response headers");

            for chunk in payload.chunks(64 * 1_024) {
                let header = format!("{:X}\r\n", chunk.len());
                if stream.write_all(header.as_bytes()).await.is_err()
                    || stream.write_all(chunk).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        });
        let adapter = CustomOpenAiAdapter::with_test_resolver(
            &format!("http://gateway.example.org:{}/v1", address.port()),
            "configured/upstream-model",
            Arc::new(SequenceResolver::new([vec![address]])),
        )
        .expect("test adapter");

        let result = discover_models_with_adapter(&adapter, "key")
            .await
            .expect("oversized response is a fallback result");

        assert_eq!(
            result.unavailable_reason,
            Some(CustomModelDiscoveryUnavailableReason::InvalidResponse)
        );
        assert!(result.models.is_empty());
        server.await.expect("server task");
    }

    #[test]
    fn discovery_skips_invalid_capacity_sources_and_uses_only_returned_metadata() {
        let result = parse_discovered_models(
            serde_json::to_vec(&json!({
                "data": [{
                    "id": gateway::UPSTREAM_MODEL_ID,
                    "name": "Endpoint name",
                    "context_length": -1,
                    "max_model_len": 48_000,
                    "top_provider": {"max_completion_tokens": 6_000},
                    "supported_parameters": ["temperature"],
                    "architecture": {"input_modalities": ["text"]},
                    "default_parameters": {"temperature": 0.2}
                }]
            }))
            .expect("json")
            .as_slice(),
            "key",
            "https://llm-gateway.iseyon.workers.dev/v1",
        )
        .expect("models");

        let model = &result[0];
        assert_eq!(model.display_name, "Endpoint name");
        assert_eq!(model.context_length, Some(48_000));
        assert_eq!(model.max_output_tokens, Some(6_000));
        assert_eq!(model.supports_tools, Some(false));
        assert_eq!(model.supports_vision, Some(false));
        assert_eq!(model.sampling.temperature, Some(0.2));
        assert_eq!(model.sampling.top_p, None);
    }

    #[test]
    fn discovery_does_not_infer_gateway_model_settings() {
        let models = parse_discovered_models(
            serde_json::to_vec(&json!({"data": [{"id": gateway::UPSTREAM_MODEL_ID}]}))
                .expect("json")
                .as_slice(),
            "key",
            "https://llm-gateway.iseyon.workers.dev/v1",
        )
        .expect("models");
        let model = &models[0];
        assert_eq!(model.context_length, None);
        assert_eq!(model.max_output_tokens, None);
        assert_eq!(model.sampling, CustomModelSampling::default());
        assert_eq!(model.supports_tools, None);
        assert_eq!(model.supports_vision, None);
    }

    #[test]
    fn malformed_optional_capability_arrays_remain_unknown() {
        let models = parse_discovered_models(
            serde_json::to_vec(&json!({
                "data": [{
                    "id": "organization/model",
                    "supported_parameters": [null],
                    "architecture": {"input_modalities": [null]}
                }]
            }))
            .expect("json")
            .as_slice(),
            "key",
            "https://api.example.org/v1",
        )
        .expect("models");

        assert_eq!(models[0].supports_tools, None);
        assert_eq!(models[0].supports_vision, None);
        assert_eq!(models[0].supported_sampling_parameters, None);
    }

    #[tokio::test]
    async fn discovery_refuses_secret_reflection_in_model_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"id": "model-secret-canary"}]
            })))
            .mount(&server)
            .await;
        let address = *server.address();
        let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));

        let result = discover_models_with_adapter(&adapter, "secret-canary")
            .await
            .expect("invalid provider payload is a fallback result");

        assert!(result.models.is_empty());
        assert_eq!(
            result.unavailable_reason,
            Some(CustomModelDiscoveryUnavailableReason::InvalidResponse)
        );
    }

    #[tokio::test]
    async fn discovery_does_not_follow_redirects() {
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"id": "redirected-model"}]
            })))
            .mount(&destination)
            .await;
        let source = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/v1/models", destination.uri())),
            )
            .mount(&source)
            .await;
        let address = *source.address();
        let adapter = test_adapter(&source, Arc::new(SequenceResolver::new([vec![address]])));

        let result = discover_models_with_adapter(&adapter, "key")
            .await
            .expect("redirect becomes manual fallback");

        assert_eq!(
            result.unavailable_reason,
            Some(CustomModelDiscoveryUnavailableReason::Unreachable)
        );
        assert!(destination
            .received_requests()
            .await
            .expect("destination requests")
            .is_empty());
    }

    #[tokio::test]
    async fn discovery_classifies_unsupported_and_authentication_statuses() {
        for status in [404, 405, 501] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let address = *server.address();
            let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));
            let result = discover_models_with_adapter(&adapter, "key")
                .await
                .expect("unsupported is a fallback result");
            assert_eq!(
                result.unavailable_reason,
                Some(CustomModelDiscoveryUnavailableReason::Unsupported)
            );
        }
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let address = *server.address();
            let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));
            assert!(matches!(
                discover_models_with_adapter(&adapter, "key").await,
                Err(CustomModelDiscoveryError::AuthenticationFailed)
            ));
        }
    }

    #[tokio::test]
    async fn generic_requests_send_only_explicit_sampling_overrides() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let mut adapter = test_adapter(&server, resolver_for(&server, 1));
        adapter.sampling = CustomModelSampling {
            temperature: Some(0.2),
            top_p: Some(0.7),
        };

        run_stream(&adapter, &request("model"))
            .await
            .expect("stream");

        let body = &sent_bodies(&server).await[0];
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 0.7);
        assert!(body.get("stream_options").is_none());
    }

    #[tokio::test]
    async fn connection_test_uses_the_real_stream_consumer_and_accepts_a_complete_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer probe-secret"))
            .respond_with(ok_stream())
            .expect(1)
            .mount(&server)
            .await;
        let mut adapter = test_adapter(&server, resolver_for(&server, 1));
        adapter.sampling = CustomModelSampling {
            temperature: Some(0.3),
            top_p: Some(0.8),
        };

        verify_connection_with_adapter(&adapter, "probe-secret", 32, Duration::from_secs(1))
            .await
            .expect("complete nonempty answer verifies the connection");

        let body = &sent_bodies(&server).await[0];
        assert_eq!(body["model"], "configured/upstream-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 32);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["top_p"], 0.8);
        assert!(body.get("tools").is_none());
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn connection_test_reports_authentication_rejection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "bad-secret", 32, Duration::from_secs(1),)
                .await,
            Err(CustomModelConnectionError::AuthenticationFailed)
        );
    }

    #[tokio::test]
    async fn connection_test_rejects_an_invalid_success_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw("data: not-json\n\ndata: [DONE]\n\n", "text/event-stream"),
            )
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_secs(1),).await,
            Err(CustomModelConnectionError::ModelRejected)
        );
    }

    #[tokio::test]
    async fn connection_test_rejects_partial_text_from_a_truncated_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_secs(1),).await,
            Err(CustomModelConnectionError::ModelRejected)
        );
    }

    #[tokio::test]
    async fn connection_test_rejects_text_cut_off_by_the_output_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_secs(1),).await,
            Err(CustomModelConnectionError::ModelRejected)
        );
    }

    #[tokio::test]
    async fn connection_test_reports_provider_errors_without_the_response_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("secret provider diagnostics must not escape"),
            )
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_secs(1),).await,
            Err(CustomModelConnectionError::Unreachable)
        );
    }

    #[tokio::test]
    async fn connection_test_timeout_bounds_the_whole_http_operation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(500))
                    .set_body_raw(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"too late\"}}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_millis(50),)
                .await,
            Err(CustomModelConnectionError::TimedOut)
        );
    }

    #[tokio::test]
    async fn connection_test_timeout_includes_dns_resolution() {
        let server = MockServer::start().await;
        let adapter = test_adapter(&server, Arc::new(HangingResolver));

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_millis(50),)
                .await,
            Err(CustomModelConnectionError::TimedOut)
        );
        assert!(server
            .received_requests()
            .await
            .expect("requests")
            .is_empty());
    }

    #[tokio::test]
    async fn connection_test_timeout_includes_the_gateway_queue() {
        let server = MockServer::start().await;
        let mut adapter = test_adapter(&server, resolver_for(&server, 1));
        adapter.gateway = Some(GatewayTransport {
            queue: Arc::new(Semaphore::new(0)),
            retry_delays: vec![Duration::from_millis(500)],
            last_stream_cut: Arc::default(),
        });

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_millis(50),)
                .await,
            Err(CustomModelConnectionError::TimedOut)
        );
        assert!(server
            .received_requests()
            .await
            .expect("requests")
            .is_empty());
    }

    #[tokio::test]
    async fn connection_test_timeout_includes_gateway_retry_delays() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let mut adapter = test_adapter(&server, resolver_for(&server, 1));
        adapter.gateway = Some(GatewayTransport {
            queue: Arc::new(Semaphore::new(1)),
            retry_delays: vec![Duration::from_millis(500)],
            last_stream_cut: Arc::default(),
        });

        assert_eq!(
            verify_connection_with_adapter(&adapter, "secret", 32, Duration::from_millis(50),)
                .await,
            Err(CustomModelConnectionError::TimedOut)
        );
    }

    #[test]
    fn another_model_on_the_company_host_does_not_receive_the_gateway_profile() {
        let adapter = CustomOpenAiAdapter::new(
            "https://llm-gateway.iseyon.workers.dev/v1",
            "organization/another-model",
        )
        .expect("valid custom model");
        assert!(adapter.gateway.is_none());
    }

    #[tokio::test]
    async fn hostname_is_resolved_again_for_every_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let address = *server.address();
        let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));

        adapter
            .stream(
                &request("model"),
                "secret",
                &CancellationToken::new(),
                &mut |_| {},
            )
            .await
            .expect("first request resolves the host");
        assert_eq!(
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .await,
            Err(ProviderError::EndpointUnavailable)
        );
    }

    #[tokio::test]
    async fn a_loopback_ip_literal_endpoint_is_reached_through_the_production_constructor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ok_stream())
            .expect(1)
            .mount(&server)
            .await;
        let adapter = CustomOpenAiAdapter::new(&format!("http://{}/v1", server.address()), "model")
            .expect("loopback http endpoint is accepted");
        let mut events = Vec::new();

        adapter
            .stream(
                &request("model"),
                "secret",
                &CancellationToken::new(),
                &mut |event| events.push(event),
            )
            .await
            .expect("stream succeeds");

        assert!(events.contains(&ProviderStreamEvent::TextDelta("ok".to_owned())));
        assert!(events.contains(&ProviderStreamEvent::Completed));
    }

    #[tokio::test]
    async fn dns_failure_is_distinct_from_provider_unavailability() {
        let server = MockServer::start().await;
        let resolver = Arc::new(SequenceResolver::new(std::iter::empty()));
        let adapter = test_adapter(&server, resolver);

        assert_eq!(
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .await,
            Err(ProviderError::EndpointUnavailable)
        );
    }

    #[tokio::test]
    async fn connection_failure_is_distinct_from_provider_unavailability() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind reset server");
        let address = listener.local_addr().expect("reset server address");
        let reset = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept connection");
            drop(stream);
        });
        let adapter = CustomOpenAiAdapter::with_test_resolver(
            &format!("http://gateway.example.org:{}/v1", address.port()),
            "configured/upstream-model",
            Arc::new(SequenceResolver::new([vec![address]])),
        )
        .expect("test adapter");

        assert_eq!(
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .await,
            Err(ProviderError::EndpointUnavailable)
        );
        reset.await.expect("reset server task");
    }

    #[tokio::test]
    async fn custom_endpoint_authentication_failures_identify_the_bound_key() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            let address = *server.address();
            let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));

            assert_eq!(
                adapter
                    .stream(
                        &request("model"),
                        "wrong-secret",
                        &CancellationToken::new(),
                        &mut |_| {},
                    )
                    .await,
                Err(ProviderError::CustomAuthenticationFailed)
            );
        }
    }

    #[tokio::test]
    async fn redirect_to_an_unchecked_host_is_not_followed() {
        let destination = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&destination)
            .await;

        let source = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("location", format!("{}/stolen", destination.uri()).as_str()),
            )
            .expect(1)
            .mount(&source)
            .await;
        let address = *source.address();
        let adapter = test_adapter(&source, Arc::new(SequenceResolver::new([vec![address]])));
        assert_eq!(
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .await,
            Err(ProviderError::Unavailable)
        );
        assert!(destination
            .received_requests()
            .await
            .expect("request recording")
            .is_empty());
    }

    #[tokio::test]
    async fn transient_status_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let address = *server.address();
        let adapter = test_adapter(&server, Arc::new(SequenceResolver::new([vec![address]])));

        assert_eq!(
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .await,
            Err(ProviderError::Unavailable)
        );
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("request recording")
                .len(),
            1
        );
    }

    fn gateway_adapter(
        server: &MockServer,
        resolver: Arc<dyn AddressResolver>,
        queue: Arc<Semaphore>,
    ) -> CustomOpenAiAdapter {
        let mut adapter = test_adapter(server, resolver);
        adapter.upstream_model_id = gateway::UPSTREAM_MODEL_ID.to_owned();
        adapter.gateway = Some(GatewayTransport {
            queue,
            retry_delays: vec![Duration::from_millis(5); 3],
            last_stream_cut: Arc::default(),
        });
        adapter
    }

    fn resolver_for(server: &MockServer, answers: usize) -> Arc<dyn AddressResolver> {
        let address = *server.address();
        Arc::new(SequenceResolver::new(vec![vec![address]; answers]))
    }

    async fn run_stream(
        adapter: &CustomOpenAiAdapter,
        request: &ProviderChatRequest,
    ) -> Result<(), ProviderError> {
        adapter
            .stream(request, "secret", &CancellationToken::new(), &mut |_| {})
            .await
    }

    fn ok_stream() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        )
    }

    async fn sent_bodies(server: &MockServer) -> Vec<serde_json::Value> {
        server
            .received_requests()
            .await
            .expect("request recording")
            .iter()
            .map(|received| serde_json::from_slice(&received.body).expect("json body"))
            .collect()
    }

    #[tokio::test]
    async fn exact_gateway_requests_omit_unset_sampling_and_keep_thinking_on() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 1),
            Arc::new(Semaphore::new(1)),
        );

        run_stream(&adapter, &request("model"))
            .await
            .expect("stream");

        let body = &sent_bodies(&server).await[0];
        assert!(body.get("temperature").is_none(), "{body}");
        assert!(body.get("top_p").is_none(), "{body}");
        assert_eq!(body["stream_options"], json!({"include_usage": true}));
        assert!(body.get("reasoning_effort").is_none(), "{body}");
        for unwanted in ["top_k", "min_p"] {
            assert!(body.get(unwanted).is_none(), "{unwanted}");
        }
    }

    #[tokio::test]
    async fn exact_gateway_requests_send_explicit_sampling_values() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let mut adapter = gateway_adapter(
            &server,
            resolver_for(&server, 1),
            Arc::new(Semaphore::new(1)),
        );
        adapter.sampling = CustomModelSampling {
            temperature: Some(0.35),
            top_p: Some(0.8),
        };

        run_stream(&adapter, &request("model"))
            .await
            .expect("stream");

        let body = &sent_bodies(&server).await[0];
        assert_eq!(body["model"], gateway::UPSTREAM_MODEL_ID);
        assert_eq!(body["temperature"], 0.35);
        assert_eq!(body["top_p"], 0.8);
        assert_eq!(body["stream_options"], json!({"include_usage": true}));
    }

    #[tokio::test]
    async fn gateway_requests_that_do_not_want_reasoning_turn_it_off() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 1),
            Arc::new(Semaphore::new(1)),
        );
        let mut quiet = request("model");
        quiet.allow_reasoning = false;

        run_stream(&adapter, &quiet).await.expect("stream");

        assert_eq!(sent_bodies(&server).await[0]["reasoning_effort"], "none");
    }

    #[tokio::test]
    async fn gateway_tool_requests_keep_planning_separate_without_delaying_answers() {
        for has_system in [false, true] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Check the data.\"}}]}\n\n\
                     data: {\"choices\":[{\"delta\":{\"content\":\"Answer.\"}}]}\n\n\
                     data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ))
                .mount(&server)
                .await;
            let adapter = gateway_adapter(
                &server,
                resolver_for(&server, 1),
                Arc::new(Semaphore::new(1)),
            );
            let mut ask = request("model");
            ask.tools.push(crate::providers::ProviderToolSchema {
                name: "query".to_owned(),
                description: "Query the data".to_owned(),
                input_schema: json!({"type": "object"}),
            });
            if has_system {
                ask.messages.insert(
                    0,
                    ContextMessage::new(ContextRole::System, "Tenant instructions."),
                );
            }
            let expected_tools = build_request_body(&ask, &DIALECT)["tools"].clone();
            let mut events = Vec::new();
            adapter
                .stream(&ask, "secret", &CancellationToken::new(), &mut |event| {
                    events.push(event)
                })
                .await
                .expect("stream");
            let body = &sent_bodies(&server).await[0];
            let system = body["messages"][0]["content"]
                .as_str()
                .expect("system text");
            assert!(system.contains("Use the reasoning channel for thinking."));
            assert!(system.contains("without a textual prelude"));
            assert!(system.contains("the user's answer or a necessary clarification"));
            if has_system {
                assert!(system.starts_with("Tenant instructions.\n\n"));
            }
            assert_eq!(
                body["messages"][1],
                json!({"role": "user", "content": "hello"})
            );
            assert_eq!(body["tools"], expected_tools);
            assert_eq!(body["stream"], true);
            assert!(body.get("reasoning_effort").is_none());
            assert!(events.contains(&ProviderStreamEvent::ReasoningDelta(
                "Check the data.".to_owned()
            )));
            assert!(events.contains(&ProviderStreamEvent::TextDelta("Answer.".to_owned())));
            assert!(
                events
                    .iter()
                    .position(|event| matches!(event, ProviderStreamEvent::TextDelta(_)))
                    < events
                        .iter()
                        .position(|event| matches!(event, ProviderStreamEvent::Completed))
            );
        }
    }

    #[tokio::test]
    async fn other_private_endpoints_keep_their_request_unchanged() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let adapter = test_adapter(&server, resolver_for(&server, 1));
        let mut quiet = request("model");
        quiet.allow_reasoning = false;
        quiet.messages.insert(
            0,
            ContextMessage::new(ContextRole::System, "Tenant instructions."),
        );
        quiet.tools.push(crate::providers::ProviderToolSchema {
            name: "query".to_owned(),
            description: "Query the data".to_owned(),
            input_schema: json!({"type": "object"}),
        });
        let expected_messages = build_request_body(&quiet, &DIALECT)["messages"].clone();

        run_stream(&adapter, &quiet).await.expect("stream");

        let body = &sent_bodies(&server).await[0];
        assert_eq!(body["messages"], expected_messages);
        for absent in ["temperature", "top_p", "stream_options", "reasoning_effort"] {
            assert!(body.get(absent).is_none(), "{absent}");
        }
    }

    #[tokio::test]
    async fn gateway_retries_502_and_503_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 3),
            Arc::new(Semaphore::new(1)),
        );

        run_stream(&adapter, &request("model"))
            .await
            .expect("third attempt succeeds");

        assert_eq!(sent_bodies(&server).await.len(), 3);
    }

    #[tokio::test]
    async fn gateway_stops_after_three_retries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 4),
            Arc::new(Semaphore::new(1)),
        );

        assert_eq!(
            run_stream(&adapter, &request("model")).await,
            Err(ProviderError::Unavailable)
        );
        assert_eq!(sent_bodies(&server).await.len(), 4);
    }

    #[tokio::test]
    async fn gateway_never_retries_504_a_rejected_request_or_an_in_stream_error() {
        for (response, expected) in [
            (ResponseTemplate::new(504), ProviderError::Unavailable),
            (ResponseTemplate::new(400), ProviderError::InvalidResponse),
            (
                ResponseTemplate::new(200).set_body_raw(
                    "data: {\"error\":{\"code\":502,\"message\":\"upstream\"}}\n\n",
                    "text/event-stream",
                ),
                // The gateway answered and reported an upstream failure, so its
                // host is fine and the reader is not sent to check it.
                ProviderError::Unavailable,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(response)
                .mount(&server)
                .await;
            let adapter = gateway_adapter(
                &server,
                resolver_for(&server, 4),
                Arc::new(Semaphore::new(1)),
            );

            assert_eq!(run_stream(&adapter, &request("model")).await, Err(expected));
            assert_eq!(sent_bodies(&server).await.len(), 1);
        }
    }

    #[tokio::test]
    async fn a_gateway_that_closes_without_done_after_finishing_still_completes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 1),
            Arc::new(Semaphore::new(1)),
        );
        let mut events = Vec::new();

        adapter
            .stream(
                &request("model"),
                "secret",
                &CancellationToken::new(),
                &mut |event| events.push(event),
            )
            .await
            .expect("stream");

        assert!(events.contains(&ProviderStreamEvent::TextDelta("ok".to_owned())));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
    }

    #[tokio::test]
    async fn gateway_retries_a_refused_connection() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind reset server");
        let address = listener.local_addr().expect("reset server address");
        let accepted = tokio::spawn(async move {
            let mut count = 0;
            while let Ok(Ok((stream, _))) =
                tokio::time::timeout(Duration::from_millis(500), listener.accept()).await
            {
                drop(stream);
                count += 1;
            }
            count
        });
        let mut adapter = CustomOpenAiAdapter::with_test_resolver(
            &format!("http://gateway.example.org:{}/v1", address.port()),
            "configured/upstream-model",
            Arc::new(SequenceResolver::new(vec![vec![address]; 4])),
        )
        .expect("test adapter");
        adapter.gateway = Some(GatewayTransport {
            queue: Arc::new(Semaphore::new(1)),
            retry_delays: vec![Duration::from_millis(5); 3],
            last_stream_cut: Arc::default(),
        });

        assert_eq!(
            run_stream(&adapter, &request("model")).await,
            Err(ProviderError::EndpointUnavailable)
        );
        assert_eq!(accepted.await.expect("accept task"), 4);
    }

    #[test]
    fn the_gateway_thinks_about_a_new_message_but_not_about_tool_results() {
        let mut ask = request("model");
        let mut body = json!({});
        apply_gateway_parameters(&mut body, &ask, &CustomModelSampling::default());
        assert!(body.get("reasoning_effort").is_none());

        ask.messages.push(ContextMessage::tool_result(
            "call-1".to_owned(),
            "read_file".to_owned(),
            "contents".to_owned(),
        ));
        let mut body = json!({});
        apply_gateway_parameters(&mut body, &ask, &CustomModelSampling::default());
        assert_eq!(body["reasoning_effort"], "none");
    }

    fn reasoning_chunk(bytes: usize) -> String {
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"reasoning\":\"{}\"}}}}]}}\n\n",
            "t".repeat(bytes)
        )
    }

    #[tokio::test]
    async fn the_gateway_model_is_stopped_when_it_thinks_endlessly_and_never_answers() {
        let server = MockServer::start().await;
        let mut body = String::new();
        for _ in 0..80 {
            body.push_str(&reasoning_chunk(1_000));
        }
        body.push_str("data: {\"choices\":[{\"delta\":{\"content\":\"too late\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 1),
            Arc::new(Semaphore::new(1)),
        );
        let mut events = Vec::new();

        adapter
            .stream(
                &request("model"),
                "secret",
                &CancellationToken::new(),
                &mut |event| events.push(event),
            )
            .await
            .expect("a runaway turn ends cleanly");

        assert!(!events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::TextDelta(_))));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::OutputLimit,
                ..
            }
        )));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
    }

    #[tokio::test]
    async fn long_thinking_after_the_answer_began_is_left_alone() {
        let server = MockServer::start().await;
        let mut body =
            String::from("data: {\"choices\":[{\"delta\":{\"content\":\"Answer.\"}}]}\n\n");
        for _ in 0..80 {
            body.push_str(&reasoning_chunk(1_000));
        }
        body.push_str(
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 1),
            Arc::new(Semaphore::new(1)),
        );
        let mut events = Vec::new();

        adapter
            .stream(
                &request("model"),
                "secret",
                &CancellationToken::new(),
                &mut |event| events.push(event),
            )
            .await
            .expect("stream");

        assert!(events.contains(&ProviderStreamEvent::TextDelta("Answer.".to_owned())));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::Natural,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn after_a_cut_stream_the_gateway_is_asked_for_the_whole_answer() {
        use wiremock::matchers::body_partial_json;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"stream": true})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"reasoning\":\"thinking\"}}]}\n\n",
                "text/event-stream",
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"stream": false})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Paris.",
                        "reasoning": "thinking",
                        "tool_calls": [
                            {"id": "a", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a\"}"}},
                            {"id": "b", "type": "function", "function": {"name": "list_files", "arguments": "{}"}}
                        ]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 20, "completion_tokens": 5}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let adapter = gateway_adapter(
            &server,
            resolver_for(&server, 3),
            Arc::new(Semaphore::new(1)),
        );

        let mut first = Vec::new();
        adapter
            .stream(
                &request("model"),
                "secret",
                &CancellationToken::new(),
                &mut |event| first.push(event),
            )
            .await
            .expect("first request");
        assert!(first.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::StreamCut,
                ..
            }
        )));

        for _ in 0..2 {
            let mut whole = Vec::new();
            adapter
                .stream(
                    &request("model"),
                    "secret",
                    &CancellationToken::new(),
                    &mut |event| whole.push(event),
                )
                .await
                .expect("whole request");
            assert!(whole.contains(&ProviderStreamEvent::TextDelta("Paris.".to_owned())));
            let calls: Vec<_> = whole
                .iter()
                .filter_map(|event| match event {
                    ProviderStreamEvent::ToolCallRequested { call_id, name, .. } => {
                        Some((call_id.as_str(), name.as_str()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(calls, vec![("a", "read_file"), ("b", "list_files")]);
            assert!(whole.contains(&ProviderStreamEvent::PromptSize { tokens: 20 }));
            assert!(whole.iter().any(|event| matches!(
                event,
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    ..
                }
            )));
            assert_eq!(whole.last(), Some(&ProviderStreamEvent::Completed));
        }
    }

    #[tokio::test]
    async fn gateway_waits_for_the_engine_slot_before_sending() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok_stream())
            .mount(&server)
            .await;
        let queue = Arc::new(Semaphore::new(1));
        let busy = queue.clone().acquire_owned().await.expect("slot");
        let adapter = Arc::new(gateway_adapter(&server, resolver_for(&server, 1), queue));
        let waiting = tokio::spawn({
            let adapter = adapter.clone();
            async move { run_stream(&adapter, &request("model")).await }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            sent_bodies(&server).await.is_empty(),
            "sent while the slot was taken"
        );
        drop(busy);

        waiting
            .await
            .expect("join")
            .expect("stream after the slot frees");
        assert_eq!(sent_bodies(&server).await.len(), 1);
    }

    #[tokio::test]
    async fn a_cancelled_run_leaves_the_queue_without_sending() {
        let server = MockServer::start().await;
        let queue = Arc::new(Semaphore::new(1));
        let _busy = queue.clone().acquire_owned().await.expect("slot");
        let adapter = gateway_adapter(&server, resolver_for(&server, 1), queue);
        let cancel = CancellationToken::new();
        cancel.cancel();

        assert_eq!(
            adapter
                .stream(&request("model"), "secret", &cancel, &mut |_| {})
                .await,
            Err(ProviderError::Cancelled)
        );
        assert!(sent_bodies(&server).await.is_empty());
    }
}
