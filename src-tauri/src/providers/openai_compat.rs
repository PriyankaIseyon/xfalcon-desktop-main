//! The OpenAI-compatible Chat Completions wire, shared by every adapter that
//! speaks it (`super::openai`, `super::openrouter`).
//!
//! Mirrors xFalcon Web's reference implementation
//! (`xfalcon-web/src/lib/server/providers/openai.ts`), which serves OpenAI and
//! OpenRouter through one adapter for the same reason: the request shape, the
//! `delta.tool_calls` accumulation rules, and the `[DONE]` sentinel are the
//! protocol, not the vendor. What differs per vendor is the endpoint, the
//! authorization/attribution headers, and a small set of body extras — that is
//! exactly what [`CompatDialect`] names, so a second vendor costs a dialect and
//! a header set rather than a second copy of the parser.

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::RequestBuilder;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use base64::Engine as _;

use crate::agent::context::{ContextAttachment, ContextMessage, ContextRole};

use super::adapter::{ProviderChatRequest, ProviderError, ProviderStopReason, ProviderStreamEvent};
use super::http::{classify_status_code, stream_sse_lines_limited, DEFAULT_STREAM_BYTES};

/// The per-vendor differences in an otherwise identical request.
pub(super) struct CompatDialect {
    /// Whether to ask for a final usage chunk with
    /// `stream_options: {include_usage: true}`.
    ///
    /// OpenAI requires it: without the flag no usage is reported at all. On
    /// OpenRouter both it and `usage: {include: true}` are documented as
    /// deprecated and inert — usage rides the last SSE message unconditionally
    /// — so sending it there would be cargo cult, and the parser reads usage
    /// the same way either way.
    pub request_usage_via_stream_options: bool,
}

pub(super) fn build_request_body(request: &ProviderChatRequest, dialect: &CompatDialect) -> Value {
    let messages: Vec<Value> = request.messages.iter().map(render_message).collect();
    let mut body = json!({
        "model": request.model_id,
        "messages": messages,
        "stream": true,
    });
    // Optional on this wire, and omitted rather than guessed: the provider
    // then applies the model's own maximum, which is the only number that is
    // right for every model behind an OpenRouter key. A ceiling set here would
    // also be the ceiling the model's thinking shares with its answer.
    if let Some(max_output_tokens) = request.max_output_tokens {
        body["max_tokens"] = json!(max_output_tokens);
    }
    if dialect.request_usage_via_stream_options {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema,
                        },
                    })
                })
                .collect(),
        );
    }
    body
}

/// Sends one already-authorized request and normalizes its SSE body.
///
/// `request_builder` arrives with the vendor's headers already attached, so this
/// function never has to know how a given vendor is authenticated.
pub(super) async fn stream_completion(
    request_builder: RequestBuilder,
    body: &Value,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
) -> Result<(), ProviderError> {
    let response = request_builder.json(body).send().await.map_err(|error| {
        let error_kind = if error.is_connect() {
            "connect"
        } else if error.is_timeout() {
            "timeout"
        } else if error.is_request() {
            "request"
        } else {
            "transport"
        };
        tracing::error!(error_kind, "provider request did not reach the endpoint");
        super::http::classify_transport_error(&error)
    })?;

    if !response.status().is_success() {
        return Err(super::http::classify_response(response).await);
    }

    consume_completion_stream(response, cancel, DEFAULT_STREAM_BYTES, on_event).await
}

/// Parses one successful OpenAI-compatible SSE response.
///
/// Kept separate from request sending so guarded dynamic transports can apply
/// their own credential and connection classification without forking the
/// protocol parser used by fixed providers.
pub(super) async fn consume_completion_stream(
    response: reqwest::Response,
    cancel: &CancellationToken,
    byte_limit: usize,
    on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
) -> Result<(), ProviderError> {
    debug_assert!(response.status().is_success());

    let mut state = ParseState::default();
    // An OpenAI-compatible gateway may report a *fatal* condition inside a
    // 200 body (see `parse_event`). `stream_sse_lines`'s callback cannot
    // return an error, so the error is captured here and the read is stopped
    // through a child token — cancelling the caller's own token instead would
    // report this run's failure as the whole conversation being cancelled.
    let stop = cancel.child_token();
    let started = std::time::Instant::now();
    let mut fatal: Option<ProviderError> = None;
    let mut received_output = false;
    let outcome = stream_sse_lines_limited(response, &stop, byte_limit, |data| {
        if fatal.is_some() {
            return;
        }
        match parse_event(data, &mut state) {
            Ok(events) => {
                for event in events {
                    received_output |= !matches!(event, ProviderStreamEvent::Progress);
                    on_event(event);
                }
            }
            Err(error) => {
                fatal = Some(error);
                stop.cancel();
            }
        }
    })
    .await;

    if let Some(error) = fatal {
        return Err(error);
    }
    // A body that ends cleanly without `[DONE]`, or a connection that breaks
    // after output has begun, leaves a turn to close. A break before any output
    // is a plain failure of the request and is reported as one.
    let cut_mid_answer = matches!(
        outcome,
        Err(ProviderError::Unavailable | ProviderError::TimedOut)
    ) && received_output;
    if outcome.is_ok() || cut_mid_answer {
        for event in close_without_done(&mut state, started.elapsed()) {
            on_event(event);
        }
        return Ok(());
    }
    outcome
}

/// Parses one successful, non-streamed Chat Completions response.
///
/// The body is one `chat.completion` object. It is turned into the single chunk
/// and the `[DONE]` marker a stream would have carried, so both shapes end in
/// the same events. The whole answer arrives at once.
pub(super) async fn consume_completion_body(
    response: reqwest::Response,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
) -> Result<(), ProviderError> {
    let bytes = super::http::read_body_capped(response, cancel).await?;
    let completion: Value =
        serde_json::from_slice(&bytes).map_err(|_| ProviderError::InvalidResponse)?;
    let mut state = ParseState::default();
    for data in [completion_as_chunk(&completion), "[DONE]".to_owned()] {
        for event in parse_event(&data, &mut state)? {
            on_event(event);
        }
    }
    Ok(())
}

