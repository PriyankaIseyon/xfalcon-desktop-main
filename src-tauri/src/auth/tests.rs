use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::account::canonical_account_id;
use super::config::AuthConfig;
use super::discovery::{build_discovery_client, discover_auth, DiscoveryError};
use super::service::{login_may_complete, AuthService};
use super::session::AuthSessionStore;
use crate::security::account_cleanup::CurrentAccountScope;
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::auth::SessionProjection;

#[test]
fn logout_returns_to_unauthenticated_session() {
    let store = Arc::new(AuthSessionStore::default());
    let service = AuthService::new(store);
    assert!(matches!(
        service.logout(),
        SessionProjection::Unauthenticated { .. }
    ));
}

#[test]
fn canonical_account_id_is_tenant_scoped() {
    let first = IdentityProjection::new("user-1", "tenant-1", "User", None);
    let second = IdentityProjection::new("user-1", "tenant-2", "User", None);

    assert_eq!(
        canonical_account_id(&first).expect("canonical account"),
        "tenant-1:user-1"
    );
    assert_ne!(canonical_account_id(&first), canonical_account_id(&second));
}

#[test]
fn canonical_account_id_rejects_ambiguous_or_oversized_components() {
    let left = IdentityProjection::new("c", "a:b", "User", None);
    let right = IdentityProjection::new("b:c", "a", "User", None);
    assert!(canonical_account_id(&left).is_err());
    assert!(canonical_account_id(&right).is_err());

    let maximum = "x".repeat(160);
    let maximum_identity = IdentityProjection::new(maximum.clone(), maximum, "User", None);
    assert!(canonical_account_id(&maximum_identity).is_err());
}

#[test]
fn authenticated_snapshot_never_mixes_identity_token_or_resource_generations() {
    let store = AuthSessionStore::default();
    store.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-old", "tenant-old", "Old", None),
            120_000,
        ),
        "access-old",
        None,
    );
    store.set_authorized_resource("https://old.example/mcp");
    store.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-new", "tenant-new", "New", None),
            180_000,
        ),
        "access-new",
        None,
    );
    assert!(store
        .authenticated_snapshot()
        .expect("new authenticated snapshot")
        .authorized_resource
        .is_none());
    store.set_authorized_resource("https://new.example/mcp");

    let snapshot = store
        .authenticated_snapshot()
        .expect("new authenticated snapshot");
    let super::session::AuthenticatedSessionSnapshot {
        identity,
        access_token,
        authorized_resource,
    } = snapshot;
    assert_eq!(identity.user_id, "user-new");
    assert_eq!(identity.tenant_id, "tenant-new");
    assert_eq!(access_token.as_str(), "access-new");
    assert_eq!(
        authorized_resource.as_deref(),
        Some("https://new.example/mcp")
    );
}

#[test]
fn clearing_session_removes_authenticated_snapshot_and_access_token() {
    let store = AuthSessionStore::default();
    store.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            120_000,
        ),
        "access",
        Some("refresh".to_owned()),
    );

    store.clear();

    assert!(store.authenticated_snapshot().is_none());
    assert!(store.access_token().is_none());
    assert!(matches!(
        store.projection(),
        SessionProjection::Unauthenticated { .. }
    ));
}

#[test]
fn every_change_of_who_is_signed_in_moves_the_generation() {
    let store = AuthSessionStore::default();
    let start = store.generation();

    store.set(SessionProjection::authenticating());
    let after_set = store.generation();
    store.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            120_000,
        ),
        "access",
        None,
    );
    let after_set_authenticated = store.generation();
    store.clear();

    assert!(after_set > start);
    assert!(after_set_authenticated > after_set);
    assert!(store.generation() > after_set_authenticated);
}

#[test]
fn a_guarded_write_lands_only_while_the_generation_is_unchanged() {
    let store = AuthSessionStore::default();
    let expected = store.generation();
    let session = |user: &str| {
        SessionProjection::authenticated(
            IdentityProjection::new(user, "tenant-1", "User", None),
            120_000,
        )
    };

    assert!(store.set_authenticated_if_generation(
        expected,
        session("user-1"),
        "access-first",
        None,
    ));
    // The same expectation a second time: the first write moved the session.
    assert!(!store.set_authenticated_if_generation(
        expected,
        session("user-2"),
        "access-second",
        None,
    ));

    let snapshot = store
        .authenticated_snapshot()
        .expect("the first write stands");
    assert_eq!(snapshot.identity.user_id, "user-1");
    assert_eq!(snapshot.access_token.as_str(), "access-first");
}

