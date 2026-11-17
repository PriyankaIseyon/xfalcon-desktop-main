//! Anthropic Messages API adapter — mirrors xFalcon Web's reference implementation
//! (`xfalcon-web/src/lib/server/providers/anthropic.ts`) so desktop and web normalize
//! the same wire events the same way.

use std::collections::BTreeMap;

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

const MESSAGES_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(crate) struct AnthropicAdapter;

impl AnthropicAdapter {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProviderAdapter for AnthropicAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Anthropic
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let client = shared_streaming_client()?;
        let body = build_request_body(request);
        let response = client
            .post(MESSAGES_ENDPOINT)
            .header("x-api-key", sensitive_header(secret)?)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|error| classify_transport_error(&error))?;

        if !response.status().is_success() {
            return Err(classify_response(response).await);
        }

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
}

fn build_request_body(request: &ProviderChatRequest) -> Value {
    let system: String = request
        .messages
        .iter()
        .filter(|message| message.role == ContextRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    let turns: Vec<&ContextMessage> = request
        .messages
        .iter()
        .filter(|message| message.role != ContextRole::System)
        .collect();

    // Anthropic requires strict user/assistant alternation; merge consecutive
    // same-role turns instead of sending an invalid message sequence. This is
    // not merely cosmetic for tool results: the API requires every
    // `tool_result` answering one assistant turn to arrive in a *single* user
    // message, so parallel tool calls depend on this merge for correctness.
    let mut messages: Vec<Value> = Vec::with_capacity(turns.len());
    for turn in turns {
        let role = role_name(turn.role);
        let content = render_content(turn);
        if let Some(last) = messages.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some(role) {
                merge_content(&mut last["content"], content);
                continue;
            }
        }
        messages.push(json!({ "role": role, "content": content }));
    }

    let mut body = json!({
        "model": request.model_id,
        "messages": messages,
        // Required by the Messages API — it has no "let the model decide"
        // form, so this is the one wire that must name a number.
        "max_tokens": request
            .max_output_tokens
            .unwrap_or(super::adapter::REQUIRED_MAX_OUTPUT_TOKENS),
        "stream": true,
    });
    if !system.trim().is_empty() {
        body["system"] = Value::String(system);
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.input_schema,
                    })
                })
                .collect(),
        );
    }
    body
}

/// Renders one turn's content: a bare string for plain text (the shape this
/// adapter has always sent, and what the merge path below concatenates), or a
/// content-block array once tool data is involved.
fn render_content(message: &ContextMessage) -> Value {
    if let Some(responds_to) = message.responds_to.as_ref() {
        return json!([{
            "type": "tool_result",
            "tool_use_id": responds_to.call_id,
            "content": message.content,
        }]);
    }

    if message.tool_calls.is_empty() && message.attachments.is_empty() {
        return Value::String(message.content.clone());
    }

    let mut blocks: Vec<Value> =
        Vec::with_capacity(message.tool_calls.len() + message.attachments.len() + 1);
    // An empty `text` block is rejected by the API, and a pure tool-call turn
    // legitimately has none — omit rather than emit blank.
    if !message.content.is_empty() {
        blocks.push(json!({ "type": "text", "text": message.content }));
    }
    for attachment in &message.attachments {
        match attachment {
            ContextAttachment::Image { media_type, bytes } => blocks.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type.as_str(),
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            })),
        }
    }
    for call in &message.tool_calls {
        blocks.push(json!({
            "type": "tool_use",
            "id": call.call_id,
            "name": call.name,
            // Unlike OpenAI, Anthropic takes tool input as a parsed object.
            // Arguments reaching here are provider-produced JSON; an
            // unparseable payload degrades to `{}` rather than failing the
            // whole request, so one malformed call cannot strand the run.
            "input": serde_json::from_str::<Value>(&call.arguments)
                .unwrap_or_else(|_| json!({})),
        }));
    }
    json!(blocks)
}

