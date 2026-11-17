//! Maps the `stream-json` output of one `claude -p` turn to provider stream events.
//!
//! The CLI prints one JSON object per line. A turn ends at `message_stop`, or at
//! the first line that fails it. The model limits come from the final `result`
//! line, and are recorded only when that line arrives before the turn ends. The
//! relay stops reading at the stop, so in production no limits are recorded.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::declare_server::SERVER_KEY;
use crate::providers::{ProviderError, ProviderStopReason, ProviderStreamEvent};

/// Maps a tool name the model called to Desktop's wire name. `None` means the
/// model was never given that tool.
pub(crate) type NameLookup = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// A sign-in failure ends the turn once the CLI reports this attempt. Earlier
/// attempts are repeated, because the CLI refreshes an expired token before it retries.
const SIGN_IN_RETRY_LIMIT: u64 = 3;

#[derive(Debug, Default)]
pub(crate) struct Mapped {
    pub(crate) events: Vec<ProviderStreamEvent>,
    /// The turn is over and the `claude` process must be stopped.
    pub(crate) stop_process: bool,
    pub(crate) error: Option<ProviderError>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionLimits {
    pub(crate) context_window: Option<u64>,
    pub(crate) max_output_tokens: Option<u64>,
}

pub(crate) struct StreamMapper {
    lookup: NameLookup,
    done: bool,
    limits: SessionLimits,
    /// Tool blocks still streaming, keyed by the content-block index that their
    /// deltas and stop event carry.
    tool_blocks: BTreeMap<u64, PendingToolCall>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    stop_reason: Option<String>,
    emitted_tool_call: bool,
    discarded_tool_calls: u32,
    /// The id of the message whose stream events are arriving.
    message_id: Option<String>,
    /// Messages whose text arrived as deltas. Their `assistant` snapshot is not sent again.
    streamed_text_messages: BTreeSet<String>,
    /// Whether any reply text has been sent this turn.
    emitted_text: bool,
}

struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl StreamMapper {
    pub(crate) fn new(lookup: NameLookup) -> Self {
        Self {
            lookup,
            done: false,
            limits: SessionLimits::default(),
            tool_blocks: BTreeMap::new(),
            input_tokens: None,
            output_tokens: None,
            stop_reason: None,
            emitted_tool_call: false,
            discarded_tool_calls: 0,
            message_id: None,
            streamed_text_messages: BTreeSet::new(),
            emitted_text: false,
        }
    }

    pub(crate) fn push_line(&mut self, line: &str) -> Mapped {
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line.trim()) else {
            return Mapped::default();
        };
        match object.get("type").and_then(Value::as_str) {
            // The CLI prints the result after message_stop, so its limits are
            // recorded even once the turn is over. It never produces events.
            Some("result") => self.result(&object),
            _ if self.done => Mapped::default(),
            Some("stream_event") => self.stream_event(&object),
            Some("assistant") => self.assistant(&object),
            Some("system") => self.system(&object),
            Some("rate_limit_event") => self.rate_limit(&object),
            _ => Mapped::default(),
        }
    }

    /// Call once the output has ended. A turn that never reached its end is unavailable.
    pub(crate) fn finish(&mut self) -> Mapped {
        if self.done {
            return Mapped::default();
        }
        Mapped {
            error: Some(ProviderError::Unavailable),
            ..Mapped::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn session_limits(&self) -> SessionLimits {
        self.limits
    }

    #[cfg(test)]
    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    fn stream_event(&mut self, line: &Map<String, Value>) -> Mapped {
        let Some(event) = line.get("event").and_then(Value::as_object) else {
            return Mapped::default();
        };
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => self.message_start(event),
            Some("content_block_start") => self.content_block_start(event),
            Some("content_block_delta") => self.content_block_delta(event),
            Some("content_block_stop") => self.content_block_stop(event),
            Some("message_delta") => {
                self.message_delta(event);
                Mapped::default()
            }
            Some("message_stop") => self.message_stop(),
            _ => Mapped::default(),
        }
    }

    fn message_start(&mut self, event: &Map<String, Value>) -> Mapped {
        let message = event.get("message");
        self.message_id = message
            .and_then(|message| message.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let usage = message.and_then(|message| message.get("usage"));
        self.input_tokens = input_total(usage);
        // Cached input is still prompt, so it counts toward the prompt size.
        let tokens: u64 = [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .into_iter()
        .map(|key| u64::from(token_count(usage, key).unwrap_or(0)))
        .sum();
        emit(ProviderStreamEvent::PromptSize { tokens })
    }

    fn content_block_start(&mut self, event: &Map<String, Value>) -> Mapped {
        let (Some(index), Some(block)) = (
            block_index(event),
            event.get("content_block").and_then(Value::as_object),
        ) else {
            return Mapped::default();
        };
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            return Mapped::default();
        }
        self.tool_blocks.insert(
            index,
            PendingToolCall {
                id: string_field(block, "id"),
                name: string_field(block, "name"),
                arguments: String::new(),
            },
        );
        emit(ProviderStreamEvent::Progress)
    }

    fn content_block_delta(&mut self, event: &Map<String, Value>) -> Mapped {
        let Some(delta) = event.get("delta").and_then(Value::as_object) else {
            return Mapped::default();
        };
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => match delta.get("text").and_then(Value::as_str) {
                Some(text) if !text.is_empty() => {
                    self.emitted_text = true;
                    if let Some(id) = &self.message_id {
                        self.streamed_text_messages.insert(id.clone());
                    }
                    emit(ProviderStreamEvent::TextDelta(text.to_owned()))
                }
                _ => Mapped::default(),
            },
            Some("thinking_delta") => match delta.get("thinking").and_then(Value::as_str) {
                Some(thinking) if !thinking.is_empty() => {
                    emit(ProviderStreamEvent::ReasoningDelta(thinking.to_owned()))
                }
                _ => Mapped::default(),
            },
            Some("input_json_delta") => {
                let (Some(index), Some(partial)) = (
                    block_index(event),
                    delta.get("partial_json").and_then(Value::as_str),
                ) else {
                    return Mapped::default();
                };
                // Without an open tool block there is no buffer to extend.
                let Some(pending) = self.tool_blocks.get_mut(&index) else {
                    return Mapped::default();
                };
                pending.arguments.push_str(partial);
                emit(ProviderStreamEvent::Progress)
            }
            // Signatures and unknown deltas carry nothing the app shows.
            _ => Mapped::default(),
        }
    }

    fn content_block_stop(&mut self, event: &Map<String, Value>) -> Mapped {
        let Some(pending) = block_index(event).and_then(|index| self.tool_blocks.remove(&index))
        else {
            return Mapped::default();
        };
        Mapped {
            events: self.complete_tool_call(pending).into_iter().collect(),
            ..Mapped::default()
        }
    }

    /// Turns a finished tool block into a call. A call that cannot be run, because
    /// its name was never declared, its id is missing or its arguments are not
    /// JSON, is discarded and logged.
    fn complete_tool_call(&mut self, pending: PendingToolCall) -> Option<ProviderStreamEvent> {
        let wire_name = pending.name.as_deref().and_then(|name| (self.lookup)(name));
        let arguments_empty = pending.arguments.trim().is_empty();
        let arguments_valid =
            arguments_empty || serde_json::from_str::<Value>(&pending.arguments).is_ok();
        match (pending.id, wire_name, arguments_valid) {
            (Some(call_id), Some(name), true) => {
                self.emitted_tool_call = true;
                // A tool the model called without arguments is an empty object.
                let arguments = if arguments_empty {
                    "{}".to_owned()
                } else {
                    pending.arguments
                };
                Some(ProviderStreamEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments,
                })
            }
            (id, resolved, _) => {
                log_discard(
                    discard_reason(resolved.is_some(), id.is_some()),
                    pending.name.as_deref(),
                    pending.arguments.len(),
                    self.stop_reason.as_deref(),
                );
                self.discarded_tool_calls = self.discarded_tool_calls.saturating_add(1);
                None
            }
        }
    }

