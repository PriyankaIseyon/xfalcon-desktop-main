//! Google Gemini streaming adapter — mirrors xFalcon Web's reference implementation
//! (`xfalcon-web/src/lib/server/providers/google.ts`) so desktop and web normalize the
//! same wire events the same way.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;

use base64::Engine as _;

use crate::agent::context::{ContextAttachment, ContextMessage, ContextRole};

use super::adapter::{
    ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStopReason, ProviderStreamEvent,
};
use super::http::{
    classify_response, classify_transport_error, sensitive_header, shared_streaming_client,
    stream_sse_lines,
};

const GENERATIVE_LANGUAGE_HOST: &str = "https://generativelanguage.googleapis.com";

pub(crate) struct GoogleAdapter;

impl GoogleAdapter {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProviderAdapter for GoogleAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Google
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let client = shared_streaming_client()?;
        let url = format!(
            "{GENERATIVE_LANGUAGE_HOST}/v1beta/models/{}:streamGenerateContent?alt=sse",
            urlencode_model_segment(&request.model_id)
        );
        let body = build_request_body(request);
        let response = client
            .post(url)
            .header("x-goog-api-key", sensitive_header(secret)?)
            .json(&body)
            .send()
            .await
            .map_err(|error| classify_transport_error(&error))?;

        if !response.status().is_success() {
            return Err(classify_response(response).await);
        }

        consume_stream(response, cancel, on_event).await
    }
}

async fn consume_stream(
    response: reqwest::Response,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
) -> Result<(), ProviderError> {
    let mut state = ParseState::default();
    let stop = cancel.child_token();
    let stop_on_invalid = stop.clone();
    let outcome = stream_sse_lines(response, &stop, |data| {
        for event in parse_event(data, &mut state) {
            on_event(event);
        }
        if state.invalid {
            stop_on_invalid.cancel();
        }
    })
    .await;
    if state.invalid {
        return Err(ProviderError::InvalidResponse);
    }
    outcome
}

/// Model ids are compiled/tenant-catalog values, never raw user input — this is
/// defense in depth so a malformed id cannot smuggle path segments into the URL.
fn urlencode_model_segment(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .collect()
}

fn build_request_body(request: &ProviderChatRequest) -> Value {
    let system: String = request
        .messages
        .iter()
        .filter(|message| message.role == ContextRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    let contents: Vec<Value> = request
        .messages
        .iter()
        .filter(|message| message.role != ContextRole::System)
        .map(|message| {
            json!({
                "role": role_name(message.role),
                "parts": render_parts(message),
            })
        })
        .collect();

    let mut body = json!({
        "contents": contents,
        "generationConfig": { "maxOutputTokens": request
            .max_output_tokens
            .unwrap_or(super::adapter::REQUIRED_MAX_OUTPUT_TOKENS) },
    });
    if !system.trim().is_empty() {
        body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
    }
    if !request.tools.is_empty() {
        // Gemini nests declarations one level deeper than the other two
        // providers: a single tool entry holding every function.
        body["tools"] = json!([{
            "functionDeclarations": request
                .tools
                .iter()
                .map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                }))
                .collect::<Vec<_>>(),
        }]);
    }
    body
}

/// Renders one turn's `parts` array.
///
/// A tool result becomes a `functionResponse` keyed by tool **name** — Gemini's
/// wire format carries no call id, which is why
/// [`crate::agent::context::ContextToolRef`] preserves the name alongside it.
fn render_parts(message: &ContextMessage) -> Value {
    if let Some(responds_to) = message.responds_to.as_ref() {
        return json!([{
            "functionResponse": {
                "name": responds_to.name,
                "response": { "content": message.content },
            },
        }]);
    }

    if message.tool_calls.is_empty() && message.attachments.is_empty() {
        return json!([{ "text": message.content }]);
    }

    let mut parts: Vec<Value> =
        Vec::with_capacity(message.tool_calls.len() + message.attachments.len() + 1);
    if !message.content.is_empty() {
        parts.push(json!({ "text": message.content }));
    }
    for attachment in &message.attachments {
        match attachment {
            ContextAttachment::Image { media_type, bytes } => parts.push(json!({
                "inlineData": {
                    "mimeType": media_type.as_str(),
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            })),
        }
    }
    for call in &message.tool_calls {
        parts.push(json!({
            "functionCall": {
                "name": call.name,
                // Gemini takes args as a parsed object. An unparseable payload
                // degrades to `{}` so one malformed call cannot fail the whole
                // request.
                "args": serde_json::from_str::<Value>(&call.arguments)
                    .unwrap_or_else(|_| json!({})),
            },
        }));
    }
    json!(parts)
}

const fn role_name(role: ContextRole) -> &'static str {
    match role {
        ContextRole::User | ContextRole::Tool => "user",
        ContextRole::Assistant | ContextRole::System => "model",
    }
}

