//! Turns the `data:` events of an OpenAI Responses API stream into the provider
//! events the agent consumes.
//!
//! A function call is reported at `response.output_item.done`, the first point at
//! which its arguments are complete. Reasoning items are held until the turn ends,
//! because they are echoed back on the next request exactly as the model sent them.

use serde_json::Value;

use crate::providers::{ProviderError, ProviderStopReason, ProviderStreamEvent};

/// What one payload, or the end of the connection, produced.
#[derive(Debug, Default)]
pub(crate) struct Mapped {
    pub(crate) events: Vec<ProviderStreamEvent>,
    /// Whether the turn has ended. Once true, every later payload is ignored.
    pub(crate) done: bool,
    /// Why the turn failed. A failed turn never emits [`ProviderStreamEvent::Completed`].
    pub(crate) error: Option<ProviderError>,
}

#[derive(Default)]
pub(crate) struct ResponsesStreamMapper {
    done: bool,
    emitted_tool_call: bool,
    discarded_tool_calls: u32,
    reasoning_items: Vec<Value>,
}

impl ResponsesStreamMapper {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Maps one `data:` payload. Anything that is not a JSON object with a known
    /// `type` carries no event for this mapper, so it is skipped rather than
    /// failing the turn.
    pub(crate) fn push(&mut self, data: &str) -> Mapped {
        let mut mapped = Mapped::default();
        if !self.done {
            if let Ok(event) = serde_json::from_str::<Value>(data) {
                self.map_event(&event, &mut mapped);
            }
        }
        mapped.done = self.done;
        mapped
    }

    /// Closes the connection. A stream that ends before the turn did is a failure.
    pub(crate) fn finish_eof(&mut self) -> Mapped {
        let mut mapped = Mapped::default();
        if !self.done {
            self.done = true;
            mapped.error = Some(ProviderError::Unavailable);
        }
        mapped.done = self.done;
        mapped
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    fn map_event(&mut self, event: &Value, mapped: &mut Mapped) {
        match event.get("type").and_then(Value::as_str) {
            // A refusal streams as text, so the user sees what the model said. Its
            // `done` event repeats that text, so it adds nothing.
            Some("response.output_text.delta" | "response.refusal.delta") => {
                push_delta(event, ProviderStreamEvent::TextDelta, mapped);
            }
            Some("response.refusal.done") => {}
            Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => {
                push_delta(event, ProviderStreamEvent::ReasoningDelta, mapped);
            }
            Some("response.output_item.added") => {
                if event.pointer("/item/type").and_then(Value::as_str) == Some("function_call") {
                    mapped.events.push(ProviderStreamEvent::Progress);
                }
            }
            Some("response.function_call_arguments.delta") => {
                mapped.events.push(ProviderStreamEvent::Progress);
            }
            Some("response.output_item.done") => self.map_output_item_done(event, mapped),
            Some("response.completed") => {
                let reason = if self.emitted_tool_call {
                    ProviderStopReason::ToolUse
                } else {
                    ProviderStopReason::Natural
                };
                self.end_turn(event, reason, mapped);
            }
            Some("response.incomplete") => {
                let reason = incomplete_reason(event);
                self.end_turn(event, reason, mapped);
            }
            Some("response.failed") => {
                let code = event
                    .pointer("/response/error/code")
                    .and_then(Value::as_str);
                self.fail(code, mapped);
            }
            Some("error") => self.fail(event.get("code").and_then(Value::as_str), mapped),
            _ => {}
        }
    }

    fn map_output_item_done(&mut self, event: &Value, mapped: &mut Mapped) {
        let Some(item) = event.get("item") else {
            return;
        };
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                if let Some(call) = tool_call_requested(item) {
                    self.emitted_tool_call = true;
                    mapped.events.push(call);
                } else {
                    self.discarded_tool_calls = self.discarded_tool_calls.saturating_add(1);
                }
            }
            // Stored as the model sent it, because the replay has to match the
            // model's own output and no field is read or rebuilt here.
            Some("reasoning") => self.reasoning_items.push(item.clone()),
            _ => {}
        }
    }

    /// Closes a turn that reached its own end: usage, any reasoning, the stop
    /// reason, then `Completed`.
    fn end_turn(&mut self, event: &Value, reason: ProviderStopReason, mapped: &mut Mapped) {
        if let Some(usage) = event
            .pointer("/response/usage")
            .filter(|usage| usage.is_object())
        {
            let input_tokens = token_count(usage, "input_tokens");
            mapped.events.push(ProviderStreamEvent::Usage {
                input_tokens,
                output_tokens: token_count(usage, "output_tokens"),
            });
            if let Some(tokens) = input_tokens {
                mapped.events.push(ProviderStreamEvent::PromptSize {
                    tokens: u64::from(tokens),
                });
            }
        }
        if !self.reasoning_items.is_empty() {
            let items = std::mem::take(&mut self.reasoning_items);
            mapped
                .events
                .push(ProviderStreamEvent::ReasoningDetails(items));
        }
        mapped.events.push(ProviderStreamEvent::Stopped {
            reason,
            discarded_tool_calls: self.discarded_tool_calls,
        });
        mapped.events.push(ProviderStreamEvent::Completed);
        self.done = true;
    }

    fn fail(&mut self, code: Option<&str>, mapped: &mut Mapped) {
        self.done = true;
        mapped.error = Some(code.map_or(ProviderError::InvalidResponse, map_error_code));
    }
}