    fn message_delta(&mut self, event: &Map<String, Value>) {
        if let Some(reason) = event
            .get("delta")
            .and_then(|delta| delta.get("stop_reason"))
            .and_then(Value::as_str)
        {
            self.stop_reason = Some(reason.to_owned());
        }
        let usage = event.get("usage");
        if let Some(input_tokens) = input_total(usage) {
            self.input_tokens = Some(input_tokens);
        }
        if let Some(output_tokens) = token_count(usage, "output_tokens") {
            self.output_tokens = Some(output_tokens);
        }
    }

    fn message_stop(&mut self) -> Mapped {
        // A block still open here never reached its stop event, so its arguments
        // were cut off mid-stream.
        for pending in self.tool_blocks.values() {
            log_discard(
                "block_unfinished",
                pending.name.as_deref(),
                pending.arguments.len(),
                self.stop_reason.as_deref(),
            );
        }
        let unfinished = u32::try_from(self.tool_blocks.len()).unwrap_or(u32::MAX);
        self.tool_blocks.clear();
        self.discarded_tool_calls = self.discarded_tool_calls.saturating_add(unfinished);
        self.done = true;
        Mapped {
            events: vec![
                ProviderStreamEvent::Usage {
                    input_tokens: self.input_tokens,
                    output_tokens: self.output_tokens,
                },
                ProviderStreamEvent::Stopped {
                    reason: self.provider_stop_reason(),
                    discarded_tool_calls: self.discarded_tool_calls,
                },
                ProviderStreamEvent::Completed,
            ],
            stop_process: true,
            ..Mapped::default()
        }
    }

    fn provider_stop_reason(&self) -> ProviderStopReason {
        match self.stop_reason.as_deref() {
            Some("tool_use") => ProviderStopReason::ToolUse,
            Some("end_turn" | "stop_sequence") => ProviderStopReason::Natural,
            Some("max_tokens") => ProviderStopReason::OutputLimit,
            Some("model_context_window_exceeded") => ProviderStopReason::ContextLimit,
            Some("refusal") => ProviderStopReason::Refusal,
            Some(_) => ProviderStopReason::Other,
            // A call was emitted, so the model was asking for a tool even though
            // the stop reason did not say so.
            None if self.emitted_tool_call => ProviderStopReason::ToolUse,
            None => ProviderStopReason::Other,
        }
    }

    /// A failed API call arrives as an assistant message that names its category.
    /// Otherwise the message's text is reply text, unless that message already streamed it.
    fn assistant(&mut self, line: &Map<String, Value>) -> Mapped {
        if let Some(error) = line.get("error").filter(|error| !error.is_null()) {
            return self.end_with_error(map_error_category(error.as_str().unwrap_or("")));
        }
        let Some(message) = line.get("message").and_then(Value::as_object) else {
            return Mapped::default();
        };
        // A snapshot without an id can only be checked against the turn's text so far.
        let already_sent = match message.get("id").and_then(Value::as_str) {
            Some(id) => self.streamed_text_messages.contains(id),
            None => self.emitted_text,
        };
        let text = message_text(message);
        if already_sent || text.is_empty() {
            return Mapped::default();
        }
        self.emitted_text = true;
        emit(ProviderStreamEvent::TextDelta(text))
    }

    fn system(&mut self, line: &Map<String, Value>) -> Mapped {
        match line.get("subtype").and_then(Value::as_str) {
            Some("init") => {
                warn_if_tool_server_failed(line);
                Mapped::default()
            }
            Some("api_retry") => self.api_retry(line),
            _ => Mapped::default(),
        }
    }

    /// `api_retry` reports an attempt the CLI is about to repeat, so it is progress and
    /// not a verdict. A sign-in failure can clear on the next attempt, so it ends the
    /// turn only once the attempt reaches [`SIGN_IN_RETRY_LIMIT`].
    fn api_retry(&mut self, line: &Map<String, Value>) -> Mapped {
        let category = line.get("error").and_then(Value::as_str).unwrap_or("");
        match category {
            "authentication_failed" | "oauth_org_not_allowed" => {
                let attempt = line.get("attempt").and_then(Value::as_u64).unwrap_or(0);
                if attempt >= SIGN_IN_RETRY_LIMIT {
                    self.end_with_error(map_error_category(category))
                } else {
                    Mapped::default()
                }
            }
            // Retrying these cannot succeed.
            "account_on_hold" | "billing_error" => {
                self.end_with_error(map_error_category(category))
            }
            // The other categories are transient, and the CLI retries them itself.
            _ => Mapped::default(),
        }
    }

    /// Only a rejected window ends the turn, and only while no extra usage is allowed
    /// or in use. The CLI carries on with extra usage, and other statuses are not a stop.
    fn rate_limit(&mut self, line: &Map<String, Value>) -> Mapped {
        let Some(info) = line.get("rate_limit_info").and_then(Value::as_object) else {
            return Mapped::default();
        };
        let rejected = info.get("status").and_then(Value::as_str) == Some("rejected");
        if rejected && !overage_in_use(info) {
            // Waiting does not lift a plan limit, so the turn ends here.
            self.end_with_error(ProviderError::PlanUsageLimit)
        } else {
            Mapped::default()
        }
    }