/// Folds `incoming` into an existing same-role turn's content, preserving the
/// long-standing string-concat behaviour when both sides are plain text and
/// promoting to a block array as soon as either side carries blocks.
fn merge_content(existing: &mut Value, incoming: Value) {
    match (existing.as_str(), incoming) {
        (Some(left), Value::String(right)) => {
            *existing = Value::String(format!("{left}\n\n{right}"));
        }
        (Some(left), Value::Array(mut right)) => {
            let mut blocks = vec![json!({ "type": "text", "text": left })];
            blocks.append(&mut right);
            *existing = Value::Array(blocks);
        }
        (None, Value::String(right)) => {
            if let Some(blocks) = existing.as_array_mut() {
                blocks.push(json!({ "type": "text", "text": right }));
            }
        }
        (None, Value::Array(mut right)) => {
            if let Some(blocks) = existing.as_array_mut() {
                blocks.append(&mut right);
            }
        }
        (_, other) => *existing = other,
    }
}

const fn role_name(role: ContextRole) -> &'static str {
    match role {
        ContextRole::User | ContextRole::Tool => "user",
        ContextRole::Assistant | ContextRole::System => "assistant",
    }
}

/// One `tool_use` content block being assembled. Anthropic opens the block
/// with its id and name, streams the arguments as `input_json_delta`
/// fragments, and closes it with `content_block_stop` — only then is the call
/// complete.
#[derive(Default)]
struct PartialToolCall {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct ParseState {
    done: bool,
    invalid: bool,
    prompt_tokens: Option<u32>,
    /// Keyed by content-block `index`. A message interleaves text and
    /// `tool_use` blocks, so the index is the only way to attribute an
    /// `input_json_delta` to the right call.
    pending_tool_calls: BTreeMap<u64, PartialToolCall>,
    stop_reason: Option<String>,
    /// Tool calls that closed, or were still open at `message_stop`, without
    /// complete arguments. Never emitted: half a call is not a call.
    discarded_tool_calls: u32,
}

fn stop_reason_policy(reason: Option<&str>) -> ProviderStopReason {
    match reason {
        Some("tool_use") => ProviderStopReason::ToolUse,
        Some("max_tokens") => ProviderStopReason::OutputLimit,
        Some("model_context_window_exceeded") => ProviderStopReason::ContextLimit,
        Some("refusal") => ProviderStopReason::Refusal,
        Some("end_turn" | "stop_sequence") | None => ProviderStopReason::Natural,
        Some(_) => ProviderStopReason::Other,
    }
}

fn parse_event(data: &str, state: &mut ParseState) -> Vec<ProviderStreamEvent> {
    if state.done || state.invalid {
        return Vec::new();
    }
    if data.trim().is_empty() {
        return Vec::new();
    }
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        state.invalid = true;
        return Vec::new();
    };
    let Some(event_type) = parsed.get("type").and_then(Value::as_str) else {
        state.invalid = true;
        return Vec::new();
    };