/// Maps an OpenAI error `code` onto the provider error the app acts on. The
/// `subscription_sharing_*` and `chatpass_v2_*` codes are the ChatGPT-plan contract;
/// the rest are generic Responses codes. An unknown code is an unparseable response,
/// not a guess at a remedy.
pub(crate) fn map_error_code(code: &str) -> ProviderError {
    match code {
        "subscription_sharing_usage_limit_exceeded" => ProviderError::PlanUsageLimit,
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            ProviderError::Unavailable
        }
        "subscription_sharing_user_not_eligible"
        | "subscription_sharing_route_not_supported"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => ProviderError::PlanNotEligible,
        "subscription_sharing_unsupported_capability" => ProviderError::InvalidResponse,
        "subscription_sharing_invalid_user" => ProviderError::PlanSignInRequired,
        "context_length_exceeded" => ProviderError::ContextOverflow,
        "insufficient_quota" => ProviderError::PaymentRequired,
        "rate_limit_exceeded" | "slow_down" => ProviderError::RateLimited { retry_after: None },
        "server_is_overloaded" => ProviderError::Unavailable,
        _ => ProviderError::InvalidResponse,
    }
}

/// An empty delta changes nothing, so it is not an event.
fn push_delta(event: &Value, variant: fn(String) -> ProviderStreamEvent, mapped: &mut Mapped) {
    if let Some(delta) = event
        .get("delta")
        .and_then(Value::as_str)
        .filter(|delta| !delta.is_empty())
    {
        mapped.events.push(variant(delta.to_owned()));
    }
}

/// The call a `function_call` item carries, or `None` when it cannot be replayed: a
/// field is missing or empty, or the arguments are not JSON. Consumers parse the
/// arguments unconditionally, so a call that does not parse is dropped here.
fn tool_call_requested(item: &Value) -> Option<ProviderStreamEvent> {
    let call_id = item
        .get("call_id")
        .and_then(Value::as_str)
        .filter(|call_id| !call_id.is_empty())?;
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    let arguments = item.get("arguments").and_then(Value::as_str)?;
    let arguments = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    if serde_json::from_str::<Value>(arguments).is_err() {
        return None;
    }
    Some(ProviderStreamEvent::ToolCallRequested {
        call_id: call_id.to_owned(),
        name: name.to_owned(),
        arguments: arguments.to_owned(),
    })
}

/// An incomplete turn never reports `ToolUse`: it was cut short, so it did not stop
/// for its calls. Only the two reasons with a policy of their own are named.
fn incomplete_reason(event: &Value) -> ProviderStopReason {
    match event
        .pointer("/response/incomplete_details/reason")
        .and_then(Value::as_str)
    {
        Some("max_output_tokens") => ProviderStopReason::OutputLimit,
        Some("content_filter") => ProviderStopReason::Refusal,
        _ => ProviderStopReason::Other,
    }
}