#[test]
fn a_guarded_write_is_refused_after_sign_out() {
    let store = AuthSessionStore::default();
    store.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            120_000,
        ),
        "access",
        Some("refresh".to_owned()),
    );
    let expected = store.generation();
    store.clear();

    assert!(!store.set_authenticated_if_generation(
        expected,
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            120_000,
        ),
        "renewed",
        None,
    ));
    assert!(store.authenticated_snapshot().is_none());
    assert!(matches!(
        store.projection(),
        SessionProjection::Unauthenticated { .. }
    ));
}

#[test]
fn a_sign_in_cancelled_after_the_callback_writes_no_session() {
    let store = Arc::new(AuthSessionStore::default());
    let service = AuthService::new(store.clone());
    // The state `begin_login` leaves behind before `run_login` starts.
    store.set(SessionProjection::authenticating());
    let generation = store.generation();

    service.cancel_login();

    assert!(!login_may_complete(false, generation, store.generation()));
    assert!(!store.set_authenticated_if_generation(
        generation,
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            120_000,
        ),
        "access",
        None,
    ));
    assert!(store.authenticated_snapshot().is_none());
    assert!(matches!(
        store.projection(),
        SessionProjection::Unauthenticated { .. }
    ));
}

#[test]
fn a_sign_in_may_complete_only_while_uncancelled_and_unmoved() {
    assert!(login_may_complete(false, 7, 7));
    assert!(!login_may_complete(true, 7, 7));
    assert!(!login_may_complete(false, 7, 8));
    assert!(!login_may_complete(true, 7, 8));
}

const OWNED_ACCOUNT: &str = "tenant-1:user-1";

/// A store signed in as `tenant-1:user-1` whose bearer expires at
/// `expires_at_ms`, and an account scope bound to that account exactly as
/// `auth_login` binds it.
fn signed_in(
    expires_at_ms: i64,
    refresh_token: Option<&str>,
) -> (Arc<AuthSessionStore>, CurrentAccountScope) {
    let store = Arc::new(AuthSessionStore::default());
    store.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "User", None),
            expires_at_ms,
        ),
        "access",
        refresh_token.map(str::to_owned),
    );
    let accounts = CurrentAccountScope::default();
    accounts.bind_native(OWNED_ACCOUNT).expect("bind account");
    (store, accounts)
}

fn an_hour_from_now() -> i64 {
    chrono::Utc::now().timestamp_millis() + 3_600_000
}

fn an_hour_ago() -> i64 {
    chrono::Utc::now().timestamp_millis() - 3_600_000
}

#[test]
fn a_signed_in_session_owns_the_account_it_bound() {
    let (store, accounts) = signed_in(an_hour_from_now(), Some("refresh"));

    assert!(store.owns_bound_account(&accounts));
}

#[test]
fn no_bound_account_is_not_signed_in() {
    let (store, _) = signed_in(an_hour_from_now(), Some("refresh"));

    assert!(!store.owns_bound_account(&CurrentAccountScope::default()));
}

#[test]
fn a_session_for_another_account_does_not_own_the_bound_one() {
    let (store, _) = signed_in(an_hour_from_now(), Some("refresh"));
    let accounts = CurrentAccountScope::default();
    accounts
        .bind_native("tenant-1:user-2")
        .expect("bind account");

    assert!(!store.owns_bound_account(&accounts));
}

/// The gap this closes: a session that ends short of a sign-out leaves the
/// account scope bound. The scope is asserted still bound, so the refusal is
/// proven to come from the session, not from a scope someone cleared.
#[test]
fn an_account_scope_that_outlives_a_cancelled_session_is_not_signed_in() {
    let (store, accounts) = signed_in(an_hour_from_now(), Some("refresh"));

    AuthService::new(store.clone()).cancel_login();

    assert!(accounts.current().expect("scope").is_some());
    assert!(!store.owns_bound_account(&accounts));
}

