//! `rmcp` connection/session lifecycle for the bundled DA MCP server.
//!
//! Applies the same HTTP hardening this codebase uses everywhere it makes
//! outbound requests (`providers::mod::FixedOriginCredentialVerifier` is the
//! sibling precedent: no redirects, bounded connect/read/total timeouts, a
//! `Content-Length` pre-check AND a live streaming-chunk accumulation cap —
//! never trusting `Content-Length` alone) to the `rmcp` Streamable HTTP
//! client, since the stock `reqwest`-backed transport `rmcp` ships
//! (`rmcp::transport::common::reqwest`) buffers response bodies with no size
//! bound at all.
//!
//! Also owns:
//! - protocol-version pinning: negotiation is checked against exactly
//!   [`ProtocolVersion::V_2025_11_25`] after `initialize`, never rmcp's
//!   crate-default `LATEST` (which happens to equal `V_2025_11_25` in this
//!   rmcp release, but a future point release could move it — pinning
//!   explicitly means an rmcp bump can never silently change what protocol
//!   revision this app speaks, matching
//!   `docs/decisions.md`'s explicit
//!   instruction not to opt into the `2026-07-28` release candidate);
//! - cancellation-first requests, mirroring `providers::http::stream_sse_lines`'s
//!   `biased` `tokio::select!` (cancellation is checked before every network
//!   wait, not polled after the fact); and
//! - reconnect backoff via `tokio::time::sleep` inside a cancellable wait —
//!   never an idle busy-poll loop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::{HeaderName, HeaderValue};
use rmcp::model::{
    ClientCapabilities, ClientInfo, Implementation, ProgressNotificationParam, ProgressToken,
    ProtocolVersion,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    StreamableHttpError, StreamableHttpPostResponse,
};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use sse_stream::{Error as SseError, Sse, SseStream};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::auth::{McpAuthError, McpAuthProvider};
use super::dns::PublicOnlyResolver;
use super::server_resolver::McpServerDescriptor;
use crate::agent::TOOL_CALL_TIMEOUT;

/// Never trust a single `Content-Length` header: bound the live streaming
/// accumulation too, exactly like `providers::mod::MAX_VERIFICATION_RESPONSE_BYTES`.
pub(crate) const MAX_MCP_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest silence the client accepts inside one request.
///
/// A server that streams its answer (Server-Sent Events) sends keep-alive
/// pings while a tool runs; the DA server sends one every 15 s. A server that
/// answers with plain JSON sends nothing until it is done, and `rmcp` sends a
/// session's requests one at a time while such an answer is pending, so this
/// stays short: a longer one would hold every other call to that server.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Ceiling on one request, set past the tool loop's own per-call budget.
///
/// At 45 s this ended any `tools/call` running a warehouse query longer than
/// that, as a dropped connection, well inside the 120 s the loop allows it,
/// and the model then ran the same query into the same wall. Past the budget,
/// the loop's timeout fires first and reports the call as out of time.
/// Connects keep their own deadlines (`runtime::REGISTRATION_DEADLINE`, the
/// settings test deadline), so a dead server is not waited on any longer.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(TOOL_CALL_TIMEOUT.as_secs() + 10);
const _: () = assert!(TOTAL_TIMEOUT.as_secs() > TOOL_CALL_TIMEOUT.as_secs());

