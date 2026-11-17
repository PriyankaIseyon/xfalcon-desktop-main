use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{NotificationContext, Peer, RequestContext};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{RoleServer, ServerHandler};
use serde_json::json;
use tokio::sync::{Barrier, Notify};
use tokio_util::sync::CancellationToken;

/// `McpSession` deliberately does not derive `Debug` (it wraps a live
/// `rmcp` transport/service and this codebase avoids Debug-formatting
/// live/sensitive resources by convention — see `tools::invocation::ToolResult`'s
/// hand-written, redacted `Debug` for the sibling precedent), so tests
/// unwrap failed connection attempts manually rather than via
/// `Result::unwrap_err`/`expect_err`, both of which require the `Ok` type to
/// implement `Debug`.
pub(super) fn expect_connect_err(
    result: Result<crate::mcp::client::McpSession, crate::mcp::client::McpClientError>,
) -> crate::mcp::client::McpClientError {
    match result {
        Ok(_) => panic!("expected a connection error"),
        Err(error) => error,
    }
}

/// Keeps a spawned test server's background task and cancellation token
/// alive; dropping it tears the server down.
pub(super) struct ServerGuard {
    cancel: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.handle.abort();
    }
}

async fn spawn_router(router: Router) -> (String, ServerGuard) {
    let cancel = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let shutdown = cancel.clone();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
    });
    (format!("http://{addr}/mcp"), ServerGuard { cancel, handle })
}

/// Pauses a real fixture request until the test releases it.
#[derive(Clone, Default)]
pub(super) struct RequestGate {
    entered: Arc<Notify>,
    released: Arc<Notify>,
}

impl RequestGate {
    pub(super) async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    pub(super) fn release(&self) {
        self.released.notify_one();
    }

    async fn wait_for_release(&self) {
        self.entered.notify_one();
        self.released.notified().await;
    }
}

/// Shared state a test can inspect after driving a client against
/// [`FixtureServer`].
#[derive(Clone, Default)]
pub(super) struct FixtureState {
    pub(super) call_count: Arc<AtomicUsize>,
    pub(super) list_tools_call_count: Arc<AtomicUsize>,
    pub(super) last_call_cancelled: Arc<std::sync::atomic::AtomicBool>,
    echo_requires_number: Arc<AtomicBool>,
    fail_next_list_tools: Arc<AtomicBool>,
    pause_next_list_tools: Arc<Mutex<Option<RequestGate>>>,
    /// Whether the most recent `slow_progress` call actually carried a
    /// `_meta.progressToken`. Separates "the client never asked for progress"
    /// from "the client asked but dropped the notification" — without it, a
    /// progress assertion failing says only "no progress observed" and gives
    /// no way to tell which side is at fault.
    pub(super) last_call_had_progress_token: Arc<std::sync::atomic::AtomicBool>,
    peer: Arc<Mutex<Option<Peer<RoleServer>>>>,
}

impl FixtureState {
    pub(super) fn require_number_for_echo(&self) {
        self.echo_requires_number.store(true, Ordering::SeqCst);
    }

    pub(super) fn fail_next_list_tools(&self) {
        self.fail_next_list_tools.store(true, Ordering::SeqCst);
    }

    pub(super) fn pause_next_list_tools(&self) -> RequestGate {
        let gate = RequestGate::default();
        *self
            .pause_next_list_tools
            .lock()
            .expect("listing gate lock") = Some(gate.clone());
        gate
    }

    /// Pushes a real `notifications/tools/list_changed` to the connected
    /// client, if a session has completed its handshake.
    pub(super) async fn push_tool_list_changed(&self) {
        let peer = self.peer.lock().expect("peer lock poisoned").clone();
        if let Some(peer) = peer {
            let _ = peer.notify_tool_list_changed().await;
        }
    }
}

/// The named (non-paginated-filler) tools every [`FixtureServer`] advertises.
///
/// Kept next to the declarations below so adding a fixture tool updates the
/// tests' expectations in the same edit. `fabricated_progress` was added
/// without touching the catalog test's hard-coded count, and the resulting
/// failure sat unread while the Windows harness could not launch.
pub(super) const FIXTURE_NAMED_TOOLS: [&str; 4] = [
    "echo",
    "slow_progress",
    "fabricated_progress",
    "ida_save_dashboard",
];

/// What the fixture's `ida_save_dashboard` answers: the shape DA's tool returns
/// for a kit it just created, so the report-origin link is proven against it.
pub(super) const FIXTURE_SAVE_ANSWER: &str = "# ✅ Dashboard Saved\n\n**Weekly** — kit id `kit-fixture`\n- Scope: user\n\nRecall it later with `ida_recall_dashboard(kit_id='kit-fixture')`.";