/// A `chat.completion` object as the one stream chunk that says the same.
fn completion_as_chunk(completion: &Value) -> String {
    let choice = completion.get("choices").and_then(|choices| choices.get(0));
    let mut delta = choice
        .and_then(|choice| choice.get("message"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(calls) = delta.get_mut("tool_calls").and_then(Value::as_array_mut) {
        for (index, call) in calls.iter_mut().enumerate() {
            call["index"] = json!(index);
        }
    }
    let mut chunk = json!({
        "choices": [{
            "delta": delta,
            "finish_reason": choice.and_then(|choice| choice.get("finish_reason")).cloned().unwrap_or(Value::Null),
        }],
    });
    if let Some(usage) = completion.get("usage") {
        chunk["usage"] = usage.clone();
    }
    if let Some(error) = completion.get("error") {
        chunk["error"] = error.clone();
    }
    chunk.to_string()
}

/// What to do when the response body ends without the `[DONE]` marker.
///
/// Many OpenAI-compatible gateways simply close the stream after the last
/// chunk. When a `finish_reason` already arrived the turn is complete, so it is
/// closed exactly as `[DONE]` would have closed it. With none, the body was cut
/// off mid-answer, perhaps by a proxy: the turn is closed as a cut-off one.
/// Finished tool calls are kept, a half-written one is discarded, and the run
/// asks the model for the rest in smaller pieces, as it does for an output
/// limit. The log says how far the turn had got.
fn close_without_done(
    state: &mut ParseState,
    elapsed: std::time::Duration,
) -> Vec<ProviderStreamEvent> {
    if state.done {
        return Vec::new();
    }
    if state.stop.is_some() {
        tracing::warn!(
            finish_reason = ?state.stop,
            elapsed_secs = elapsed.as_secs(),
            "model stream closed without [DONE] after a finish reason; completing the turn"
        );
        return parse_event("[DONE]", state).unwrap_or_default();
    }
    tracing::warn!(
        pending_tool_calls = state.pending_tool_calls.len(),
        elapsed_secs = elapsed.as_secs(),
        "model stream closed without a finish reason or [DONE]; the response was cut off"
    );
    state.stop = Some(ProviderStopReason::StreamCut);
    parse_event("[DONE]", state).unwrap_or_default()
}

/// Renders one context message in Chat Completions shape.
///
/// Two shapes are special: an assistant turn carrying tool calls attaches a
/// `tool_calls` array (with `function.arguments` as a *string*, which is what
/// the API takes and what the model itself produced), and a tool result
/// becomes a `role: "tool"` message keyed by `tool_call_id` — the API rejects
/// an unkeyed one.
fn render_message(message: &ContextMessage) -> Value {
    if message.role == ContextRole::Tool {
        if let Some(responds_to) = message.responds_to.as_ref() {
            return json!({
                "role": "tool",
                "tool_call_id": responds_to.call_id,
                "content": message.content,
            });
        }
    }

    let mut rendered = json!({
        "role": role_name(message.role),
        "content": render_content(message),
    });
    // Handed back exactly as the provider streamed it, in the order it
    // streamed it. The condition the replay must satisfy is that "the entire
    // sequence of consecutive reasoning blocks must match the outputs
    // generated by the model during the original request", so this is a copy,
    // not a re-render. Absent for every provider and model that streams none,
    // which leaves those requests byte-identical.
    if !message.reasoning_details.is_empty() {
        rendered["reasoning_details"] = Value::Array(message.reasoning_details.clone());
    }
    if !message.tool_calls.is_empty() {
        rendered["tool_calls"] = Value::Array(
            message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.call_id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments,
                        },
                    })
                })
                .collect(),
        );
    }
    rendered
}

/// Plain text, or -- when the turn carries images -- the multimodal parts
/// array: the text first, then each image as a `data:` URL.
fn render_content(message: &ContextMessage) -> Value {
    if message.attachments.is_empty() {
        return Value::String(message.content.clone());
    }
    let mut parts = Vec::with_capacity(message.attachments.len() + 1);
    if !message.content.is_empty() {
        parts.push(json!({ "type": "text", "text": message.content }));
    }
    for attachment in &message.attachments {
        match attachment {
            ContextAttachment::Image { media_type, bytes } => parts.push(json!({
                "type": "image_url",
                "image_url": {
                    "url": format!(
                        "data:{};base64,{}",
                        media_type.as_str(),
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ),
                },
            })),
        }
    }
    Value::Array(parts)
}

const fn role_name(role: ContextRole) -> &'static str {
    match role {
        ContextRole::System => "system",
        ContextRole::User => "user",
        ContextRole::Assistant => "assistant",
        ContextRole::Tool => "tool",
    }
}

/// One tool call being assembled from `delta.tool_calls` fragments.
///
/// The id and name arrive on the first fragment for a given `index` and
/// `arguments` then streams in pieces; the call is emitted only when the stream
/// closes it. Partial arguments emit only internal progress.
#[derive(Default)]
struct PartialToolCall {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub(super) struct ParseState {
    done: bool,
    /// Keyed by the wire's `index`, which is the API's ordering key for
    /// parallel calls — arrival order across chunks is not guaranteed to
    /// match it.
    pending_tool_calls: BTreeMap<u64, PartialToolCall>,
    /// Reasoning blocks under assembly, keyed by the wire's `index` — the
    /// same ordering key `pending_tool_calls` uses, and for the same reason.
    pending_reasoning: BTreeMap<u64, Value>,
    /// The wire's `finish_reason`, once a chunk carries one.
    stop: Option<ProviderStopReason>,
    /// Calls dropped because the turn ended before their arguments did.
    discarded_tool_calls: u32,
}

impl ParseState {
    /// Drops every call whose arguments are not a complete JSON object, and
    /// emits the ones that are.
    ///
    /// Used when the turn was cut off: the output limit can land inside any
    /// call's arguments, and the wire gives no marker for which. A call that
    /// parses was finished before the cut.
    fn flush_complete_tool_calls(&mut self) -> Vec<ProviderStreamEvent> {
        let mut events = Vec::new();
        for call in std::mem::take(&mut self.pending_tool_calls).into_values() {
            if call.call_id.is_empty() {
                continue;
            }
            let arguments = normalize_arguments(call.arguments);
            if serde_json::from_str::<Value>(&arguments).is_ok_and(|input| input.is_object()) {
                events.push(ProviderStreamEvent::ToolCallRequested {
                    call_id: call.call_id,
                    name: call.name,
                    arguments,
                });
            } else {
                self.discarded_tool_calls = self.discarded_tool_calls.saturating_add(1);
            }
        }
        events
    }

    /// Drains the assembled reasoning blocks in `index` order.
    ///
    /// Emitted once per turn rather than per fragment because a fragment is
    /// not a block: OpenRouter streams one `reasoning_details` entry per
    /// chunk, each carrying a slice of the same indexed block. Echoing those
    /// slices back would not "match the outputs generated by the model", which
    /// is the condition the replay has to satisfy.
    fn flush_reasoning_details(&mut self) -> Option<ProviderStreamEvent> {
        let blocks: Vec<Value> = std::mem::take(&mut self.pending_reasoning)
            .into_values()
            .collect();
        (!blocks.is_empty()).then_some(ProviderStreamEvent::ReasoningDetails(blocks))
    }

