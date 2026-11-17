//! The ChatGPT plan route: one `POST` to the Responses API per model turn, sent
//! with the user's own access token.
//!
//! The body is built in [`super::request`] and the events are mapped in
//! [`super::sse`]. This file owns the transport, the status-to-error table and
//! how a stream is judged finished. It never refreshes a token or reads stored
//! credentials: the caller passes the current access token as `secret`.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Response, Url};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;
use zeroize::Zeroizing;

use super::request::build_request_body;
use super::sse::{map_error_code, ResponsesStreamMapper};
use crate::providers::http::{
    classify_status_code, classify_transport_error, is_context_overflow_text, sensitive_header,
    shared_streaming_client, stream_sse_lines_limited, DEFAULT_STREAM_BYTES,
};
use crate::providers::{ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent};

pub(crate) const RESPONSES_ENDPOINT: &str = "https://api.openai.com/v1/responses";

/// How much of a rejected response is read. The error code and any context-window
/// wording come first, so a rejection never needs more than this.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// The longest conversation id that is sent. A conversation id is a UUID today.
const MAX_CONVERSATION_ID_BYTES: usize = 64;

pub(crate) struct ChatgptPlanAdapter {
    /// `None` only if [`RESPONSES_ENDPOINT`] stops parsing. `stream` then fails
    /// the turn instead of panicking.
    endpoint: Option<Url>,
    /// Sent as the conversation headers. Set only through [`Self::with_conversation`].
    conversation: Option<String>,
}

impl ChatgptPlanAdapter {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            endpoint: Url::parse(RESPONSES_ENDPOINT).ok(),
            conversation: None,
        }
    }

    /// Sends `id` as the `session-id`, `thread-id` and `x-client-request-id` headers,
    /// which route the prompt cache. OpenAI's Codex client and opencode send the same
    /// three headers to this route, and opencode notes that ChatGPT derives its
    /// prompt-cache affinity from `session-id`. No body field is sent for this on
    /// purpose: nothing confirms that the route accepts `prompt_cache_key`. An id
    /// that is not 1 to 64 ASCII letters, digits, `-` or `_` is dropped and never sent.
    pub(crate) fn with_conversation(mut self, id: Option<&str>) -> Self {
        self.conversation = id
            .filter(|id| {
                (1..=MAX_CONVERSATION_ID_BYTES).contains(&id.len())
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
            .map(str::to_owned);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_endpoint(endpoint: Url) -> Self {
        Self {
            endpoint: Some(endpoint),
            conversation: None,
        }
    }
}

#[async_trait]
impl ProviderAdapter for ChatgptPlanAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::ChatgptPlan
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let Some(endpoint) = self.endpoint.clone() else {
            return Err(ProviderError::InvalidResponse);
        };
        let body = build_request_body(request).map_err(|_| ProviderError::InvalidResponse)?;
        let payload = serde_json::to_vec(&body).map_err(|_| ProviderError::InvalidResponse)?;
        let client = shared_streaming_client()?;
        let bearer = Zeroizing::new(format!("Bearer {secret}"));
        let builder = client
            .post(endpoint)
            .header(AUTHORIZATION, sensitive_header(bearer.as_str())?)
            .header(ACCEPT, "text/event-stream")
            .header(CONTENT_TYPE, "application/json");
        let builder = match &self.conversation {
            Some(id) => builder
                .header("session-id", id.as_str())
                .header("thread-id", id.as_str())
                .header("x-client-request-id", id.as_str()),
            None => builder,
        };
        let builder = builder.body(payload);

        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            sent = builder.send() => sent.map_err(|error| classify_transport_error(&error))?,
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            tracing::warn!(status, "chatgpt plan request rejected");
            let retry_after = retry_after_seconds(&response);
            let body = read_error_body(response, cancel).await?;
            return Err(classify_rejection(status, retry_after, &body));
        }
        stream_response(response, cancel, on_event).await
    }
}