#[derive(Default)]
struct ParseState {
    invalid: bool,
    /// Gemini supplies no call id, but every layer above keys invocations by
    /// one. This counter makes the synthesized id unique within the stream,
    /// which is the scope over which a result must be matched back to its
    /// call — without reaching for a clock or RNG, keeping `parse_event`
    /// deterministic and directly testable.
    next_call_ordinal: usize,
}

fn parse_event(data: &str, state: &mut ParseState) -> Vec<ProviderStreamEvent> {
    if state.invalid {
        return Vec::new();
    }
    if data.trim().is_empty() {
        return Vec::new();
    }
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        state.invalid = true;
        return Vec::new();
    };

    let mut events = Vec::new();
    let candidate = match parsed.get("candidates").filter(|value| !value.is_null()) {
        Some(candidates) => match candidates.as_array() {
            Some(candidates) => candidates.first(),
            None => {
                state.invalid = true;
                return Vec::new();
            }
        },
        None => None,
    };
    if let Some(candidate) = candidate {
        if !candidate.is_object() {
            state.invalid = true;
            return Vec::new();
        }
        if let Some(parts) = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .filter(|value| !value.is_null())
        {
            let Some(parts) = parts.as_array() else {
                state.invalid = true;
                return Vec::new();
            };
            for part in parts {
                let Some(fields) = part.as_object() else {
                    state.invalid = true;
                    return Vec::new();
                };
                if let Some(text) = fields.get("text").filter(|value| !value.is_null()) {
                    let Some(text) = text.as_str() else {
                        state.invalid = true;
                        return Vec::new();
                    };
                    if !text.is_empty() {
                        events.push(ProviderStreamEvent::TextDelta(text.to_owned()));
                    }
                }
                // Unlike OpenAI/Anthropic, a `functionCall` part arrives whole
                // — there is nothing to accumulate.
                if let Some(call) = fields.get("functionCall").filter(|value| !value.is_null()) {
                    let Some(call) = call.as_object() else {
                        state.invalid = true;
                        return Vec::new();
                    };
                    let Some(name) = call
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())
                    else {
                        state.invalid = true;
                        return Vec::new();
                    };
                    let arguments = match call.get("args") {
                        Some(arguments) if arguments.is_object() => arguments.to_string(),
                        None | Some(Value::Null) => "{}".to_owned(),
                        Some(_) => {
                            state.invalid = true;
                            return Vec::new();
                        }
                    };
                    events.push(ProviderStreamEvent::ToolCallRequested {
                        call_id: format!("{name}-{}", state.next_call_ordinal),
                        name: name.to_owned(),
                        arguments,
                    });
                    state.next_call_ordinal += 1;
                }
            }
        }
        let finish_reason = candidate
            .get("finishReason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty());
        if let Some(reason) = finish_reason {
            // Gemini delivers each `functionCall` whole, so a limit never
            // leaves half a call behind: nothing to discard.
            events.push(ProviderStreamEvent::Stopped {
                reason: match reason {
                    "MAX_TOKENS" => ProviderStopReason::OutputLimit,
                    "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
                        ProviderStopReason::Refusal
                    }
                    "STOP" => ProviderStopReason::Natural,
                    _ => ProviderStopReason::Other,
                },
                discarded_tool_calls: 0,
            });
            events.push(ProviderStreamEvent::Completed);
        }
    }

    if let Some(usage) = parsed.get("usageMetadata") {
        let input_tokens = usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        let output_tokens = usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        if let Some(tokens) = input_tokens {
            events.push(ProviderStreamEvent::PromptSize {
                tokens: u64::from(tokens),
            });
        }
        if input_tokens.is_some() || output_tokens.is_some() {
            events.push(ProviderStreamEvent::Usage {
                input_tokens,
                output_tokens,
            });
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::super::adapter::ProviderToolSchema;
    use super::*;
    use crate::agent::context::{ContextMessage, ContextToolCall};

    #[test]
    fn a_user_turn_with_an_image_renders_an_inline_data_part() {
        let message = ContextMessage::user_with_attachments(
            "What does this chart show?",
            vec![crate::agent::context::ContextAttachment::Image {
                media_type: crate::agent::context::ImageMediaType::Png,
                bytes: vec![1, 2, 3],
            }],
        );
        let parts = render_parts(&message);
        assert_eq!(parts[0], json!({"text":"What does this chart show?"}));
        assert_eq!(
            parts[1],
            json!({"inlineData":{"mimeType":"image/png","data":"AQID"}})
        );
    }

    #[test]
    fn parses_text_delta() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"candidates":[{"content":{"parts":[{"text":"Hi"}],"role":"model"}}]}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![ProviderStreamEvent::TextDelta("Hi".to_owned())]
        );
    }

    #[test]
    fn finish_reason_emits_completed_and_usage() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"candidates":[{"content":{"parts":[{"text":"end"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":9}}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::TextDelta("end".to_owned()),
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0
                },
                ProviderStreamEvent::Completed,
                ProviderStreamEvent::PromptSize { tokens: 5 },
                ProviderStreamEvent::Usage {
                    input_tokens: Some(5),
                    output_tokens: Some(9)
                },
            ]
        );
    }

    #[test]
    fn max_tokens_is_reported_as_an_output_limit() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"candidates":[{"content":{"parts":[{"text":"cut"}]},"finishReason":"MAX_TOKENS"}]}"#,
            &mut state,
        );
        assert!(events.contains(&ProviderStreamEvent::Stopped {
            reason: ProviderStopReason::OutputLimit,
            discarded_tool_calls: 0
        }));
    }

    /// Gemini delivers a `functionCall` part whole — no delta accumulation —
    /// but supplies no call id at all. The adapter must synthesize one that is
    /// stable and unique within the stream, because every layer above keys
    /// invocations by id.
    #[test]
    fn function_call_is_emitted_whole_with_a_synthesized_id() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"project_search","args":{"query":"budget"}}}]}}]}"#,
            &mut state,
        );

        assert_eq!(
            events,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "project_search-0".to_owned(),
                name: "project_search".to_owned(),
                arguments: r#"{"query":"budget"}"#.to_owned(),
            }]
        );
    }

    #[test]
    fn synthesized_ids_stay_unique_across_repeated_calls_to_one_tool() {
        let mut state = ParseState::default();
        let chunk =
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"x","args":{}}}]}}]}"#;

        let first = parse_event(chunk, &mut state);
        let second = parse_event(chunk, &mut state);

        let id_of = |events: &[ProviderStreamEvent]| match &events[0] {
            ProviderStreamEvent::ToolCallRequested { call_id, .. } => call_id.clone(),
            other => panic!("expected a tool call, got {other:?}"),
        };
        assert_ne!(
            id_of(&first),
            id_of(&second),
            "two calls to the same tool must not collide on one synthesized id"
        );
    }

    #[test]
    fn function_call_with_absent_args_normalizes_to_an_empty_object() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"x"}}]}}]}"#,
            &mut state,
        );

        assert_eq!(
            events,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "x-0".to_owned(),
                name: "x".to_owned(),
                arguments: "{}".to_owned(),
            }]
        );
    }

    #[test]
    fn declares_tools_under_function_declarations() {
        let request = ProviderChatRequest {
            model_id: "gemini-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: vec![ProviderToolSchema {
                name: "project_search".to_owned(),
                description: "Search the active project".to_owned(),
                input_schema: json!({"type":"object","properties":{}}),
            }],
            openrouter: None,
        };

        let body = build_request_body(&request);

        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "project_search"
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["parameters"],
            json!({"type":"object","properties":{}})
        );
    }

    #[test]
    fn omits_the_tools_key_entirely_when_no_tool_is_available() {
        let request = ProviderChatRequest {
            model_id: "gemini-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        assert!(build_request_body(&request).get("tools").is_none());
    }

    #[test]
    fn renders_function_call_and_response_parts() {
        let request = ProviderChatRequest {
            model_id: "gemini-x".to_owned(),
            messages: vec![
                ContextMessage::new(ContextRole::User, "search"),
                ContextMessage::assistant_tool_calls(
                    "",
                    vec![ContextToolCall::new(
                        "project_search-0",
                        "project_search",
                        r#"{"query":"budget"}"#,
                    )],
                ),
                ContextMessage::tool_result("project_search-0", "project_search", "3 matches"),
            ],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request);
        let contents = body["contents"].as_array().expect("contents array");

        assert_eq!(contents[1]["role"], "model");
        assert_eq!(
            contents[1]["parts"][0]["functionCall"]["args"],
            json!({"query":"budget"})
        );
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(
            contents[2]["parts"][0]["functionResponse"]["name"], "project_search",
            "Gemini keys a functionResponse by tool name — the id is not part of its wire format"
        );
    }

    #[test]
    fn malformed_json_poisons_the_whole_turn() {
        let mut state = ParseState::default();
        assert!(parse_event("", &mut state).is_empty());
        assert!(!state.invalid);
        assert!(parse_event("not json", &mut state).is_empty());
        assert!(state.invalid);
        assert!(parse_event(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{}}}]},"finishReason":"STOP"}]}"#,
            &mut state
        )
        .is_empty());
    }

    #[test]
    fn malformed_recognized_part_poisons_the_whole_turn() {
        let mut state = ParseState::default();
        assert!(parse_event(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":[]}]}}]}"#,
            &mut state,
        )
        .is_empty());
        assert!(state.invalid);
    }

    async fn consume_test_stream(
        frames: &[Value],
    ) -> (Result<(), ProviderError>, Vec<ProviderStreamEvent>) {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = frames
            .iter()
            .map(|frame| format!("data: {frame}\n\n"))
            .collect::<String>();
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let response = reqwest::Client::new()
            .get(server.uri())
            .send()
            .await
            .expect("SSE response");
        let mut events = Vec::new();
        let outcome = consume_stream(response, &CancellationToken::new(), &mut |event| {
            events.push(event);
        })
        .await;
        (outcome, events)
    }

    #[tokio::test]
    async fn nullable_optional_fields_preserve_text_and_parallel_calls_over_http() {
        let (outcome, events) = consume_test_stream(&[
            json!({"candidates": null}),
            json!({"candidates": [{"content": null}]}),
            json!({"candidates": [{"content": {"parts": null}}]}),
            json!({"candidates": [{"content": {"parts": [
                {"text": "Working", "functionCall": null},
                {"text": null, "functionCall": {"name": "project_list_files", "args": null}},
                {"text": null, "functionCall": {"name": "project_read_file", "args": {"path": "a.csv"}}}
            ]}}]}),
            json!({"candidates": [{"content": null, "finishReason": "STOP"}]}),
        ])
        .await;

        assert_eq!(outcome, Ok(()));
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::TextDelta("Working".to_owned()),
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "project_list_files-0".to_owned(),
                    name: "project_list_files".to_owned(),
                    arguments: "{}".to_owned(),
                },
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "project_read_file-1".to_owned(),
                    name: "project_read_file".to_owned(),
                    arguments: r#"{"path":"a.csv"}"#.to_owned(),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[tokio::test]
    async fn malformed_non_null_fields_still_reject_the_stream_over_http() {
        let valid_part =
            json!({"functionCall": {"name": "project_read_file", "args": {"path": "a.csv"}}});
        let mut malformed_frames = vec![
            json!({"candidates": {}}),
            json!({"candidates": [{"content": {"parts": {}}}]}),
        ];
        for part in [
            json!({"text": []}),
            json!({"functionCall": []}),
            json!({"functionCall": {"name": "project_read_file", "args": []}}),
        ] {
            malformed_frames
                .push(json!({"candidates": [{"content": {"parts": [valid_part, part]}}]}));
        }
        for malformed in malformed_frames {
            let (outcome, events) = consume_test_stream(&[
                malformed.clone(),
                json!({"candidates": [{"content": {"parts": [valid_part]}, "finishReason": "STOP"}]}),
            ])
            .await;
            assert_eq!(outcome, Err(ProviderError::InvalidResponse), "{malformed}");
            assert!(
                events.is_empty(),
                "a malformed batch cannot yield partial calls: {events:?}"
            );
        }
    }

    /// The security property is "no path separator survives", not "no dot
    /// survives" — real model ids contain dots (`gemini-2.5-pro`), so
    /// stripping them would break the happy path. `..` with no separator is
    /// inert: it cannot climb a path segment on its own.
    ///
    /// This assertion previously demanded `"geminietc"`, which contradicted
    /// the dot-preserving case directly above it. It went unnoticed because
    /// the Windows test harness could not launch at all between 2026-07-29
    /// and this change (see `build.rs::embed_test_manifest`).
    #[test]
    fn urlencode_model_segment_strips_path_separators_but_keeps_version_dots() {
        assert_eq!(urlencode_model_segment("gemini-2.5-pro"), "gemini-2.5-pro");

        let sanitized = urlencode_model_segment("gemini/../etc");
        assert_eq!(sanitized, "gemini..etc");
        assert!(
            !sanitized.contains('/') && !sanitized.contains('\\'),
            "no separator may survive, or the id could climb out of the models path"
        );

        assert_eq!(urlencode_model_segment("a?b#c&d=e"), "abcde");
        assert_eq!(
            urlencode_model_segment("gemini:streamGenerateContent"),
            "geministreamGenerateContent",
            "the colon that delimits the method must not be forgeable from an id"
        );
    }
}
