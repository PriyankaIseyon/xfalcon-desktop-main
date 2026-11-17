//! Security-focused MCP client tests: audience mismatch/token passthrough
//! rejection, frame/result size limits, access-token expiry and reconnect,
//! and reconnect backoff without idle busy polling.
//!
//! `mcp::auth`'s own unit tests already cover the pure audience-matching
//! logic exhaustively; the test here proves the *client* actually calls
//! that check before any network I/O happens (a mismatched provider must
//! never even dial the bundled server).

use std::sync::Mutex;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::support::{expect_connect_err, spawn_raw_fixture, RawBehavior, RawFixtureState};
use crate::auth::AuthConfig;
use crate::mcp::auth::{McpAuth, McpAuthError, McpAuthProvider};
use crate::mcp::catalog::McpToolCatalog;
use crate::mcp::client::{connect, connect_with_reconnect, McpClientError, ReconnectPolicy};
use crate::mcp::executor::McpExecutor;
use crate::mcp::server_resolver::{McpServerDescriptor, McpServerResolver};

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

struct MismatchedAuth;

#[async_trait::async_trait]
impl McpAuthProvider for MismatchedAuth {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        McpAuth::bind(
            Zeroizing::new("token-bound-elsewhere".to_owned()),
            "https://attacker.example/mcp",
            server,
        )
    }
}

#[tokio::test]
async fn audience_mismatch_is_rejected_before_any_network_call() {
    // Deliberately does NOT spawn a fixture server: a mismatched resource
    // must be rejected before `connect` ever dials the network. Using an
    // address nothing listens on proves this — if the client attempted a
    // connection first, this test would fail with a transport error
    // instead of the expected `Auth` rejection.
    let config = AuthConfig {
        da_base_url: reqwest::Url::parse("http://127.0.0.1:1").unwrap(),
        casdoor_issuer: reqwest::Url::parse("https://auth.example").unwrap(),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    let server = McpServerResolver::resolve(&config);
    let cancel = CancellationToken::new();

    let error = expect_connect_err(connect(&server, &MismatchedAuth, &cancel).await);
    assert!(matches!(
        error,
        McpClientError::Auth(McpAuthError::AudienceMismatch)
    ));
}

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

#[tokio::test]
async fn tool_arguments_can_never_override_the_attached_bearer() {
    let fixture_state = RawFixtureState::default();
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RequireBearer {
            accepted_bearer: "expected-token",
        },
        fixture_state.clone(),
    )
    .await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("expected-token");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    let catalog = McpToolCatalog::refresh(&session)
        .await
        .expect("catalog refreshes");
    let executor = McpExecutor::new(&session, &catalog);

    // A hostile/confused tool argument tries to smuggle a different bearer.
    // The executor's public API has no parameter for a caller-supplied
    // token at all — this call can only ever attach the one bound at
    // connect time.
    let _ = executor
        .call_tool(
            "echo",
            serde_json::json!({"text": "hi", "authorization": "Bearer evil-token"}),
            &cancel,
        )
        .await;

    let seen = fixture_state
        .authorization_seen
        .lock()
        .expect("lock")
        .clone();
    assert!(!seen.is_empty(), "the fixture never observed a request");
    for bearer in &seen {
        assert_eq!(bearer.as_deref(), Some("expected-token"));
    }
    session.close().await;
}

#[tokio::test]
async fn oversized_tool_result_is_rejected_not_buffered_unbounded() {
    let (url, _guard) =
        spawn_raw_fixture(RawBehavior::OversizedToolResult, RawFixtureState::default()).await;
    let server = descriptor_for_fixture(&url);
    let auth = StaticAuth("expected-token");
    let cancel = CancellationToken::new();

    let session = connect(&server, &auth, &cancel).await.expect("connects");
    // The bundled server's own tool schema is irrelevant here — this test
    // targets `McpSession::peer`'s bounded HTTP client directly, so a
    // `tools/call` request is issued without going through catalog schema
    // validation (there is no catalog for this raw fixture).
    let params = rmcp::model::CallToolRequestParams::new("anything");
    let result = session.peer().call_tool(params).await;
    assert!(
        result.is_err(),
        "an oversized response must not be silently accepted"
    );
    session.close().await;
}

