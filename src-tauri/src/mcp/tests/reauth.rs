//! A grant that expires while a run is using it (spec: mcp-reauth-prompt).
//!
//! The failure these cover: a refused credential used to arrive at the agent
//! layer as `Disconnected` — the same thing a dead pipe produces — so the run
//! retried it, lost the server's tools, and told the user nothing. The model
//! then explained it had no way to do the thing, while the server sat one
//! sign-in away from working.
//!
//! Every server here is a real HTTP server answering a real 401 with a real
//! `WWW-Authenticate` challenge, so the classification is exercised end to end
//! rather than mocked at the seam it is meant to prove.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::task::Poll;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::mcp::{
    McpPermissionMode, McpReauthReason, McpReauthRequired, BUNDLED_SERVER_ID,
};
use xfalcon_contracts::tools::ToolId;
use zeroize::Zeroizing;

use super::support::{
    expect_connect_err, spawn_raw_fixture, MixedCallFailureControl, RawBehavior, RawFixtureState,
    RequestGate,
};
use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort};
use crate::mcp::auth::{McpAuth, McpAuthError, McpAuthProvider};
use crate::mcp::client::{connect_with_reconnect, McpClientError, ReconnectPolicy};
use crate::mcp::dispatch::{McpToolDispatcher, ReauthNotifier};
use crate::mcp::runtime::ServerContribution;
use crate::mcp::runtime::{McpRuntime, McpRuntimeSet, McpServerIdentity, SessionInvalidation};
use crate::mcp::server_resolver::McpServerDescriptor;

const EXPIRED_CHALLENGE: &str = r#"Bearer error="invalid_token", error_description="expired""#;
const NARROW_CHALLENGE: &str = r#"Bearer error="insufficient_scope", scope="mcp:tools:write""#;

// tracing-core 0.1.36's one-registrar fast path asks the thread that first
// reaches a callsite for its current dispatcher. Keep a second registrar alive
// so an unscoped parallel test cannot cache this module's captured callsites as
// permanently disabled before the scoped diagnostics future reaches them.
static DIAGNOSTICS_TEST_REGISTRAR: LazyLock<tracing::Dispatch> =
    LazyLock::new(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::new()));

fn diagnostics_test_subscriber(
    log: std::fs::File,
) -> impl tracing::Subscriber + Send + Sync + 'static {
    LazyLock::force(&DIAGNOSTICS_TEST_REGISTRAR);
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(Mutex::new(log))
        .finish()
}

/// Counts how often the credential was forgotten, which is the observable half
/// of "a refused grant is evicted rather than replayed".
struct CountingAuth {
    forgotten: AtomicUsize,
    blocking_forget: Option<Arc<BlockingForgetControl>>,
    auth_calls: AtomicUsize,
}

impl Default for CountingAuth {
    fn default() -> Self {
        Self {
            forgotten: AtomicUsize::new(0),
            blocking_forget: None,
            auth_calls: AtomicUsize::new(0),
        }
    }
}

impl CountingAuth {
    fn with_blocking_forget(control: Arc<BlockingForgetControl>) -> Self {
        Self {
            forgotten: AtomicUsize::new(0),
            blocking_forget: Some(control),
            auth_calls: AtomicUsize::new(0),
        }
    }
}

struct BlockingForgetControl {
    started: tokio::sync::Notify,
    released: (Mutex<bool>, Condvar),
}

impl BlockingForgetControl {
    fn new() -> Self {
        Self {
            started: tokio::sync::Notify::new(),
            released: (Mutex::new(false), Condvar::new()),
        }
    }

    async fn wait_until_started(&self) {
        self.started.notified().await;
    }

    fn release(&self) {
        let (released, wake) = &self.released;
        *released.lock().expect("release lock") = true;
        wake.notify_one();
    }

    fn block_until_released(&self) {
        let (released, wake) = &self.released;
        let mut released = released.lock().expect("release lock");
        while !*released {
            released = wake.wait(released).expect("release lock after wait");
        }
    }
}