    fn result(&mut self, line: &Map<String, Value>) -> Mapped {
        self.record_limits(line);
        if self.done {
            return Mapped::default();
        }
        // Only an explicit `false` is a clean finish, and only the `success` subtype is
        // a clean result. Anything else is a failure.
        let subtype_ok = match line.get("subtype") {
            None => true,
            Some(subtype) => subtype.as_str() == Some("success"),
        };
        let clean = line.get("is_error").and_then(Value::as_bool) == Some(false) && subtype_ok;
        if !clean {
            return self.end_with_error(classify_result_error(
                &result_error_text(line),
                line.get("api_error_status").and_then(Value::as_u64),
                line.get("terminal_reason").and_then(Value::as_str),
            ));
        }
        self.done = true;
        let mut events = Vec::new();
        // Nothing streamed the reply and no assistant message carried it, so the
        // result's text is the reply. A turn that called a tool keeps its own text.
        if !self.emitted_text && !self.emitted_tool_call && self.discarded_tool_calls == 0 {
            if let Some(text) = line
                .get("result")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                events.push(ProviderStreamEvent::TextDelta(text.to_owned()));
            }
        }
        events.push(ProviderStreamEvent::Stopped {
            reason: ProviderStopReason::Natural,
            discarded_tool_calls: 0,
        });
        events.push(ProviderStreamEvent::Completed);
        Mapped {
            events,
            ..Mapped::default()
        }
    }

    fn record_limits(&mut self, line: &Map<String, Value>) {
        // The CLI lists the one model the turn ran on, so its first entry is that model.
        let Some(model) = line
            .get("modelUsage")
            .and_then(Value::as_object)
            .and_then(|models| models.values().next())
        else {
            return;
        };
        if let Some(context_window) = model.get("contextWindow").and_then(Value::as_u64) {
            self.limits.context_window = Some(context_window);
        }
        if let Some(max_output_tokens) = model.get("maxOutputTokens").and_then(Value::as_u64) {
            self.limits.max_output_tokens = Some(max_output_tokens);
        }
    }

    fn end_with_error(&mut self, error: ProviderError) -> Mapped {
        self.done = true;
        Mapped {
            stop_process: true,
            error: Some(error),
            ..Mapped::default()
        }
    }
}

/// Maps a failure category the CLI reports to the error Desktop shows. Categories
/// this table does not name, such as `max_output_tokens`, `invalid_request` and
/// `model_not_found`, leave no response this app can use.
pub(crate) fn map_error_category(category: &str) -> ProviderError {
    match category {
        "authentication_failed" | "oauth_org_not_allowed" => ProviderError::PlanSignInRequired,
        "billing_error" | "account_on_hold" => ProviderError::PaymentRequired,
        "rate_limit" => ProviderError::RateLimited { retry_after: None },
        "overloaded" | "server_error" => ProviderError::Unavailable,
        _ => ProviderError::InvalidResponse,
    }
}

/// Classifies a failed `result` from the facts it carries. The structured fields
/// decide first, and the text is read only for what they leave open. Its text is
/// matched case-insensitively. Sign-in wording wins over limit wording. A text rate
/// limit is `RateLimited`, as the `rate_limit` category is, so it is retried. A
/// usage limit is never retried, and a 429 status is reported as one.
fn classify_result_error(
    text: &str,
    api_error_status: Option<u64>,
    terminal_reason: Option<&str>,
) -> ProviderError {
    let text = text.to_ascii_lowercase();
    let mentions = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if terminal_reason == Some("prompt_too_long") {
        ProviderError::ContextOverflow
    } else if terminal_reason == Some("blocking_limit") || api_error_status == Some(429) {
        ProviderError::PlanUsageLimit
    } else if matches!(api_error_status, Some(401 | 403))
        || mentions(&[
            "not logged in",
            "/login",
            "failed to authenticate",
            "oauth token revoked",
        ])
    {
        ProviderError::PlanSignInRequired
    } else if api_error_status.is_some_and(|status| (500..=599).contains(&status)) {
        // The range includes 529, the overloaded status.
        ProviderError::Unavailable
    } else if text.contains("rate limit") {
        ProviderError::RateLimited { retry_after: None }
    } else if mentions(&["usage limit", "limit reached"]) {
        ProviderError::PlanUsageLimit
    } else {
        ProviderError::InvalidResponse
    }
}

/// The text that describes a failed `result`: its `result` string, or else its
/// `errors`. The CLI's `[ede_diagnostic]` entries are telemetry, not the user's error.
fn result_error_text(line: &Map<String, Value>) -> String {
    if let Some(text) = line
        .get("result")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return text.to_owned();
    }
    line.get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|error| !error.starts_with("[ede_diagnostic]"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text blocks of an assistant message, in order.
fn message_text(message: &Map<String, Value>) -> String {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect()
}

/// Whether extra usage is allowed or already in use for a rate-limit window. The CLI
/// reports these in `rate_limit_info`.
fn overage_in_use(info: &Map<String, Value>) -> bool {
    matches!(
        info.get("overageStatus").and_then(Value::as_str),
        Some("allowed" | "allowed_warning")
    ) || info.get("isUsingOverage").and_then(Value::as_bool) == Some(true)
        || info.get("overageInUse").and_then(Value::as_bool) == Some(true)
}

/// The `init` line lists the MCP servers. The xFalcon tool server carries the
/// declared tools. When it failed to connect, the turn still runs without them.
fn warn_if_tool_server_failed(line: &Map<String, Value>) {
    let failed = line
        .get("mcp_servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|server| {
            server.get("name").and_then(Value::as_str) == Some(SERVER_KEY)
                && server.get("status").and_then(Value::as_str) == Some("failed")
        });
    if failed {
        tracing::warn!("the xFalcon tool server failed to connect");
    }
}

/// The reason a call is discarded. The checks run in order: name, id, arguments.
fn discard_reason(resolved: bool, id_present: bool) -> &'static str {
    if !resolved {
        "name_not_declared"
    } else if !id_present {
        "id_missing"
    } else {
        "arguments_not_json"
    }
}

/// Logs one discarded tool call. Arguments are never logged: only their byte count is passed.
fn log_discard(
    reason: &'static str,
    name: Option<&str>,
    argument_bytes: usize,
    stop_reason: Option<&str>,
) {
    tracing::warn!(
        reason,
        name = %log_token(name.unwrap_or_default()),
        argument_bytes,
        stop_reason = %log_token(stop_reason.unwrap_or("none")),
        "tool call discarded"
    );
}

/// Makes provider text safe for a log line: at most 64 characters, and every
/// character outside `[A-Za-z0-9_.:-]` becomes `?`.
fn log_token(text: &str) -> String {
    text.chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-') {
                c
            } else {
                '?'
            }
        })
        .collect()
}