    /// Drains every accumulated call in `index` order.
    fn flush_tool_calls(&mut self) -> Result<Vec<ProviderStreamEvent>, ProviderError> {
        std::mem::take(&mut self.pending_tool_calls)
            .into_values()
            .map(|call| {
                if call.call_id.is_empty() || call.name.is_empty() {
                    return Err(ProviderError::InvalidResponse);
                }
                Ok(ProviderStreamEvent::ToolCallRequested {
                    call_id: call.call_id,
                    name: call.name,
                    arguments: normalize_arguments(call.arguments),
                })
            })
            .collect()
    }
}

/// A tool taking no arguments streams an empty string, which is not parseable
/// JSON. Normalizing here means every consumer above can parse unconditionally
/// instead of special-casing the empty case.
fn normalize_arguments(arguments: String) -> String {
    if arguments.trim().is_empty() {
        "{}".to_owned()
    } else {
        arguments
    }
}

/// Merges streamed `reasoning_details` fragments into one block per `index`.
///
/// The text-bearing key is appended; every other key is taken from whichever
/// fragment carried it. `signature` in particular arrives on a later fragment
/// than the text it signs, and a block echoed back without it is rejected.
///
/// The block keeps whatever field names the provider used. Which key holds the
/// payload differs by block type (`text`, `summary`, `data`), and this app has
/// no reason to normalize them: it never reads these blocks, it only hands
/// them back.
fn accumulate_reasoning_details(state: &mut ParseState, deltas: &[Value]) {
    for delta in deltas {
        let Some(fields) = delta.as_object() else {
            continue;
        };
        let index = delta.get("index").and_then(Value::as_u64).unwrap_or(0);
        let block = state
            .pending_reasoning
            .entry(index)
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let Some(target) = block.as_object_mut() else {
            continue;
        };
        for (key, value) in fields {
            match (target.get_mut(key), value.as_str()) {
                // Only the payload string grows across fragments; an id or a
                // format repeated on every chunk must not be concatenated with
                // itself.
                (Some(Value::String(existing)), Some(fragment))
                    if matches!(key.as_str(), "text" | "summary" | "data") =>
                {
                    existing.push_str(fragment);
                }
                _ => {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

/// Which pending call a streamed fragment belongs to.
///
/// The wire's `index` is the key, and a call's `id` arrives with its first
/// fragment. Servers differ in how they keep to that: some leave `index` out,
/// and some number every parallel call 0. Two calls merged into one would
/// carry corrupted arguments, so a fragment whose `id` names a different call
/// than the one stored at its index is a new call, and a fragment with no
/// `index` follows its `id`, or the call before it when it has none.
fn tool_call_slot(state: &ParseState, delta: &Value) -> u64 {
    let wire_index = delta.get("index").and_then(Value::as_u64);
    let id = delta
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let by_id = |id: &str| {
        state
            .pending_tool_calls
            .iter()
            .find_map(|(slot, call)| (call.call_id == id).then_some(*slot))
    };
    let next_free = || {
        state
            .pending_tool_calls
            .keys()
            .next_back()
            .map_or(0, |last| last + 1)
    };
    match (wire_index, id) {
        (Some(index), Some(id)) => match state.pending_tool_calls.get(&index) {
            Some(call) if !call.call_id.is_empty() && call.call_id != id => {
                by_id(id).unwrap_or_else(next_free)
            }
            _ => index,
        },
        (Some(index), None) => index,
        (None, Some(id)) => by_id(id).unwrap_or_else(next_free),
        (None, None) => state
            .pending_tool_calls
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0),
    }
}

fn accumulate_tool_calls(state: &mut ParseState, deltas: &[Value]) -> Result<(), ProviderError> {
    for delta in deltas {
        let fields = delta.as_object().ok_or(ProviderError::InvalidResponse)?;
        if fields.contains_key("index") && delta.get("index").and_then(Value::as_u64).is_none() {
            return Err(ProviderError::InvalidResponse);
        }
        if let Some(id) = fields.get("id") {
            match id {
                Value::Null => {}
                Value::String(_) => {}
                _ => return Err(ProviderError::InvalidResponse),
            }
        }
        if let Some(call_type) = fields.get("type") {
            match call_type {
                Value::Null => {}
                Value::String(call_type) if call_type == "function" => {}
                _ => return Err(ProviderError::InvalidResponse),
            }
        }
        if let Some(function) = fields.get("function") {
            if !function.is_null() {
                let function = function.as_object().ok_or(ProviderError::InvalidResponse)?;
                if let Some(name) = function.get("name") {
                    match name {
                        Value::Null => {}
                        Value::String(_) => {}
                        _ => return Err(ProviderError::InvalidResponse),
                    }
                }
                if let Some(arguments) = function.get("arguments") {
                    if !arguments.is_null() && !arguments.is_string() {
                        return Err(ProviderError::InvalidResponse);
                    }
                }
            }
        }
        let index = tool_call_slot(state, delta);
        let entry = state.pending_tool_calls.entry(index).or_default();
        if let Some(id) = delta.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                entry.call_id = id.to_owned();
            }
        }
        if let Some(function) = delta.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if !name.is_empty() {
                    entry.name = name.to_owned();
                }
            }
            if let Some(fragment) = function.get("arguments").and_then(Value::as_str) {
                entry.arguments.push_str(fragment);
            }
        }
    }
    Ok(())
}

/// Normalizes one `data:` payload.
///
/// `Err` means the stream is over and the run failed: an aggregator that has
/// already sent response headers cannot change the status code, so it reports a
/// mid-stream failure as an `error` object (or `finish_reason: "error"`) inside
/// a 200 body. Treating that as an unparseable chunk would strand the run —
/// text just stops with no failure anyone can surface.
pub(super) fn parse_event(
    data: &str,
    state: &mut ParseState,
) -> Result<Vec<ProviderStreamEvent>, ProviderError> {
    if state.done {
        return Ok(Vec::new());
    }
    if data.trim().is_empty() {
        return Ok(Vec::new());
    }
    if data == "[DONE]" {
        state.done = true;
        // Some OpenAI-compatible gateways close with `[DONE]` and never send
        // `finish_reason: "tool_calls"`. Flushing here rather than dropping
        // keeps those gateways from silently stranding a run.
        let truncated = state.stop.is_some_and(ProviderStopReason::is_truncation);
        let mut events = if truncated {
            state.flush_complete_tool_calls()
        } else {
            state.flush_tool_calls()?
        };
        // Thinking cut off mid-block has no signature to echo back.
        if !truncated {
            events.extend(state.flush_reasoning_details());
        }
        events.push(ProviderStreamEvent::Stopped {
            reason: state.stop.unwrap_or(ProviderStopReason::Natural),
            discarded_tool_calls: state.discarded_tool_calls,
        });
        events.push(ProviderStreamEvent::Completed);
        return Ok(events);
    }

    let parsed = serde_json::from_str::<Value>(data).map_err(|_| ProviderError::InvalidResponse)?;

    if let Some(error) = parsed.get("error") {
        state.done = true;
        return Err(classify_stream_error(error));
    }

    let mut events = Vec::new();
    let first_choice = match parsed.get("choices") {
        Some(choices) => {
            let choices = choices.as_array().ok_or(ProviderError::InvalidResponse)?;
            choices.first()
        }
        None => None,
    };
    if let Some(choice) = first_choice {
        if !choice.is_object() {
            return Err(ProviderError::InvalidResponse);
        }
        if let Some(delta) = choice.get("delta") {
            if !delta.is_object() {
                return Err(ProviderError::InvalidResponse);
            }
            // Reasoning precedes content for every model that emits both, and
            // is passed through as its own event so the run record can keep
            // them apart. `openrouter.rs` DOES request it (`reasoning:
            // {enabled: true}` for every shipped vendor), which obliges the
            // tool loop to echo `reasoning_details` back verbatim on later
            // turns — `stream_turn` collects them for exactly that. A model
            // that reasons unprompted on any other dialect still reports it
            // here rather than having it dropped.
            // Ollama-backed gateways name the same field `reasoning_content`.
            let thinking = delta
                .get("reasoning")
                .or_else(|| delta.get("reasoning_content"))
                .and_then(Value::as_str);
            if let Some(text) = thinking {
                if !text.is_empty() {
                    events.push(ProviderStreamEvent::ReasoningDelta(text.to_owned()));
                }
            }
            // The structured form of the same thinking. Read for the replay,
            // never for display: `delta.reasoning` above is what the reader
            // sees, and a block whose payload is encrypted has no display form
            // at all.
            if let Some(details) = delta.get("reasoning_details").and_then(Value::as_array) {
                accumulate_reasoning_details(state, details);
                if details.iter().any(|detail| {
                    ["text", "data", "signature"].iter().any(|field| {
                        detail[*field]
                            .as_str()
                            .is_some_and(|fragment| !fragment.is_empty())
                    })
                }) {
                    events.push(ProviderStreamEvent::Progress);
                }
            }
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    events.push(ProviderStreamEvent::TextDelta(text.to_owned()));
                }
            }
            if let Some(tool_calls_value) = delta.get("tool_calls").filter(|value| !value.is_null())
            {
                let tool_calls = tool_calls_value
                    .as_array()
                    .ok_or(ProviderError::InvalidResponse)?;
                accumulate_tool_calls(state, tool_calls)?;
                if tool_calls.iter().any(|call| {
                    call.pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .is_some_and(|fragment| !fragment.is_empty())
                }) {
                    events.push(ProviderStreamEvent::Progress);
                }
            }
        }
        match choice.get("finish_reason").and_then(Value::as_str) {
            Some("tool_calls") => {
                state.stop = Some(ProviderStopReason::ToolUse);
                events.extend(state.flush_tool_calls()?);
                events.extend(state.flush_reasoning_details());
            }
            // The output ceiling ended the turn, possibly inside a call's
            // arguments. Ignoring it would emit half-written JSON as a
            // finished call and report the turn as a normal stop.
            Some("length") => {
                state.stop = Some(ProviderStopReason::OutputLimit);
                events.extend(state.flush_complete_tool_calls());
            }
            Some("content_filter") => state.stop = Some(ProviderStopReason::Refusal),
            Some("stop") => state.stop = Some(ProviderStopReason::Natural),
            // Reached only when the gateway sent no `error` object alongside
            // it; with one, the branch above already returned the classified
            // error.
            Some("error") => {
                state.done = true;
                return Err(ProviderError::Unavailable);
            }
            _ => {}
        }
    }

    // `include_usage` makes a server send `"usage": null` on every content
    // chunk and the real object once, on the last. Only the object is usage.
    if let Some(usage) = parsed.get("usage").filter(|usage| usage.is_object()) {
        let input_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let output_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        if let Some(tokens) = input_tokens {
            events.push(ProviderStreamEvent::PromptSize {
                tokens: u64::from(tokens),
            });
        }
        events.push(ProviderStreamEvent::Usage {
            input_tokens,
            output_tokens,
        });
    }

    Ok(events)
}