/// The MCP protocol revision this application negotiates. Pinned per
/// `docs/threat-model.md` and the MVP design spec — never
/// `ProtocolVersion::LATEST`/`ProtocolVersion::default()`, so an `rmcp`
/// upgrade cannot silently move this app onto an un-reviewed revision.
pub(crate) const PINNED_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReconnectPolicy {
    pub(crate) max_attempts: u32,
    pub(crate) base_delay: Duration,
    pub(crate) max_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl ReconnectPolicy {
    /// Pure exponential backoff (doubling, capped), zero-indexed by attempt
    /// number. Kept as a pure function so its sequence is exhaustively unit
    /// tested without any I/O.
    #[must_use]
    pub(crate) fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let scale = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        let scaled = self.base_delay.saturating_mul(scale.max(1));
        scaled.min(self.max_delay)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpClientError {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Auth(#[from] McpAuthError),
    #[error("mcp handshake failed: {0}")]
    Initialize(String),
    #[error("bundled server negotiated unsupported protocol version {0:?}")]
    UnsupportedProtocolVersion(String),
    #[error("bundled server did not report handshake info")]
    MissingServerInfo,
    #[error("mcp transport error: {0}")]
    Transport(String),
    /// The server answered 401 with a `WWW-Authenticate` challenge: the
    /// credential attached is missing, expired, or revoked. Distinct from
    /// [`Self::Auth`], which is this device declining to attach one at all.
    #[error("the server requires a new authorization")]
    AuthorizationRequired,
    /// The server answered 403 `insufficient_scope`: the grant is real but too
    /// narrow. `scope` is what the server said would satisfy it, when it said.
    #[error("the authorization does not cover what this server requires")]
    InsufficientScope { scope: Option<String> },
    /// The server answered HTTP 429 to the handshake: the credential and the
    /// network are fine, and the account is asked to wait. Carries the wait its
    /// `Retry-After` named, when it named one.
    #[error("the server is rate limiting requests")]
    RateLimited(McpRateLimit),
    #[error("reconnect exhausted after {attempts} attempt(s): {last_error}")]
    ReconnectExhausted {
        attempts: u32,
        last_error: Box<McpClientError>,
    },
}

/// Snapshot of one progress update for an in-flight tool call.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProgressSnapshot {
    pub(crate) progress: f64,
    pub(crate) total: Option<f64>,
    pub(crate) message: Option<String>,
}

impl From<&ProgressNotificationParam> for ProgressSnapshot {
    fn from(value: &ProgressNotificationParam) -> Self {
        Self {
            progress: value.progress,
            total: value.total,
            message: value.message.clone(),
        }
    }
}

/// Hard ceiling on how many calls' progress this session tracks at once.
/// The primary defense against a hostile/misbehaving server growing this
/// map is structural — [`McpNotifications::register_progress`] is the only
/// way an entry can ever exist, and only a token this client itself
/// registered for an outstanding call can occupy one (see `on_progress`
/// below) — this cap is a defense-in-depth backstop behind that, in
/// practice never approached since concurrent in-flight calls are bounded
/// elsewhere.
const MAX_TRACKED_PROGRESS_TOKENS: usize = 128;

/// Bounded shared state our [`ClientHandler`] impl records notifications
/// into, so `super::executor`/`super::catalog` can observe them without
/// owning a handler themselves.
///
/// `progress`'s value is `Option<ProgressSnapshot>` rather than
/// `ProgressSnapshot` so a token can be *registered* (present, `None`)
/// before any update has arrived for it — that registration step is what
/// makes `on_progress` accepting-by-membership rather than
/// accepting-unconditionally structural rather than a convention.
#[derive(Default)]
pub(crate) struct McpNotifications {
    progress: Mutex<HashMap<ProgressToken, Option<ProgressSnapshot>>>,
    tool_list_changed: Mutex<bool>,
    /// Monotonic count of progress notifications *accepted* — i.e. carrying a
    /// token this client had registered. Never decremented and never cleared,
    /// unlike `progress`, whose entries `McpToolExecutor` removes the instant
    /// a call returns.
    ///
    /// That cleanup is correct (the map must stay bounded to in-flight calls)
    /// but it leaves no evidence a notification was ever received, so the only
    /// way to observe one was to poll `snapshot_progress` while the call was
    /// still running — a race the test could lose whenever the whole call
    /// completed inside one poll interval. This counter records the same fact
    /// durably, so acceptance can be asserted after the call, deterministically.
    accepted_progress: AtomicU64,
}

impl McpNotifications {
    /// Registers `token` as belonging to a call this client itself started.
    /// Must be called before the request carrying `token` is sent — it is
    /// the only thing that makes `on_progress` store anything for `token`
    /// at all, which is what stops a hostile/misbehaving bundled server
    /// from growing this map by sending `notifications/progress` for
    /// tokens nobody asked about: those are silently dropped, not stored
    /// (see `on_progress`). Silently declines past
    /// [`MAX_TRACKED_PROGRESS_TOKENS`] rather than growing further; the
    /// call this token belongs to is unaffected, it just will not observe
    /// progress.
    pub(crate) fn register_progress(&self, token: ProgressToken) {
        let mut progress = self.progress.lock().expect("progress lock poisoned");
        if progress.len() >= MAX_TRACKED_PROGRESS_TOKENS && !progress.contains_key(&token) {
            return;
        }
        progress.entry(token).or_insert(None);
    }

    pub(crate) fn progress_for(&self, token: &ProgressToken) -> Option<ProgressSnapshot> {
        self.progress
            .lock()
            .expect("progress lock poisoned")
            .get(token)
            .cloned()
            .flatten()
    }

    pub(crate) fn clear_progress(&self, token: &ProgressToken) {
        self.progress
            .lock()
            .expect("progress lock poisoned")
            .remove(token);
    }

    /// Reads and clears the tool-list-changed flag. Reading is
    /// destructive-once so a caller that polls it can tell "changed since I
    /// last checked" without a separate sequence number.
    pub(crate) fn take_tool_list_changed(&self) -> bool {
        let mut flag = self.tool_list_changed.lock().expect("flag lock poisoned");
        std::mem::take(&mut *flag)
    }

    /// A bounded snapshot of every progress update actually received so far
    /// (registered tokens with no update yet are omitted). Primarily for
    /// observability/tests — there is normally at most a handful of
    /// concurrent in-flight calls.
    /// How many progress notifications have been accepted over this
    /// session's lifetime. A notification for an unregistered token never
    /// increments it, which is what makes it a usable assertion for both the
    /// accepted and the dropped case.
    #[cfg(test)]
    pub(crate) fn accepted_progress_count(&self) -> u64 {
        self.accepted_progress.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn snapshot_progress(&self) -> Vec<(ProgressToken, ProgressSnapshot)> {
        self.progress
            .lock()
            .expect("progress lock poisoned")
            .iter()
            .filter_map(|(token, snapshot)| {
                snapshot.clone().map(|snapshot| (token.clone(), snapshot))
            })
            .collect()
    }
}

/// Our `ClientHandler`: pins the initialize request's protocol version and
/// forwards the only two server notifications this client acts on
/// (progress, tool-list-changed) into [`McpNotifications`]. Every other
/// `ClientHandler` method keeps its inert default (this client never offers
/// sampling/roots/elicitation to the bundled server).
#[derive(Clone)]
struct XfalconClientHandler {
    notifications: Arc<McpNotifications>,
}

impl ClientHandler for XfalconClientHandler {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("xfalcon-desktop", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(PINNED_PROTOCOL_VERSION)
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let snapshot = ProgressSnapshot::from(&params);
        let mut progress = self
            .notifications
            .progress
            .lock()
            .expect("progress lock poisoned");
        // Only update an entry that `register_progress` already created for
        // an outstanding call of ours — never insert a new one. A token the
        // server invented (or replayed/duplicated) that this client never
        // registered is silently dropped here, not stored: this is what
        // keeps the map bounded to calls this client actually started
        // rather than whatever a hostile/misbehaving server chooses to
        // send.
        if let Some(slot) = progress.get_mut(&params.progress_token) {
            *slot = Some(snapshot);
            self.notifications
                .accepted_progress
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        *self
            .notifications
            .tool_list_changed
            .lock()
            .expect("flag lock poisoned") = true;
    }
}

/// A live, version-checked connection to the bundled DA MCP server.
pub(crate) struct McpSession {
    running: RunningService<RoleClient, XfalconClientHandler>,
    notifications: Arc<McpNotifications>,
    server_identity: String,
    /// Unique to this successful handshake. A reconnect at the same endpoint
    /// can advertise different tools or schemas.
    cache_generation: Arc<()>,
    /// The transport's own challenge recorder, shared by `Clone`.
    ///
    /// A handshake is not the only request that can be refused: a grant expires
    /// mid-session, and then it is a `tools/call` that comes back 401. `rmcp`
    /// discards the underlying `StreamableHttpError` for those too, so without
    /// this the refusal arrives as an opaque service error and reads as a dead
    /// transport. Keeping the recorder for the session's life is what lets
    /// [`Self::take_refusal`] answer the same question after the handshake that
    /// [`classify_handshake_failure`] answers during it.
    observed: BoundedMcpHttpClient,
}

impl McpSession {
    #[must_use]
    pub(crate) fn peer(&self) -> &rmcp::Peer<RoleClient> {
        self.running.peer()
    }

    /// Whether the failure just observed was this server refusing the
    /// credential, and which kind of refusal.
    ///
    /// Taken rather than copied, for the same reason the handshake takes it: a
    /// later failure must not inherit an older refusal's verdict. `None` means
    /// no challenge was recorded — the failure was something else, and must not
    /// be dressed up as an authorization problem the user can fix by signing in.
    #[must_use]
    pub(crate) fn take_refusal(&self) -> Option<McpRefusal> {
        classify_refusal(&self.observed)
    }

    /// Whether the failure just observed was the server rate limiting this
    /// account. Taken, like [`Self::take_refusal`].
    #[must_use]
    pub(crate) fn take_rate_limit(&self) -> Option<McpRateLimit> {
        self.observed.take_rate_limit()
    }

    #[must_use]
    pub(crate) fn notifications(&self) -> &McpNotifications {
        &self.notifications
    }

    /// Stable identity for cache-keying (`super::catalog`): the endpoint
    /// this session actually connected to.
    #[must_use]
    pub(crate) fn server_identity(&self) -> &str {
        &self.server_identity
    }

    #[must_use]
    pub(crate) fn cache_generation(&self) -> &Arc<()> {
        &self.cache_generation
    }

    #[must_use]
    pub(crate) fn negotiated_protocol_version(&self) -> Option<ProtocolVersion> {
        self.running
            .peer_info()
            .map(|info| info.protocol_version.clone())
    }

    #[must_use]
    pub(crate) fn is_closed(&self) -> bool {
        self.running.is_closed()
    }

    /// Gracefully closes the session. Best-effort: a transport already gone
    /// is not an error worth surfacing to a caller that is disconnecting
    /// anyway.
    pub(crate) async fn close(mut self) {
        let _ = self.running.close().await;
    }
}

/// Establishes exactly one Streamable HTTP session against `server`,
/// authenticated with `auth`, pinned to [`PINNED_PROTOCOL_VERSION`].
///
/// Cancellation-first: `cancel` is checked before the connection attempt
/// even begins, and races the handshake itself (`biased` — cancellation wins
/// ties), so a caller that cancels during a slow/hanging handshake is never
/// left waiting on it.
pub(crate) async fn connect(
    server: &McpServerDescriptor,
    auth_provider: &dyn McpAuthProvider,
    cancel: &CancellationToken,
) -> Result<McpSession, McpClientError> {
    if cancel.is_cancelled() {
        return Err(McpClientError::Cancelled);
    }

    let auth = auth_provider.current_auth(server).await?;
    let http_client = BoundedMcpHttpClient::new(server.public_hosts_only())?;
    // Cloned before the transport takes ownership: `Clone` shares the challenge
    // slot, so this handle reads whatever the transport's own copy recorded.
    let observed = http_client.clone();
    let mut transport_config =
        StreamableHttpClientTransportConfig::with_uri(Arc::<str>::from(server.endpoint().as_str()))
            .reinit_on_expired_session(true);
    // Exactly one of these applies per scheme, and an anonymous scheme applies
    // neither — `McpAuth` is the only type that can produce either, so a tool
    // argument or model output has no path to a header here.
    if let Some(bearer) = auth.bearer_value() {
        transport_config = transport_config.auth_header(bearer.to_owned());
    }
    if let Some((name, value)) = auth.custom_header() {
        transport_config.custom_headers.insert(name, value);
    }
    let transport = StreamableHttpClientTransport::with_client(http_client, transport_config);

    let notifications = Arc::new(McpNotifications::default());
    let handler = XfalconClientHandler {
        notifications: notifications.clone(),
    };

    let running = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(McpClientError::Cancelled),
        result = handler.serve_with_ct(transport, cancel.child_token()) => {
            result.map_err(|error| classify_handshake_failure(&observed, error))?
        }
    };

    let negotiated = running
        .peer_info()
        .ok_or(McpClientError::MissingServerInfo)?;
    if negotiated.protocol_version != PINNED_PROTOCOL_VERSION {
        let version = negotiated.protocol_version.as_str().to_owned();
        let _ = running.cancel().await;
        return Err(McpClientError::UnsupportedProtocolVersion(version));
    }

    Ok(McpSession {
        running,
        notifications,
        server_identity: server.identity(),
        cache_generation: Arc::new(()),
        observed,
    })
}

/// Connects with bounded exponential backoff between attempts. Every wait is
/// a single cancellable `tokio::time::sleep` — never a busy-poll loop — and
/// cancellation always wins a race against the sleep (`biased`).
///
/// Once `policy.max_attempts` is reached, returns
/// [`McpClientError::ReconnectExhausted`] wrapping the last attempt's actual
/// error — never that raw error directly — so a caller can distinguish "this
/// failed after retrying" from "this failed on the first attempt" (a
/// distinction later observability/error-handling code needs and which a
/// bare pass-through of the last error would silently lose).
///
/// A rate limit ends the sequence at once, like a refusal: the next attempt
/// would be the request the server has just asked to wait out.
pub(crate) async fn connect_with_reconnect(
    server: &McpServerDescriptor,
    auth_provider: &dyn McpAuthProvider,
    policy: &ReconnectPolicy,
    cancel: &CancellationToken,
) -> Result<McpSession, McpClientError> {
    let mut attempt = 0u32;
    loop {
        match connect(server, auth_provider, cancel).await {
            Ok(session) => return Ok(session),
            Err(
                error @ (McpClientError::Cancelled
                | McpClientError::AuthorizationRequired
                | McpClientError::InsufficientScope { .. }
                | McpClientError::RateLimited(_)
                | McpClientError::Auth(McpAuthError::MissingBearer)),
            ) => return Err(error),
            Err(error) => {
                attempt += 1;
                if attempt >= policy.max_attempts {
                    return Err(McpClientError::ReconnectExhausted {
                        attempts: attempt,
                        last_error: Box::new(error),
                    });
                }
                let delay = policy.delay_for_attempt(attempt);
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(McpClientError::Cancelled),
                    () = sleep(delay) => {}
                }
            }
        }
    }
}

/// Turns a failed handshake into the most specific error the exchange supports.
///
/// A 401/403 that carried a `WWW-Authenticate` challenge is an authorization
/// problem the user can fix by signing in again — reporting it as "could not be
/// reached" sends them to check their network instead. The challenge is parsed
/// rather than pattern-matched on the status alone, because `insufficient_scope`
/// and an expired grant need different words and only the header distinguishes
/// them (RFC 6750 §3, RFC 9728 §5.1).
///
/// A 429 is a rate limit the user can wait out, so it is reported as one rather
/// than as a failed handshake. A refusal wins if both were recorded.
fn classify_handshake_failure<E: std::fmt::Display>(
    observed: &BoundedMcpHttpClient,
    error: E,
) -> McpClientError {
    match classify_refusal(observed) {
        Some(refusal) => refusal.into(),
        None => match observed.take_rate_limit() {
            Some(limit) => McpClientError::RateLimited(limit),
            None => McpClientError::Initialize(error.to_string()),
        },
    }
}

/// How a server refused the credential it was given, when it did.
///
/// Shared by the handshake and by a live session's calls so both answer the
/// question the same way — a refusal that counted while connecting cannot stop
/// counting once connected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum McpRefusal {
    /// The credential was refused outright: expired, revoked, wrong audience.
    AuthorizationRequired,
    /// The credential is genuine and too narrow. `scope` is what the server
    /// said would satisfy it, when it said.
    InsufficientScope { scope: Option<String> },
}

impl From<McpRefusal> for McpClientError {
    fn from(value: McpRefusal) -> Self {
        match value {
            McpRefusal::AuthorizationRequired => Self::AuthorizationRequired,
            McpRefusal::InsufficientScope { scope } => Self::InsufficientScope { scope },
        }
    }
}

/// Reads the recorded challenge, if any, and says what kind of refusal it was.
///
/// Taken, not copied: a challenge belongs to the failure that produced it, and
/// leaving it in place would let the next unrelated failure be classified by
/// it. No challenge means no refusal — the caller must not infer one.
///
/// Any challenge that is not `insufficient_scope` counts as a refusal, even one
/// naming an error this build does not recognize. That default is deliberate:
/// the server refused the credential, which is the actionable part, and the
/// alternative — falling through to a generic transport failure — is the exact
/// bug this whole path exists to fix.
fn classify_refusal(observed: &BoundedMcpHttpClient) -> Option<McpRefusal> {
    let header = observed.take_challenge()?;
    let challenge = super::oauth::challenge::parse(Some(header.as_str()));
    Some(match challenge.insufficient_scope() {
        Some(scope) => McpRefusal::InsufficientScope {
            scope: Some(scope.to_owned()),
        },
        None => McpRefusal::AuthorizationRequired,
    })
}

/// A `StreamableHttpClient` that mirrors `rmcp`'s own `reqwest`-backed
/// implementation exactly (same headers, same status-code handling, same
/// session-id/auth-required/insufficient-scope semantics — deliberately not
/// reimplemented from scratch, to avoid subtly diverging from the spec
/// behavior `rmcp` already gets right) except that every response body —
/// both the one-shot `post_message` response *and* the persistent
/// server-push channel `get_stream` opens for the life of the session — is
/// read through [`read_bounded_bytes`]/[`bound_byte_stream`], which enforce
/// [`MAX_MCP_RESPONSE_BYTES`] via both a `Content-Length` pre-check (where a
/// length is even known) and a live streaming-chunk accumulation cap.
/// `delete_session` delegates to the stock `reqwest::Client` impl unchanged:
/// it has no response body of consequence.
#[derive(Clone)]
pub(crate) struct BoundedMcpHttpClient {
    inner: reqwest::Client,
    /// The last `WWW-Authenticate` challenge this client saw, if any.
    ///
    /// `rmcp` reports a failed handshake as its own error type and does not
    /// preserve the `StreamableHttpError` underneath, so by the time `connect`
    /// sees a failure the 401's challenge — the only thing that says *why* the
    /// server refused, and what scope would satisfy it — is gone. Recording it
    /// here, in the layer that actually read the header, is what lets `connect`
    /// report "sign in again" instead of "could not be reached".
    challenge: Arc<Mutex<Option<String>>>,
    /// The last rate-limit answer (HTTP 429) this client saw, with the wait
    /// its `Retry-After` asked for. Recorded here for the same reason as
    /// `challenge`: `rmcp` hands the caller an opaque error, and without this a
    /// rate limit reads as a dead connection and costs the session.
    rate_limit: Arc<Mutex<Option<McpRateLimit>>>,
}

/// The server answered HTTP 429: too many requests from this account.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McpRateLimit {
    /// The wait the server's `Retry-After` named, in whole seconds, capped at
    /// [`MAX_RETRY_AFTER`]. `None` when it sent none or an HTTP-date.
    pub(crate) retry_after: Option<Duration>,
}