/// `sessions.clear()` is what a refused renewal (`invalid_grant`) and a failed
/// sign-in both do to the store, and neither touches the account scope.
#[test]
fn an_account_scope_that_outlives_a_cleared_session_is_not_signed_in() {
    let (store, accounts) = signed_in(an_hour_from_now(), Some("refresh"));

    store.clear();

    assert!(accounts.current().expect("scope").is_some());
    assert!(!store.owns_bound_account(&accounts));
}

#[test]
fn a_sign_in_in_progress_owns_no_account() {
    let (store, accounts) = signed_in(an_hour_from_now(), Some("refresh"));
    store.set(SessionProjection::authenticating());

    assert!(!store.owns_bound_account(&accounts));
}

/// Offline past expiry is not a sign-out: the refresh token renews the session
/// on the next request that reaches the tenant.
#[test]
fn an_expired_session_that_can_still_renew_owns_its_account() {
    let (store, accounts) = signed_in(an_hour_ago(), Some("refresh"));

    assert!(store.owns_bound_account(&accounts));
}

/// Expired, and nothing on this device can ever renew it: no server would
/// accept its bearer again, so nothing on this device may act for it either.
#[test]
fn a_dead_session_owns_no_account() {
    let (store, accounts) = signed_in(an_hour_ago(), None);

    assert!(!store.owns_bound_account(&accounts));
}

/// The same tolerance token verification allows: a bearer the tenant would
/// still accept is not refused here first.
#[test]
fn a_session_just_past_expiry_but_inside_the_clock_tolerance_owns_its_account() {
    let thirty_seconds_ago = chrono::Utc::now().timestamp_millis() - 30_000;
    let (store, accounts) = signed_in(thirty_seconds_ago, None);

    assert!(store.owns_bound_account(&accounts));
}

#[tokio::test]
async fn discovery_rejects_mismatched_authorization_server() {
    let da_server = MockServer::start().await;
    let issuer_server = MockServer::start().await;

    // The configured issuer is the mock server, so the metadata request is
    // served locally; the protected resource names a different authorization
    // server, which is the mismatch under test.
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resource": "https://tenant.example/mcp",
            "authorization_servers": ["https://elsewhere.example"]
        })))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": issuer_server.uri(),
            "authorization_endpoint": format!("{}/login/oauth/authorize", issuer_server.uri()),
            "token_endpoint": format!("{}/api/login/oauth/access_token", issuer_server.uri()),
            "jwks_uri": format!("{}/keys/current", issuer_server.uri())
        })))
        .mount(&issuer_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer_server.uri()).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let result = discover_auth(&client, &config).await;
    assert_eq!(result, Err(DiscoveryError::IssuerNotAllowlisted));
}

#[tokio::test]
async fn discovery_accepts_allowlisted_issuer_and_metadata() {
    let da_server = MockServer::start().await;
    let metadata_server = MockServer::start().await;
    let issuer = metadata_server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resource": "https://tenant.example/mcp",
            "authorization_servers": [issuer]
        })))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/login/oauth/authorize"),
            "token_endpoint": format!("{issuer}/api/login/oauth/access_token"),
            "jwks_uri": format!("{issuer}/keys/current")
        })))
        .mount(&metadata_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let discovered = discover_auth(&client, &config)
        .await
        .expect("discovery should succeed");
    assert_eq!(discovered.resource, "https://tenant.example/mcp");
    assert_eq!(
        discovered.issuer.as_str().trim_end_matches('/'),
        issuer.trim_end_matches('/')
    );
    assert_eq!(
        discovered.jwks_uri.as_str(),
        format!("{issuer}/keys/current")
    );
}

#[tokio::test]
async fn discovery_rejects_cross_origin_jwks_uri() {
    let da_server = MockServer::start().await;
    let metadata_server = MockServer::start().await;
    let issuer = metadata_server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resource": "https://tenant.example/mcp",
            "authorization_servers": [issuer]
        })))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/login/oauth/authorize"),
            "token_endpoint": format!("{issuer}/api/login/oauth/access_token"),
            "jwks_uri": "https://keys.example.invalid/current"
        })))
        .mount(&metadata_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    assert_eq!(
        discover_auth(&client, &config).await,
        Err(DiscoveryError::InvalidMetadata)
    );
}

