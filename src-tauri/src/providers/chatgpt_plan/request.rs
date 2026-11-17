//! The body of `POST https://api.openai.com/v1/responses` on the ChatGPT plan
//! route.
//!
//! The route is narrower than the public Responses API. It refuses the fields in
//! `FORBIDDEN_FIELDS`, takes system text only as a developer item at `input[0]`
//! because it rejects `role: "system"`, and accepts function tools alone. The
//! shape follows what OpenAI's own Codex client sends to the same route.

use base64::Engine as _;
use serde_json::{json, Value};

use super::models::DOCUMENTED_MODELS;
use crate::agent::context::{ContextAttachment, ContextMessage, ContextRole};
use crate::providers::{ProviderChatRequest, ProviderToolSchema};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum RequestError {
    #[error("an image attachment could not be encoded")]
    Attachment,
    #[error("the request has no usable messages")]
    Empty,
    #[error("the model is not one the desktop offers")]
    UnknownModel,
}

/// Top-level keys and item fields the route refuses. The builder never writes
/// them, and the tests assert that none of them reaches a body.
#[cfg(test)]
pub(crate) const FORBIDDEN_FIELDS: [&str; 17] = [
    "background",
    "conversation",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "moderation",
    "multi_agent",
    "prompt",
    "prompt_cache_retention",
    "safety_identifier",
    "temperature",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "previous_response_id",
    "instructions",
];

const PLAN_MODEL_PREFIX: &str = "chatgpt_plan/";

/// Appended to a tool result that was cut short, so the model can tell the output
/// it reads is not the whole of it.
const TRUNCATED_MARKER: &str = "\n[truncated]";

/// Ignores `max_output_tokens`, which this route refuses, and `openrouter`, which
/// is OpenRouter's own state and means nothing here. A model without documented
/// limits is refused before anything is built, so nothing is sent for it.
pub(crate) fn build_request_body(request: &ProviderChatRequest) -> Result<Value, RequestError> {
    let model = request
        .model_id
        .strip_prefix(PLAN_MODEL_PREFIX)
        .unwrap_or(request.model_id.as_str());
    if !DOCUMENTED_MODELS
        .iter()
        .any(|(slug, _, _, _)| *slug == model)
    {
        return Err(RequestError::UnknownModel);
    }
    let mut input = Vec::new();
    if let Some(instructions) = developer_instructions(&request.messages) {
        input.push(json!({
            "type": "message",
            "role": "developer",
            "content": [{"type": "input_text", "text": instructions}],
        }));
    }
    for message in &request.messages {
        match message.role {
            ContextRole::System => {}
            ContextRole::User => push_user(&mut input, message)?,
            ContextRole::Assistant => push_assistant(&mut input, message),
            ContextRole::Tool => push_tool_result(&mut input, message),
        }
    }
    if input.is_empty() {
        return Err(RequestError::Empty);
    }

    // Nothing is stored server-side: the app keeps its own history and sends all
    // of it on every turn, so `previous_response_id` never appears.
    let mut body = json!({
        "model": model,
        "stream": true,
        "store": false,
        "input": input,
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(function_tool).collect());
        body["tool_choice"] = json!("auto");
        body["parallel_tool_calls"] = json!(true);
    }
    if request.allow_reasoning {
        body["reasoning"] = json!({"summary": "auto"});
        body["include"] = json!(["reasoning.encrypted_content"]);
    }
    Ok(body)
}

/// The system messages, joined into the one developer item this route takes in
/// their place.
fn developer_instructions(messages: &[ContextMessage]) -> Option<String> {
    let text = messages
        .iter()
        .filter(|message| message.role == ContextRole::System && !message.content.is_empty())
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(text)
}

fn push_user(input: &mut Vec<Value>, message: &ContextMessage) -> Result<(), RequestError> {
    let mut content = Vec::new();
    if !message.content.is_empty() {
        content.push(json!({"type": "input_text", "text": message.content}));
    }
    for attachment in &message.attachments {
        match attachment {
            ContextAttachment::Image { media_type, bytes } => {
                // An empty payload is not an image. Sending it would hand the
                // provider a data URL it rejects after the run has already sent it.
                if bytes.is_empty() {
                    return Err(RequestError::Attachment);
                }
                content.push(json!({
                    "type": "input_image",
                    "image_url": format!(
                        "data:{};base64,{}",
                        media_type.as_str(),
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ),
                }));
            }
        }
    }
    if !content.is_empty() {
        input.push(json!({"type": "message", "role": "user", "content": content}));
    }
    Ok(())
}