/// A real, spec-compliant MCP server (built on `rmcp`'s own server
/// transport) advertising `tool_count` generic tools (paginated one per
/// page, to exercise `next_cursor` deterministically) plus `echo` and
/// `slow_progress` tools used to exercise call/cancel/progress behavior.
#[derive(Clone)]
pub(super) struct FixtureServer {
    tools: Arc<Vec<Tool>>,
    state: FixtureState,
}

impl FixtureServer {
    pub(super) fn new(tool_count: usize, state: FixtureState) -> Self {
        let mut tools: Vec<Tool> = (0..tool_count)
            .map(|index| {
                Tool::new(
                    format!("tool_{index}"),
                    "a generic paginated fixture tool",
                    json!({"type": "object"}).as_object().cloned().unwrap(),
                )
            })
            .collect();
        tools.push(Tool::new(
            "echo",
            "echoes the provided text",
            json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false,
            })
            .as_object()
            .cloned()
            .unwrap(),
        ));
        tools.push(Tool::new(
            "slow_progress",
            "reports progress twice, then completes unless cancelled first",
            json!({"type": "object"}).as_object().cloned().unwrap(),
        ));
        tools.push(Tool::new(
            "fabricated_progress",
            "sends a progress notification for a token nobody registered, \
             simulating a hostile/misbehaving server",
            json!({"type": "object"}).as_object().cloned().unwrap(),
        ));
        tools.push(Tool::new(
            "ida_save_dashboard",
            "stands in for DA's save tool: answers as DA does for a saved kit",
            json!({"type": "object"}).as_object().cloned().unwrap(),
        ));
        Self {
            tools: Arc::new(tools),
            state,
        }
    }

    /// Starts the server and returns its `/mcp` base URL plus a guard that
    /// tears it down on drop.
    pub(super) async fn spawn(self) -> (String, ServerGuard) {
        let service: StreamableHttpService<Self, LocalSessionManager> = StreamableHttpService::new(
            move || Ok(self.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_sse_keep_alive(None),
        );
        let router = Router::new().nest_service("/mcp", service);
        spawn_router(router).await
    }
}

impl ServerHandler for FixtureServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + Send + '_ {
        *self.state.peer.lock().expect("peer lock poisoned") = Some(context.peer);
        std::future::ready(())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let cursor: usize = request
            .and_then(|params| params.cursor)
            .and_then(|cursor| cursor.parse().ok())
            .unwrap_or(0);
        if cursor == 0 {
            // Counts pagination *cycles* (one per logical `list_all_tools`
            // call), not raw page fetches, so cache-reuse tests stay
            // independent of how many tools/pages a given fixture has.
            self.state
                .list_tools_call_count
                .fetch_add(1, Ordering::SeqCst);
            let pause = self
                .state
                .pause_next_list_tools
                .lock()
                .expect("listing gate lock")
                .take();
            if let Some(pause) = pause {
                pause.wait_for_release().await;
            }
            if self
                .state
                .fail_next_list_tools
                .swap(false, Ordering::SeqCst)
            {
                return Err(ErrorData::internal_error(
                    "injected tools/list failure",
                    None,
                ));
            }
        }
        let page = self.tools.get(cursor).cloned().map(|tool| {
            if tool.name.as_ref() == "echo"
                && self.state.echo_requires_number.load(Ordering::SeqCst)
            {
                Tool::new(
                    "echo",
                    "echoes the provided number",
                    json!({
                        "type": "object",
                        "properties": {"text": {"type": "number"}},
                        "required": ["text"],
                        "additionalProperties": false,
                    })
                    .as_object()
                    .cloned()
                    .unwrap(),
                )
            } else {
                tool
            }
        });
        let next_cursor = if cursor + 1 < self.tools.len() {
            Some((cursor + 1).to_string())
        } else {
            None
        };
        Ok(ListToolsResult {
            meta: None,
            next_cursor,
            tools: page.into_iter().collect(),
        })
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.state.call_count.fetch_add(1, Ordering::SeqCst);
        match request.name.as_ref() {
            "echo" => {
                let text = request
                    .arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get("text"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned();
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            "slow_progress" => {
                // `context.meta`, not `request.meta`: rmcp's receive path moves
                // `_meta` *out* of the deserialized params and into the request
                // context (`service.rs`'s `std::mem::swap(&mut meta,
                // request.get_meta_mut())`), so the params-level field this
                // previously read is empty by construction. Reading it there
                // made the fixture silently never send progress at all, which
                // presented as a client-side delivery failure.
                let progress_token = context.meta.get_progress_token();
                self.state
                    .last_call_had_progress_token
                    .store(progress_token.is_some(), Ordering::SeqCst);
                for step in 1..=2u32 {
                    if let Some(token) = progress_token.clone() {
                        let notification = ProgressNotificationParam::new(token, f64::from(step))
                            .with_total(2.0)
                            .with_message(format!("step {step}"));
                        let _ = context.peer.notify_progress(notification).await;
                    }
                    tokio::select! {
                        biased;
                        () = context.ct.cancelled() => {
                            self.state
                                .last_call_cancelled
                                .store(true, Ordering::SeqCst);
                            return Err(ErrorData::internal_error("cancelled", None));
                        }
                        () = tokio::time::sleep(Duration::from_millis(50)) => {}
                    }
                }
                Ok(CallToolResult::success(vec![ContentBlock::text("done")]))
            }
            "ida_save_dashboard" => Ok(CallToolResult::success(vec![ContentBlock::text(
                FIXTURE_SAVE_ANSWER,
            )])),
            "fabricated_progress" => {
                // Deliberately ignores the request's own progress token and
                // sends a notification for a token this client never
                // registered — simulating a hostile/misbehaving server
                // trying to grow the client's progress-tracking map.
                let notification = ProgressNotificationParam::new(fabricated_progress_token(), 1.0)
                    .with_message("nobody asked for this");
                let _ = context.peer.notify_progress(notification).await;
                Ok(CallToolResult::success(vec![ContentBlock::text("done")]))
            }
            other => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "unknown tool {other}"
            ))])),
        }
    }
}