#[async_trait::async_trait]
impl McpAuthProvider for CountingAuth {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        McpAuth::bind(
            Zeroizing::new("stale-token".to_owned()),
            server.da_origin().as_str(),
            server,
        )
    }

    fn forget(&self) {
        self.forgotten.fetch_add(1, Ordering::SeqCst);
        if let Some(control) = &self.blocking_forget {
            control.started.notify_one();
            control.block_until_released();
        }
    }
}

#[derive(Default)]
struct RecordingNotifier {
    prompts: Mutex<Vec<McpReauthRequired>>,
}

impl RecordingNotifier {
    fn prompts(&self) -> Vec<McpReauthRequired> {
        self.prompts.lock().expect("lock").clone()
    }
}

impl ReauthNotifier for RecordingNotifier {
    fn reauth_required(&self, prompt: McpReauthRequired) {
        self.prompts.lock().expect("lock").push(prompt);
    }
}

fn runtime_for(
    slug: &str,
    url: &str,
    auth: Arc<CountingAuth>,
    identity: Option<McpServerIdentity>,
) -> Arc<McpRuntime> {
    let endpoint = reqwest::Url::parse(url).expect("valid fixture url");
    let runtime = McpRuntime::new(
        slug.to_owned(),
        McpPermissionMode::Allow,
        McpServerDescriptor::registered(endpoint),
        auth,
    );
    Arc::new(match identity {
        Some(identity) => runtime.with_identity(identity),
        None => runtime,
    })
}

fn identity() -> McpServerIdentity {
    McpServerIdentity {
        id: "srv-acme".to_owned(),
        label: "Acme CRM".to_owned(),
    }
}

/// A dispatcher over one registered server, plus the notifier watching it.
async fn harness(
    behavior: RawBehavior,
    identity: Option<McpServerIdentity>,
) -> (
    McpToolDispatcher,
    Arc<RecordingNotifier>,
    Arc<CountingAuth>,
    super::support::ServerGuard,
) {
    let (url, guard) = spawn_raw_fixture(behavior, RawFixtureState::default()).await;
    let auth = Arc::new(CountingAuth::default());
    let notifier = Arc::new(RecordingNotifier::default());
    // The set requires a bundled slot; the call below is routed at the
    // registered server by its own slug, and only that one carries `auth`, so
    // the eviction count belongs to it alone.
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::new(CountingAuth::default()),
            None,
        )),
        vec![runtime_for("acme-crm", &url, Arc::clone(&auth), identity)],
    ));
    let dispatcher = McpToolDispatcher::new(servers)
        .with_reauth_notifier(Arc::clone(&notifier) as Arc<dyn ReauthNotifier>);
    (dispatcher, notifier, auth, guard)
}

async fn call(dispatcher: &McpToolDispatcher) -> Result<(), ToolExecutionError> {
    dispatcher
        .execute(
            &ToolId::from("mcp.acme-crm.echo"),
            &json!({"text": "hi"}),
            CancellationToken::new(),
        )
        .await
        .map(|_| ())
}