/// Maps a mid-stream `error` object onto the same taxonomy a status code gets.
///
/// `code` is an HTTP status for every gateway this module speaks to, so reusing
/// [`classify_status_code`] keeps one table rather than two that can disagree.
/// A non-numeric code (`"server_error"`) carries no such signal, so it lands on
/// the retryable arm — which is what an aggregator dropping its upstream is.
/// `Retry-After` is unavailable here: the headers were sent long before this
/// chunk, so backoff falls to [`super::retry`].
fn classify_stream_error(error: &Value) -> ProviderError {
    let status = error
        .get("code")
        .and_then(Value::as_u64)
        .and_then(|code| u16::try_from(code).ok());
    if super::http::can_be_context_overflow(status)
        && super::http::is_context_overflow_text(&error.to_string().to_ascii_lowercase())
    {
        return ProviderError::ContextOverflow;
    }
    status.map_or(ProviderError::Unavailable, |status| {
        classify_status_code(status, None::<Duration>)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_user_turn_with_an_image_renders_text_then_a_data_url_part() {
        let message = crate::agent::context::ContextMessage::user_with_attachments(
            "What does this chart show?",
            vec![crate::agent::context::ContextAttachment::Image {
                media_type: crate::agent::context::ImageMediaType::Png,
                bytes: vec![1, 2, 3],
            }],
        );
        let rendered = super::render_message(&message);
        assert_eq!(rendered["role"], "user");
        assert_eq!(
            rendered["content"][0],
            serde_json::json!({"type":"text","text":"What does this chart show?"})
        );
        assert_eq!(rendered["content"][1]["type"], "image_url");
        assert_eq!(
            rendered["content"][1]["image_url"]["url"],
            "data:image/png;base64,AQID"
        );
    }

    #[test]
    fn a_text_turn_still_renders_as_a_plain_string() {
        let message = crate::agent::context::ContextMessage::new(
            crate::agent::context::ContextRole::User,
            "hi",
        );
        assert_eq!(super::render_message(&message)["content"], "hi");
    }

    struct SuggestionTestAdapter(String);

    #[async_trait::async_trait]
    impl crate::providers::ProviderAdapter for SuggestionTestAdapter {
        fn provider_id(&self) -> xfalcon_contracts::credentials::ProviderId {
            xfalcon_contracts::credentials::ProviderId::OpenAi
        }

        async fn stream(
            &self,
            request: &super::ProviderChatRequest,
            secret: &str,
            cancel: &tokio_util::sync::CancellationToken,
            on_event: &mut (dyn FnMut(super::ProviderStreamEvent) + Send),
        ) -> Result<(), super::ProviderError> {
            let dialect = super::CompatDialect {
                request_usage_via_stream_options: true,
            };
            let body = super::build_request_body(request, &dialect);
            let builder = reqwest::Client::new().post(&self.0).bearer_auth(secret);
            super::stream_completion(builder, &body, cancel, on_event).await
        }
    }

    #[tokio::test]
    async fn stop_cancels_a_provider_waiting_for_response_headers() {
        use crate::providers::{retry::RetryingAdapter, ProviderAdapter};
        use std::{sync::Arc, time::Duration};
        use tokio_util::sync::CancellationToken;
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = RetryingAdapter::new(Arc::new(SuggestionTestAdapter(server.uri())));
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            let request = super::ProviderChatRequest {
                model_id: "test-model".to_owned(),
                messages: Vec::new(),
                max_output_tokens: Some(128),
                allow_reasoning: false,
                tools: Vec::new(),
                openrouter: None,
            };
            adapter
                .stream(&request, "test-secret", &task_cancel, &mut |_| {})
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !server
                    .received_requests()
                    .await
                    .expect("requests")
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("provider request reaches the HTTP server");
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("Stop must not wait for provider response headers")
            .expect("provider task joins");
        assert_eq!(result, Err(super::ProviderError::Cancelled));
    }

    #[tokio::test]
    async fn follow_up_generation_uses_bounded_tool_free_provider_request() {
        use wiremock::{
            matchers::{header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let chunk = serde_json::json!({"choices":[{"delta":{"reasoning":"This is private reasoning, not a chip", "content":"[\"Compare regional revenue\",\"Check margin drivers\",\"Chart weekly sales\"]"}}]});
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-secret"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let suggestions = crate::chat::follow_ups::generate_suggestions(
            &SuggestionTestAdapter(format!("{}/chat/completions", server.uri())),
            "model-id",
            "Regional revenue rose 10% while margins fell.",
            Some("Review revenue"),
            "test-secret",
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            suggestions,
            vec![
                "Compare regional revenue",
                "Check margin drivers",
                "Chart weekly sales"
            ]
        );
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "model-id");
        assert_eq!(body["max_tokens"], 180);
        assert!(body.get("tools").is_none());
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("ETL, reporting and dashboard assistant"));
        assert!(system.contains("Never suggest translation"));
        assert!(system.contains("lack an actionable analytics topic, return []"));
        assert!(body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Regional revenue rose 10%"));
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn follow_up_generation_rejects_partial_or_oversized_provider_output() {
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};
        for (text, done) in [
            ("[\"Compare revenue\"]".to_owned(), false),
            ("x".repeat(5000), true),
        ] {
            let server = MockServer::start().await;
            let chunk = serde_json::json!({"choices":[{"delta":{"content":text}}]});
            let end = if done { "data: [DONE]\n\n" } else { "" };
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(format!("data: {chunk}\n\n{end}")),
                )
                .expect(1)
                .mount(&server)
                .await;
            let result = crate::chat::follow_ups::generate_suggestions(
                &SuggestionTestAdapter(server.uri()),
                "model-id",
                "Revenue rose",
                None,
                "test-secret",
                &tokio_util::sync::CancellationToken::new(),
            )
            .await;
            assert!(result.is_err());
        }
    }

    use super::super::adapter::ProviderToolSchema;
    use super::*;
    use crate::agent::context::ContextToolCall;

    fn openai_dialect() -> CompatDialect {
        CompatDialect {
            request_usage_via_stream_options: true,
        }
    }

    fn parse(data: &str, state: &mut ParseState) -> Vec<ProviderStreamEvent> {
        parse_event(data, state).expect("chunk is not a failure")
    }

    /// Omitted, not defaulted: the provider then applies the model's own
    /// maximum. A number here would be the ceiling the model's thinking shares
    /// with its answer, which is how a thinking turn truncates its own reply.
    #[test]
    fn no_ceiling_asked_for_means_no_max_tokens_key_at_all() {
        let request = ProviderChatRequest {
            model_id: "gpt-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: None,
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };
        assert!(build_request_body(&request, &openai_dialect())
            .get("max_tokens")
            .is_none());
    }

    #[test]
    fn an_explicit_ceiling_is_still_sent() {
        let request = ProviderChatRequest {
            model_id: "gpt-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(512),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };
        assert_eq!(
            build_request_body(&request, &openai_dialect())["max_tokens"],
            json!(512)
        );
    }

    /// The replay contract in one test: fragments of one indexed block merge
    /// into one block, the payload concatenates, a `signature` arriving on a
    /// later fragment is kept, and repeated scalars are not appended to
    /// themselves. A block echoed back mis-merged is a 400, not a degraded
    /// answer.
    #[test]
    fn reasoning_detail_fragments_merge_into_one_block_per_index() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"First ","id":"r-1","format":"anthropic-claude-v1","index":0}]}}]}"#,
            &mut state,
        );
        parse(
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"then.","id":"r-1","format":"anthropic-claude-v1","index":0,"signature":"sig-abc"}]}}]}"#,
            &mut state,
        );
        let events = parse(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut state,
        );
        assert!(events.is_empty());

        let events = parse("[DONE]", &mut state);
        let blocks = events
            .iter()
            .find_map(|event| match event {
                ProviderStreamEvent::ReasoningDetails(blocks) => Some(blocks.clone()),
                _ => None,
            })
            .expect("reasoning details are flushed at the end of the turn");

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["text"], json!("First then."));
        assert_eq!(blocks[0]["signature"], json!("sig-abc"));
        assert_eq!(blocks[0]["id"], json!("r-1"));
        assert_eq!(blocks[0]["format"], json!("anthropic-claude-v1"));
    }

    /// Ordering is the other half of the contract: the replay must match the
    /// sequence the model produced, and chunks do not arrive index-ordered.
    #[test]
    fn reasoning_blocks_flush_in_index_order_not_arrival_order() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"second","index":1}]}}]}"#,
            &mut state,
        );
        parse(
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.encrypted","data":"first","index":0}]}}]}"#,
            &mut state,
        );
        let events = parse("[DONE]", &mut state);
        let blocks = events
            .iter()
            .find_map(|event| match event {
                ProviderStreamEvent::ReasoningDetails(blocks) => Some(blocks.clone()),
                _ => None,
            })
            .expect("reasoning details are flushed");

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["data"], json!("first"));
        assert_eq!(blocks[1]["text"], json!("second"));
    }

    /// A tool round is exactly where the replay matters, and `tool_calls` is
    /// the finish reason that ends it.
    #[test]
    fn reasoning_blocks_flush_on_a_tool_call_turn() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"I should search","index":0}]}}]}"#,
            &mut state,
        );
        let events = parse(
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            &mut state,
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::ReasoningDetails(_))));
    }

    #[test]
    fn a_turn_without_reasoning_emits_no_reasoning_details_event() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"content":"Answer"}}]}"#,
            &mut state,
        );
        let events = parse("[DONE]", &mut state);
        assert!(!events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::ReasoningDetails(_))));
    }

    /// Echoed verbatim, and absent entirely when there is nothing to echo — so
    /// a request for a provider that streams no reasoning is byte-identical to
    /// what it was before the field existed.
    #[test]
    fn an_assistant_turn_echoes_its_reasoning_blocks_unchanged() {
        let blocks = vec![json!({
            "type": "reasoning.text",
            "text": "why I chose this",
            "signature": "sig-abc",
            "index": 0
        })];
        let rendered = render_message(
            &ContextMessage::assistant_tool_calls(
                "looking",
                vec![ContextToolCall::new("call-1", "search", "{}")],
            )
            .with_reasoning_details(blocks.clone()),
        );
        assert_eq!(rendered["reasoning_details"], json!(blocks));

        let plain = render_message(&ContextMessage::new(ContextRole::Assistant, "no thinking"));
        assert!(plain.get("reasoning_details").is_none());
    }

    #[test]
    fn parses_text_delta() {
        let mut state = ParseState::default();
        let events = parse(
            r#"{"choices":[{"delta":{"content":"Hel"},"index":0}]}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![ProviderStreamEvent::TextDelta("Hel".to_owned())]
        );
    }

    #[test]
    fn parses_done_sentinel_once() {
        let mut state = ParseState::default();
        assert_eq!(
            parse("[DONE]", &mut state),
            vec![
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0
                },
                ProviderStreamEvent::Completed
            ]
        );
        assert!(state.done);
        assert!(parse(r#"{"choices":[{"delta":{"content":"late"}}]}"#, &mut state).is_empty());
    }

    #[test]
    fn parses_final_usage_chunk_with_no_choices() {
        let mut state = ParseState::default();
        let events = parse(
            r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::PromptSize { tokens: 12 },
                ProviderStreamEvent::Usage {
                    input_tokens: Some(12),
                    output_tokens: Some(34)
                }
            ]
        );
    }

    #[test]
    fn malformed_json_fails_the_stream() {
        let mut state = ParseState::default();
        assert_eq!(parse_event("", &mut state), Ok(Vec::new()));
        assert_eq!(
            parse_event("not json", &mut state),
            Err(ProviderError::InvalidResponse)
        );
    }

    #[test]
    fn malformed_recognized_tool_call_fails_instead_of_emitting_an_incomplete_call() {
        let mut state = ParseState::default();
        assert_eq!(
            parse_event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"read-a","function":[]}]}}]}"#,
                &mut state,
            ),
            Err(ProviderError::InvalidResponse)
        );

        let mut state = ParseState::default();
        assert!(parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"read-a"}]}}]}"#,
            &mut state,
        )
        .is_empty());
        assert_eq!(
            parse_event(
                r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
                &mut state,
            ),
            Err(ProviderError::InvalidResponse)
        );
    }

    /// The wire streams one tool call across many chunks: the id and name land
    /// on the first delta for a given `index`, then `arguments` arrives as
    /// raw JSON fragments that must be concatenated in arrival order. No call
    /// may be emitted until `finish_reason` closes it, or the loop above
    /// would dispatch a tool with truncated arguments.
    #[test]
    fn accumulates_a_tool_call_across_argument_deltas() {
        let mut state = ParseState::default();

        assert!(parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"project_search","arguments":""}}]}}]}"#,
            &mut state,
        )
        .is_empty());
        assert_eq!(
            parse(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":"}}]}}]}"#,
                &mut state,
            ),
            vec![ProviderStreamEvent::Progress]
        );
        assert_eq!(
            parse(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"budget\"}"}}]}}]}"#,
                &mut state,
            ),
            vec![ProviderStreamEvent::Progress]
        );

        let events = parse(
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            &mut state,
        );
        assert_eq!(
            events,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "call_1".to_owned(),
                name: "project_search".to_owned(),
                arguments: r#"{"query":"budget"}"#.to_owned(),
            }]
        );
    }

    #[test]
    fn empty_argument_and_reasoning_frames_do_not_count_as_progress() {
        let mut state = ParseState::default();
        assert!(parse(
            r#"{"choices":[{"delta":{"role":"assistant","reasoning_details":[{"index":0,"type":"reasoning.encrypted","data":""}],"tool_calls":[{"index":0,"id":"edit","function":{"name":"project_propose_patch","arguments":""}}]}}]}"#,
            &mut state,
        ).is_empty());
        assert_eq!(
            parse(
                r#"{"choices":[{"delta":{"reasoning_details":[{"index":0,"type":"reasoning.encrypted","data":"opaque"}]}}]}"#,
                &mut state,
            ),
            vec![ProviderStreamEvent::Progress]
        );
        let events = parse("[DONE]", &mut state);
        assert!(events.contains(&ProviderStreamEvent::ReasoningDetails(vec![
            json!({"index":0,"type":"reasoning.encrypted","data":"opaque"})
        ])));
    }

    #[test]
    fn emits_parallel_tool_calls_in_index_order() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"b","arguments":"{}"}}]}}]}"#,
            &mut state,
        );
        parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"a","arguments":"{}"}}]}}]}"#,
            &mut state,
        );

        let events = parse(
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            &mut state,
        );

        let ids: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                ProviderStreamEvent::ToolCallRequested { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ids,
            vec!["call_a", "call_b"],
            "index is the wire's ordering key, not arrival order"
        );
    }

    /// Some OpenAI-compatible gateways close the stream with `[DONE]` and
    /// never send `finish_reason":"tool_calls"`. Dropping the accumulated
    /// call there would strand the run silently.
    #[test]
    fn flushes_pending_tool_calls_on_the_done_sentinel() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"x","arguments":"{}"}}]}}]}"#,
            &mut state,
        );

        let events = parse("[DONE]", &mut state);

        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "call_1".to_owned(),
                    name: "x".to_owned(),
                    arguments: "{}".to_owned(),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    /// The output ceiling can land inside any call's arguments. A half-written
    /// body must never reach the tool loop as if the model had finished it.
    #[test]
    fn a_length_stop_keeps_finished_calls_and_drops_half_written_ones() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"path\":\"a\"}"}},{"index":1,"id":"call_2","function":{"name":"write","arguments":"{\"path\":\"b.html\",\"content\":\"<ht"}}]}}]}"#,
            &mut state,
        );
        let finish = parse(
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
            &mut state,
        );
        assert_eq!(
            finish,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "call_1".to_owned(),
                name: "read".to_owned(),
                arguments: r#"{"path":"a"}"#.to_owned(),
            }]
        );
        assert_eq!(
            parse("[DONE]", &mut state),
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
    fn empty_streamed_arguments_normalize_to_an_empty_json_object() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"x","arguments":""}}]}}]}"#,
            &mut state,
        );

        let events = parse(
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            &mut state,
        );

        assert_eq!(
            events,
            vec![ProviderStreamEvent::ToolCallRequested {
                call_id: "call_1".to_owned(),
                name: "x".to_owned(),
                arguments: "{}".to_owned(),
            }],
            "a no-argument tool must produce parseable JSON, not an empty string"
        );
    }

    #[test]
    fn declares_tools_in_openai_function_shape() {
        let request = ProviderChatRequest {
            model_id: "gpt-x".to_owned(),
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

        let body = build_request_body(&request, &openai_dialect());

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "project_search");
        assert_eq!(
            body["tools"][0]["function"]["parameters"],
            json!({"type":"object","properties":{}})
        );
    }

    #[test]
    fn omits_the_tools_key_entirely_when_no_tool_is_available() {
        let request = ProviderChatRequest {
            model_id: "gpt-x".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request, &openai_dialect());

        assert!(
            body.get("tools").is_none(),
            "an empty tools array is rejected by some OpenAI-compatible gateways"
        );
    }

    #[test]
    fn round_trips_an_assistant_tool_call_and_its_result() {
        let request = ProviderChatRequest {
            model_id: "gpt-x".to_owned(),
            messages: vec![
                ContextMessage::new(ContextRole::User, "search"),
                ContextMessage::assistant_tool_calls(
                    "",
                    vec![ContextToolCall::new("call_1", "project_search", "{}")],
                ),
                ContextMessage::tool_result("call_1", "project_search", "3 matches"),
            ],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(&request, &openai_dialect());
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["arguments"], "{}",
            "OpenAI takes function arguments as a JSON *string*, not an object"
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(
            messages[2]["tool_call_id"], "call_1",
            "an unkeyed tool message is rejected by the Chat Completions API"
        );
        assert_eq!(messages[2]["content"], "3 matches");
    }

    #[test]
    fn empty_text_delta_emits_nothing() {
        let mut state = ParseState::default();
        assert!(parse(r#"{"choices":[{"delta":{"content":""}}]}"#, &mut state).is_empty());
    }

    #[test]
    fn a_dialect_that_does_not_want_stream_options_omits_the_key() {
        let request = ProviderChatRequest {
            model_id: "vendor/model".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };

        let body = build_request_body(
            &request,
            &CompatDialect {
                request_usage_via_stream_options: false,
            },
        );

        assert_eq!(body["stream"], true);
        assert!(
            body.get("stream_options").is_none(),
            "a gateway that reports usage unconditionally must not be sent the deprecated flag"
        );
    }

    /// A reasoning model streams its thinking in `delta.reasoning` alongside
    /// `delta.content`. Reasoning must arrive as its own event, before the
    /// answer text of the same chunk.
    #[test]
    fn reasoning_deltas_are_reported_separately_and_first() {
        let mut state = ParseState::default();

        let events = parse(
            r#"{"choices":[{"delta":{"reasoning":"weighing","content":"Answer"}}]}"#,
            &mut state,
        );

        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::ReasoningDelta("weighing".to_owned()),
                ProviderStreamEvent::TextDelta("Answer".to_owned()),
            ]
        );
    }

    #[test]
    fn a_stream_that_closes_after_its_finish_reason_still_completes() {
        for (finish, expected) in [
            ("stop", ProviderStopReason::Natural),
            ("length", ProviderStopReason::OutputLimit),
        ] {
            let mut state = ParseState::default();
            parse(
                &format!(
                    r#"{{"choices":[{{"delta":{{"content":"hi"}},"finish_reason":"{finish}"}}]}}"#
                ),
                &mut state,
            );

            let events = close_without_done(&mut state, std::time::Duration::ZERO);

            assert!(
                events.iter().any(|event| matches!(
                    event,
                    ProviderStreamEvent::Stopped { reason, .. } if *reason == expected
                )),
                "{finish}: {events:?}"
            );
            assert_eq!(
                events.last(),
                Some(&ProviderStreamEvent::Completed),
                "{finish}"
            );
            assert!(
                close_without_done(&mut state, std::time::Duration::ZERO).is_empty(),
                "closing is idempotent"
            );
        }
    }

    #[test]
    fn a_stream_cut_off_before_any_finish_reason_is_a_cut_off_turn_not_a_failure() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{"content":"Here is the file","tool_calls":[{"index":0,"id":"done1","function":{"name":"read_file","arguments":"{}"}}]}}]}"#,
            &mut state,
        );
        parse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c1","function":{"name":"write_file","arguments":"{\"path\":"}}]}}]}"#,
            &mut state,
        );

        let events = close_without_done(&mut state, std::time::Duration::ZERO);

        assert!(
            events.iter().any(|event| matches!(
                event,
                ProviderStreamEvent::ToolCallRequested { call_id, .. } if call_id == "done1"
            )),
            "a finished call is kept: {events:?}"
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                ProviderStreamEvent::ToolCallRequested { call_id, .. } if call_id == "c1"
            )),
            "a half-written call is not run: {events:?}"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::StreamCut,
                discarded_tool_calls: 1,
            }
        )));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
    }

    fn finished_calls(chunks: &[&str]) -> Vec<(String, String, String)> {
        let mut state = ParseState::default();
        let mut calls = Vec::new();
        for chunk in chunks {
            for event in parse(chunk, &mut state) {
                if let ProviderStreamEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments,
                } = event
                {
                    calls.push((call_id, name, arguments));
                }
            }
        }
        calls
    }

    #[test]
    fn parallel_calls_without_an_index_stay_separate() {
        let calls = finished_calls(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"b","function":{"name":"read_file","arguments":"{\"path\":\"b\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);

        assert_eq!(
            calls,
            vec![
                ("a".into(), "read_file".into(), r#"{"path":"a"}"#.into()),
                ("b".into(), "read_file".into(), r#"{"path":"b"}"#.into()),
            ]
        );
    }

    #[test]
    fn parallel_calls_that_all_claim_index_zero_stay_separate() {
        let calls = finished_calls(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"b","function":{"name":"list_files","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);

        assert_eq!(
            calls,
            vec![
                ("a".into(), "read_file".into(), r#"{"path":"a"}"#.into()),
                ("b".into(), "list_files".into(), "{}".into()),
            ]
        );
    }

    #[test]
    fn one_call_streamed_in_pieces_is_still_one_call() {
        // With an index, without one, and with the id repeated on every piece.
        for chunks in [
            [
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"write_file","arguments":"{\"pa"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":1}"}}]}}]}"#,
            ],
            [
                r#"{"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"write_file","arguments":"{\"pa"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"th\":1}"}}]}}]}"#,
            ],
            [
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"write_file","arguments":"{\"pa"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"arguments":"th\":1}"}}]}}]}"#,
            ],
        ] {
            let mut all = chunks.to_vec();
            all.push(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);
            assert_eq!(
                finished_calls(&all),
                vec![("a".into(), "write_file".into(), r#"{"path":1}"#.into())],
                "{chunks:?}"
            );
        }
    }

    /// `stream_options.include_usage` makes a server send `"usage": null` on
    /// every content chunk. None of those is a usage report, and none may
    /// overwrite the real one that arrives last.
    #[test]
    fn a_null_usage_field_is_not_a_usage_report() {
        let mut state = ParseState::default();
        let chunk = parse(
            r#"{"choices":[{"delta":{"content":"hi"}}],"usage":null}"#,
            &mut state,
        );
        assert_eq!(chunk, vec![ProviderStreamEvent::TextDelta("hi".to_owned())]);

        let last = parse(
            r#"{"choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":34,"total_tokens":1234}}"#,
            &mut state,
        );
        assert_eq!(
            last,
            vec![
                ProviderStreamEvent::PromptSize { tokens: 1200 },
                ProviderStreamEvent::Usage {
                    input_tokens: Some(1200),
                    output_tokens: Some(34),
                },
            ]
        );
    }

    #[test]
    fn a_stream_that_ended_with_done_is_not_closed_twice() {
        let mut state = ParseState::default();
        parse(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut state,
        );
        parse("[DONE]", &mut state);

        assert!(close_without_done(&mut state, std::time::Duration::ZERO).is_empty());
    }

    /// Ollama-backed gateways stream thinking as `reasoning_content`.
    #[test]
    fn reasoning_content_is_read_as_thinking() {
        let mut state = ParseState::default();
        let events = parse(
            r#"{"choices":[{"delta":{"reasoning_content":"weighing","content":"Answer"}}]}"#,
            &mut state,
        );

        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::ReasoningDelta("weighing".to_owned()),
                ProviderStreamEvent::TextDelta("Answer".to_owned()),
            ]
        );
    }

    #[test]
    fn empty_reasoning_delta_emits_nothing() {
        let mut state = ParseState::default();
        assert!(parse(r#"{"choices":[{"delta":{"reasoning":""}}]}"#, &mut state).is_empty());
    }

    /// A gateway that has already sent 200 headers reports a failure inside the
    /// body. Its `code` is an HTTP status, so it classifies exactly as the same
    /// status would have on the response itself.
    #[test]
    fn a_mid_stream_error_object_fails_the_run_with_its_status_meaning() {
        for (code, expected) in [
            (401_u16, ProviderError::AuthenticationFailed),
            (429, ProviderError::RateLimited { retry_after: None }),
            (400, ProviderError::InvalidResponse),
            (502, ProviderError::Unavailable),
        ] {
            let mut state = ParseState::default();
            let chunk = format!(r#"{{"error":{{"code":{code},"message":"upstream"}}}}"#);

            assert_eq!(
                parse_event(&chunk, &mut state),
                Err(expected),
                "a mid-stream {code} must fail the run, not be skipped"
            );
            assert!(state.done, "the stream is over once it has failed");
        }
    }

    /// A throttle or an outage whose message mentions tokens or a window is
    /// not a prompt that was too long.
    #[test]
    fn a_mid_stream_error_with_a_non_rejection_status_is_not_a_context_overflow() {
        for (code, expected) in [
            (429_u16, ProviderError::RateLimited { retry_after: None }),
            (502, ProviderError::Unavailable),
            (503, ProviderError::Unavailable),
        ] {
            let mut state = ParseState::default();
            let chunk = format!(
                r#"{{"error":{{"code":{code},"message":"Too many tokens, please wait. Context window busy."}}}}"#
            );
            assert_eq!(parse_event(&chunk, &mut state), Err(expected), "{code}");
        }
        for code in [400_u16, 413, 422] {
            let mut state = ParseState::default();
            let chunk = format!(
                r#"{{"error":{{"code":{code},"message":"maximum context length is 8192 tokens"}}}}"#
            );
            assert_eq!(
                parse_event(&chunk, &mut state),
                Err(ProviderError::ContextOverflow),
                "{code}"
            );
        }
        let mut state = ParseState::default();
        assert_eq!(
            parse_event(
                r#"{"error":{"code":"context_length_exceeded","message":"x"}}"#,
                &mut state
            ),
            Err(ProviderError::ContextOverflow)
        );
    }

    #[test]
    fn a_non_numeric_mid_stream_error_code_is_treated_as_retryable() {
        let mut state = ParseState::default();

        assert_eq!(
            parse_event(
                r#"{"error":{"code":"server_error","message":"provider disconnected"}}"#,
                &mut state,
            ),
            Err(ProviderError::Unavailable)
        );
    }

    /// The documented shape carries both an `error` object and
    /// `finish_reason: "error"`; the object is the more specific signal.
    #[test]
    fn an_error_object_wins_over_the_bare_error_finish_reason() {
        let mut state = ParseState::default();

        assert_eq!(
            parse_event(
                r#"{"error":{"code":429,"message":"rate limited"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}"#,
                &mut state,
            ),
            Err(ProviderError::RateLimited { retry_after: None })
        );
    }

    #[test]
    fn a_bare_error_finish_reason_still_fails_the_run() {
        let mut state = ParseState::default();

        assert_eq!(
            parse_event(
                r#"{"choices":[{"index":0,"delta":{},"finish_reason":"error"}]}"#,
                &mut state,
            ),
            Err(ProviderError::Unavailable)
        );
    }

    /// Serves one response that promises more bytes than it sends, then drops
    /// the connection: what a proxy that resets a long request does.
    async fn consume_a_reset_response(
        sent: &str,
    ) -> (Result<(), ProviderError>, Vec<ProviderStreamEvent>) {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let body = sent.to_owned();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0u8; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
                body.len() + 1000
            );
            socket.write_all(head.as_bytes()).await.expect("head");
            socket.write_all(body.as_bytes()).await.expect("body");
            socket.flush().await.expect("flush");
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("headers arrive");
        let mut events = Vec::new();
        let outcome = consume_completion_stream(
            response,
            &tokio_util::sync::CancellationToken::new(),
            DEFAULT_STREAM_BYTES,
            &mut |event| events.push(event),
        )
        .await;
        (outcome, events)
    }

    /// Serves one complete stream of `lines` filler chunks and `[DONE]`.
    async fn consume_a_big_stream(lines: usize, byte_limit: usize) -> Result<(), ProviderError> {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0u8; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
                .await
                .expect("head");
            let chunk = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"x\"}}]}\n\n".repeat(1_000);
            for _ in 0..lines / 1_000 {
                if socket.write_all(chunk.as_bytes()).await.is_err() {
                    return;
                }
            }
            let _ = socket.write_all(b"data: [DONE]\n\n").await;
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("headers arrive");
        consume_completion_stream(
            response,
            &tokio_util::sync::CancellationToken::new(),
            byte_limit,
            &mut |_| {},
        )
        .await
    }

    #[tokio::test]
    async fn a_stream_past_the_default_bound_is_refused_unless_the_caller_raises_it() {
        // About 20 MB of small chunks, as a long thinking turn from a server
        // that wraps every token in a large envelope.
        assert_eq!(
            consume_a_big_stream(400_000, DEFAULT_STREAM_BYTES).await,
            Err(ProviderError::InvalidResponse)
        );
        assert_eq!(
            consume_a_big_stream(400_000, crate::providers::gateway::STREAM_BYTE_LIMIT).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_connection_that_breaks_after_output_began_is_a_cut_off_turn() {
        let (outcome, events) = consume_a_reset_response(
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        )
        .await;

        assert_eq!(outcome, Ok(()));
        assert!(events.contains(&ProviderStreamEvent::TextDelta("partial".to_owned())));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderStreamEvent::Stopped {
                reason: ProviderStopReason::StreamCut,
                ..
            }
        )));
        assert_eq!(events.last(), Some(&ProviderStreamEvent::Completed));
    }

    #[tokio::test]
    async fn a_connection_that_breaks_before_any_output_is_a_failure() {
        let (outcome, events) = consume_a_reset_response("").await;

        assert_eq!(outcome, Err(ProviderError::Unavailable));
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn a_connection_reset_after_only_partial_arguments_remains_retryable() {
        let (outcome, events) = consume_a_reset_response(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"edit\",\"function\":{\"name\":\"project_propose_patch\",\"arguments\":\"{\"}}]}}]}\n\n",
        ).await;
        assert_eq!(outcome, Err(ProviderError::Unavailable));
        assert_eq!(events, vec![ProviderStreamEvent::Progress]);
    }

    #[tokio::test]
    async fn a_corrupt_frame_between_parallel_calls_fails_before_any_call_can_run() {
        use crate::providers::ProviderAdapter;
        use std::sync::Mutex;
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"read-a\",\"function\":{\"name\":\"project_read_file\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"read-b\",\"function\":[]}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = SuggestionTestAdapter(server.uri());
        let request = ProviderChatRequest {
            model_id: "test-model".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "read both")],
            max_output_tokens: Some(128),
            allow_reasoning: false,
            tools: Vec::new(),
            openrouter: None,
        };
        let events = Mutex::new(Vec::new());
        let outcome = adapter
            .stream(
                &request,
                "test-secret",
                &CancellationToken::new(),
                &mut |event| events.lock().expect("events").push(event),
            )
            .await;

        assert_eq!(outcome, Err(ProviderError::InvalidResponse));
        assert!(events
            .into_inner()
            .expect("events")
            .into_iter()
            .all(|event| !matches!(event, ProviderStreamEvent::ToolCallRequested { .. })));
    }

    #[tokio::test]
    async fn nullable_optional_fields_preserve_two_parallel_calls_over_real_http() {
        use crate::providers::ProviderAdapter;
        use std::sync::Mutex;
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Working.\",\"tool_calls\":null}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"read-a\",\"type\":\"function\",\"function\":{\"name\":\"project_read_file\",\"arguments\":\"{\\\"path\\\":\"}},{\"index\":1,\"id\":\"read-b\",\"type\":\"function\",\"function\":{\"name\":\"project_read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":null,\"type\":null,\"function\":{\"name\":null,\"arguments\":\"\\\"a.csv\\\"}\"}},{\"index\":1,\"id\":null,\"type\":null,\"function\":{\"name\":null,\"arguments\":\"\\\"b.csv\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":null,\"type\":null,\"function\":{\"name\":null,\"arguments\":null}},{\"index\":1,\"id\":null,\"type\":null,\"function\":{\"name\":null,\"arguments\":null}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"\",\"function\":{\"name\":\"\",\"arguments\":\"\"}},{\"index\":1,\"id\":\"\",\"function\":{\"name\":\"\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":null,\"type\":null,\"function\":null},{\"index\":1,\"id\":null,\"type\":null,\"function\":null}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = SuggestionTestAdapter(server.uri());
        let request = ProviderChatRequest {
            model_id: "test-model".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "read both")],
            max_output_tokens: Some(128),
            allow_reasoning: false,
            tools: Vec::new(),
            openrouter: None,
        };
        let events = Mutex::new(Vec::new());

        let outcome = adapter
            .stream(
                &request,
                "test-secret",
                &CancellationToken::new(),
                &mut |event| events.lock().expect("events").push(event),
            )
            .await;

        assert_eq!(outcome, Ok(()));
        let events = events.into_inner().expect("events");
        assert!(events.contains(&ProviderStreamEvent::TextDelta("Working.".to_owned())));
        let calls: Vec<_> = events
            .into_iter()
            .filter_map(|event| match event {
                ProviderStreamEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments,
                } => Some((call_id, name, arguments)),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                (
                    "read-a".to_owned(),
                    "project_read_file".to_owned(),
                    r#"{"path":"a.csv"}"#.to_owned(),
                ),
                (
                    "read-b".to_owned(),
                    "project_read_file".to_owned(),
                    r#"{"path":"b.csv"}"#.to_owned(),
                ),
            ]
        );
    }
}