/// Both discovery documents are slow. Discovery must pay one delay, not two,
/// and must return what the serial version returned.
#[tokio::test]
async fn discovery_fetches_both_documents_without_serial_latency() {
    let da_server = MockServer::start().await;
    let metadata_server = MockServer::start().await;
    let issuer = metadata_server.uri();
    let delay = Duration::from_millis(1000);

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_delay(delay).set_body_json(
            serde_json::json!({
                "resource": "https://tenant.example/mcp",
                "authorization_servers": [issuer]
            }),
        ))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_delay(delay).set_body_json(
            serde_json::json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/login/oauth/authorize"),
                "token_endpoint": format!("{issuer}/api/login/oauth/access_token"),
                "jwks_uri": format!("{issuer}/keys/current")
            }),
        ))
        .mount(&metadata_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let started = Instant::now();
    let discovered = discover_auth(&client, &config)
        .await
        .expect("discovery should succeed");
    let elapsed = started.elapsed();
    eprintln!("discover_auth with 1000 ms per endpoint took {elapsed:?}");

    assert_eq!(
        discovered,
        super::discovery::DiscoveredAuth {
            resource: "https://tenant.example/mcp".to_owned(),
            authorization_endpoint: Url::parse(&format!("{issuer}/login/oauth/authorize"))
                .expect("authorize"),
            token_endpoint: Url::parse(&format!("{issuer}/api/login/oauth/access_token"))
                .expect("token"),
            jwks_uri: Url::parse(&format!("{issuer}/keys/current")).expect("jwks"),
            issuer: Url::parse(&issuer).expect("issuer"),
        }
    );
    assert!(
        elapsed < Duration::from_millis(1600),
        "discovery ran its two requests serially: {elapsed:?}"
    );
}

/// The metadata request is slow and the protected resource fails at once.
/// Discovery must report the protected resource failure without waiting for
/// the metadata request, which would hold the caller's renewal lock.
#[tokio::test]
async fn a_failed_protected_resource_fetch_does_not_wait_for_a_slow_metadata_fetch() {
    let da_server = MockServer::start().await;
    let metadata_server = MockServer::start().await;
    let issuer = metadata_server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(3))
                .set_body_json(serde_json::json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{issuer}/login/oauth/authorize"),
                    "token_endpoint": format!("{issuer}/api/login/oauth/access_token"),
                    "jwks_uri": format!("{issuer}/keys/current")
                })),
        )
        .mount(&metadata_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let started = Instant::now();
    let result = discover_auth(&client, &config).await;
    let elapsed = started.elapsed();

    assert_eq!(result, Err(DiscoveryError::ProtectedResourceUnavailable));
    assert!(
        elapsed < Duration::from_millis(1500),
        "discovery waited for the slow metadata request: {elapsed:?}"
    );
}

#[tokio::test]
async fn protected_resource_failure_is_reported_before_a_broken_metadata_document() {
    let da_server = MockServer::start().await;
    let metadata_server = MockServer::start().await;
    let issuer = metadata_server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&metadata_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    assert_eq!(
        discover_auth(&client, &config).await,
        Err(DiscoveryError::ProtectedResourceUnavailable)
    );
}

#[tokio::test]
async fn non_allowlisted_issuer_is_reported_before_a_broken_metadata_document() {
    let da_server = MockServer::start().await;
    let metadata_server = MockServer::start().await;
    let issuer = metadata_server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resource": "https://tenant.example/mcp",
            "authorization_servers": ["https://elsewhere.example"]
        })))
        .mount(&da_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&metadata_server)
        .await;

    let client = build_discovery_client().expect("client");
    let config = AuthConfig {
        da_base_url: Url::parse(&da_server.uri()).expect("da url"),
        casdoor_issuer: Url::parse(&issuer).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    assert_eq!(
        discover_auth(&client, &config).await,
        Err(DiscoveryError::IssuerNotAllowlisted)
    );
}