fn token_count(usage: &Value, field: &str) -> Option<u32> {
    usage
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|count| u32::try_from(count).ok())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Feeds each payload in order and gathers what the turn produced.
    fn feed(mapper: &mut ResponsesStreamMapper, payloads: Vec<Value>) -> Mapped {
        let mut gathered = Mapped::default();
        for payload in payloads {
            let mapped = mapper.push(&payload.to_string());
            gathered.events.extend(mapped.events);
            gathered.done = mapped.done;
            gathered.error = gathered.error.or(mapped.error);
        }
        gathered
    }

    fn reasoning_item() -> Value {
        json!({
            "type": "reasoning",
            "id": "rs_1",
            "summary": [{"type": "summary_text", "text": "Checking the file"}],
            "encrypted_content": "opaque-blob"
        })
    }

    fn call_done(call_id: &str, name: &str, arguments: &str) -> Value {
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "id": format!("fc_{call_id}"),
                "call_id": call_id,
                "name": name,
                "arguments": arguments,
                "status": "completed"
            }
        })
    }

    fn completed(usage: Option<Value>) -> Value {
        let mut response = json!({"id": "resp_1"});
        if let Some(usage) = usage {
            response["usage"] = usage;
        }
        json!({"type": "response.completed", "response": response})
    }

    fn text_delta(delta: &str) -> Value {
        json!({"type": "response.output_text.delta", "delta": delta, "output_index": 0})
    }

    fn failed(code: &str) -> Value {
        json!({
            "type": "response.failed",
            "response": {
                "id": "resp_1",
                "error": {"code": code, "message": "redacted by the test"}
            }
        })
    }

    #[test]
    fn a_text_turn_maps_its_deltas_usage_and_natural_completion() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                json!({"type": "response.created", "response": {"id": "resp_1"}}),
                text_delta("Hel"),
                text_delta("lo"),
                json!({"type": "response.output_text.done", "text": "Hello"}),
                completed(Some(json!({
                    "input_tokens": 12,
                    "input_tokens_details": {"cached_tokens": 4},
                    "output_tokens": 5,
                    "output_tokens_details": {"reasoning_tokens": 0},
                    "total_tokens": 17
                }))),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::TextDelta("Hel".to_owned()),
                ProviderStreamEvent::TextDelta("lo".to_owned()),
                ProviderStreamEvent::Usage {
                    input_tokens: Some(12),
                    output_tokens: Some(5),
                },
                ProviderStreamEvent::PromptSize { tokens: 12 },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert_eq!(mapped.error, None);
        assert!(mapped.done);
        assert!(mapper.is_done());
    }

    #[test]
    fn a_refusal_streams_as_text_and_its_done_event_adds_nothing() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                json!({
                    "type": "response.refusal.delta",
                    "delta": "I can't help",
                    "output_index": 0,
                    "content_index": 0
                }),
                json!({
                    "type": "response.refusal.delta",
                    "delta": " with that.",
                    "output_index": 0,
                    "content_index": 0
                }),
                json!({
                    "type": "response.refusal.done",
                    "refusal": "I can't help with that.",
                    "output_index": 0,
                    "content_index": 0
                }),
                completed(None),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::TextDelta("I can't help".to_owned()),
                ProviderStreamEvent::TextDelta(" with that.".to_owned()),
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert_eq!(mapped.error, None);
        assert!(mapped.done);
    }

    #[test]
    fn a_tool_turn_maps_reasoning_the_call_and_tool_use_stop() {
        let mut mapper = ResponsesStreamMapper::new();
        let item = reasoning_item();
        let mapped = feed(
            &mut mapper,
            vec![
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "delta": "Checking the file",
                    "summary_index": 0
                }),
                json!({"type": "response.output_item.done", "item": item}),
                call_done("call_1", "read_file", r#"{"path":"a.txt"}"#),
                completed(None),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::ReasoningDelta("Checking the file".to_owned()),
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"a.txt"}"#.to_owned(),
                },
                ProviderStreamEvent::ReasoningDetails(vec![item]),
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert!(mapped.done);
    }

    #[test]
    fn parallel_calls_are_reported_in_stream_order() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                call_done("call_a", "read_file", r#"{"path":"a.txt"}"#),
                call_done("call_b", "list_dir", r#"{"path":"."}"#),
                completed(None),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_a".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"a.txt"}"#.to_owned(),
                },
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_b".to_owned(),
                    name: "list_dir".to_owned(),
                    arguments: r#"{"path":"."}"#.to_owned(),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[test]
    fn a_call_with_no_arguments_gets_an_empty_json_object() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![call_done("call_1", "now", ""), completed(None)],
        );

        assert_eq!(
            mapped.events[0],
            ProviderStreamEvent::ToolCallRequested {
                call_id: "call_1".to_owned(),
                name: "now".to_owned(),
                arguments: "{}".to_owned(),
            }
        );
    }

    #[test]
    fn a_call_that_cannot_be_replayed_is_discarded_and_counted() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                call_done("call_1", "read_file", r#"{"path":"a.txt"}"#),
                call_done("call_2", "read_file", r#"{"path":"#),
                json!({
                    "type": "response.output_item.done",
                    "item": {"type": "function_call", "name": "read_file", "arguments": "{}"}
                }),
                completed(None),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"a.txt"}"#.to_owned(),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 2,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[test]
    fn an_incomplete_turn_at_the_output_limit_stops_for_output_limit() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                text_delta("Partial"),
                json!({
                    "type": "response.incomplete",
                    "response": {
                        "id": "resp_1",
                        "incomplete_details": {"reason": "max_output_tokens"},
                        "usage": {"input_tokens": 9, "output_tokens": 64}
                    }
                }),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::TextDelta("Partial".to_owned()),
                ProviderStreamEvent::Usage {
                    input_tokens: Some(9),
                    output_tokens: Some(64),
                },
                ProviderStreamEvent::PromptSize { tokens: 9 },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::OutputLimit,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert!(mapped.done);
    }

    #[test]
    fn an_incomplete_turn_stopped_by_content_filter_is_a_refusal_even_with_a_call() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                call_done("call_1", "read_file", r#"{"path":"a.txt"}"#),
                json!({
                    "type": "response.incomplete",
                    "response": {"id": "resp_1", "incomplete_details": {"reason": "content_filter"}}
                }),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"a.txt"}"#.to_owned(),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Refusal,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[test]
    fn an_incomplete_turn_with_no_known_reason_stops_as_other() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![json!({"type": "response.incomplete", "response": {"id": "resp_1"}})],
        );

        assert_eq!(
            mapped.events,
            vec![
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Other,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[test]
    fn each_documented_failure_code_ends_the_turn_with_its_error_and_no_completion() {
        let cases = [
            (
                "subscription_sharing_usage_limit_exceeded",
                ProviderError::PlanUsageLimit,
            ),
            (
                "subscription_sharing_usage_unavailable",
                ProviderError::Unavailable,
            ),
            (
                "subscription_sharing_user_unavailable",
                ProviderError::Unavailable,
            ),
            (
                "subscription_sharing_user_not_eligible",
                ProviderError::PlanNotEligible,
            ),
            (
                "subscription_sharing_unsupported_capability",
                ProviderError::InvalidResponse,
            ),
            (
                "subscription_sharing_route_not_supported",
                ProviderError::PlanNotEligible,
            ),
            (
                "subscription_sharing_invalid_user",
                ProviderError::PlanSignInRequired,
            ),
            (
                "chatpass_v2_scope_not_authorized",
                ProviderError::PlanNotEligible,
            ),
            (
                "chatpass_v2_invalid_authorization_context",
                ProviderError::PlanNotEligible,
            ),
            ("context_length_exceeded", ProviderError::ContextOverflow),
            ("insufficient_quota", ProviderError::PaymentRequired),
            (
                "rate_limit_exceeded",
                ProviderError::RateLimited { retry_after: None },
            ),
            (
                "slow_down",
                ProviderError::RateLimited { retry_after: None },
            ),
            ("server_is_overloaded", ProviderError::Unavailable),
            ("some_code_not_in_the_table", ProviderError::InvalidResponse),
        ];

        for (code, expected) in cases {
            let mut mapper = ResponsesStreamMapper::new();
            let mapped = feed(&mut mapper, vec![failed(code)]);

            assert_eq!(mapped.error, Some(expected), "code {code}");
            assert!(mapped.events.is_empty(), "code {code}");
            assert!(mapped.done, "code {code}");
        }
    }

    #[test]
    fn a_top_level_error_event_fails_the_turn_after_the_text_it_follows() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                text_delta("Par"),
                json!({
                    "type": "error",
                    "code": "subscription_sharing_usage_limit_exceeded",
                    "message": "redacted by the test",
                    "param": null,
                    "sequence_number": 2
                }),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![ProviderStreamEvent::TextDelta("Par".to_owned())]
        );
        assert_eq!(mapped.error, Some(ProviderError::PlanUsageLimit));
        assert!(mapped.done);
    }

    #[test]
    fn a_failure_without_a_code_is_an_invalid_response() {
        for payload in [
            json!({"type": "error", "code": null, "message": "redacted by the test"}),
            json!({"type": "response.failed", "response": {"id": "resp_1"}}),
        ] {
            let mut mapper = ResponsesStreamMapper::new();
            let mapped = feed(&mut mapper, vec![payload]);

            assert_eq!(mapped.error, Some(ProviderError::InvalidResponse));
            assert!(mapped.done);
        }
    }

    #[test]
    fn garbage_and_events_this_mapper_does_not_act_on_are_ignored() {
        let mut mapper = ResponsesStreamMapper::new();
        for payload in [
            "",
            "   ",
            "[DONE]",
            "not json",
            "[1,2]",
            "42",
            r#"{"delta":"orphan"}"#,
            r#"{"type":7}"#,
            r#"{"type":"response.output_text.delta","delta":""}"#,
            r#"{"type":"response.output_text.delta"}"#,
            r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
            r#"{"type":"response.in_progress","response":{"id":"resp_1"}}"#,
            r#"{"type":"response.output_item.done","item":{"type":"message","content":[]}}"#,
            r#"{"type":"response.some_future_event","delta":"x"}"#,
        ] {
            let mapped = mapper.push(payload);

            assert!(mapped.events.is_empty(), "payload {payload:?}");
            assert_eq!(mapped.error, None, "payload {payload:?}");
            assert!(!mapped.done, "payload {payload:?}");
        }
    }

    #[test]
    fn argument_fragments_report_progress_and_never_a_call() {
        let mut mapper = ResponsesStreamMapper::new();
        let mapped = feed(
            &mut mapper,
            vec![
                json!({
                    "type": "response.output_item.added",
                    "item": {"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": ""}
                }),
                json!({"type": "response.function_call_arguments.delta", "delta": "{\"pa"}),
                json!({"type": "response.output_item.added", "item": {"type": "message", "content": []}}),
            ],
        );

        assert_eq!(
            mapped.events,
            vec![ProviderStreamEvent::Progress, ProviderStreamEvent::Progress]
        );
    }

    #[test]
    fn a_connection_that_ends_before_the_turn_does_is_unavailable() {
        let mut mapper = ResponsesStreamMapper::new();
        feed(&mut mapper, vec![text_delta("Par")]);

        let mapped = mapper.finish_eof();

        assert!(mapped.events.is_empty());
        assert_eq!(mapped.error, Some(ProviderError::Unavailable));
        assert!(mapped.done);
        assert!(mapper.is_done());
    }

    #[test]
    fn a_connection_that_ends_after_completion_adds_nothing() {
        let mut mapper = ResponsesStreamMapper::new();
        feed(&mut mapper, vec![completed(None)]);

        let mapped = mapper.finish_eof();

        assert!(mapped.events.is_empty());
        assert_eq!(mapped.error, None);
        assert!(mapped.done);
    }

    #[test]
    fn payloads_after_the_turn_ends_are_ignored() {
        let mut completed_mapper = ResponsesStreamMapper::new();
        feed(&mut completed_mapper, vec![completed(None)]);
        let mut failed_mapper = ResponsesStreamMapper::new();
        feed(
            &mut failed_mapper,
            vec![failed("subscription_sharing_invalid_user")],
        );

        for mapper in [&mut completed_mapper, &mut failed_mapper] {
            for payload in [
                text_delta("late").to_string(),
                completed(None).to_string(),
                failed("slow_down").to_string(),
            ] {
                let mapped = mapper.push(&payload);

                assert!(mapped.events.is_empty());
                assert_eq!(mapped.error, None);
                assert!(mapped.done);
            }
        }
    }
}