/// Feeds one successful response through the mapper. Events reach `on_event` as
/// they are mapped. The turn's outcome comes from the mapper's own end-of-turn
/// rules, not from how the connection closed.
async fn stream_response(
    response: Response,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
) -> Result<(), ProviderError> {
    let mut mapper = ResponsesStreamMapper::new();
    let mut failure: Option<ProviderError> = None;
    let streamed = stream_sse_lines_limited(response, cancel, DEFAULT_STREAM_BYTES, |data| {
        if mapper.is_done() {
            return;
        }
        let mapped = mapper.push(data);
        for event in mapped.events {
            on_event(event);
        }
        failure = failure.or(mapped.error);
    })
    .await;

    // A recorded failure is the vendor's own answer, so it outranks the transport
    // state that followed it.
    if let Some(error) = failure {
        return Err(error);
    }
    if mapper.is_done() {
        return Ok(());
    }
    // Not finished, and the transport failed or was cancelled: that is the reason.
    streamed?;
    match mapper.finish_eof().error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Reads at most [`MAX_ERROR_BODY_BYTES`] of a rejected response. A body that
/// breaks part way still leaves its status to classify.
async fn read_error_body(
    response: Response,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut body = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            chunk = body.next() => {
                let Some(Ok(chunk)) = chunk else {
                    return Ok(bytes);
                };
                let room = MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if bytes.len() >= MAX_ERROR_BODY_BYTES {
                    return Ok(bytes);
                }
            }
        }
    }
}

/// The error a rejected request maps to. A code the vendor names decides the
/// error on its own. The status decides only when the body names none.
fn classify_rejection(status: u16, retry_after: Option<Duration>, body: &[u8]) -> ProviderError {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .and_then(Value::as_str);
    if let Some(error) = code
        .map(map_error_code)
        .filter(|error| *error != ProviderError::InvalidResponse)
    {
        // Only a throttle has a wait the header can add to.
        return match error {
            ProviderError::RateLimited { .. } => ProviderError::RateLimited { retry_after },
            other => other,
        };
    }
    match status {
        401 => ProviderError::PlanSignInRequired,
        // A 403 names ineligibility only through its code, and a code that names it
        // returned above. A bare 403 says nothing about the plan.
        403 => ProviderError::Unavailable,
        400 | 404 | 422
            if is_context_overflow_text(&String::from_utf8_lossy(body).to_ascii_lowercase()) =>
        {
            ProviderError::ContextOverflow
        }
        _ => classify_status_code(status, retry_after),
    }
}