/// A fixed token no real call ever registers, used by the
/// `"fabricated_progress"` fixture tool to simulate a hostile/misbehaving
/// server sending progress for a token nobody asked about.
pub(super) fn fabricated_progress_token() -> rmcp::model::ProgressToken {
    rmcp::model::ProgressToken(rmcp::model::NumberOrString::String(
        "fabricated-token-nobody-registered".into(),
    ))
}

/// A raw (non-`ServerHandler`) axum fixture for adversarial cases a
/// well-behaved MCP server wouldn't produce: a wrong negotiated protocol
/// version, an oversized `tools/call` response body, and a server that 401s
/// until it observes a specific bearer (for expiry/reconnect and
/// token-passthrough-rejection tests). It also records every
/// `Authorization` header it receives.
#[derive(Clone, Default)]
pub(super) struct RawFixtureState {
    pub(super) call_count: Arc<AtomicUsize>,
    pub(super) authorization_seen: Arc<Mutex<Vec<Option<String>>>>,
    pub(super) request_bodies: Arc<Mutex<Vec<serde_json::Value>>>,
}

/// Coordinates two calls from one live session so one transport failure can
/// clear the runtime before the other call reports an authorization refusal.
#[derive(Clone)]
pub(super) struct MixedCallFailureControl {
    calls: Arc<AtomicUsize>,
    both_arrived: Arc<Barrier>,
    refusal_waiting: Arc<Notify>,
    release_refusal: Arc<Notify>,
}

impl MixedCallFailureControl {
    pub(super) fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            both_arrived: Arc::new(Barrier::new(2)),
            refusal_waiting: Arc::new(Notify::new()),
            release_refusal: Arc::new(Notify::new()),
        }
    }

    pub(super) async fn wait_for_refusal(&self) {
        self.refusal_waiting.notified().await;
    }

    pub(super) fn release_refusal(&self) {
        self.release_refusal.notify_one();
    }
}