/// The longest `Retry-After` taken at its word. A larger value is still a rate
/// limit; it is just not a wait anyone should sit through inside a tool call.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3_600);

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|seconds| Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

impl BoundedMcpHttpClient {
    /// `public_hosts_only` routes DNS through [`PublicOnlyResolver`], so a name
    /// resolving to a non-public address is refused before any connection.
    pub(crate) fn new(public_hosts_only: bool) -> Result<Self, McpClientError> {
        let mut builder = reqwest::Client::builder()
            .user_agent(super::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .timeout(TOTAL_TIMEOUT);
        if public_hosts_only {
            builder = builder.dns_resolver(Arc::new(PublicOnlyResolver::new()));
        }
        let inner = builder
            .build()
            .map_err(|error| McpClientError::Transport(error.to_string()))?;
        Ok(Self {
            inner,
            challenge: Arc::new(Mutex::new(None)),
            rate_limit: Arc::new(Mutex::new(None)),
        })
    }

    /// The challenge observed during this client's lifetime, taken (not copied)
    /// so a later reconnect attempt cannot be classified by an older refusal.
    fn take_challenge(&self) -> Option<String> {
        self.challenge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// The rate limit observed since the last take, taken so a later failure
    /// cannot inherit an older verdict.
    fn take_rate_limit(&self) -> Option<McpRateLimit> {
        self.rate_limit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn record_rate_limit(&self, limit: McpRateLimit) {
        *self
            .rate_limit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(limit);
    }

    fn record_challenge(&self, header: &str) {
        *self
            .challenge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(header.to_owned());
    }
}

impl StreamableHttpClient for BoundedMcpHttpClient {
    type Error = reqwest::Error;

    /// The persistent server-initiated push channel (progress, list-changed,
    /// and any other out-of-band notification for the *entire lifetime of
    /// the session*) — not a one-shot request/response like `post_message`.
    /// The stock `reqwest`-backed implementation this mirrors
    /// (`rmcp::transport::common::reqwest::streamable_http_client`) calls
    /// `response.bytes_stream()` straight into `SseStream::from_bytes_stream`
    /// with no bound at all, so a compromised bundled server could stream
    /// unbounded bytes down this channel for as long as the session stays
    /// open. This mirrors that implementation's exact request/status/
    /// content-type handling but wraps the byte stream through
    /// [`bound_byte_stream`] first, identically to `post_message`'s SSE
    /// branch.
    ///
    /// The cap is enforced per *connection attempt* to this channel, not
    /// cumulatively for the session's entire life: `rmcp`'s worker calls
    /// `get_stream` again (with `last_event_id` for resumption) every time
    /// this stream ends or errors — including when this bound trips, which
    /// ends the stream with an error the same way any other transport
    /// failure would — so a long-lived healthy session naturally gets a
    /// fresh budget on each reconnect rather than being penalized for
    /// accumulating small, legitimate notifications over time.
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        use rmcp::transport::common::http_header::{
            EVENT_STREAM_MIME_TYPE, HEADER_LAST_EVENT_ID, HEADER_SESSION_ID, JSON_MIME_TYPE,
        };

        let mut request = self
            .inner
            .get(uri.as_ref())
            .header(
                reqwest::header::ACCEPT,
                [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
            )
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(last_event_id) = last_event_id {
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(bearer) = auth_header {
            request = request.bearer_auth(bearer);
        }
        for (name, value) in custom_headers {
            request = request.header(name, value);
        }

        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        let response = response.error_for_status()?;
        match response.headers().get(reqwest::header::CONTENT_TYPE) {
            Some(content_type) => {
                if !content_type
                    .as_bytes()
                    .starts_with(EVENT_STREAM_MIME_TYPE.as_bytes())
                    && !content_type
                        .as_bytes()
                        .starts_with(JSON_MIME_TYPE.as_bytes())
                {
                    return Err(StreamableHttpError::UnexpectedContentType(Some(
                        String::from_utf8_lossy(content_type.as_bytes()).into_owned(),
                    )));
                }
            }
            None => return Err(StreamableHttpError::UnexpectedContentType(None)),
        }

        let bounded = bound_byte_stream(response.bytes_stream());
        Ok(SseStream::from_bytes_stream(bounded).boxed())
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.inner
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: rmcp::model::ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        use http::header::WWW_AUTHENTICATE;
        use rmcp::transport::common::http_header::HEADER_SESSION_ID;
        use rmcp::transport::streamable_http_client::{AuthRequiredError, InsufficientScopeError};

        const ACCEPT_VALUE: &str = "text/event-stream, application/json";

        let mut request = self
            .inner
            .post(uri.as_ref())
            .header(reqwest::header::ACCEPT, ACCEPT_VALUE);
        if let Some(bearer) = auth_header {
            request = request.bearer_auth(bearer);
        }
        for (name, value) in custom_headers {
            request = request.header(name, value);
        }
        let session_was_attached = session_id.is_some();
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = request.json(&message).send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header = header
                    .to_str()
                    .map_err(|_| {
                        StreamableHttpError::UnexpectedServerResponse(
                            "invalid www-authenticate header value".into(),
                        )
                    })?
                    .to_owned();
                self.record_challenge(&header);
                return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                    header,
                )));
            }
            // No challenge, so no refusal is recorded and the failure stays a
            // handshake failure — `classify_refusal` must not invent a claim the
            // server never made, and `connect_with_reconnect` still gets to
            // retry with a re-fetched credential.
            //
            // The cost is that an expired OAuth session on a server that omits
            // `WWW-Authenticate` is indistinguishable here from a dead endpoint,
            // and both are reported `Unreachable`. Log it, because that
            // ambiguity is otherwise invisible and the two have opposite fixes.
            tracing::warn!(
                "MCP server answered 401 with no WWW-Authenticate challenge; \
                 reported as a handshake failure, not a sign-in prompt"
            );
        }
        if response.status() == reqwest::StatusCode::FORBIDDEN {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header_str = header.to_str().map_err(|_| {
                    StreamableHttpError::UnexpectedServerResponse(
                        "invalid www-authenticate header value".into(),
                    )
                })?;
                self.record_challenge(header_str);
                return Err(StreamableHttpError::InsufficientScope(
                    InsufficientScopeError::new(header_str.to_owned(), None),
                ));
            }
        }

        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == reqwest::StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
        let content_length = response.content_length();
        let response_session_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        if status.is_success()
            && content_length == Some(0)
            && matches!(
                message,
                rmcp::model::ClientJsonRpcMessage::Notification(_)
                    | rmcp::model::ClientJsonRpcMessage::Response(_)
                    | rmcp::model::ClientJsonRpcMessage::Error(_)
            )
        {
            return Ok(StreamableHttpPostResponse::Accepted);
        }

        // A notification has no caller waiting on its answer, so its 429 is not
        // recorded: a verdict left here would be taken by the next unrelated
        // failure on this session and reported as a rate limit.
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
            && !matches!(message, rmcp::model::ClientJsonRpcMessage::Notification(_))
        {
            self.record_rate_limit(McpRateLimit {
                retry_after: retry_after(&response),
            });
        }
        if !status.is_success() {
            let body = read_bounded_text(response)
                .await
                .unwrap_or_else(|_| "<response exceeded the maximum allowed size>".to_owned());
            if content_type
                .as_deref()
                .is_some_and(|value| value.starts_with("application/json"))
            {
                if let Ok(rmcp::model::JsonRpcMessage::Error(parsed)) =
                    serde_json::from_str::<rmcp::model::ServerJsonRpcMessage>(&body)
                {
                    return Ok(StreamableHttpPostResponse::Json(
                        rmcp::model::JsonRpcMessage::Error(parsed),
                        response_session_id,
                    ));
                }
            }
            return Err(StreamableHttpError::UnexpectedServerResponse(
                format!("HTTP {status}: {body}").into(),
            ));
        }

        match content_type.as_deref() {
            Some(value) if value.starts_with("text/event-stream") => {
                let bounded = bound_byte_stream(response.bytes_stream());
                let event_stream = SseStream::from_bytes_stream(bounded).boxed();
                Ok(StreamableHttpPostResponse::Sse(
                    event_stream,
                    response_session_id,
                ))
            }
            Some(value) if value.starts_with("application/json") => {
                let bytes = read_bounded_bytes(response).await?;
                match serde_json::from_slice::<rmcp::model::ServerJsonRpcMessage>(&bytes) {
                    Ok(parsed) => Ok(StreamableHttpPostResponse::Json(
                        parsed,
                        response_session_id,
                    )),
                    Err(_) => Ok(StreamableHttpPostResponse::Accepted),
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }
}

/// Reads a response body up to [`MAX_MCP_RESPONSE_BYTES`], checking
/// `Content-Length` first and enforcing the same cap against the live
/// streamed total — a response that lies about its `Content-Length` (or
/// omits it, as chunked transfer does) cannot bypass the bound.
async fn read_bounded_bytes(
    response: reqwest::Response,
) -> Result<Vec<u8>, StreamableHttpError<reqwest::Error>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MCP_RESPONSE_BYTES as u64)
    {
        return Err(too_large());
    }
    let mut buffer = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.extend_from_slice(&chunk);
        if buffer.len() > MAX_MCP_RESPONSE_BYTES {
            return Err(too_large());
        }
    }
    Ok(buffer)
}

