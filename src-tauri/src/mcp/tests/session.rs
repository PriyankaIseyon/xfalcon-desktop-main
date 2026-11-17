//! Protocol/session tests against a real, spec-compliant MCP server
//! (`super::support::FixtureServer`, built on `rmcp`'s own server
//! transport) and, for cases a well-behaved server wouldn't produce, a raw
//! axum fixture (`super::support::spawn_raw_fixture`).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::ProtocolVersion;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::support::{
    expect_connect_err, fabricated_progress_token, spawn_raw_fixture, FixtureServer, FixtureState,
    RawBehavior, RawFixtureState, FIXTURE_NAMED_TOOLS,
};

/// Generic paginated filler tools the catalog test asks the fixture for, on
/// top of [`FIXTURE_NAMED_TOOLS`].
const FIXTURE_GENERIC_TOOLS: usize = 5;
use crate::auth::AuthConfig;
use crate::mcp::auth::{McpAuth, McpAuthError, McpAuthProvider};
use crate::mcp::catalog::{McpToolCatalog, McpToolCatalogCache};
use crate::mcp::client::{connect, McpClientError};
use crate::mcp::executor::McpExecutor;
use crate::mcp::server_resolver::{McpServerDescriptor, McpServerResolver};

struct StaticAuth(&'static str);

#[async_trait::async_trait]
impl McpAuthProvider for StaticAuth {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        McpAuth::bind(
            Zeroizing::new(self.0.to_owned()),
            server.da_origin().as_str(),
            server,
        )
    }
}

/// The fixture's HTTP base already ends in `/mcp` (see
/// `FixtureServer::spawn`/`spawn_raw_fixture`'s router), so build a
/// descriptor whose origin matches it exactly rather than going through
/// `McpServerResolver::resolve` (which would derive its own `/mcp` path from
/// an origin-only `da_base_url` — the same outcome here, spelled out
/// directly for clarity in tests).
fn descriptor_for_fixture(mcp_url: &str) -> McpServerDescriptor {
    let url = reqwest::Url::parse(mcp_url).expect("valid fixture url");
    let mut origin_only = url.clone();
    origin_only.set_path("");
    let config = AuthConfig {
        da_base_url: origin_only,
        casdoor_issuer: reqwest::Url::parse("https://auth.example").unwrap(),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    McpServerResolver::resolve(&config)
}

#[tokio::test]
async fn negotiates_and_pins_protocol_version_2025_11_25() {
    let (url, _guard) = FixtureServer::new(1, FixtureState::default()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    assert_eq!(
        session.negotiated_protocol_version(),
        Some(ProtocolVersion::V_2025_11_25)
    );
    session.close().await;
}

#[tokio::test]
async fn rejects_server_that_negotiates_a_different_protocol_version() {
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::WrongProtocolVersion("2024-11-05"),
        RawFixtureState::default(),
    )
    .await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let error = expect_connect_err(connect(&server, &auth, &cancel).await);
    match error {
        McpClientError::UnsupportedProtocolVersion(version) => {
            assert_eq!(version, "2024-11-05");
        }
        other => panic!("expected UnsupportedProtocolVersion, got {other:?}"),
    }
}

#[tokio::test]
async fn session_survives_multiple_sequential_tool_calls() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");
    let executor = McpExecutor::new(&session, &catalog);

    for expected in ["first", "second", "third"] {
        let result = executor
            .call_tool("echo", serde_json::json!({"text": expected}), &cancel)
            .await
            .expect("echo succeeds");
        assert!(!result.is_error());
    }
    assert_eq!(state.call_count.load(Ordering::SeqCst), 3);
    assert!(!session.is_closed());
    session.close().await;
}

#[tokio::test]
async fn catalog_discovers_all_tools_across_pagination() {
    // FixtureServer pages one tool at a time, so five generic tools plus the
    // three named fixtures requires eight `tools/list` round trips.
    let (url, _guard) = FixtureServer::new(FIXTURE_GENERIC_TOOLS, FixtureState::default())
        .spawn()
        .await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");

    assert_eq!(
        catalog.len(),
        FIXTURE_GENERIC_TOOLS + FIXTURE_NAMED_TOOLS.len()
    );
    for index in 0..FIXTURE_GENERIC_TOOLS {
        assert!(catalog.get(&format!("tool_{index}")).is_some());
    }
    for named in FIXTURE_NAMED_TOOLS {
        assert!(catalog.get(named).is_some(), "missing fixture tool {named}");
    }
    session.close().await;
}