    match event_type {
        "message_start" => {
            state.prompt_tokens = parsed
                .get("message")
                .and_then(|message| message.get("usage"))
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(Value::as_u64)
                .map(|value| value as u32);
            Vec::new()
        }
        "content_block_start" => {
            let Some(block) = parsed.get("content_block") else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(block) = block.as_object() else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(block_type) = block.get("type").and_then(Value::as_str) else {
                state.invalid = true;
                return Vec::new();
            };
            if block_type != "tool_use" {
                return Vec::new();
            }
            // Record and stay silent: the arguments have not streamed yet, so
            // emitting here (as this adapter did before Task 9a) would hand the
            // tool loop a call with no input at all.
            let Some(index) = block_index(&parsed) else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(call_id) = block
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(name) = block
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
            else {
                state.invalid = true;
                return Vec::new();
            };
            state.pending_tool_calls.insert(
                index,
                PartialToolCall {
                    call_id: call_id.to_owned(),
                    name: name.to_owned(),
                    arguments: String::new(),
                },
            );
            Vec::new()
        }
        "content_block_delta" => {
            let Some(delta) = parsed.get("delta") else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(delta) = delta.as_object() else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(index) = block_index(&parsed) else {
                state.invalid = true;
                return Vec::new();
            };
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => match delta.get("text").and_then(Value::as_str) {
                    Some("") => Vec::new(),
                    Some(text) => vec![ProviderStreamEvent::TextDelta(text.to_owned())],
                    None => {
                        state.invalid = true;
                        Vec::new()
                    }
                },
                Some("thinking_delta") => match delta.get("thinking").and_then(Value::as_str) {
                    Some("") => Vec::new(),
                    Some(text) => vec![ProviderStreamEvent::ReasoningDelta(text.to_owned())],
                    None => {
                        state.invalid = true;
                        Vec::new()
                    }
                },
                Some("input_json_delta") => {
                    let Some(pending) = state.pending_tool_calls.get_mut(&index) else {
                        state.invalid = true;
                        return Vec::new();
                    };
                    let Some(fragment) = delta.get("partial_json").and_then(Value::as_str) else {
                        state.invalid = true;
                        return Vec::new();
                    };
                    pending.arguments.push_str(fragment);
                    if !fragment.is_empty() {
                        return vec![ProviderStreamEvent::Progress];
                    }
                    Vec::new()
                }
                Some(_) => Vec::new(),
                None => {
                    state.invalid = true;
                    Vec::new()
                }
            }
        }
        "content_block_stop" => {
            let Some(index) = block_index(&parsed) else {
                state.invalid = true;
                return Vec::new();
            };
            let Some(call) = state
                .pending_tool_calls
                .remove(&index)
                .filter(|call| !call.call_id.is_empty())
            else {
                return Vec::new();
            };
            // A tool taking no arguments streams nothing at all; normalizing
            // keeps every consumer able to parse unconditionally.
            let arguments = if call.arguments.trim().is_empty() {
                "{}".to_owned()
            } else {
                call.arguments
            };
            if !serde_json::from_str::<Value>(&arguments).is_ok_and(|input| input.is_object()) {
                state.discarded_tool_calls = state.discarded_tool_calls.saturating_add(1);
                return Vec::new();
            }
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: call.call_id,
                name: call.name,
                arguments,
            }]
        }
        "message_delta" => {
            if let Some(reason) = parsed
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str)
            {
                state.stop_reason = Some(reason.to_owned());
            }
            let output_tokens = parsed
                .get("usage")
                .and_then(|usage| usage.get("output_tokens"))
                .and_then(Value::as_u64)
                .map(|value| value as u32);
            if output_tokens.is_some() {
                vec![ProviderStreamEvent::Usage {
                    input_tokens: state.prompt_tokens,
                    output_tokens,
                }]
            } else {
                Vec::new()
            }
        }
        "message_stop" => {
            state.done = true;
            let still_open = u32::try_from(state.pending_tool_calls.len()).unwrap_or(u32::MAX);
            state.pending_tool_calls.clear();
            vec![
                ProviderStreamEvent::Stopped {
                    reason: stop_reason_policy(state.stop_reason.as_deref()),
                    discarded_tool_calls: state.discarded_tool_calls.saturating_add(still_open),
                },
                ProviderStreamEvent::Completed,
            ]
        }
        _ => Vec::new(),
    }
}