fn emit(event: ProviderStreamEvent) -> Mapped {
    Mapped {
        events: vec![event],
        ..Mapped::default()
    }
}

fn block_index(event: &Map<String, Value>) -> Option<u64> {
    event.get("index").and_then(Value::as_u64)
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Token counts are JSON numbers. A missing or out-of-range count is absent.
fn token_count(usage: Option<&Value>, key: &str) -> Option<u32> {
    usage?
        .get(key)?
        .as_u64()
        .and_then(|count| u32::try_from(count).ok())
}

/// The input a turn read: uncached input plus the cached input the CLI reports apart
/// from it. Absent when the usage has no input count.
fn input_total(usage: Option<&Value>) -> Option<u32> {
    let uncached = token_count(usage, "input_tokens")?;
    let cached: u64 = ["cache_creation_input_tokens", "cache_read_input_tokens"]
        .into_iter()
        .map(|key| u64::from(token_count(usage, key).unwrap_or(0)))
        .sum();
    u32::try_from(u64::from(uncached) + cached).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::{claude_runtime::declare_server::ToolNameMap, ProviderToolSchema};

    const TOOL_USE_TURN: &str = include_str!("fixtures/turn_tool_use.jsonl");
    const TEXT_TURN: &str = include_str!("fixtures/turn_text.jsonl");
    /// Data-server tools that the app declares in practice. They are short, so none is aliased.
    const DATA_SERVER_TOOLS: [&str; 4] = [
        "mcp_tenant_ida_query",
        "mcp_tenant_ida_save_dashboard",
        "mcp_tenant_ida_get_query_patterns",
        "mcp_tenant_ida_annotation_feedback",
    ];

    fn declared_tools() -> NameLookup {
        Box::new(|name: &str| {
            (name == "mcp__xf__project_search_text").then(|| "project_search_text".to_owned())
        })
    }

    /// The lookup the adapter builds: the declared tools resolved by `ToolNameMap`.
    fn lookup_for(tools: &[&str]) -> NameLookup {
        let schemas: Vec<ProviderToolSchema> = tools
            .iter()
            .map(|name| ProviderToolSchema {
                name: (*name).to_owned(),
                description: String::new(),
                input_schema: json!({"type": "object"}),
            })
            .collect();
        let names = ToolNameMap::new(&schemas).expect("names");
        Box::new(move |called: &str| names.resolve(called))
    }

    /// The events of one turn in which the model calls `name` with no arguments.
    fn turn_calling(lookup: NameLookup, name: &str) -> Vec<ProviderStreamEvent> {
        let mut mapper = StreamMapper::new(lookup);
        let mapped = feed(
            &mut mapper,
            &[
                tool_start(0, "toolu_call", name),
                input_json(0, "{}"),
                block_stop(0),
                message_delta(Some("tool_use"), 5),
                message_stop(),
            ],
        );
        events(&mapped)
    }

    fn feed<T: AsRef<str>>(mapper: &mut StreamMapper, lines: &[T]) -> Vec<Mapped> {
        lines
            .iter()
            .map(|line| mapper.push_line(line.as_ref()))
            .collect()
    }

    fn events(mapped: &[Mapped]) -> Vec<ProviderStreamEvent> {
        mapped
            .iter()
            .flat_map(|mapped| mapped.events.iter().cloned())
            .collect()
    }

    fn is_silent(mapped: &Mapped) -> bool {
        mapped.events.is_empty() && !mapped.stop_process && mapped.error.is_none()
    }

    fn line(value: Value) -> String {
        value.to_string()
    }

    fn event(value: Value) -> String {
        line(json!({"type": "stream_event", "event": value}))
    }

    fn message_start(input_tokens: u64) -> String {
        event(json!({
            "type": "message_start",
            "message": {"usage": {"input_tokens": input_tokens}}
        }))
    }

    fn tool_start(index: u64, id: &str, name: &str) -> String {
        event(json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
        }))
    }

    fn input_json(index: u64, partial_json: &str) -> String {
        event(json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {"type": "input_json_delta", "partial_json": partial_json}
        }))
    }

    fn block_stop(index: u64) -> String {
        event(json!({"type": "content_block_stop", "index": index}))
    }

    fn message_delta(stop_reason: Option<&str>, output_tokens: u64) -> String {
        event(json!({
            "type": "message_delta",
            "delta": {"stop_reason": stop_reason},
            "usage": {"output_tokens": output_tokens}
        }))
    }

    fn message_stop() -> String {
        event(json!({"type": "message_stop"}))
    }

    fn message_start_of(id: &str) -> String {
        event(json!({
            "type": "message_start",
            "message": {"id": id, "usage": {"input_tokens": 1}}
        }))
    }

    fn text_start(index: u64) -> String {
        event(json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {"type": "text", "text": ""}
        }))
    }

    fn text_delta(index: u64, text: &str) -> String {
        event(json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {"type": "text_delta", "text": text}
        }))
    }

    /// An `assistant` line carrying one text block of message `id`.
    fn assistant_text(id: &str, text: &str) -> String {
        line(json!({
            "type": "assistant",
            "message": {"id": id, "content": [{"type": "text", "text": text}]}
        }))
    }

    fn api_retry_line(category: &str, attempt: u64) -> String {
        line(json!({
            "type": "system",
            "subtype": "api_retry",
            "attempt": attempt,
            "error": category
        }))
    }

    fn rate_limit_event(info: Value) -> String {
        line(json!({"type": "rate_limit_event", "rate_limit_info": info}))
    }

    /// The text of each `TextDelta` in the turn, in order.
    fn text_sent(mapped: &[Mapped]) -> Vec<String> {
        events(mapped)
            .into_iter()
            .filter_map(|event| match event {
                ProviderStreamEvent::TextDelta(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn tool_use_turn_maps_to_one_complete_call_then_stops() {
        let mut mapper = StreamMapper::new(declared_tools());
        let lines: Vec<&str> = TOOL_USE_TURN.lines().collect();
        let mapped = feed(&mut mapper, &lines);

        let stopping: Vec<usize> = mapped
            .iter()
            .enumerate()
            .filter(|(_, mapped)| mapped.stop_process)
            .map(|(position, _)| position)
            .collect();
        assert_eq!(
            stopping,
            vec![9],
            "only the message_stop line ends the turn"
        );
        assert!(mapper.is_done());
        assert_eq!(
            events(&mapped),
            vec![
                ProviderStreamEvent::PromptSize { tokens: 958 },
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::ToolCallRequested {
                    call_id: "toolu_01ATNoVfX5Pvj3RFmaDkd6PS".to_owned(),
                    name: "project_search_text".to_owned(),
                    arguments: "{\"query\": \"budget\"}".to_owned(),
                },
                ProviderStreamEvent::Usage {
                    input_tokens: Some(958),
                    output_tokens: Some(57),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );

        let after = feed(
            &mut mapper,
            &[event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "late"}
            }))],
        );
        assert!(is_silent(&after[0]));
    }

    #[test]
    fn text_turn_streams_its_text_and_records_limits_from_the_result_after_done() {
        let mut mapper = StreamMapper::new(declared_tools());
        let lines: Vec<&str> = TEXT_TURN.lines().collect();
        let (turn, result) = lines.split_at(lines.len() - 1);

        let mapped = feed(&mut mapper, turn);
        assert!(mapper.is_done());
        assert_eq!(mapper.session_limits(), SessionLimits::default());

        let final_text = lines
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|value| value["type"] == "assistant")
            .and_then(|value| {
                value["message"]["content"][0]["text"]
                    .as_str()
                    .map(str::to_owned)
            })
            .expect("the fixture has a final assistant text");
        let streamed_events = events(&mapped);
        let streamed: String = streamed_events
            .iter()
            .filter_map(|event| match event {
                ProviderStreamEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(streamed, final_text);

        assert_eq!(
            streamed_events.first(),
            Some(&ProviderStreamEvent::PromptSize { tokens: 1211 })
        );
        assert_eq!(
            streamed_events[streamed_events.len() - 3..].to_vec(),
            vec![
                ProviderStreamEvent::Usage {
                    input_tokens: Some(1211),
                    output_tokens: Some(169),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        let stopping: Vec<usize> = mapped
            .iter()
            .enumerate()
            .filter(|(_, mapped)| mapped.stop_process)
            .map(|(position, _)| position)
            .collect();
        assert_eq!(stopping, vec![50]);

        let after = feed(&mut mapper, result);
        assert!(is_silent(&after[0]));
        assert_eq!(
            mapper.session_limits(),
            SessionLimits {
                context_window: Some(1_000_000),
                max_output_tokens: Some(128_000),
            }
        );
    }

    #[test]
    fn unknown_tool_name_is_discarded_and_counted() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                message_start(10),
                tool_start(0, "toolu_other", "mcp__xf__not_declared"),
                input_json(0, "{}"),
                block_stop(0),
                message_delta(Some("tool_use"), 5),
                message_stop(),
            ],
        );

        assert_eq!(
            events(&mapped),
            vec![
                ProviderStreamEvent::PromptSize { tokens: 10 },
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 1,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[test]
    fn invalid_argument_json_is_discarded_and_counted() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                message_start(10),
                tool_start(0, "toolu_cut", "mcp__xf__project_search_text"),
                input_json(0, "{\"query\": \"bud"),
                block_stop(0),
                message_delta(Some("tool_use"), 5),
                message_stop(),
            ],
        );

        assert_eq!(
            events(&mapped),
            vec![
                ProviderStreamEvent::PromptSize { tokens: 10 },
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                },
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 1,
                },
                ProviderStreamEvent::Completed,
            ]
        );
    }

    #[test]
    fn tool_call_without_arguments_gets_an_empty_object() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                tool_start(0, "toolu_empty", "mcp__xf__project_search_text"),
                block_stop(0),
                message_delta(Some("tool_use"), 5),
                message_stop(),
            ],
        );

        assert!(
            events(&mapped).contains(&ProviderStreamEvent::ToolCallRequested {
                call_id: "toolu_empty".to_owned(),
                name: "project_search_text".to_owned(),
                arguments: "{}".to_owned(),
            })
        );
    }

    #[test]
    fn tool_block_cut_off_before_its_stop_event_is_discarded_at_message_stop() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                tool_start(0, "toolu_open", "mcp__xf__project_search_text"),
                input_json(0, "{\"que"),
                message_delta(Some("tool_use"), 5),
                message_stop(),
            ],
        );

        assert!(!events(&mapped)
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::ToolCallRequested { .. })));
        assert!(events(&mapped).contains(&ProviderStreamEvent::Stopped {
            reason: ProviderStopReason::ToolUse,
            discarded_tool_calls: 1,
        }));
    }

    #[test]
    fn stop_reasons_map_to_provider_stop_reasons() {
        let cases = [
            (Some("end_turn"), ProviderStopReason::Natural),
            (Some("stop_sequence"), ProviderStopReason::Natural),
            (Some("max_tokens"), ProviderStopReason::OutputLimit),
            (
                Some("model_context_window_exceeded"),
                ProviderStopReason::ContextLimit,
            ),
            (Some("refusal"), ProviderStopReason::Refusal),
            (Some("pause_turn"), ProviderStopReason::Other),
            (None, ProviderStopReason::Other),
        ];
        for (stop_reason, expected) in cases {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(
                &mut mapper,
                &[
                    message_start(1),
                    message_delta(stop_reason, 1),
                    message_stop(),
                ],
            );
            assert!(
                events(&mapped).contains(&ProviderStreamEvent::Stopped {
                    reason: expected,
                    discarded_tool_calls: 0,
                }),
                "stop reason {stop_reason:?}"
            );
        }
    }

    #[test]
    fn missing_stop_reason_after_an_emitted_call_is_a_tool_use() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                tool_start(0, "toolu_1", "mcp__xf__project_search_text"),
                block_stop(0),
                message_delta(None, 5),
                message_stop(),
            ],
        );

        assert!(events(&mapped).contains(&ProviderStreamEvent::Stopped {
            reason: ProviderStopReason::ToolUse,
            discarded_tool_calls: 0,
        }));
    }

    #[test]
    fn sign_in_failure_result_ends_the_turn() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "is_error": true,
                "result": "Not logged in · Please run /login"
            }))],
        );

        assert!(mapped[0].events.is_empty());
        assert!(mapped[0].stop_process);
        assert_eq!(mapped[0].error, Some(ProviderError::PlanSignInRequired));
        assert!(mapper.is_done());
    }

    #[test]
    fn rate_limit_result_is_retryable_not_a_plan_usage_limit() {
        // Both spellings of the text: the "limit reached" tail must not win.
        for text in ["API Error: Rate limit exceeded", "Rate limit reached"] {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(
                &mut mapper,
                &[line(json!({
                    "type": "result",
                    "is_error": true,
                    "result": text
                }))],
            );

            assert!(mapped[0].stop_process, "{text}");
            assert_eq!(
                mapped[0].error,
                Some(ProviderError::RateLimited { retry_after: None }),
                "{text}"
            );
        }
    }

    #[test]
    fn log_in_wording_alone_is_not_a_sign_in_failure() {
        assert_eq!(
            classify_result_error("Failed to log in to the MCP server", None, None),
            ProviderError::InvalidResponse
        );
        assert_eq!(
            classify_result_error("Not logged in · Please run /login", None, None),
            ProviderError::PlanSignInRequired
        );
        assert_eq!(
            classify_result_error("Please log in again", None, None),
            ProviderError::InvalidResponse
        );
    }

    #[test]
    fn usage_limit_result_ends_the_turn() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "is_error": true,
                "result": "Claude AI usage limit reached|1792130400"
            }))],
        );

        assert!(mapped[0].events.is_empty());
        assert!(mapped[0].stop_process);
        assert_eq!(mapped[0].error, Some(ProviderError::PlanUsageLimit));
        assert!(mapper.is_done());
    }

    #[test]
    fn failed_result_with_no_known_cause_is_an_invalid_response() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "is_error": true,
                "result": "Something unexpected happened"
            }))],
        );

        assert_eq!(mapped[0].error, Some(ProviderError::InvalidResponse));
        assert!(mapped[0].stop_process);
    }

    #[test]
    fn clean_result_without_a_message_stop_completes_the_turn() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(
                json!({"type": "result", "is_error": false, "result": "done"}),
            )],
        );

        assert_eq!(
            events(&mapped),
            vec![
                ProviderStreamEvent::TextDelta("done".to_owned()),
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert!(mapper.is_done());
    }

    #[test]
    fn rejected_rate_limit_ends_the_turn_and_allowed_ones_do_not() {
        let mut mapper = StreamMapper::new(declared_tools());
        let warned = feed(
            &mut mapper,
            &[line(json!({
                "type": "rate_limit_event",
                "rate_limit_info": {"status": "allowed_warning"}
            }))],
        );
        assert!(is_silent(&warned[0]));
        assert!(!mapper.is_done());

        let rejected = feed(
            &mut mapper,
            &[line(json!({
                "type": "rate_limit_event",
                "rate_limit_info": {"status": "rejected"}
            }))],
        );
        assert_eq!(rejected[0].error, Some(ProviderError::PlanUsageLimit));
        assert!(rejected[0].stop_process);
        assert!(mapper.is_done());
    }

    #[test]
    fn rate_limit_line_without_a_string_status_is_ignored() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                line(json!({"type": "rate_limit_event"})),
                line(json!({"type": "rate_limit_event", "rate_limit_info": {}})),
                line(json!({"type": "rate_limit_event", "rate_limit_info": {"status": null}})),
                line(json!({"type": "rate_limit_event", "rate_limit_info": {"status": 429}})),
            ],
        );
        assert!(mapped.iter().all(is_silent));
        assert!(!mapper.is_done());
    }

    #[test]
    fn empty_thinking_delta_emits_nothing() {
        let mut mapper = StreamMapper::new(declared_tools());
        let empty = feed(
            &mut mapper,
            &[event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "thinking_delta", "thinking": ""}
            }))],
        );
        assert!(is_silent(&empty[0]));

        let thought = feed(
            &mut mapper,
            &[event(json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "thinking_delta", "thinking": "weighing the options"}
            }))],
        );
        assert_eq!(
            events(&thought),
            vec![ProviderStreamEvent::ReasoningDelta(
                "weighing the options".to_owned()
            )]
        );
    }

    #[test]
    fn terminal_api_retry_ends_the_turn_and_transient_retries_are_left_to_the_cli() {
        for transient in ["server_error", "rate_limit", "overloaded", "something_new"] {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(&mut mapper, &[api_retry_line(transient, 1)]);
            assert!(is_silent(&mapped[0]), "{transient} must not end the turn");
            assert!(!mapper.is_done());
        }

        for category in ["account_on_hold", "billing_error"] {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(&mut mapper, &[api_retry_line(category, 1)]);
            assert_eq!(
                mapped[0].error,
                Some(ProviderError::PaymentRequired),
                "{category}"
            );
            assert!(mapped[0].stop_process);
            assert!(mapper.is_done());
        }
    }

    #[test]
    fn sign_in_failure_is_retried_until_the_third_attempt() {
        for category in ["authentication_failed", "oauth_org_not_allowed"] {
            for attempt in [1, 2] {
                let mut mapper = StreamMapper::new(declared_tools());
                let mapped = feed(&mut mapper, &[api_retry_line(category, attempt)]);
                assert!(
                    is_silent(&mapped[0]),
                    "{category} attempt {attempt} must not end the turn"
                );
                assert!(!mapper.is_done());
            }

            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(&mut mapper, &[api_retry_line(category, 3)]);
            assert_eq!(
                mapped[0].error,
                Some(ProviderError::PlanSignInRequired),
                "{category}"
            );
            assert!(mapped[0].stop_process);
            assert!(mapper.is_done());
        }
    }

    #[test]
    fn sign_in_retry_that_succeeds_completes_the_turn() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                api_retry_line("authentication_failed", 1),
                assistant_text("msg_retried", "Signed in again."),
                line(json!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "result": "Signed in again."
                })),
            ],
        );

        assert_eq!(
            events(&mapped),
            vec![
                ProviderStreamEvent::TextDelta("Signed in again.".to_owned()),
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert!(mapper.is_done());
    }

    #[test]
    fn assistant_message_with_an_error_ends_the_turn() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "assistant",
                "error": "authentication_failed",
                "message": {
                    "id": "msg_failed",
                    "content": [{"type": "text", "text": "Failed to authenticate."}]
                }
            }))],
        );

        assert_eq!(mapped[0].error, Some(ProviderError::PlanSignInRequired));
        assert!(
            mapped[0].events.is_empty(),
            "the error text is not reply text"
        );
        assert!(mapped[0].stop_process);
        assert!(mapper.is_done());
    }

    #[test]
    fn crlf_lines_map_like_lf_lines() {
        let lf: Vec<&str> = TOOL_USE_TURN.lines().collect();
        let crlf: Vec<String> = lf.iter().map(|line| format!("{line}\r")).collect();

        let mut lf_mapper = StreamMapper::new(declared_tools());
        let mut crlf_mapper = StreamMapper::new(declared_tools());
        let lf_mapped = feed(&mut lf_mapper, &lf);
        let crlf_mapped = feed(&mut crlf_mapper, &crlf);

        assert_eq!(events(&lf_mapped), events(&crlf_mapped));
        assert!(crlf_mapper.is_done());
    }

    #[test]
    fn garbage_lines_are_ignored() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                "",
                "   ",
                "\r",
                "not json",
                "[1, 2]",
                "42",
                "\"text\"",
                "null",
                r#"{"type":"stream_event""#,
                r#"{"type":"unknown"}"#,
            ],
        );

        assert!(mapped.iter().all(is_silent));
        assert!(!mapper.is_done());
    }

    #[test]
    fn finish_before_the_turn_completes_is_unavailable() {
        let mut mapper = StreamMapper::new(declared_tools());
        feed(&mut mapper, &[message_start(10)]);

        let finished = mapper.finish();
        assert_eq!(finished.error, Some(ProviderError::Unavailable));
        assert!(finished.events.is_empty());
        assert!(!finished.stop_process);

        feed(&mut mapper, &[message_stop()]);
        assert!(is_silent(&mapper.finish()));
    }

    #[test]
    fn a_bare_wire_name_of_an_aliased_tool_is_emitted_under_its_wire_name() {
        let long = "abcdefghij".repeat(6);
        let events = turn_calling(lookup_for(&[long.as_str()]), &long);

        assert!(events.contains(&ProviderStreamEvent::ToolCallRequested {
            call_id: "toolu_call".to_owned(),
            name: long.clone(),
            arguments: "{}".to_owned(),
        }));
        assert!(events.contains(&ProviderStreamEvent::Stopped {
            reason: ProviderStopReason::ToolUse,
            discarded_tool_calls: 0,
        }));
    }

    #[test]
    fn a_data_server_tool_called_by_its_bare_or_prefixed_name_is_emitted_and_not_discarded() {
        let wire = "mcp_tenant_ida_save_dashboard";
        for called in [wire.to_owned(), format!("mcp__xf__{wire}")] {
            let events = turn_calling(lookup_for(&DATA_SERVER_TOOLS), &called);

            assert!(events.contains(&ProviderStreamEvent::ToolCallRequested {
                call_id: "toolu_call".to_owned(),
                name: wire.to_owned(),
                arguments: "{}".to_owned(),
            }));
            assert!(
                events.contains(&ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 0,
                }),
                "{called}"
            );
        }
    }

    #[test]
    fn a_call_under_a_name_no_form_resolves_is_discarded_and_counted() {
        for called in [
            "mcp__xf__mcp_tenant_ida",
            "MCP__XF__mcp_tenant_ida_save_dashboard",
            "mcp__xf__not_declared",
        ] {
            let events = turn_calling(lookup_for(&DATA_SERVER_TOOLS), called);

            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, ProviderStreamEvent::ToolCallRequested { .. })),
                "{called}"
            );
            assert!(
                events.contains(&ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::ToolUse,
                    discarded_tool_calls: 1,
                }),
                "{called}"
            );
        }
    }

    #[test]
    fn discard_reason_names_the_first_failed_check() {
        assert_eq!(discard_reason(false, true), "name_not_declared");
        assert_eq!(discard_reason(false, false), "name_not_declared");
        assert_eq!(discard_reason(true, false), "id_missing");
        assert_eq!(discard_reason(true, true), "arguments_not_json");
    }

    #[test]
    fn log_token_keeps_safe_characters_and_truncates_to_64() {
        assert_eq!(log_token("mcp__xf.tool-1:v2"), "mcp__xf.tool-1:v2");
        assert_eq!(log_token("a b\nc\u{e9}"), "a?b?c?");
        assert_eq!(log_token(&"a".repeat(70)), "a".repeat(64));
        assert_eq!(log_token(&"\u{e9}".repeat(70)), "?".repeat(64));
    }

    #[test]
    fn error_categories_map_to_provider_errors() {
        let cases = [
            ("authentication_failed", ProviderError::PlanSignInRequired),
            ("oauth_org_not_allowed", ProviderError::PlanSignInRequired),
            ("billing_error", ProviderError::PaymentRequired),
            ("account_on_hold", ProviderError::PaymentRequired),
            (
                "rate_limit",
                ProviderError::RateLimited { retry_after: None },
            ),
            ("overloaded", ProviderError::Unavailable),
            ("server_error", ProviderError::Unavailable),
            ("max_output_tokens", ProviderError::InvalidResponse),
            ("invalid_request", ProviderError::InvalidResponse),
            ("model_not_found", ProviderError::InvalidResponse),
            ("something_new", ProviderError::InvalidResponse),
        ];
        for (category, expected) in cases {
            assert_eq!(map_error_category(category), expected, "{category}");
        }
    }

    #[test]
    fn assistant_text_without_deltas_is_sent_once() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                message_start_of("msg_snapshot"),
                assistant_text("msg_snapshot", "Found it."),
                message_delta(Some("end_turn"), 4),
                message_stop(),
            ],
        );

        assert_eq!(text_sent(&mapped), vec!["Found it.".to_owned()]);
        assert!(mapper.is_done());
    }

    #[test]
    fn streamed_text_is_not_sent_again_by_the_assistant_snapshot() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                message_start_of("msg_streamed"),
                text_start(0),
                text_delta(0, "Found "),
                text_delta(0, "it."),
                assistant_text("msg_streamed", "Found it."),
                block_stop(0),
                message_delta(Some("end_turn"), 4),
                message_stop(),
            ],
        );

        assert_eq!(
            text_sent(&mapped),
            vec!["Found ".to_owned(), "it.".to_owned()]
        );
    }

    #[test]
    fn result_text_is_the_reply_when_nothing_else_carried_it() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "result": "Found it."
            }))],
        );

        assert_eq!(
            events(&mapped),
            vec![
                ProviderStreamEvent::TextDelta("Found it.".to_owned()),
                ProviderStreamEvent::Stopped {
                    reason: ProviderStopReason::Natural,
                    discarded_tool_calls: 0,
                },
                ProviderStreamEvent::Completed,
            ]
        );
        assert!(mapper.is_done());
    }

    #[test]
    fn result_text_is_not_sent_again_after_the_assistant_text() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                assistant_text("msg_answer", "Found it."),
                line(json!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "result": "Found it."
                })),
            ],
        );

        assert_eq!(text_sent(&mapped), vec!["Found it.".to_owned()]);
        assert!(mapper.is_done());
    }

    #[test]
    fn result_text_is_not_sent_after_a_tool_call() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                tool_start(0, "toolu_call", "mcp__xf__project_search_text"),
                block_stop(0),
                line(json!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "result": "Here is what I found."
                })),
            ],
        );

        assert!(text_sent(&mapped).is_empty());
        assert!(
            events(&mapped).contains(&ProviderStreamEvent::ToolCallRequested {
                call_id: "toolu_call".to_owned(),
                name: "project_search_text".to_owned(),
                arguments: "{}".to_owned(),
            })
        );
    }

    #[test]
    fn empty_successful_turn_sends_no_text() {
        // The turn driver reports a turn that produced no output as an error.
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                message_start_of("msg_empty"),
                message_delta(Some("end_turn"), 0),
                message_stop(),
            ],
        );

        assert!(text_sent(&mapped).is_empty());
        assert!(events(&mapped).contains(&ProviderStreamEvent::Completed));
    }

    #[test]
    fn rejected_rate_limit_with_extra_usage_does_not_end_the_turn() {
        let with_extra_usage = [
            json!({"status": "rejected", "overageStatus": "allowed"}),
            json!({"status": "rejected", "overageStatus": "allowed_warning"}),
            json!({"status": "rejected", "isUsingOverage": true}),
            json!({"status": "rejected", "overageInUse": true}),
        ];
        for info in with_extra_usage {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(&mut mapper, &[rate_limit_event(info.clone())]);
            assert!(is_silent(&mapped[0]), "{info}");
            assert!(!mapper.is_done());
        }
    }

    #[test]
    fn rejected_rate_limit_without_extra_usage_ends_the_turn() {
        let without_extra_usage = [
            json!({"status": "rejected", "overageStatus": "rejected", "isUsingOverage": false}),
            json!({"status": "rejected", "overageStatus": "rejected", "overageInUse": false}),
        ];
        for info in without_extra_usage {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(&mut mapper, &[rate_limit_event(info.clone())]);
            assert_eq!(
                mapped[0].error,
                Some(ProviderError::PlanUsageLimit),
                "{info}"
            );
            assert!(mapped[0].stop_process);
            assert!(mapper.is_done());
        }
    }

    #[test]
    fn rate_limit_statuses_other_than_a_rejected_window_do_not_end_the_turn() {
        let other = [
            json!({"status": "allowed", "overageStatus": "rejected", "isUsingOverage": false}),
            json!({"status": "allowed_warning"}),
            json!({"status": "queued"}),
            json!({"status": "something_new"}),
        ];
        for info in other {
            let mut mapper = StreamMapper::new(declared_tools());
            let mapped = feed(&mut mapper, &[rate_limit_event(info.clone())]);
            assert!(is_silent(&mapped[0]), "{info}");
            assert!(!mapper.is_done());
        }
    }

    #[test]
    fn failed_results_map_by_their_structured_fields_before_their_text() {
        let cases = [
            (
                "Prompt is too long",
                None,
                Some("prompt_too_long"),
                ProviderError::ContextOverflow,
            ),
            (
                "Claude AI usage limit reached",
                None,
                Some("blocking_limit"),
                ProviderError::PlanUsageLimit,
            ),
            (
                "API Error: 429 Too many requests",
                Some(429),
                None,
                ProviderError::PlanUsageLimit,
            ),
            (
                "Failed to authenticate. API Error: 401 Invalid authentication credentials",
                Some(401),
                Some("completed"),
                ProviderError::PlanSignInRequired,
            ),
            (
                "API Error: 403 Forbidden",
                Some(403),
                None,
                ProviderError::PlanSignInRequired,
            ),
            (
                "Failed to authenticate.",
                None,
                None,
                ProviderError::PlanSignInRequired,
            ),
            (
                "Your oauth token revoked",
                None,
                None,
                ProviderError::PlanSignInRequired,
            ),
            (
                "API Error: 529 Overloaded",
                Some(529),
                None,
                ProviderError::Unavailable,
            ),
            (
                "API Error: 500 Internal error",
                Some(500),
                None,
                ProviderError::Unavailable,
            ),
            (
                "Rate limit exceeded",
                Some(529),
                None,
                ProviderError::Unavailable,
            ),
            (
                "usage limit reached",
                Some(401),
                None,
                ProviderError::PlanSignInRequired,
            ),
            (
                "Something unexpected happened",
                None,
                Some("api_error"),
                ProviderError::InvalidResponse,
            ),
        ];
        for (text, status, reason, expected) in cases {
            assert_eq!(
                classify_result_error(text, status, reason),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn failed_result_without_a_result_string_is_classified_from_its_errors() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "errors": [
                    "[ede_diagnostic] result_type=user",
                    "Not logged in · Please run /login"
                ]
            }))],
        );

        assert_eq!(mapped[0].error, Some(ProviderError::PlanSignInRequired));
        assert!(mapped[0].stop_process);
        assert!(mapper.is_done());
    }

    #[test]
    fn ede_diagnostic_entries_are_not_part_of_the_failure_text() {
        let result = json!({
            "type": "result",
            "subtype": "error_during_execution",
            "is_error": true,
            "errors": ["[ede_diagnostic] usage limit reached", "Request was aborted."]
        });
        let object = result.as_object().expect("a JSON object");
        assert_eq!(result_error_text(object), "Request was aborted.");

        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "errors": ["[ede_diagnostic] usage limit reached"]
            }))],
        );
        assert_eq!(mapped[0].error, Some(ProviderError::InvalidResponse));
    }

    #[test]
    fn a_non_success_subtype_is_a_failure_even_when_is_error_is_false() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": false,
                "errors": ["Error: Request was aborted."]
            }))],
        );

        assert!(mapped[0].events.is_empty());
        assert!(mapped[0].stop_process);
        assert_eq!(mapped[0].error, Some(ProviderError::InvalidResponse));
        assert!(mapper.is_done());
    }

    #[test]
    fn usage_counts_cached_input_as_input() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[
                event(json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_cached",
                        "usage": {
                            "input_tokens": 2,
                            "cache_creation_input_tokens": 156,
                            "cache_read_input_tokens": 1053
                        }
                    }
                })),
                event(json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {
                        "input_tokens": 2,
                        "cache_creation_input_tokens": 156,
                        "cache_read_input_tokens": 1053,
                        "output_tokens": 169
                    }
                })),
                message_stop(),
            ],
        );

        let sent = events(&mapped);
        assert!(sent.contains(&ProviderStreamEvent::PromptSize { tokens: 1211 }));
        assert!(sent.contains(&ProviderStreamEvent::Usage {
            input_tokens: Some(1211),
            output_tokens: Some(169),
        }));
    }

    #[test]
    fn a_failed_tool_server_at_init_does_not_change_the_turn() {
        let mut mapper = StreamMapper::new(declared_tools());
        let mapped = feed(
            &mut mapper,
            &[line(json!({
                "type": "system",
                "subtype": "init",
                "mcp_servers": [
                    {"name": "xf", "status": "failed"},
                    {"name": "other", "status": "connected"}
                ]
            }))],
        );

        assert!(is_silent(&mapped[0]));
        assert!(!mapper.is_done());
    }
}