#[tokio::test]
async fn a_grant_that_expires_mid_session_is_a_refusal_not_a_disconnection() {
    // The regression this pins. `Disconnected` is retryable, so the run spent
    // attempts reconnecting with the same refused token and reported nothing a
    // user could act on.
    let (dispatcher, notifier, auth, _guard) = harness(
        RawBehavior::RefusesCallsWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        Some(identity()),
    )
    .await;

    let error = call(&dispatcher).await.expect_err("the call is refused");

    match error {
        ToolExecutionError::AuthorizationRefused { label } => assert_eq!(label, "Acme CRM"),
        other => panic!("expected a refusal naming the server, got {other:?}"),
    }
    assert_eq!(
        auth.forgotten.load(Ordering::SeqCst),
        1,
        "the refused grant must be dropped, or the next attempt replays it"
    );
    let prompts = notifier.prompts();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].id.as_str(), "srv-acme");
    assert_eq!(prompts[0].label, "Acme CRM");
    assert_eq!(prompts[0].reason, McpReauthReason::CredentialRefused);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_refusal_from_the_latest_generation_still_forgets_the_credential() {
    let control = MixedCallFailureControl::new();
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::MixedCallFailures {
            control: control.clone(),
            challenge: EXPIRED_CHALLENGE,
        },
        RawFixtureState::default(),
    )
    .await;
    let blocked_forget = Arc::new(BlockingForgetControl::new());
    let auth = Arc::new(CountingAuth::with_blocking_forget(Arc::clone(
        &blocked_forget,
    )));
    let notifier = Arc::new(RecordingNotifier::default());
    let runtime = runtime_for("acme-crm", &url, Arc::clone(&auth), Some(identity()));
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::new(CountingAuth::default()),
            None,
        )),
        vec![Arc::clone(&runtime)],
    ));
    let dispatcher = Arc::new(
        McpToolDispatcher::new(servers)
            .with_reauth_notifier(Arc::clone(&notifier) as Arc<dyn ReauthNotifier>),
    );
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let dispatcher = Arc::clone(&dispatcher);
        calls.spawn(async move { call(dispatcher.as_ref()).await });
    }

    tokio::time::timeout(Duration::from_secs(35), control.wait_for_refusal())
        .await
        .expect("both calls reach the fixture within the MCP call deadline");
    let first = tokio::time::timeout(Duration::from_secs(35), calls.join_next())
        .await
        .expect("generic failure completes within the MCP call deadline")
        .expect("generic failure completes")
        .expect("call task does not panic");
    assert!(matches!(first, Err(ToolExecutionError::Disconnected)));
    assert_eq!(auth.forgotten.load(Ordering::SeqCst), 0);
    assert_eq!(auth.auth_calls.load(Ordering::SeqCst), 1);

    control.release_refusal();
    tokio::time::timeout(Duration::from_secs(5), blocked_forget.wait_until_started())
        .await
        .expect("late latest-generation refusal reaches credential cleanup");
    let reconnect_cancel = CancellationToken::new();
    let mut reconnect = Box::pin(runtime.connected(&reconnect_cancel));
    let first_reconnect_poll = futures_util::poll!(&mut reconnect);
    let auth_calls_while_forget_is_blocked = auth.auth_calls.load(Ordering::SeqCst);
    blocked_forget.release();
    assert!(matches!(first_reconnect_poll, Poll::Pending));
    assert_eq!(
        auth_calls_while_forget_is_blocked, 1,
        "a replacement must not begin authentication until refused-credential cleanup finishes"
    );

    let second = tokio::time::timeout(Duration::from_secs(5), calls.join_next())
        .await
        .expect("refused call completes after credential cleanup")
        .expect("refusal completes")
        .expect("call task does not panic");
    assert!(matches!(
        second,
        Err(ToolExecutionError::AuthorizationRefused { .. })
    ));
    assert_eq!(
        auth.forgotten.load(Ordering::SeqCst),
        1,
        "the late refusal belongs to the latest successful handshake"
    );
    assert_eq!(notifier.prompts().len(), 1);
    let reconnect = tokio::time::timeout(Duration::from_secs(5), reconnect)
        .await
        .expect("replacement connects after cleanup");
    match reconnect {
        Ok((_session, _catalog)) => {}
        Err(error) => panic!("replacement connects after cleanup: {error}"),
    }
}

#[tokio::test]
async fn a_refusal_while_connecting_is_the_same_condition() {
    // A run reaches a server twice — to connect and to call — and either can be
    // refused. Only one of the two used to be classified at all.
    let (dispatcher, notifier, auth, _guard) = harness(
        RawBehavior::RefusesWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        Some(identity()),
    )
    .await;

    let error = call(&dispatcher).await.expect_err("the connect is refused");

    assert!(
        matches!(error, ToolExecutionError::AuthorizationRefused { .. }),
        "{error:?}"
    );
    assert!(auth.forgotten.load(Ordering::SeqCst) >= 1);
    assert_eq!(notifier.prompts().len(), 1);
}