#[tokio::test]
async fn tool_list_changed_notification_is_observable() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(1, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    assert!(!session.notifications().take_tool_list_changed());

    state.push_tool_list_changed().await;
    // Give the client's background read loop a moment to process the
    // server-initiated notification.
    let mut observed = false;
    for _ in 0..50 {
        if session.notifications().take_tool_list_changed() {
            observed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(observed, "tool list changed notification was not observed");
    session.close().await;
}

#[tokio::test]
async fn cancellation_propagates_to_the_server() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let connect_cancel = CancellationToken::new();

    let session = connect(&server, &auth, &connect_cancel)
        .await
        .expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");
    let executor = McpExecutor::new(&session, &catalog);

    let call_cancel = CancellationToken::new();
    let cancel_after = call_cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel_after.cancel();
    });

    let result = executor
        .call_tool("slow_progress", serde_json::json!({}), &call_cancel)
        .await;
    assert!(matches!(
        result,
        Err(crate::mcp::executor::McpExecutorError::Cancelled)
    ));

    let mut observed = false;
    for _ in 0..50 {
        if state.last_call_cancelled.load(Ordering::SeqCst) {
            observed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(observed, "server never observed the cancellation");
    assert!(
        session.notifications().snapshot_progress().is_empty(),
        "a cancelled protocol call must remove its registered progress token"
    );
    session.close().await;
}

#[tokio::test]
async fn progress_notifications_are_recorded_during_a_call() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");
    let executor = McpExecutor::new(&session, &catalog);

    // Poll the session's notifications concurrently with the call itself —
    // Observed through the session's monotonic accepted counter rather than
    // `snapshot_progress`, because `McpToolExecutor` clears a token's entry
    // the instant its call returns — so the snapshot window closed whenever
    // the whole call finished inside one poll interval.
    //
    // Still polled, but *after* the call rather than racing it: under
    // Streamable HTTP a `notifications/progress` frame and the POST response
    // can arrive on different streams, so acceptance is ordered-after-send
    // but not ordered-before-response. This waits for delivery instead of
    // hoping to sample inside a window that may already have closed.
    let before = session.notifications().accepted_progress_count();

    let result = executor
        .call_tool("slow_progress", serde_json::json!({}), &cancel)
        .await
        .expect("slow_progress completes");
    assert!(!result.is_error());

    let mut accepted = session.notifications().accepted_progress_count();
    for _ in 0..200 {
        if accepted > before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        accepted = session.notifications().accepted_progress_count();
    }

    assert!(
        state.last_call_had_progress_token.load(Ordering::SeqCst),
        "the request reached the server without a _meta.progressToken, so the \
         server had nothing to report progress against"
    );
    assert!(
        accepted > before,
        "the server received a progress token and sent notifications, but none \
         was accepted by the client"
    );
    session.close().await;
}

#[tokio::test]
async fn dropping_a_running_mcp_call_clears_its_progress_registration() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();
    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");
    let executor = McpExecutor::new(&session, &catalog);
    let mut call = Box::pin(executor.call_tool("slow_progress", serde_json::json!({}), &cancel));

    tokio::select! {
        result = &mut call => panic!("slow call completed before it could be dropped: {result:?}"),
        () = async {
            for _ in 0..100 {
                if !session.notifications().snapshot_progress().is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("the running call never registered and received progress");
        } => {}
    }

    drop(call);
    assert!(
        session.notifications().snapshot_progress().is_empty(),
        "dropping the running executor future must clear its progress token"
    );
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        session.notifications().snapshot_progress().is_empty(),
        "late server progress must not recreate a dropped call's entry"
    );
    session.close().await;
}

#[tokio::test]
async fn session_close_is_clean() {
    let (url, _guard) = FixtureServer::new(0, FixtureState::default()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    assert!(!session.is_closed());
    session.close().await;
}

#[tokio::test]
async fn catalog_cache_reuses_the_same_catalog_until_tool_list_changes() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(2, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let cache = McpToolCatalogCache::new();

    let first = cache
        .get_or_refresh(&session)
        .await
        .expect("first refresh populates the cache");
    let second = cache
        .get_or_refresh(&session)
        .await
        .expect("second call should be a cache hit");
    assert_eq!(
        state.list_tools_call_count.load(Ordering::SeqCst),
        1,
        "an unchanged session/protocol version must not refetch"
    );
    assert!(
        Arc::ptr_eq(&first, &second),
        "a cache hit must return the exact cached instance"
    );

    // A server-pushed capability-change notification must force the next
    // call to refetch, even though identity/protocol version are unchanged.
    state.push_tool_list_changed().await;
    let mut refetched = false;
    for _ in 0..50 {
        let _ = cache
            .get_or_refresh(&session)
            .await
            .expect("refresh after list-changed");
        if state.list_tools_call_count.load(Ordering::SeqCst) == 2 {
            refetched = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        refetched,
        "cache never refetched after a tool-list-changed notification"
    );
    session.close().await;
}

#[tokio::test]
async fn a_failed_list_changed_refresh_cannot_make_the_old_catalog_clean_again() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();
    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let cache = McpToolCatalogCache::new();

    let initial = cache
        .get_or_refresh(&session)
        .await
        .expect("initial schema refreshes");
    assert!(initial
        .validate_arguments("echo", &serde_json::json!({"text": "before"}))
        .is_ok());

    state.require_number_for_echo();
    state.fail_next_list_tools();
    state.push_tool_list_changed().await;

    let mut saw_injected_failure = false;
    for _ in 0..50 {
        if cache.get_or_refresh(&session).await.is_err() {
            saw_injected_failure = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saw_injected_failure, "changed-schema refresh never ran");

    let refreshed = cache
        .get_or_refresh(&session)
        .await
        .expect("the failed refresh leaves the cache dirty");
    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 3);
    assert!(refreshed
        .validate_arguments("echo", &serde_json::json!({"text": 7}))
        .is_ok());
    assert_eq!(
        refreshed.validate_arguments("echo", &serde_json::json!({"text": "stale"})),
        Err(crate::mcp::catalog::ArgumentsError::SchemaViolation)
    );
    session.close().await;
}

#[tokio::test]
async fn concurrent_catalog_reads_share_one_refresh() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(2, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();
    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let cache = McpToolCatalogCache::new();

    let (first, second) = tokio::join!(
        cache.get_or_refresh(&session),
        cache.get_or_refresh(&session),
    );
    let first = first.expect("first refresh succeeds");
    let second = second.expect("concurrent reader shares the refresh");

    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&first, &second));
    session.close().await;
}