#[tokio::test]
async fn oversized_persistent_push_stream_is_rejected_not_buffered_unbounded() {
    // `get_stream` opens the persistent server-initiated push channel used
    // for the whole life of a real session (progress, list-changed, etc.),
    // not a one-shot request/response — a full `rmcp` session's worker
    // tolerates this channel failing or being absent entirely, which would
    // swallow the failure this test cares about. So this targets
    // `BoundedMcpHttpClient::get_stream` directly against a raw fixture that
    // answers the GET with a `text/event-stream` body larger than
    // `MAX_MCP_RESPONSE_BYTES`, proving the bound applies to this channel
    // exactly like it already does to `post_message`.
    use futures_util::StreamExt;
    use rmcp::transport::streamable_http_client::StreamableHttpClient;
    use std::collections::HashMap;
    use std::sync::Arc;

    let (url, _guard) = super::support::spawn_oversized_sse_push_fixture(
        crate::mcp::client::MAX_MCP_RESPONSE_BYTES + 4096,
    )
    .await;

    let client =
        crate::mcp::client::BoundedMcpHttpClient::new(false).expect("bounded client builds");
    let mut stream = client
        .get_stream(
            Arc::<str>::from(url.as_str()),
            Arc::<str>::from("test-session"),
            None,
            None,
            HashMap::new(),
        )
        .await
        .expect("opening the stream itself must succeed (only its body is oversized)");

    let mut total_bytes_yielded = 0usize;
    let mut saw_error = false;
    while let Some(event) = stream.next().await {
        match event {
            Ok(sse) => {
                total_bytes_yielded += sse.data.map(|data| data.len()).unwrap_or(0);
            }
            Err(_) => {
                saw_error = true;
                break;
            }
        }
    }

    assert!(
        saw_error,
        "an oversized push-stream body must surface as an error, not be silently accepted"
    );
    assert!(
        total_bytes_yielded <= crate::mcp::client::MAX_MCP_RESPONSE_BYTES,
        "must never buffer more than the bound before erroring, got {total_bytes_yielded} bytes"
    );
}