async fn assert_stale_refusal_is_retryable(clear_replacement: bool) {
    let gate = RequestGate::default();
    let fixture_state = RawFixtureState::default();
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::DeferredCallRefusal {
            gate: gate.clone(),
            challenge: EXPIRED_CHALLENGE,
        },
        fixture_state.clone(),
    )
    .await;
    let auth = Arc::new(CountingAuth::default());
    let notifier = Arc::new(RecordingNotifier::default());
    let runtime = runtime_for("acme-crm", &url, Arc::clone(&auth), Some(identity()));
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::new(CountingAuth::default()),
            None,
        )),
        vec![Arc::clone(&runtime)],
    ));
    let dispatcher = Arc::new(
        McpToolDispatcher::new(servers)
            .with_reauth_notifier(Arc::clone(&notifier) as Arc<dyn ReauthNotifier>),
    );
    let cancel = CancellationToken::new();
    let (old_session, _) = runtime.connected(&cancel).await.expect("first connection");
    let pending_dispatcher = Arc::clone(&dispatcher);
    let pending = tokio::spawn(async move { call(pending_dispatcher.as_ref()).await });
    tokio::time::timeout(Duration::from_secs(5), gate.wait_until_entered())
        .await
        .expect("old-session call reaches the server");

    assert_eq!(
        runtime.invalidate(&old_session).await,
        SessionInvalidation::LatestGeneration
    );
    let (replacement, _) = runtime
        .connected(&cancel)
        .await
        .expect("replacement connects");
    if clear_replacement {
        assert_eq!(
            runtime.invalidate(&replacement).await,
            SessionInvalidation::LatestGeneration
        );
    }
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("old refusal completes")
        .expect("call task completes")
        .expect_err("old call is refused");

    assert_eq!(error, ToolExecutionError::Disconnected);
    assert_eq!(auth.forgotten.load(Ordering::SeqCst), 0);
    assert!(notifier.prompts().is_empty());
    tokio::time::timeout(Duration::from_secs(5), call(dispatcher.as_ref()))
        .await
        .expect("retry completes")
        .expect("retry uses a healthy connection");
    assert_eq!(fixture_state.call_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        auth.auth_calls.load(Ordering::SeqCst),
        if clear_replacement { 3 } else { 2 }
    );
    if !clear_replacement {
        let (current, _) = runtime
            .connected(&cancel)
            .await
            .expect("replacement remains current");
        assert!(Arc::ptr_eq(&current, &replacement));
    }
}

#[tokio::test]
async fn a_stale_refusal_from_a_replaced_session_is_retryable() {
    assert_stale_refusal_is_retryable(false).await;
}

#[tokio::test]
async fn a_stale_refusal_stays_retryable_after_the_replacement_is_cleared() {
    assert_stale_refusal_is_retryable(true).await;
}

#[tokio::test]
async fn a_grant_too_narrow_prompts_with_its_own_reason() {
    let (dispatcher, notifier, _auth, _guard) = harness(
        RawBehavior::RefusesCallsWithChallenge {
            status: StatusCode::FORBIDDEN,
            challenge: NARROW_CHALLENGE,
        },
        Some(identity()),
    )
    .await;

    let _ = call(&dispatcher).await.expect_err("the call is refused");

    let prompts = notifier.prompts();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].reason, McpReauthReason::InsufficientScope);
}

#[tokio::test]
async fn three_refused_calls_on_one_server_prompt_once() {
    // A round with several tool calls fails all of them. Prompting per failure
    // would stack three identical modals for one expired grant.
    let (dispatcher, notifier, _auth, _guard) = harness(
        RawBehavior::RefusesCallsWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        Some(identity()),
    )
    .await;

    for _ in 0..3 {
        let _ = call(&dispatcher).await.expect_err("each call is refused");
    }

    assert_eq!(
        notifier.prompts().len(),
        1,
        "one expired grant is one prompt"
    );
}

#[tokio::test]
async fn a_server_with_no_row_is_not_offered_a_reconnect_it_cannot_do() {
    // The bundled server rides the app's own session and has no DA row, so
    // `mcp_server_sign_in` has nothing to sign into. Prompting would point the
    // user at something that cannot fix it.
    let (dispatcher, notifier, auth, _guard) = harness(
        RawBehavior::RefusesCallsWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        None,
    )
    .await;

    let error = call(&dispatcher).await.expect_err("the call is refused");

    // Still classified and still terminal — only the prompt is withheld.
    assert!(
        matches!(error, ToolExecutionError::AuthorizationRefused { .. }),
        "{error:?}"
    );
    assert_eq!(auth.forgotten.load(Ordering::SeqCst), 1);
    assert!(notifier.prompts().is_empty());
}