#[tokio::test]
async fn a_fresh_session_at_the_same_endpoint_does_not_reuse_stale_schemas() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();
    let cache = McpToolCatalogCache::new();

    let first_session = connect(&server, &auth, &cancel)
        .await
        .expect("first connects");
    let first_catalog = cache
        .get_or_refresh(&first_session)
        .await
        .expect("first schema refreshes");
    assert!(first_catalog
        .validate_arguments("echo", &serde_json::json!({"text": "before"}))
        .is_ok());
    first_session.close().await;

    state.require_number_for_echo();
    let second_session = connect(&server, &auth, &cancel)
        .await
        .expect("second connects at the same endpoint");
    let second_catalog = cache
        .get_or_refresh(&second_session)
        .await
        .expect("fresh session refreshes its schema");

    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 2);
    assert!(second_catalog
        .validate_arguments("echo", &serde_json::json!({"text": 7}))
        .is_ok());
    assert_eq!(
        second_catalog.validate_arguments("echo", &serde_json::json!({"text": "stale"})),
        Err(crate::mcp::catalog::ArgumentsError::SchemaViolation)
    );
    second_session.close().await;
}

#[tokio::test]
async fn catalog_cache_does_not_serve_a_different_server_identity() {
    let state_a = FixtureState::default();
    let (url_a, _guard_a) = FixtureServer::new(1, state_a.clone()).spawn().await;
    let server_a = descriptor_for_fixture(&url_a);

    let state_b = FixtureState::default();
    let (url_b, _guard_b) = FixtureServer::new(1, state_b.clone()).spawn().await;
    let server_b = descriptor_for_fixture(&url_b);

    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();
    let session_a = connect(&server_a, &auth, &cancel)
        .await
        .expect("connects a");
    let session_b = connect(&server_b, &auth, &cancel)
        .await
        .expect("connects b");

    let cache = McpToolCatalogCache::new();
    let _ = cache
        .get_or_refresh(&session_a)
        .await
        .expect("refresh against server a");
    assert_eq!(state_a.list_tools_call_count.load(Ordering::SeqCst), 1);
    assert_eq!(state_b.list_tools_call_count.load(Ordering::SeqCst), 0);

    // The same cache handed a *different* session (different server
    // identity) must not reuse server a's cached catalog.
    let _ = cache
        .get_or_refresh(&session_b)
        .await
        .expect("refresh against server b");
    assert_eq!(state_b.list_tools_call_count.load(Ordering::SeqCst), 1);

    session_a.close().await;
    session_b.close().await;
}

#[tokio::test]
async fn fabricated_progress_token_is_dropped_not_stored() {
    let state = FixtureState::default();
    let (url, _guard) = FixtureServer::new(0, state.clone()).spawn().await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("session-bearer");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");
    let executor = McpExecutor::new(&session, &catalog);

    let result = executor
        .call_tool("fabricated_progress", serde_json::json!({}), &cancel)
        .await
        .expect("fabricated_progress completes");
    assert!(!result.is_error());

    // Give the client's background read loop a moment to process the
    // notification the fixture just sent for a token nobody registered.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        session
            .notifications()
            .progress_for(&fabricated_progress_token())
            .is_none(),
        "a progress notification for an unregistered token must never be stored"
    );
    // The call's own (registered) token is cleared on completion either
    // way, so the whole map must be empty: nothing else could have snuck
    // in.
    assert!(
        session.notifications().snapshot_progress().is_empty(),
        "no progress entries should remain after a completed call, \
         fabricated or otherwise"
    );
    session.close().await;
}