// ---------------------------------------------------------------------
// Session renewal (RFC 6749 §6)
// ---------------------------------------------------------------------

/// Discovery output for the refresh tests, pointed at one mock token endpoint.
fn discovered_for(server: &MockServer) -> super::discovery::DiscoveredAuth {
    let issuer = server.uri();
    super::discovery::DiscoveredAuth {
        issuer: Url::parse(&issuer).expect("issuer"),
        authorization_endpoint: Url::parse(&format!("{issuer}/login/oauth/authorize"))
            .expect("authorize"),
        token_endpoint: Url::parse(&format!("{issuer}/api/login/oauth/access_token"))
            .expect("token"),
        jwks_uri: Url::parse(&format!("{issuer}/keys/current")).expect("jwks"),
        resource: "https://tenant.example/mcp".to_owned(),
    }
}

fn config_for(server: &MockServer) -> AuthConfig {
    AuthConfig {
        da_base_url: Url::parse("https://tenant.example").expect("da url"),
        casdoor_issuer: Url::parse(&server.uri()).expect("issuer"),
        client_id: "desktop-client".to_owned(),
        organization: "tenant".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    }
}

#[tokio::test]
async fn a_refresh_asks_for_the_same_audience_the_expiring_token_had() {
    // RFC 8707 `resource` is what makes the renewed bearer acceptable to the
    // bundled MCP server. Omitting it would renew the session and silently
    // break every tool call in it — `McpAuth::bind` refuses a bearer whose
    // audience it cannot match.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/login/oauth/access_token"))
        .and(wiremock::matchers::body_string_contains(
            "grant_type=refresh_token",
        ))
        .and(wiremock::matchers::body_string_contains("resource=https"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "renewed-access-token",
            "refresh_token": "rotated-refresh-token",
            "token_type": "Bearer"
        })))
        .mount(&server)
        .await;

    let client = build_discovery_client().expect("client");
    let tokens = super::oauth::exchange_refresh_token(
        &client,
        &discovered_for(&server),
        &config_for(&server),
        "held-refresh-token",
    )
    .await
    .expect("the server accepted the refresh");

    assert_eq!(tokens.access_token, "renewed-access-token");
    assert_eq!(
        tokens.refresh_token.as_deref(),
        Some("rotated-refresh-token")
    );
}

#[tokio::test]
async fn a_server_that_does_not_rotate_refresh_tokens_leaves_the_held_one_in_place() {
    // Reported as `None` here, and `renewal::ensure_fresh_session` keeps the
    // token it already holds. Treating an omitted field as "the grant is gone"
    // would make the *second* renewal impossible.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "renewed-access-token",
            "token_type": "Bearer"
        })))
        .mount(&server)
        .await;

    let client = build_discovery_client().expect("client");
    let tokens = super::oauth::exchange_refresh_token(
        &client,
        &discovered_for(&server),
        &config_for(&server),
        "held-refresh-token",
    )
    .await
    .expect("a response with no rotated token is still a success");

    assert!(tokens.refresh_token.is_none());
}

#[tokio::test]
async fn an_invalid_grant_is_distinguished_from_a_transport_failure() {
    // The two demand opposite responses: `invalid_grant` means the session is
    // over and the user must sign in again; anything else means keep the token
    // and try later. Collapsing them would sign people out over a flaky network.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "refresh token expired"
        })))
        .mount(&server)
        .await;

    let client = build_discovery_client().expect("client");
    let error = super::oauth::exchange_refresh_token(
        &client,
        &discovered_for(&server),
        &config_for(&server),
        "revoked-refresh-token",
    )
    .await
    .expect_err("a revoked grant cannot renew");

    assert!(
        matches!(error, super::oauth::OAuthError::RedirectRejected),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_five_hundred_from_the_token_endpoint_is_not_treated_as_a_dead_grant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = build_discovery_client().expect("client");
    let error = super::oauth::exchange_refresh_token(
        &client,
        &discovered_for(&server),
        &config_for(&server),
        "held-refresh-token",
    )
    .await
    .expect_err("503 is a failure");

    assert!(
        matches!(error, super::oauth::OAuthError::ExchangeFailed),
        "a transient failure must not read as a revoked grant: {error:?}"
    );
}