#[tokio::test]
async fn a_refusal_with_no_notifier_still_fails_terminally() {
    // A run assembled without a window to ask must still not retry a refused
    // credential; it simply asks nobody.
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RefusesCallsWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        RawFixtureState::default(),
    )
    .await;
    let auth = Arc::new(CountingAuth::default());
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::new(CountingAuth::default()),
            None,
        )),
        vec![runtime_for(
            "acme-crm",
            &url,
            Arc::clone(&auth),
            Some(identity()),
        )],
    ));
    let dispatcher = McpToolDispatcher::new(servers);

    let error = call(&dispatcher).await.expect_err("the call is refused");

    assert!(
        matches!(error, ToolExecutionError::AuthorizationRefused { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn background_discovery_invalidates_refused_grants_without_prompting() {
    for behavior in [
        RawBehavior::RefusesWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        RawBehavior::RefusesListingWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
    ] {
        let (url, _guard) = spawn_raw_fixture(behavior, RawFixtureState::default()).await;
        let auth = Arc::new(CountingAuth::default());
        let servers = McpRuntimeSet::new(
            Some(runtime_for(
                BUNDLED_SERVER_ID,
                &url,
                Arc::new(CountingAuth::default()),
                None,
            )),
            vec![runtime_for(
                "acme-crm",
                &url,
                Arc::clone(&auth),
                Some(identity()),
            )],
        );
        let mut registry = crate::tools::registry::ToolRegistry::new();
        let contributions =
            crate::agent::tool_session::register_mcp_tools(&mut registry, &servers).await;
        assert!(!contributions
            .iter()
            .any(|(_, contribution)| matches!(contribution, ServerContribution::Served(_))));
        assert_eq!(auth.forgotten.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn diagnostics_export_keeps_failure_category_without_registered_server_details() {
    use tracing::instrument::WithSubscriber as _;

    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RefusesWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        RawFixtureState::default(),
    )
    .await;
    let servers = McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::new(CountingAuth::default()),
            None,
        )),
        vec![runtime_for(
            "private-customer-crm",
            &url,
            Arc::new(CountingAuth::default()),
            Some(identity()),
        )],
    );
    let directory = tempfile::tempdir().unwrap();
    let logs = directory.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    let log = std::fs::File::create(logs.join("xfalcon-desktop.2026-09-16.log")).unwrap();
    let capture_dispatch = tracing::Dispatch::new(diagnostics_test_subscriber(log));
    // First-register the production WARN from an unscoped real discovery. The
    // retained registrar above must keep that callsite dynamically routed so
    // the independent scoped discovery below can still capture it.
    let unscoped_auth = Arc::new(CountingAuth::default());
    let unscoped = McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::clone(&unscoped_auth),
            None,
        )),
        Vec::new(),
    );
    let unscoped_contributions = unscoped
        .register_tools(&mut crate::tools::registry::ToolRegistry::new())
        .await;
    assert!(matches!(
        unscoped_contributions.as_slice(),
        [(_, ServerContribution::Unreachable)]
    ));
    assert_eq!(unscoped_auth.forgotten.load(Ordering::SeqCst), 1);
    let mut registry = crate::tools::registry::ToolRegistry::new();
    servers
        .register_tools(&mut registry)
        .with_subscriber(capture_dispatch)
        .await;

    let bundle = crate::diagnostics::compose_diagnostics_bundle(
        directory.path(),
        &crate::diagnostics::ConfigurationFacts::default(),
    );
    assert!(bundle.contains("background MCP discovery failed"));
    assert!(bundle.contains("connection_failed"));
    for private in ["private-customer-crm", url.as_str()] {
        assert!(
            !bundle.contains(private),
            "registered fixture marker {private} reached the bundle"
        );
    }
}