/// The `Retry-After` delay, read only as delay-seconds. An HTTP-date is left
/// unparsed on purpose: the retry policy then uses its own backoff, the same as
/// when the header is absent.
fn retry_after_seconds(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::agent::context::{ContextMessage, ContextRole};
    use crate::providers::chatgpt_plan::request::FORBIDDEN_FIELDS;
    use crate::providers::ProviderStopReason;

    const TOKEN: &str = "chatgpt-test-access-token";
    const CONVERSATION: &str = "3f1c2b8e-6d4a-4f0e-9b7a-2c5d8e1f0a9b";
    const ROUTING_HEADERS: [&str; 3] = ["session-id", "thread-id", "x-client-request-id"];

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: "chatgpt_plan/gpt-6-luna".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: None,
            allow_reasoning: false,
            tools: Vec::new(),
            openrouter: None,
        }
    }

    fn adapter_at(server: &MockServer) -> ChatgptPlanAdapter {
        let endpoint = Url::parse(&format!("{}/v1/responses", server.uri())).expect("url");
        ChatgptPlanAdapter::with_endpoint(endpoint)
    }

    /// Runs one turn and returns its outcome with every event it emitted, in order.
    async fn run_turn(
        adapter: &ChatgptPlanAdapter,
        cancel: &CancellationToken,
    ) -> (Result<(), ProviderError>, Vec<ProviderStreamEvent>) {
        let mut events = Vec::new();
        let result = adapter
            .stream(
                &request(),
                TOKEN,
                cancel,
                &mut |event: ProviderStreamEvent| events.push(event),
            )
            .await;
        (result, events)
    }

    /// A body with one `data:` line per payload, the framing the stream uses.
    fn sse_body(payloads: &[Value]) -> String {
        payloads
            .iter()
            .map(|payload| format!("data: {payload}\n\n"))
            .collect()
    }

    fn text_delta(delta: &str) -> Value {
        json!({"type": "response.output_text.delta", "delta": delta})
    }

    fn completed(input_tokens: u32, output_tokens: u32) -> Value {
        json!({
            "type": "response.completed",
            "response": {"usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}},
        })
    }

    async fn serve_stream(server: &MockServer, payloads: &[Value]) {
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_body(payloads), "text/event-stream"),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn sends_the_token_and_a_stored_off_stream_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(header("accept", "text/event-stream"))
            .and(header("content-type", "application/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body(&[completed(1, 1)]), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (result, _) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        let requests = server.received_requests().await.expect("recorded");
        let sent = &requests[0];
        let body: Value = serde_json::from_slice(&sent.body).expect("json body");
        assert_eq!(body["store"], json!(false));
        assert_eq!(body["stream"], json!(true));
        for field in FORBIDDEN_FIELDS {
            assert!(body.get(field).is_none(), "`{field}` must not be sent");
        }
        assert!(!sent.url.as_str().contains(TOKEN));
    }

    #[tokio::test]
    async fn a_conversation_is_sent_as_the_three_routing_headers() {
        for id in [
            CONVERSATION.to_owned(),
            "a".repeat(MAX_CONVERSATION_ID_BYTES),
        ] {
            let server = MockServer::start().await;
            serve_stream(&server, &[completed(1, 1)]).await;
            let adapter = adapter_at(&server).with_conversation(Some(&id));

            let (result, _) = run_turn(&adapter, &CancellationToken::new()).await;

            assert_eq!(result, Ok(()));
            let requests = server.received_requests().await.expect("recorded");
            for name in ROUTING_HEADERS {
                assert_eq!(
                    requests[0]
                        .headers
                        .get(name)
                        .and_then(|value| value.to_str().ok()),
                    Some(id.as_str()),
                    "`{name}` carries the conversation"
                );
            }
        }
    }

    #[tokio::test]
    async fn no_routing_header_is_sent_without_a_conversation() {
        let server = MockServer::start().await;
        serve_stream(&server, &[completed(1, 1)]).await;

        let (result, _) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        let requests = server.received_requests().await.expect("recorded");
        for name in ROUTING_HEADERS {
            assert!(
                requests[0].headers.get(name).is_none(),
                "`{name}` must not be sent"
            );
        }
    }

    #[tokio::test]
    async fn an_invalid_conversation_id_is_never_sent() {
        let too_long = "a".repeat(MAX_CONVERSATION_ID_BYTES + 1);
        for id in [
            "",
            too_long.as_str(),
            "has space",
            "has/slash",
            "has\nnewline",
        ] {
            let server = MockServer::start().await;
            serve_stream(&server, &[completed(1, 1)]).await;
            let adapter = adapter_at(&server).with_conversation(Some(id));

            let (result, _) = run_turn(&adapter, &CancellationToken::new()).await;

            assert_eq!(result, Ok(()));
            let requests = server.received_requests().await.expect("recorded");
            for name in ROUTING_HEADERS {
                assert!(
                    requests[0].headers.get(name).is_none(),
                    "`{name}` must not be sent for {id:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_body_is_the_same_with_and_without_a_conversation() {
        let server = MockServer::start().await;
        serve_stream(&server, &[completed(1, 1)]).await;

        let (plain, _) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;
        let routed = adapter_at(&server).with_conversation(Some(CONVERSATION));
        let (with_conversation, _) = run_turn(&routed, &CancellationToken::new()).await;

        assert_eq!(plain, Ok(()));
        assert_eq!(with_conversation, Ok(()));
        let requests = server.received_requests().await.expect("recorded");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body, requests[1].body);
    }

    #[tokio::test]
    async fn a_text_answer_streams_its_deltas_then_usage_then_completes() {
        let server = MockServer::start().await;
        serve_stream(
            &server,
            &[text_delta("Hel"), text_delta("lo"), completed(12, 3)],
        )
        .await;

        let (result, events) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::TextDelta("Hel".to_owned()),
                ProviderStreamEvent::TextDelta("lo".to_owned()),
                ProviderStreamEvent::Usage {
                    input_tokens: Some(12),
                    output_tokens: Some(3),
                },
                ProviderStreamEvent::PromptSize { tokens: 12 },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[tokio::test]
    async fn a_tool_turn_requests_the_call_and_stops_for_tool_use() {
        let server = MockServer::start().await;
        serve_stream(
            &server,
            &[
                json!({
                    "type": "response.output_item.done",
                    "item": {
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "read_file",
                        "arguments": "{\"path\":\"a.txt\"}",
                    },
                }),
                completed(20, 5),
            ],
        )
        .await;

        let (result, events) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert_eq!(result, Ok(()));
        assert!(events.contains(&ProviderStreamEvent::ToolCallRequested {
            call_id: "call_1".to_owned(),
            name: "read_file".to_owned(),
            arguments: "{\"path\":\"a.txt\"}".to_owned(),
        }));
        assert!(events.contains(&ProviderStreamEvent::Stopped {
            reason: ProviderStopReason::ToolUse,
            discarded_tool_calls: 0,
        }));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
    }

    #[tokio::test]
    async fn a_stream_that_ends_before_completion_is_unavailable() {
        let server = MockServer::start().await;
        serve_stream(&server, &[text_delta("Hel")]).await;

        let (result, events) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert_eq!(result, Err(ProviderError::Unavailable));
        assert_eq!(
            events,
            vec![ProviderStreamEvent::TextDelta("Hel".to_owned())]
        );
    }

    #[tokio::test]
    async fn a_usage_limit_failing_mid_stream_keeps_the_events_already_sent() {
        let server = MockServer::start().await;
        serve_stream(
            &server,
            &[
                text_delta("Hel"),
                json!({
                    "type": "response.failed",
                    "response": {"error": {
                        "code": "subscription_sharing_usage_limit_exceeded",
                        "message": "limit reached",
                    }},
                }),
            ],
        )
        .await;

        let (result, events) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert_eq!(result, Err(ProviderError::PlanUsageLimit));
        assert_eq!(
            events,
            vec![ProviderStreamEvent::TextDelta("Hel".to_owned())]
        );
    }

    #[tokio::test]
    async fn a_rejected_request_maps_to_the_plan_error_it_names() {
        let cases: [(u16, &str, Option<&str>, ProviderError); 6] = [
            (
                429,
                r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#,
                None,
                ProviderError::PlanUsageLimit,
            ),
            (
                403,
                r#"{"error":{"code":"subscription_sharing_user_not_eligible"}}"#,
                None,
                ProviderError::PlanNotEligible,
            ),
            (
                401,
                r#"{"detail":"Unauthorized"}"#,
                None,
                ProviderError::PlanSignInRequired,
            ),
            (503, r#"{"detail":"x"}"#, None, ProviderError::Unavailable),
            (
                429,
                r#"{"detail":"slow down"}"#,
                Some("7"),
                ProviderError::RateLimited {
                    retry_after: Some(Duration::from_secs(7)),
                },
            ),
            (
                400,
                r#"{"error":{"message":"Your input exceeds the context window of this model."}}"#,
                None,
                ProviderError::ContextOverflow,
            ),
        ];
        for (status, body, retry_after, expected) in cases {
            let server = MockServer::start().await;
            let mut template = ResponseTemplate::new(status).set_body_string(body);
            if let Some(value) = retry_after {
                template = template.insert_header("retry-after", value);
            }
            Mock::given(method("POST"))
                .respond_with(template)
                .mount(&server)
                .await;

            let (result, events) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

            assert!(events.is_empty(), "{status}: a rejection emits nothing");
            let error = result.expect_err("a rejected request fails");
            assert_eq!(error, expected, "{status} {body}");
            assert!(
                !format!("{error:?}").contains(TOKEN),
                "{status}: the token must not reach an error"
            );
        }
    }

    #[tokio::test]
    async fn a_bare_forbidden_response_is_unavailable_not_ineligible() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_string(r#"{"detail":"Forbidden"}"#))
            .mount(&server)
            .await;

        let (result, events) = run_turn(&adapter_at(&server), &CancellationToken::new()).await;

        assert!(events.is_empty());
        assert_eq!(result, Err(ProviderError::Unavailable));
    }

    #[tokio::test]
    async fn cancelling_mid_stream_ends_the_turn_as_cancelled() {
        // Sends one event, then holds the connection open: the turn is still
        // running when the cancel lands, so only the cancel can end it.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let endpoint = Url::parse(&format!(
            "http://{}/v1/responses",
            listener.local_addr().expect("address")
        ))
        .expect("url");
        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let event = format!("data: {}\n\n", text_delta("Hel"));
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            socket
                .write_all(format!("{head}{event}").as_bytes())
                .await
                .expect("writes");
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        let adapter = ChatgptPlanAdapter::with_endpoint(endpoint);
        let cancel = CancellationToken::new();
        let mut events = Vec::new();
        let result = adapter
            .stream(
                &request(),
                TOKEN,
                &cancel,
                &mut |event: ProviderStreamEvent| {
                    if event == ProviderStreamEvent::TextDelta("Hel".to_owned()) {
                        cancel.cancel();
                    }
                    events.push(event);
                },
            )
            .await;

        assert_eq!(result, Err(ProviderError::Cancelled));
        assert_eq!(
            events,
            vec![ProviderStreamEvent::TextDelta("Hel".to_owned())]
        );
    }

    #[test]
    fn the_fixed_origin_is_the_responses_route() {
        let adapter = ChatgptPlanAdapter::new();

        assert_eq!(
            adapter.endpoint.as_ref().map(Url::as_str),
            Some(RESPONSES_ENDPOINT)
        );
        assert_eq!(adapter.provider_id(), ProviderId::ChatgptPlan);
    }
}