fn push_assistant(input: &mut Vec<Value>, message: &ContextMessage) {
    // Reasoning items come first and are replayed exactly as the model produced
    // them, because their encrypted content is opaque to this app.
    input.extend(message.reasoning_details.iter().cloned());
    if !message.content.is_empty() {
        input.push(json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": message.content}],
        }));
    }
    for call in &message.tool_calls {
        input.push(json!({
            "type": "function_call",
            "name": call.name,
            "arguments": arguments_text(&call.arguments),
            "call_id": call.call_id,
        }));
    }
}

/// A call that takes no arguments streams nothing, and an empty string is not a
/// JSON object the route will accept.
fn arguments_text(arguments: &str) -> &str {
    if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    }
}

fn push_tool_result(input: &mut Vec<Value>, message: &ContextMessage) {
    // A result with no call to answer has no place in the input array.
    let Some(responds_to) = message.responds_to.as_ref() else {
        return;
    };
    let mut output = message.content.clone();
    if message.truncated {
        output.push_str(TRUNCATED_MARKER);
    }
    input.push(json!({
        "type": "function_call_output",
        "call_id": responds_to.call_id,
        "output": output,
    }));
}

/// `strict` stays off because the registry's schemas are echoed verbatim, and
/// strict mode would require each one to meet OpenAI's stricter rules.
fn function_tool(tool: &ProviderToolSchema) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "strict": false,
        "parameters": tool.input_schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::{ContextToolCall, ImageMediaType};

    fn request(messages: Vec<ContextMessage>) -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: "gpt-6-luna".to_owned(),
            messages,
            max_output_tokens: None,
            allow_reasoning: false,
            tools: Vec::new(),
            openrouter: None,
        }
    }

    fn search_tool() -> ProviderToolSchema {
        ProviderToolSchema {
            name: "search".to_owned(),
            description: "Search the web".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"q": {"type": "string"}},
                "required": ["q"],
            }),
        }
    }

    /// Asserts what every body must hold: the route refuses a forbidden field, and
    /// it refuses a `system` item.
    fn built(request: &ProviderChatRequest) -> Value {
        let body = build_request_body(request).expect("request builds");
        for field in FORBIDDEN_FIELDS {
            assert!(body.get(field).is_none(), "{field} must not be sent");
        }
        let items = body["input"].as_array().expect("input is an array");
        assert!(items.iter().all(|item| item["role"] != "system"));
        body
    }

    #[test]
    fn system_text_becomes_one_developer_item_ahead_of_the_user_turn() {
        let body = built(&request(vec![
            ContextMessage::new(ContextRole::System, "Be terse."),
            ContextMessage::new(ContextRole::System, ""),
            ContextMessage::new(ContextRole::System, "Cite sources."),
            ContextMessage::new(ContextRole::User, "hi"),
        ]));

        assert_eq!(
            body,
            json!({
                "model": "gpt-6-luna",
                "stream": true,
                "store": false,
                "input": [
                    {"type": "message", "role": "developer", "content": [
                        {"type": "input_text", "text": "Be terse.\n\nCite sources."}
                    ]},
                    {"type": "message", "role": "user", "content": [
                        {"type": "input_text", "text": "hi"}
                    ]},
                ],
            })
        );
    }

    #[test]
    fn empty_system_text_adds_no_developer_item() {
        let body = built(&request(vec![
            ContextMessage::new(ContextRole::System, ""),
            ContextMessage::new(ContextRole::User, "hi"),
        ]));

        assert_eq!(body["input"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["input"][0]["role"], "user");
    }

    #[test]
    fn a_user_turn_with_neither_text_nor_images_is_skipped() {
        let body = built(&request(vec![
            ContextMessage::new(ContextRole::User, ""),
            ContextMessage::new(ContextRole::User, "hi"),
        ]));

        assert_eq!(body["input"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["input"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn no_tools_means_no_tool_keys() {
        let body = built(&request(vec![ContextMessage::new(ContextRole::User, "hi")]));

        for key in ["tools", "tool_choice", "parallel_tool_calls"] {
            assert!(
                body.get(key).is_none(),
                "{key} must be absent without tools"
            );
        }
    }

    #[test]
    fn declared_tools_become_function_tools_with_their_schema_verbatim() {
        let body = built(&ProviderChatRequest {
            tools: vec![search_tool()],
            ..request(vec![ContextMessage::new(ContextRole::User, "hi")])
        });

        assert_eq!(
            body["tools"],
            json!([{
                "type": "function",
                "name": "search",
                "description": "Search the web",
                "strict": false,
                "parameters": {
                    "type": "object",
                    "properties": {"q": {"type": "string"}},
                    "required": ["q"],
                },
            }])
        );
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
    }

    #[test]
    fn reasoning_keys_follow_allow_reasoning() {
        let messages = || vec![ContextMessage::new(ContextRole::User, "hi")];

        let off = built(&request(messages()));
        assert!(off.get("reasoning").is_none());
        assert!(off.get("include").is_none());

        let on = built(&ProviderChatRequest {
            allow_reasoning: true,
            ..request(messages())
        });
        assert_eq!(on["reasoning"], json!({"summary": "auto"}));
        assert_eq!(on["include"], json!(["reasoning.encrypted_content"]));
    }

    #[test]
    fn a_tool_round_trip_keeps_item_order_and_shapes() {
        let reasoning = json!({
            "type": "reasoning",
            "id": "rs_1",
            "summary": [{"type": "summary_text", "text": "Two lookups."}],
            "encrypted_content": "opaque-blob",
        });
        let body = built(&ProviderChatRequest {
            tools: vec![search_tool()],
            allow_reasoning: true,
            ..request(vec![
                ContextMessage::new(ContextRole::User, "search for rust"),
                ContextMessage::assistant_tool_calls(
                    "Checking two sources.",
                    vec![
                        ContextToolCall::new("call_1", "search", r#"{"q":"rust"}"#),
                        ContextToolCall::new("call_2", "read", r#"{"path":"a.md"}"#),
                    ],
                )
                .with_reasoning_details(vec![reasoning]),
                ContextMessage::tool_result("call_1", "search", "3 hits"),
                ContextMessage::tool_result("call_2", "read", "file body"),
                ContextMessage::new(ContextRole::User, "thanks"),
            ])
        });

        assert_eq!(
            body,
            json!({
                "model": "gpt-6-luna",
                "stream": true,
                "store": false,
                "input": [
                    {"type": "message", "role": "user", "content": [
                        {"type": "input_text", "text": "search for rust"}
                    ]},
                    {
                        "type": "reasoning",
                        "id": "rs_1",
                        "summary": [{"type": "summary_text", "text": "Two lookups."}],
                        "encrypted_content": "opaque-blob",
                    },
                    {"type": "message", "role": "assistant", "content": [
                        {"type": "output_text", "text": "Checking two sources."}
                    ]},
                    {
                        "type": "function_call",
                        "name": "search",
                        "arguments": "{\"q\":\"rust\"}",
                        "call_id": "call_1",
                    },
                    {
                        "type": "function_call",
                        "name": "read",
                        "arguments": "{\"path\":\"a.md\"}",
                        "call_id": "call_2",
                    },
                    {"type": "function_call_output", "call_id": "call_1", "output": "3 hits"},
                    {"type": "function_call_output", "call_id": "call_2", "output": "file body"},
                    {"type": "message", "role": "user", "content": [
                        {"type": "input_text", "text": "thanks"}
                    ]},
                ],
                "tools": [{
                    "type": "function",
                    "name": "search",
                    "description": "Search the web",
                    "strict": false,
                    "parameters": {
                        "type": "object",
                        "properties": {"q": {"type": "string"}},
                        "required": ["q"],
                    },
                }],
                "tool_choice": "auto",
                "parallel_tool_calls": true,
                "reasoning": {"summary": "auto"},
                "include": ["reasoning.encrypted_content"],
            })
        );
    }

    #[test]
    fn a_truncated_tool_result_says_so_in_its_output() {
        let mut result = ContextMessage::tool_result("call_1", "read", "first lines");
        result.truncated = true;
        let body = built(&request(vec![
            ContextMessage::new(ContextRole::User, "read it"),
            result,
        ]));

        assert_eq!(
            body["input"][1],
            json!({
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "first lines\n[truncated]",
            })
        );
    }

    #[test]
    fn a_tool_call_with_no_arguments_sends_an_empty_object() {
        let body = built(&request(vec![
            ContextMessage::new(ContextRole::User, "ping"),
            ContextMessage::assistant_tool_calls(
                "",
                vec![
                    ContextToolCall::new("call_1", "ping", ""),
                    ContextToolCall::new("call_2", "ping", "  "),
                ],
            ),
        ]));

        assert_eq!(
            body["input"][1],
            json!({"type": "function_call", "name": "ping", "arguments": "{}", "call_id": "call_1"})
        );
        assert_eq!(
            body["input"][2],
            json!({"type": "function_call", "name": "ping", "arguments": "{}", "call_id": "call_2"})
        );
    }

    #[test]
    fn a_tool_message_without_a_call_to_answer_is_skipped() {
        let body = built(&request(vec![
            ContextMessage::new(ContextRole::Tool, "orphan output"),
            ContextMessage::new(ContextRole::User, "hi"),
        ]));

        assert_eq!(
            body["input"],
            json!([{"type": "message", "role": "user", "content": [
                {"type": "input_text", "text": "hi"}
            ]}])
        );
    }

    #[test]
    fn an_image_attachment_becomes_an_input_image_data_url() {
        let body = built(&request(vec![
            ContextMessage::user_with_attachments(
                "What is this?",
                vec![ContextAttachment::Image {
                    media_type: ImageMediaType::Png,
                    bytes: vec![1, 2, 3],
                }],
            ),
            ContextMessage::user_with_attachments(
                "",
                vec![ContextAttachment::Image {
                    media_type: ImageMediaType::Jpeg,
                    bytes: vec![255],
                }],
            ),
        ]));

        assert_eq!(
            body["input"],
            json!([
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "What is this?"},
                    {"type": "input_image", "image_url": "data:image/png;base64,AQID"},
                ]},
                {"type": "message", "role": "user", "content": [
                    {"type": "input_image", "image_url": "data:image/jpeg;base64,/w=="},
                ]},
            ])
        );
    }

    #[test]
    fn an_image_with_no_bytes_is_refused_rather_than_sent_empty() {
        let result = build_request_body(&request(vec![ContextMessage::user_with_attachments(
            "look",
            vec![ContextAttachment::Image {
                media_type: ImageMediaType::Png,
                bytes: Vec::new(),
            }],
        )]));

        assert_eq!(result, Err(RequestError::Attachment));
    }

    #[test]
    fn the_plan_model_prefix_is_stripped() {
        let body = built(&ProviderChatRequest {
            model_id: "chatgpt_plan/gpt-6-luna".to_owned(),
            ..request(vec![ContextMessage::new(ContextRole::User, "hi")])
        });

        assert_eq!(body["model"], "gpt-6-luna");
    }

    #[test]
    fn every_documented_model_builds_and_any_other_is_refused_before_a_body_exists() {
        for (slug, _, _, _) in DOCUMENTED_MODELS {
            let body = built(&ProviderChatRequest {
                model_id: format!("chatgpt_plan/{slug}"),
                ..request(vec![ContextMessage::new(ContextRole::User, "hi")])
            });
            assert_eq!(body["model"], slug);
        }

        for model_id in [
            "gpt-5",
            "chatgpt_plan/gpt-5",
            "gpt-6-nova",
            "chatgpt_plan/gpt-6-nova",
            "chatgpt_plan/",
            "",
        ] {
            let refused = build_request_body(&ProviderChatRequest {
                model_id: model_id.to_owned(),
                ..request(vec![ContextMessage::new(ContextRole::User, "hi")])
            });
            assert_eq!(refused, Err(RequestError::UnknownModel), "{model_id}");
        }
    }

    #[test]
    fn a_requested_output_ceiling_never_reaches_the_body() {
        let body = built(&ProviderChatRequest {
            max_output_tokens: Some(4_096),
            ..request(vec![ContextMessage::new(ContextRole::User, "hi")])
        });

        assert!(body.get("max_output_tokens").is_none());
    }

    #[test]
    fn a_request_with_no_usable_message_is_refused() {
        assert_eq!(
            build_request_body(&request(Vec::new())),
            Err(RequestError::Empty)
        );
        assert_eq!(
            build_request_body(&request(vec![
                ContextMessage::new(ContextRole::User, ""),
                ContextMessage::new(ContextRole::Assistant, ""),
            ])),
            Err(RequestError::Empty)
        );
    }
}