#[tokio::test]
async fn diagnostics_export_redacts_captured_mcp_initialization_metadata() {
    use tracing::instrument::WithSubscriber as _;

    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::WrongProtocolVersion("private-mcp-host.example"),
        RawFixtureState::default(),
    )
    .await;
    let directory = tempfile::tempdir().unwrap();
    let logs = directory.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    let log_path = logs.join("xfalcon-desktop.2026-09-16.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let subscriber = diagnostics_test_subscriber(log);
    let descriptor = McpServerDescriptor::registered(reqwest::Url::parse(&url).unwrap());
    let result = crate::mcp::client::connect(
        &descriptor,
        &CountingAuth::default(),
        &CancellationToken::new(),
    )
    .with_subscriber(subscriber)
    .await;
    assert!(matches!(
        result,
        Err(crate::mcp::client::McpClientError::UnsupportedProtocolVersion(_))
    ));
    assert!(std::fs::read_to_string(log_path)
        .unwrap()
        .contains("private-mcp-host.example"));
    let bundle = crate::diagnostics::compose_diagnostics_bundle(
        directory.path(),
        &crate::diagnostics::ConfigurationFacts::default(),
    );
    assert!(bundle.contains("Service initialized as client"));
    assert!(bundle.contains("peer_info=[REDACTED_MCP_METADATA]"));
    for private in ["private-mcp-host.example", "raw-fixture", url.as_str()] {
        assert!(
            !bundle.contains(private),
            "registered fixture metadata reached the bundle"
        );
    }
}

#[tokio::test]
async fn a_rejected_refresh_stops_discovery_without_repeated_auth_attempts() {
    struct MissingGrant(std::sync::atomic::AtomicUsize);
    #[async_trait::async_trait]
    impl McpAuthProvider for MissingGrant {
        async fn current_auth(&self, _: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(McpAuthError::MissingBearer)
        }
    }
    let auth = Arc::new(MissingGrant(std::sync::atomic::AtomicUsize::new(0)));
    let descriptor =
        McpServerDescriptor::registered(reqwest::Url::parse("https://mcp.example/mcp").unwrap());
    let registered = Arc::new(
        McpRuntime::new(
            "acme-crm".into(),
            McpPermissionMode::Allow,
            descriptor.clone(),
            auth.clone(),
        )
        .with_identity(identity()),
    );
    let bundled = Arc::new(McpRuntime::new(
        BUNDLED_SERVER_ID.into(),
        McpPermissionMode::Ask,
        descriptor,
        Arc::new(MissingGrant(std::sync::atomic::AtomicUsize::new(0))),
    ));
    let servers = McpRuntimeSet::new(Some(bundled), vec![registered]);
    let mut registry = crate::tools::registry::ToolRegistry::new();
    servers.register_tools(&mut registry).await;
    assert_eq!(auth.0.load(Ordering::SeqCst), 1);
}

/// The fixture's `initialize` requests: one per session it was asked to open.
fn initializations(state: &RawFixtureState) -> usize {
    state
        .request_bodies
        .lock()
        .expect("lock")
        .iter()
        .filter(|body| body["method"] == "initialize")
        .count()
}

/// A dispatcher over the registered `acme-crm` server, answering `behavior`.
async fn dispatcher_over(
    behavior: RawBehavior,
) -> (
    McpToolDispatcher,
    RawFixtureState,
    super::support::ServerGuard,
) {
    let state = RawFixtureState::default();
    let (url, guard) = spawn_raw_fixture(behavior, state.clone()).await;
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime_for(
            BUNDLED_SERVER_ID,
            &url,
            Arc::new(CountingAuth::default()),
            None,
        )),
        vec![runtime_for(
            "acme-crm",
            &url,
            Arc::new(CountingAuth::default()),
            Some(identity()),
        )],
    ));
    (McpToolDispatcher::new(servers), state, guard)
}

async fn rate_limited_harness(
    refusals: usize,
    retry_after: Option<&'static str>,
) -> (
    McpToolDispatcher,
    RawFixtureState,
    super::support::ServerGuard,
) {
    dispatcher_over(RawBehavior::RateLimitsCalls {
        refusals,
        retry_after,
    })
    .await
}