#[tokio::test]
async fn access_token_expiry_triggers_reconnect_with_a_fresh_bearer() {
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RequireBearer {
            accepted_bearer: "fresh-token",
        },
        RawFixtureState::default(),
    )
    .await;
    let server = descriptor_for_fixture(&url);
    let cancel = CancellationToken::new();

    struct RotatingAuth {
        calls: Mutex<u32>,
    }
    #[async_trait::async_trait]
    impl McpAuthProvider for RotatingAuth {
        async fn current_auth(
            &self,
            server: &McpServerDescriptor,
        ) -> Result<McpAuth, McpAuthError> {
            let mut calls = self.calls.lock().expect("lock");
            *calls += 1;
            let bearer = if *calls == 1 {
                "expired-token"
            } else {
                "fresh-token"
            };
            McpAuth::bind(
                Zeroizing::new(bearer.to_owned()),
                server.da_origin().as_str(),
                server,
            )
        }
    }
    let auth = RotatingAuth {
        calls: Mutex::new(0),
    };
    let policy = ReconnectPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    };

    let session = connect_with_reconnect(&server, &auth, &policy, &cancel)
        .await
        .expect("reconnect with a refreshed bearer must succeed");
    session.close().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_backoff_sleeps_between_attempts_without_busy_polling() {
    struct UnavailableAuth;
    #[async_trait::async_trait]
    impl McpAuthProvider for UnavailableAuth {
        async fn current_auth(
            &self,
            _server: &McpServerDescriptor,
        ) -> Result<McpAuth, McpAuthError> {
            Err(McpAuthError::CredentialUnavailable)
        }
    }

    let config = AuthConfig {
        da_base_url: reqwest::Url::parse("http://127.0.0.1:1").unwrap(),
        casdoor_issuer: reqwest::Url::parse("https://auth.example").unwrap(),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    let server = McpServerResolver::resolve(&config);
    let policy = ReconnectPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(100),
    };
    let cancel = CancellationToken::new();

    let started = tokio::time::Instant::now();
    let error = expect_connect_err(
        connect_with_reconnect(&server, &UnavailableAuth, &policy, &cancel).await,
    );
    match error {
        McpClientError::ReconnectExhausted {
            attempts,
            last_error,
        } => {
            assert_eq!(attempts, policy.max_attempts);
            assert!(matches!(
                *last_error,
                McpClientError::Auth(McpAuthError::CredentialUnavailable)
            ));
        }
        other => panic!("expected ReconnectExhausted, got {other:?}"),
    }

    // Paused-time auto-advances only while every task is blocked exclusively
    // on a timer; a busy-poll loop would never yield to that point and this
    // assertion would either fail (elapsed stays near zero) or the test
    // would hang against the real wall clock. Three waits between four
    // attempts at 100ms each: 300ms of virtual time.
    assert_eq!(started.elapsed(), Duration::from_millis(300));
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_reconnect_backoff_wins_immediately() {
    struct UnavailableAuth;
    #[async_trait::async_trait]
    impl McpAuthProvider for UnavailableAuth {
        async fn current_auth(
            &self,
            _server: &McpServerDescriptor,
        ) -> Result<McpAuth, McpAuthError> {
            Err(McpAuthError::CredentialUnavailable)
        }
    }

    let config = AuthConfig {
        da_base_url: reqwest::Url::parse("http://127.0.0.1:1").unwrap(),
        casdoor_issuer: reqwest::Url::parse("https://auth.example").unwrap(),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    let server = McpServerResolver::resolve(&config);
    let policy = ReconnectPolicy {
        max_attempts: 10,
        base_delay: Duration::from_secs(60),
        max_delay: Duration::from_secs(60),
    };
    let cancel = CancellationToken::new();
    let cancel_signal = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        cancel_signal.cancel();
    });

    let started = tokio::time::Instant::now();
    let error = expect_connect_err(
        connect_with_reconnect(&server, &UnavailableAuth, &policy, &cancel).await,
    );
    assert!(matches!(error, McpClientError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "cancellation should short-circuit the 60s backoff, elapsed: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_server_that_refuses_the_bearer_is_reported_as_needing_a_new_sign_in() {
    // Before this was classified, a 401 with a challenge arrived as
    // `Initialize(..)` and Settings told the user the server "could not be
    // reached" — sending them to check a network that was working, for a
    // problem one sign-in fixes.
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RefusesWithChallenge {
            status: axum::http::StatusCode::UNAUTHORIZED,
            challenge: r#"Bearer error="invalid_token", error_description="expired""#,
        },
        RawFixtureState::default(),
    )
    .await;
    let server = descriptor_for_fixture(&url);

    let error = expect_connect_err(
        connect(
            &server,
            &StaticAuth("stale-token"),
            &CancellationToken::new(),
        )
        .await,
    );

    assert!(
        matches!(
            error,
            crate::mcp::client::McpClientError::AuthorizationRequired
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_grant_too_narrow_for_the_server_is_distinguished_from_a_dead_one() {
    // `insufficient_scope` means the sign-in is real and does not reach far
    // enough. Collapsing it into "sign in again" would still be actionable, but
    // the scope the server named is the only thing that says what to ask for.
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RefusesWithChallenge {
            status: axum::http::StatusCode::FORBIDDEN,
            challenge: r#"Bearer error="insufficient_scope", scope="mcp:tools:read mcp:tools:write""#,
        },
        RawFixtureState::default(),
    )
    .await;
    let server = descriptor_for_fixture(&url);

    let error = expect_connect_err(
        connect(
            &server,
            &StaticAuth("narrow-token"),
            &CancellationToken::new(),
        )
        .await,
    );

    match error {
        crate::mcp::client::McpClientError::InsufficientScope { scope } => {
            assert_eq!(scope.as_deref(), Some("mcp:tools:read mcp:tools:write"));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_refusal_that_says_nothing_is_still_reported_as_a_handshake_failure() {
    // A 401 with no challenge carries no statement about authorization, so it
    // stays a handshake failure rather than being upgraded into a claim the
    // server never made.
    let (url, _guard) = spawn_raw_fixture(
        RawBehavior::RequireBearer {
            accepted_bearer: "expected-token",
        },
        RawFixtureState::default(),
    )
    .await;
    let server = descriptor_for_fixture(&url);

    let error = expect_connect_err(
        connect(
            &server,
            &StaticAuth("wrong-token"),
            &CancellationToken::new(),
        )
        .await,
    );

    assert!(
        !matches!(
            error,
            crate::mcp::client::McpClientError::AuthorizationRequired
                | crate::mcp::client::McpClientError::InsufficientScope { .. }
        ),
        "{error:?}"
    );
}