pub(super) enum RawBehavior {
    /// Negotiates the given (wrong-on-purpose) protocol version, then
    /// accepts everything else normally.
    WrongProtocolVersion(&'static str),
    /// Returns a `tools/call` response whose JSON body exceeds the client's
    /// bounded response cap.
    OversizedToolResult,
    /// 401s any request whose bearer does not equal `accepted_bearer`;
    /// otherwise behaves normally. Used for token-expiry/reconnect and
    /// token-passthrough-rejection tests.
    RequireBearer { accepted_bearer: &'static str },
    /// Refuses every request with `status` and the given `WWW-Authenticate`
    /// header — a server stating *why* it refused, per RFC 6750 §3 and
    /// RFC 9728 §5.1. Distinct from `RequireBearer`, which refuses without
    /// saying anything: the client classifies the two differently, and that
    /// difference is the point of the tests that use this.
    RefusesWithChallenge {
        status: StatusCode,
        challenge: &'static str,
    },
    /// Handshakes and lists tools normally, then refuses every `tools/call`
    /// with `status` and the given `WWW-Authenticate` header.
    ///
    /// This is the grant that expires *after* the session is up — the case a
    /// handshake-only refusal cannot reach, and the one that used to arrive
    /// indistinguishable from a dead transport. Advertises one tool, because a
    /// call is validated against the catalog before it is ever sent and an empty
    /// catalog would fail it locally instead.
    RefusesListingWithChallenge {
        status: StatusCode,
        challenge: &'static str,
    },
    RefusesCallsWithChallenge {
        status: StatusCode,
        challenge: &'static str,
    },
    /// Holds the first call's refusal until a replacement session is ready.
    /// Subsequent calls succeed, so the test can use the actual retry path.
    DeferredCallRefusal {
        gate: RequestGate,
        challenge: &'static str,
    },
    /// Lets two calls enter one session, then fails one generically while the
    /// refused call waits for the test to release it.
    MixedCallFailures {
        control: MixedCallFailureControl,
        challenge: &'static str,
    },
    /// Answers the first `refusals` `tools/call` requests with HTTP 429, as
    /// DA's rate limiter does, with `Retry-After` when given; then succeeds.
    RateLimitsCalls {
        refusals: usize,
        retry_after: Option<&'static str>,
    },
    /// Answers the handshake's `initialize` with HTTP 429 and the given
    /// `Retry-After`, as DA's rate limiter does for a connect over its quota.
    RateLimitsInitialize { retry_after: &'static str },
    /// Answers the server's `notifications/cancelled` with HTTP 429. Holds the
    /// first `tools/call` until the test releases `gate`, fails the second with
    /// a plain 500 (a transport failure with no challenge), and succeeds after.
    RateLimitsCancellation { gate: RequestGate },
}

pub(super) async fn spawn_raw_fixture(
    behavior: RawBehavior,
    state: RawFixtureState,
) -> (String, ServerGuard) {
    #[derive(Clone)]
    struct Shared {
        behavior: Arc<RawBehavior>,
        state: RawFixtureState,
    }

    async fn handler(
        State(shared): State<Shared>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        let bearer = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_start_matches("Bearer ").to_owned());
        shared
            .state
            .authorization_seen
            .lock()
            .expect("lock")
            .push(bearer.clone());

        let parsed: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}})
                        .to_string(),
                );
            }
        };
        shared
            .state
            .request_bodies
            .lock()
            .expect("lock")
            .push(parsed.clone());

        if let RawBehavior::RefusesWithChallenge { status, challenge } = shared.behavior.as_ref() {
            return (
                *status,
                [(axum::http::header::WWW_AUTHENTICATE, *challenge)],
                String::new(),
            );
        }

        if let RawBehavior::RequireBearer { accepted_bearer } = shared.behavior.as_ref() {
            if bearer.as_deref() != Some(*accepted_bearer) {
                return (
                    StatusCode::UNAUTHORIZED,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    String::new(),
                );
            }
        }

        let method = parsed
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let id = parsed.get("id").cloned().unwrap_or(serde_json::Value::Null);

        if method == "initialize" {
            if let RawBehavior::RateLimitsInitialize { retry_after } = shared.behavior.as_ref() {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(axum::http::header::RETRY_AFTER, *retry_after)],
                    json!({"error": {"code": "rate_limited", "message": "Rate limit exceeded."}})
                        .to_string(),
                );
            }
            let protocol_version = match shared.behavior.as_ref() {
                RawBehavior::WrongProtocolVersion(version) => (*version).to_owned(),
                _ => ProtocolVersion::V_2025_11_25.as_str().to_owned(),
            };
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": protocol_version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "raw-fixture", "version": "0.0.0"}
                }
            });
            return (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                response.to_string(),
            );
        }
        if method == "notifications/initialized" {
            return (
                StatusCode::ACCEPTED,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                String::new(),
            );
        }
        if method == "notifications/cancelled"
            && matches!(
                shared.behavior.as_ref(),
                RawBehavior::RateLimitsCancellation { .. }
            )
        {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                String::new(),
            );
        }
        if method == "tools/call" {
            let call_number = shared.state.call_count.fetch_add(1, Ordering::SeqCst);
            if let RawBehavior::DeferredCallRefusal { gate, challenge } = shared.behavior.as_ref() {
                if call_number == 0 {
                    gate.wait_for_release().await;
                    return (
                        StatusCode::UNAUTHORIZED,
                        [(axum::http::header::WWW_AUTHENTICATE, *challenge)],
                        String::new(),
                    );
                }
            }
            if let RawBehavior::RateLimitsCancellation { gate } = shared.behavior.as_ref() {
                if call_number == 0 {
                    // The call the test cancels, so its answer is never read.
                    gate.wait_for_release().await;
                }
                if call_number == 1 {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [(axum::http::header::CONTENT_TYPE, "text/plain")],
                        "upstream failure".to_owned(),
                    );
                }
            }
            if let RawBehavior::MixedCallFailures { control, challenge } = shared.behavior.as_ref()
            {
                let call = control.calls.fetch_add(1, Ordering::SeqCst);
                control.both_arrived.wait().await;
                if call == 0 {
                    return (
                        StatusCode::UNAUTHORIZED,
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        String::new(),
                    );
                }
                control.refusal_waiting.notify_one();
                control.release_refusal.notified().await;
                return (
                    StatusCode::UNAUTHORIZED,
                    [(axum::http::header::WWW_AUTHENTICATE, *challenge)],
                    String::new(),
                );
            }
            if let RawBehavior::RateLimitsCalls {
                refusals,
                retry_after,
            } = shared.behavior.as_ref()
            {
                if call_number < *refusals {
                    let body = json!({"error": {"code": "rate_limited", "message": "Rate limit exceeded."}})
                        .to_string();
                    return match retry_after {
                        Some(seconds) => (
                            StatusCode::TOO_MANY_REQUESTS,
                            [(axum::http::header::RETRY_AFTER, *seconds)],
                            body,
                        ),
                        None => (
                            StatusCode::TOO_MANY_REQUESTS,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            body,
                        ),
                    };
                }
            }
            if let RawBehavior::RefusesCallsWithChallenge { status, challenge } =
                shared.behavior.as_ref()
            {
                return (
                    *status,
                    [(axum::http::header::WWW_AUTHENTICATE, *challenge)],
                    String::new(),
                );
            }
            if matches!(shared.behavior.as_ref(), RawBehavior::OversizedToolResult) {
                let huge_text = "x".repeat(super::super::client::MAX_MCP_RESPONSE_BYTES + 1024);
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": huge_text}], "isError": false}
                });
                return (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    response.to_string(),
                );
            }
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"content": [{"type": "text", "text": "ok"}], "isError": false}
            });
            return (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                response.to_string(),
            );
        }
        if method == "tools/list" {
            if let RawBehavior::RefusesListingWithChallenge { status, challenge } =
                shared.behavior.as_ref()
            {
                return (
                    *status,
                    [(axum::http::header::WWW_AUTHENTICATE, *challenge)],
                    String::new(),
                );
            }
            let tools = if matches!(
                shared.behavior.as_ref(),
                RawBehavior::RefusesCallsWithChallenge { .. }
                    | RawBehavior::MixedCallFailures { .. }
                    | RawBehavior::DeferredCallRefusal { .. }
                    | RawBehavior::RateLimitsCalls { .. }
                    | RawBehavior::RateLimitsCancellation { .. }
            ) {
                json!([{
                    "name": "echo",
                    "description": "Echoes its input.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"],
                        "additionalProperties": false
                    }
                }])
            } else {
                json!([])
            };
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": tools}
            });
            return (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                response.to_string(),
            );
        }

        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json!({"jsonrpc": "2.0", "id": id, "result": {}}).to_string(),
        )
    }

    let shared = Shared {
        behavior: Arc::new(behavior),
        state,
    };
    let router = Router::new()
        .route("/mcp", post(handler))
        .with_state(shared);
    spawn_router(router).await
}

/// A minimal raw fixture for exactly one adversarial case: a GET request
/// (the persistent Streamable HTTP push channel `get_stream` opens) answered
/// with a `text/event-stream` body larger than any bounded client should
/// ever accept. Deliberately separate from [`spawn_raw_fixture`]/
/// [`RawBehavior`] — this targets `BoundedMcpHttpClient::get_stream`
/// directly rather than a full `rmcp` session (whose worker tolerates a
/// failed/absent persistent stream and would not surface this failure
/// anywhere a test could directly observe it).
pub(super) async fn spawn_oversized_sse_push_fixture(
    oversized_body_len: usize,
) -> (String, ServerGuard) {
    async fn handler(
        axum::extract::State(oversized_body_len): axum::extract::State<usize>,
    ) -> impl IntoResponse {
        let huge_event = format!("data: {}\n\n", "x".repeat(oversized_body_len));
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            huge_event,
        )
    }

    let router = Router::new()
        .route("/mcp", axum::routing::get(handler))
        .with_state(oversized_body_len);
    spawn_router(router).await
}