/// A rate limit used to read as a dropped connection: the session was thrown
/// away, the model was told to retry, and the retry was refused the same way.
#[tokio::test]
async fn a_short_rate_limit_is_waited_out_once_and_the_session_is_kept() {
    let (dispatcher, state, _guard) = rate_limited_harness(1, Some("1")).await;

    let started = std::time::Instant::now();
    call(&dispatcher)
        .await
        .expect("the retry after the wait succeeds");

    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "the Retry-After wait was served"
    );
    assert_eq!(state.call_count.load(Ordering::SeqCst), 2);
    assert_eq!(initializations(&state), 1, "the session was not reopened");
}

#[tokio::test]
async fn a_rate_limit_that_persists_is_reported_as_one_with_its_wait() {
    let (dispatcher, state, _guard) = rate_limited_harness(usize::MAX, Some("1")).await;

    let error = call(&dispatcher)
        .await
        .expect_err("still limited after one wait");

    assert_eq!(
        error,
        ToolExecutionError::RateLimited {
            retry_after: Some(Duration::from_secs(1))
        }
    );
    assert_eq!(
        state.call_count.load(Ordering::SeqCst),
        2,
        "one retry, no more"
    );
    assert_eq!(initializations(&state), 1, "the session was not reopened");
}

#[tokio::test]
async fn a_long_retry_after_goes_back_to_the_model_without_waiting() {
    let (dispatcher, state, _guard) = rate_limited_harness(usize::MAX, Some("120")).await;

    let started = std::time::Instant::now();
    let error = call(&dispatcher)
        .await
        .expect_err("too long to wait inside a call");

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        error,
        ToolExecutionError::RateLimited {
            retry_after: Some(Duration::from_secs(120))
        }
    );
    assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_rate_limit_without_retry_after_waits_the_default_once() {
    let (dispatcher, state, _guard) = rate_limited_harness(usize::MAX, None).await;

    let error = call(&dispatcher).await.expect_err("still limited");

    assert_eq!(error, ToolExecutionError::RateLimited { retry_after: None });
    assert_eq!(state.call_count.load(Ordering::SeqCst), 2);
}

/// A connect refused with 429 used to be retried four times, ignoring its
/// `Retry-After`, and then reported as raw server text.
#[tokio::test]
async fn a_rate_limited_handshake_is_reported_once_with_its_wait() {
    let state = RawFixtureState::default();
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RateLimitsInitialize { retry_after: "7" },
        state.clone(),
    )
    .await;
    let server =
        McpServerDescriptor::registered(reqwest::Url::parse(&url).expect("valid fixture url"));
    let cancel = CancellationToken::new();

    let error = expect_connect_err(
        connect_with_reconnect(
            &server,
            &CountingAuth::default(),
            &ReconnectPolicy::default(),
            &cancel,
        )
        .await,
    );

    assert!(
        matches!(
            &error,
            McpClientError::RateLimited(limit)
                if limit.retry_after == Some(Duration::from_secs(7))
        ),
        "expected a rate limit carrying Retry-After: 7, got {error:?}"
    );
    assert_eq!(
        state.request_bodies.lock().expect("lock").len(),
        1,
        "the refused handshake was not retried"
    );
}

#[tokio::test]
async fn a_rate_limited_handshake_fails_the_call_as_a_rate_limit() {
    let (dispatcher, state, _guard) =
        dispatcher_over(RawBehavior::RateLimitsInitialize { retry_after: "7" }).await;

    let error = call(&dispatcher)
        .await
        .expect_err("the connect was rate limited");

    assert_eq!(
        error,
        ToolExecutionError::RateLimited {
            retry_after: Some(Duration::from_secs(7))
        }
    );
    assert_eq!(initializations(&state), 1, "the connect was not retried");
    assert_eq!(state.call_count.load(Ordering::SeqCst), 0, "no tool ran");
}