async fn read_bounded_text(
    response: reqwest::Response,
) -> Result<String, StreamableHttpError<reqwest::Error>> {
    let bytes = read_bounded_bytes(response).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn too_large() -> StreamableHttpError<reqwest::Error> {
    StreamableHttpError::UnexpectedServerResponse(
        format!("response exceeds {MAX_MCP_RESPONSE_BYTES} bytes").into(),
    )
}

/// Wraps a raw byte stream with a cumulative size cap for the SSE branch of
/// `post_message`. Once the cap trips, every subsequent item is the same
/// error (the caller/`SseStream` parser stops consuming after the first
/// error either way).
fn bound_byte_stream(
    stream: impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl futures_util::Stream<Item = Result<Bytes, BoundedBodyError>> + Send + 'static {
    let mut total = 0usize;
    let mut tripped = false;
    stream.map(move |chunk| {
        if tripped {
            return Err(BoundedBodyError::TooLarge);
        }
        let chunk = chunk.map_err(BoundedBodyError::Upstream)?;
        total = total.saturating_add(chunk.len());
        if total > MAX_MCP_RESPONSE_BYTES {
            tripped = true;
            return Err(BoundedBodyError::TooLarge);
        }
        Ok(chunk)
    })
}

#[derive(Debug, thiserror::Error)]
enum BoundedBodyError {
    #[error(transparent)]
    Upstream(#[from] reqwest::Error),
    #[error("response exceeds {MAX_MCP_RESPONSE_BYTES} bytes")]
    TooLarge,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_backoff_doubles_and_caps() {
        let policy = ReconnectPolicy {
            max_attempts: 6,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        };
        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(800));
        assert_eq!(policy.delay_for_attempt(4), Duration::from_millis(1600));
        // Capped rather than continuing to double.
        assert_eq!(policy.delay_for_attempt(5), Duration::from_secs(2));
        assert_eq!(policy.delay_for_attempt(20), Duration::from_secs(2));
    }

    #[test]
    fn pinned_protocol_version_matches_2025_11_25() {
        assert_eq!(PINNED_PROTOCOL_VERSION.as_str(), "2025-11-25");
        assert_eq!(PINNED_PROTOCOL_VERSION, ProtocolVersion::V_2025_11_25);
    }

    /// Pure unit coverage of the exact combinator both `post_message`'s SSE
    /// branch and `get_stream` (the persistent push channel) route their
    /// byte stream through — no network needed.
    #[tokio::test]
    async fn bound_byte_stream_passes_through_small_chunks_unchanged() {
        let chunks: Vec<Result<Bytes, reqwest::Error>> = vec![
            Ok(Bytes::from_static(b"hello ")),
            Ok(Bytes::from_static(b"world")),
        ];
        let mut bounded = bound_byte_stream(futures_util::stream::iter(chunks));
        let mut collected = Vec::new();
        while let Some(chunk) = bounded.next().await {
            collected.push(chunk.expect("under the cap must not error"));
        }
        let total: usize = collected.iter().map(|chunk| chunk.len()).sum();
        assert_eq!(total, "hello world".len());
    }

    #[tokio::test]
    async fn bound_byte_stream_errors_once_cumulative_size_exceeds_the_cap() {
        let oversized_chunk = Bytes::from(vec![b'x'; MAX_MCP_RESPONSE_BYTES + 1]);
        // A trailing chunk is required to exercise the `tripped` latch at all:
        // with a single-element source the second poll returns `None` from the
        // exhausted iterator, so the assertion below would be testing stream
        // termination rather than the bound holding across further data.
        let chunks: Vec<Result<Bytes, reqwest::Error>> = vec![
            Ok(oversized_chunk),
            Ok(Bytes::from_static(b"more data after the cap was exceeded")),
        ];
        let mut bounded = bound_byte_stream(futures_util::stream::iter(chunks));

        let first = bounded
            .next()
            .await
            .expect("the stream must yield at least one item");
        assert!(matches!(first, Err(BoundedBodyError::TooLarge)));

        // Once tripped, every subsequent poll must keep erroring rather than
        // silently resuming — a caller that keeps polling past the first
        // error can never observe more data than the bound.
        assert!(matches!(
            bounded.next().await,
            Some(Err(BoundedBodyError::TooLarge))
        ));
    }

    #[tokio::test]
    async fn bound_byte_stream_trips_on_cumulative_total_across_many_small_chunks() {
        // No single chunk is oversized; the cap is on the running total.
        let chunk = Bytes::from(vec![b'x'; 1024]);
        let chunk_count = (MAX_MCP_RESPONSE_BYTES / 1024) + 4;
        let chunks: Vec<Result<Bytes, reqwest::Error>> =
            std::iter::repeat_with(|| Ok(chunk.clone()))
                .take(chunk_count)
                .collect();
        let mut bounded = bound_byte_stream(futures_util::stream::iter(chunks));

        let mut total_before_error = 0usize;
        let mut saw_error = false;
        while let Some(item) = bounded.next().await {
            match item {
                Ok(bytes) => total_before_error += bytes.len(),
                Err(BoundedBodyError::TooLarge) => {
                    saw_error = true;
                    break;
                }
                Err(other) => panic!("unexpected error: {other}"),
            }
        }
        assert!(saw_error, "cumulative total must eventually trip the cap");
        assert!(total_before_error <= MAX_MCP_RESPONSE_BYTES);
    }
}