/// Content-block index for a `content_block_*` event. Anthropic always sends
/// it; absence poisons the turn rather than attributing a fragment to a
/// different parallel call.
fn block_index(parsed: &Value) -> Option<u64> {
    parsed.get("index").and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::super::adapter::ProviderToolSchema;
    use super::*;
    use crate::agent::context::{ContextMessage, ContextToolCall};

    #[test]
    fn a_user_turn_with_an_image_renders_a_base64_image_block() {
        let message = ContextMessage::user_with_attachments(
            "What does this chart show?",
            vec![crate::agent::context::ContextAttachment::Image {
                media_type: crate::agent::context::ImageMediaType::Png,
                bytes: vec![1, 2, 3],
            }],
        );
        let content = render_content(&message);
        assert_eq!(
            content[0],
            json!({"type":"text","text":"What does this chart show?"})
        );
        assert_eq!(
            content[1],
            json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"AQID"}})
        );
    }

    #[test]
    fn parses_message_start_usage_without_emitting() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":42}}}"#,
            &mut state,
        );
        assert!(events.is_empty());
        assert_eq!(state.prompt_tokens, Some(42));
    }

    #[test]
    fn parses_text_delta() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![ProviderStreamEvent::TextDelta("Hi".to_owned())]
        );
    }

    #[test]
    fn parses_thinking_delta() {
        let mut state = ParseState::default();
        let events = parse_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"..."}}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![ProviderStreamEvent::ReasoningDelta("...".to_owned())]
        );
    }

    /// Anthropic opens a `tool_use` block with its id/name, streams the
    /// arguments as `input_json_delta` fragments, and closes it with
    /// `content_block_stop`. Emitting at `content_block_start` — as this
    /// adapter did before Task 9a — would hand the loop a call with no
    /// arguments at all.
    #[test]
    fn accumulates_tool_use_input_until_the_block_stops() {
        let mut state = ParseState::default();

        assert!(parse_event(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"project_search"}}"#,
            &mut state,
        )
        .is_empty());
        assert_eq!(
            parse_event(
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}"#,
                &mut state,
            ),
            vec![ProviderStreamEvent::Progress]
        );
        assert_eq!(
            parse_event(
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"budget\"}"}}"#,
                &mut state,
            ),
            vec![ProviderStreamEvent::Progress]
        );

        let events = parse_event(r#"{"type":"content_block_stop","index":1}"#, &mut state);

        assert_eq!(
            events,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "toolu_1".to_owned(),
                name: "project_search".to_owned(),
                arguments: r#"{"query":"budget"}"#.to_owned(),
            }]
        );
    }

    #[test]
    fn text_block_stop_emits_nothing() {
        let mut state = ParseState::default();
        parse_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            &mut state,
        );

        assert!(parse_event(r#"{"type":"content_block_stop","index":0}"#, &mut state).is_empty());
    }

    #[test]
    fn tool_use_with_no_streamed_input_normalizes_to_an_empty_object() {
        let mut state = ParseState::default();
        parse_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"x"}}"#,
            &mut state,
        );

        let events = parse_event(r#"{"type":"content_block_stop","index":0}"#, &mut state);

        assert_eq!(
            events,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "toolu_1".to_owned(),
                name: "x".to_owned(),
                arguments: "{}".to_owned(),
            }]
        );
    }

    #[test]
    fn declares_tools_in_anthropic_input_schema_shape() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: vec![ProviderToolSchema {
                name: "project_search".to_owned(),
                description: "Search the active project".to_owned(),
                input_schema: json!({"type":"object","properties":{}}),
            }],
            openrouter: None,
        };

        let body = build_request_body(&request);

        assert_eq!(body["tools"][0]["name"], "project_search");
        assert_eq!(
            body["tools"][0]["input_schema"],
            json!({"type":"object","properties":{}}),
            "Anthropic names the field input_schema, not parameters"
        );
    }

    /// Anthropic streams a `thinking_delta` only when the request carries a
    /// `thinking` block, and this build sends none — so the reasoning panel is
    /// absent on Anthropic direct. That is a scoping decision, not an
    /// oversight: enabling it obliges the caller to echo every thinking block
    /// back verbatim on each later turn of a tool loop, which is state this
    /// build's context does not carry. Same refusal, same reason, as
    /// `openrouter.rs::never_requests_reasoning_it_cannot_echo_back`.
    /// The Messages API has no "let the model decide" form for `max_tokens`,
    /// so this wire always names a number even when the caller asked for none.
    #[test]
    fn a_required_ceiling_is_supplied_when_the_caller_asks_for_none() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: None,
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        assert_eq!(
            build_request_body(&request)["max_tokens"],
            json!(super::super::adapter::REQUIRED_MAX_OUTPUT_TOKENS)
        );
    }

    #[test]
    fn never_requests_thinking_it_cannot_echo_back() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        assert!(build_request_body(&request).get("thinking").is_none());
    }

    #[test]
    fn omits_the_tools_key_entirely_when_no_tool_is_available() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        assert!(build_request_body(&request).get("tools").is_none());
    }

    #[test]
    fn renders_tool_use_and_tool_result_as_content_blocks() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![
                ContextMessage::new(ContextRole::User, "search"),
                ContextMessage::assistant_tool_calls(
                    "let me look",
                    vec![ContextToolCall::new(
                        "toolu_1",
                        "project_search",
                        r#"{"query":"budget"}"#,
                    )],
                ),
                ContextMessage::tool_result("toolu_1", "project_search", "3 matches"),
            ],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request);
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["content"][1]["id"], "toolu_1");
        assert_eq!(
            messages[1]["content"][1]["input"],
            json!({"query":"budget"}),
            "Anthropic takes tool input as a parsed object, unlike OpenAI's string"
        );
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_1");
    }

    /// Anthropic requires every `tool_result` answering one assistant turn to
    /// sit in a single user message. Two parallel calls therefore merge into
    /// one message with two blocks — the same merge rule that already joins
    /// consecutive text turns, applied to block content.
    #[test]
    fn merges_parallel_tool_results_into_one_user_turn() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![
                ContextMessage::assistant_tool_calls(
                    "",
                    vec![
                        ContextToolCall::new("toolu_1", "a", "{}"),
                        ContextToolCall::new("toolu_2", "b", "{}"),
                    ],
                ),
                ContextMessage::tool_result("toolu_1", "a", "first"),
                ContextMessage::tool_result("toolu_2", "b", "second"),
            ],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request);
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["content"].as_array().map(Vec::len), Some(2));
        assert_eq!(messages[1]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(messages[1]["content"][1]["tool_use_id"], "toolu_2");
    }

    /// An assistant turn that is pure tool call carries no text, and Anthropic
    /// rejects an empty `text` block — it must be omitted, not emitted blank.
    #[test]
    fn omits_the_text_block_when_the_assistant_turn_is_pure_tool_call() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![ContextMessage::assistant_tool_calls(
                "",
                vec![ContextToolCall::new("toolu_1", "a", "{}")],
            )],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request);

        assert_eq!(
            body["messages"][0]["content"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(body["messages"][0]["content"][0]["type"], "tool_use");
    }

    #[test]
    fn message_delta_emits_combined_usage() {
        let mut state = ParseState {
            done: false,
            prompt_tokens: Some(10),
            ..ParseState::default()
        };
        let events = parse_event(
            r#"{"type":"message_delta","usage":{"output_tokens":7}}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![ProviderStreamEvent::Usage {
                input_tokens: Some(10),
                output_tokens: Some(7)
            }]
        );
    }

    #[test]
    fn message_stop_completes_and_is_terminal() {
        let mut state = ParseState::default();
        assert_eq!(
            parse_event(r#"{"type":"message_stop"}"#, &mut state),
            vec![
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0
                },
                ProviderStreamEvent::Completed
            ]
        );
        assert!(state.done);
        assert!(parse_event(r#"{"type":"message_start"}"#, &mut state).is_empty());
    }

    #[test]
    fn malformed_parallel_call_frame_poisons_the_whole_turn() {
        let mut state = ParseState::default();
        assert!(parse_event("", &mut state).is_empty());
        assert!(!state.invalid);
        let first = parse_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"read-a","name":"project_read_file"}}"#,
            &mut state,
        );
        assert!(first.is_empty());
        assert!(parse_event("{broken", &mut state).is_empty());
        assert!(state.invalid);
        assert!(parse_event(r#"{"type":"message_stop"}"#, &mut state).is_empty());
    }

    #[test]
    fn malformed_recognized_delta_poisons_the_whole_turn() {
        let mut state = ParseState::default();
        assert!(parse_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":[]}}"#,
            &mut state,
        )
        .is_empty());
        assert!(state.invalid);
    }

    #[test]
    fn ping_and_unknown_future_events_remain_compatible() {
        let mut state = ParseState::default();
        assert!(parse_event(r#"{"type":"ping"}"#, &mut state).is_empty());
        assert!(parse_event(
            r#"{"type":"future_event","payload":{"version":2}}"#,
            &mut state,
        )
        .is_empty());
        assert!(!state.invalid);
    }

    #[test]
    fn an_output_limit_drops_the_unfinished_call_and_reports_the_stop() {
        let mut state = ParseState::default();
        parse_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"write"}}"#,
            &mut state,
        );
        parse_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.html\",\"content\":\"<html"}}"#,
            &mut state,
        );
        assert!(parse_event(r#"{"type":"content_block_stop","index":0}"#, &mut state).is_empty());
        parse_event(
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
            &mut state,
        );
        assert_eq!(
            parse_event(r#"{"type":"message_stop"}"#, &mut state),
            vec![
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::OutputLimit,
                    discarded_tool_calls: 1
                },
                ProviderStreamEvent::Completed
            ]
        );
    }

    #[test]
    fn merges_consecutive_same_role_turns() {
        let request = ProviderChatRequest {
            model_id: "claude-x".to_owned(),
            messages: vec![
                ContextMessage::new(ContextRole::User, "first"),
                ContextMessage::new(ContextRole::User, "second"),
                ContextMessage::new(ContextRole::Assistant, "reply"),
            ],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };
        let body = build_request_body(&request);
        let messages = body
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], json!("first\n\nsecond"));
        assert_eq!(messages[1]["content"], json!("reply"));
    }
}