/// A 429 answered to a notification has no caller waiting on it. Recording it
/// used to hand the verdict to the next failure on the same session.
#[tokio::test]
async fn a_rate_limited_notification_is_not_blamed_on_a_later_failure() {
    let gate = RequestGate::default();
    let (dispatcher, _state, _guard) =
        dispatcher_over(RawBehavior::RateLimitsCancellation { gate: gate.clone() }).await;
    let cancel = CancellationToken::new();

    // Cancelling the first call makes the client send `notifications/cancelled`,
    // which the server answers with 429.
    let cancel_once_in_flight = async {
        gate.wait_until_entered().await;
        cancel.cancel();
    };
    let tool = ToolId::from("mcp.acme-crm.echo");
    let arguments = json!({"text": "hi"});
    let (first, ()) = tokio::join!(
        dispatcher.execute(&tool, &arguments, cancel.clone()),
        cancel_once_in_flight,
    );
    assert!(
        matches!(first, Err(ToolExecutionError::Cancelled)),
        "the first call was cancelled"
    );

    // The next call fails in transport: a dropped connection, not the rate
    // limit the cancellation notification drew.
    let error = call(&dispatcher)
        .await
        .expect_err("the server fails the second call");
    assert_eq!(error, ToolExecutionError::Disconnected);
    gate.release();
}

#[tokio::test]
async fn successful_tool_use_does_not_prompt_for_an_unused_refused_server() {
    for behavior in [
        RawBehavior::RefusesWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        RawBehavior::RefusesListingWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
        RawBehavior::RefusesCallsWithChallenge {
            status: StatusCode::UNAUTHORIZED,
            challenge: EXPIRED_CHALLENGE,
        },
    ] {
        let refuses_call = matches!(behavior, RawBehavior::RefusesCallsWithChallenge { .. });
        let state = super::support::FixtureState::default();
        let (healthy_url, _healthy_guard) = super::support::FixtureServer::new(0, state.clone())
            .spawn()
            .await;
        let (refused_url, _refused_guard) =
            spawn_raw_fixture(behavior, RawFixtureState::default()).await;
        let servers = Arc::new(McpRuntimeSet::new(
            Some(runtime_for(
                BUNDLED_SERVER_ID,
                &healthy_url,
                Arc::new(CountingAuth::default()),
                None,
            )),
            vec![runtime_for(
                "acme-crm",
                &refused_url,
                Arc::new(CountingAuth::default()),
                Some(identity()),
            )],
        ));
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
        let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());
        let notifier = Arc::new(RecordingNotifier::default());
        let session = crate::agent::tool_session::build_tool_session(
            crate::agent::tool_session::ToolSessionRequest {
                projects: Arc::new(crate::projects::ProjectsService::new(
                    root.path().into(),
                    scope.clone(),
                    secrets.clone(),
                )),
                approvals: Arc::new(crate::approvals::ApprovalsService::new(
                    root.path().into(),
                    scope,
                    secrets,
                )),
                mcp: Some(servers),
                account: crate::security::account_cleanup::ValidatedAccountId::parse(
                    "test-account",
                )
                .unwrap(),
                conversation_id: xfalcon_contracts::ConversationId::from("conversation"),
                run_id: xfalcon_contracts::RunId::from("run"),
                result_budget: crate::tools::ToolResultBudget::floor(),
                skill_scope: None,
                standing_approvals: None,
                reauth: Some(notifier.clone()),
                exact_result: None,
                audit: None,
                skills: Arc::new(crate::skills::store::SkillStore::open(
                    root.path(),
                    &crate::security::account_cleanup::ValidatedAccountId::parse("skills-unused")
                        .expect("account"),
                )),
            },
        )
        .await;
        let tool_id = session
            .tool_ids
            .values()
            .find(|id| id.as_str() == format!("mcp.{BUNDLED_SERVER_ID}.echo"))
            .unwrap();
        session
            .executor
            .execute(tool_id, &json!({"text":"hi"}), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
        assert!(
            notifier.prompts().is_empty(),
            "an unused server must not interrupt a successful query"
        );
        if refuses_call {
            let requested = ToolId::from("mcp.acme-crm.echo");
            assert!(session.tool_ids.values().any(|id| id == &requested));
            let result = session
                .executor
                .execute(&requested, &json!({"text":"hi"}), CancellationToken::new())
                .await;
            assert!(matches!(
                result,
                Err(ToolExecutionError::AuthorizationRefused { .. })
            ));
            assert_eq!(notifier.prompts().len(), 1);
            assert_eq!(notifier.prompts()[0].id.as_str(), "srv-acme");
        }
    }
}
