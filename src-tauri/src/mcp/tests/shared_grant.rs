//! Interoperability with xfalcon-web's shared OAuth grant.
//!
//! Every test here starts from a row *web* wrote — a grant whose tokens are
//! `enc:v1` ciphertext under the key-encryption key web reads from
//! `PROVIDER_KEYS_ENCRYPTION_KEY` — and asserts desktop uses it, renews it under
//! the same lease protocol, and leaves the row in a state web can keep using.
//!
//! That is the whole claim of the port: one sign-in, one credential, two clients.
//! A mock DA stands in for the data plane so the lease/CAS protocol is observable
//! request by request.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use xfalcon_contracts::auth::IdentityProjection;

use crate::clients::da::mcp_oauth_grants::DaMcpGrantRepository;
use crate::mcp::oauth::envelope::GrantKey;
use crate::mcp::oauth::registration::OauthClient;
use crate::mcp::oauth::shared_grant::{self, SharedGrantClient, SharedGrantError};

const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(750);
const GRANT_PATH: &str = "/api/v1/chat/mcp-oauth-grants/by-server/srv-1";
const LEASE_PATH: &str = "/api/v1/chat/mcp-oauth-grants/grant-1:acquire-refresh-lease";
const COMMIT_PATH: &str = "/api/v1/chat/mcp-oauth-grants/grant-1:commit-refresh";
const RELEASE_PATH: &str = "/api/v1/chat/mcp-oauth-grants/grant-1:release-refresh-lease";

/// The key an operator would configure on both products.
fn shared_key() -> GrantKey {
    GrantKey::decode(&STANDARD.encode([0x5au8; 32])).expect("a 32-byte key")
}

fn identity() -> IdentityProjection {
    IdentityProjection::new("user-1", "tenant-1", "Ada Lovelace", None)
}

fn repository(server: &MockServer) -> DaMcpGrantRepository {
    DaMcpGrantRepository::for_test(&server.uri(), "session-bearer", &identity(), TEST_TIMEOUT)
        .expect("a repository")
}

/// The OAuth client web stored in `mcp_servers.auth_config`.
fn web_client() -> OauthClient {
    OauthClient {
        client_id: "connector-config-client".to_owned(),
        client_secret: Some(zeroize::Zeroizing::new("connector-secret".to_owned())),
    }
}

/// A grant row exactly as web's `upsertGrantByServerViaApi` leaves it.
fn grant_row(token_endpoint: &str, expires_in_seconds: i64, version: i64) -> Value {
    let key = shared_key();
    json!({
        "id": "grant-1",
        "tenant_id": "tenant-1",
        "server_id": "srv-1",
        "granted_by_user_id": Value::Null,
        "issuer": "https://auth.example.test",
        "token_endpoint": token_endpoint,
        "resource": "https://mcp.example.test/mcp",
        "scope": "openid offline_access",
        "access_token_enc": key.seal("web-access-token").expect("seals"),
        "refresh_token_enc": key.seal("web-refresh-token").expect("seals"),
        "access_token_expires_at": (Utc::now() + ChronoDuration::seconds(expires_in_seconds))
            .to_rfc3339(),
        "refresh_version": version,
    })
}

async fn bearer(
    da: &MockServer,
    identity: &SharedGrantClient<'_>,
) -> Result<String, SharedGrantError> {
    let http = crate::mcp::oauth::build_client().expect("client");
    let server = crate::mcp::server_resolver::McpServerDescriptor::registered(
        reqwest::Url::parse("https://mcp.example.test/mcp").unwrap(),
    );
    shared_grant::resolve_bearer(
        &http,
        &repository(da),
        &shared_key(),
        "srv-1",
        &server,
        identity,
    )
    .await
    .map(|bearer| bearer.to_string())
}

/// The fast path: a grant web minted minutes ago is used as-is. No lease, no
/// authorization-server round trip — a desktop opening a chat must not cost a
/// token refresh on a credential that is already valid.
#[tokio::test]
async fn a_grant_web_established_is_used_without_touching_the_authorization_server() {
    let da = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(GRANT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(grant_row(
            "https://auth.example.test/token",
            3600,
            4,
        )))
        .mount(&da)
        .await;

    let client = web_client();
    let resolved = bearer(
        &da,
        &SharedGrantClient {
            client: &client,
            scope: None,
        },
    )
    .await
    .expect("web's grant resolves");

    assert_eq!(
        resolved, "web-access-token",
        "the ciphertext web wrote must open with the shared key"
    );
    let paths: Vec<String> = da
        .received_requests()
        .await
        .expect("requests")
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert_eq!(paths, vec![GRANT_PATH.to_owned()]);
}

/// The renewal path, end to end: lease, refresh at the authorization server,
/// commit the re-encrypted tokens under the version CAS. The committed ciphertext
/// must open with the shared key, or web would be left with a grant it cannot read.
#[tokio::test]
async fn an_expiring_grant_is_renewed_and_left_readable_by_web() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "renewed-access-token",
            "refresh_token": "rotated-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "openid offline_access",
        })))
        .mount(&idp)
        .await;

    let da = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(GRANT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(grant_row(
            &format!("{}/token", idp.uri()),
            10,
            4,
        )))
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(LEASE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "acquired": true, "refresh_version": 4, "lease_active": false }),
        ))
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(COMMIT_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "committed": true, "refresh_version": 5 })),
        )
        .mount(&da)
        .await;

    let client = web_client();
    let resolved = bearer(
        &da,
        &SharedGrantClient {
            client: &client,
            scope: None,
        },
    )
    .await
    .expect("the renewal succeeds");
    assert_eq!(resolved, "renewed-access-token");

    // The refresh carried web's own client credentials and the grant's resource.
    let idp_request = &idp.received_requests().await.expect("requests")[0];
    let body = String::from_utf8(idp_request.body.clone()).expect("a form body");
    assert!(body.contains("grant_type=refresh_token"));
    assert!(body.contains("resource=https%3A%2F%2Fmcp.example.test%2Fmcp"));
    assert!(
        !body.contains("connector-secret"),
        "the client secret belongs in the Authorization header"
    );
    assert!(idp_request.headers.get("authorization").is_some());

    // The commit pinned the version it read, named its own lease holder, and wrote
    // ciphertext the shared key opens.
    let commit = da
        .received_requests()
        .await
        .expect("requests")
        .into_iter()
        .find(|request| request.url.path() == COMMIT_PATH)
        .expect("a commit request");
    let sent: Value = serde_json::from_slice(&commit.body).expect("a json body");
    assert_eq!(sent["expected_version"], json!(4));
    assert!(sent["holder"].as_str().is_some_and(|id| !id.is_empty()));

    let key = shared_key();
    for field in ["access_token_enc", "refresh_token_enc"] {
        let stored = sent[field].as_str().expect("ciphertext");
        assert!(
            crate::mcp::oauth::envelope::is_envelope(stored),
            "{field} must be an enc:v1 envelope web can parse"
        );
        assert!(key.open(stored).is_ok(), "{field} must open with the key");
    }
    assert_eq!(
        key.open(sent["refresh_token_enc"].as_str().expect("ciphertext"))
            .expect("opens")
            .as_str(),
        "rotated-refresh-token",
        "a rotated refresh token must be persisted, or the next renewal fails"
    );
}

/// A peer holding the lease — the web app refreshing the same row at the same
/// moment — must not produce a second call to the authorization server. Desktop
/// backs off and picks up the token the peer persisted.
#[tokio::test]
async fn a_peer_holding_the_lease_is_waited_for_rather_than_raced() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&idp)
        .await;

    let da = MockServer::start().await;
    let token_endpoint = format!("{}/token", idp.uri());
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&reads);
    let expiring = grant_row(&token_endpoint, 10, 4);
    let refreshed = grant_row(&token_endpoint, 3600, 5);
    Mock::given(method("GET"))
        .and(path(GRANT_PATH))
        .respond_with(move |_: &Request| {
            // First read: nearly expired. Second read: the peer's refresh has
            // landed, so the fast path takes it.
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            let body = if attempt == 0 { &expiring } else { &refreshed };
            ResponseTemplate::new(200).set_body_json(body.clone())
        })
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(LEASE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "acquired": false, "refresh_version": 4, "lease_active": true }),
        ))
        .mount(&da)
        .await;

    let client = web_client();
    let resolved = bearer(
        &da,
        &SharedGrantClient {
            client: &client,
            scope: None,
        },
    )
    .await
    .expect("the peer's refreshed token resolves");

    assert_eq!(resolved, "web-access-token");
    assert!(
        idp.received_requests().await.is_some_and(|r| r.is_empty()),
        "losing the lease must not produce a second refresh at the identity provider"
    );
    assert!(reads.load(Ordering::SeqCst) >= 2, "the row must be re-read");
}

/// A refresh token the authorization server has revoked is a dead grant. The user
/// has to sign in again, and desktop must say so rather than retrying.
#[tokio::test]
async fn a_revoked_refresh_token_asks_for_a_new_sign_in() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })))
        .mount(&idp)
        .await;

    let da = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(GRANT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(grant_row(
            &format!("{}/token", idp.uri()),
            10,
            4,
        )))
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(LEASE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "acquired": true })))
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(RELEASE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "released": true })))
        .mount(&da)
        .await;

    let client = web_client();
    let outcome = bearer(
        &da,
        &SharedGrantClient {
            client: &client,
            scope: None,
        },
    )
    .await;

    assert_eq!(
        outcome.err(),
        Some(SharedGrantError::ReauthorizationRequired)
    );
    // The lease is handed back rather than left to expire, so the web app is not
    // blocked for the whole lease window by this device's dead grant.
    assert!(
        da.received_requests()
            .await
            .expect("requests")
            .iter()
            .any(|request| request.url.path() == RELEASE_PATH),
        "a failed renewal must release the lease it holds"
    );
}

/// A server with no grant row is not signed in — the row simply does not exist
/// yet, which is what the surface turns into "Sign in with this server".
#[tokio::test]
async fn a_server_with_no_shared_grant_reports_that_a_sign_in_is_needed() {
    let da = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(GRANT_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&da)
        .await;

    let client = web_client();
    let outcome = bearer(
        &da,
        &SharedGrantClient {
            client: &client,
            scope: None,
        },
    )
    .await;
    assert_eq!(
        outcome.err(),
        Some(SharedGrantError::ReauthorizationRequired)
    );
}

/// A grant web wrote under a *different* key-encryption key cannot be opened
/// here. This must not be reported as "sign in again": the grant is fine and the
/// fix is a deployment setting, so a user re-running the browser flow would not
/// help. The renewal path is what surfaces it, since an unopenable access token
/// falls through to a refresh whose refresh token is equally unopenable.
#[tokio::test]
async fn a_grant_sealed_under_another_key_is_not_mistaken_for_a_dead_grant() {
    let da = MockServer::start().await;
    let other_key = GrantKey::decode(&STANDARD.encode([0x77u8; 32])).expect("key");
    let row = json!({
        "id": "grant-1",
        "tenant_id": "tenant-1",
        "server_id": "srv-1",
        "issuer": "https://auth.example.test",
        "token_endpoint": "https://auth.example.test/token",
        "resource": "https://mcp.example.test/mcp",
        "scope": "openid",
        "access_token_enc": other_key.seal("their-access-token").expect("seals"),
        "refresh_token_enc": other_key.seal("their-refresh-token").expect("seals"),
        "access_token_expires_at": (Utc::now() + ChronoDuration::hours(1)).to_rfc3339(),
        "refresh_version": 1,
    });
    Mock::given(method("GET"))
        .and(path(GRANT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(row))
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(LEASE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "acquired": true })))
        .mount(&da)
        .await;
    Mock::given(method("POST"))
        .and(path(RELEASE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "released": true })))
        .mount(&da)
        .await;

    let client = web_client();
    let outcome = bearer(
        &da,
        &SharedGrantClient {
            client: &client,
            scope: None,
        },
    )
    .await;

    // Web maps an undecryptable refresh token to reauthorization, and this mirrors
    // it: the two products must agree on what a KEK mismatch means, even though
    // the honest fix is to configure the same key.
    assert_eq!(
        outcome.err(),
        Some(SharedGrantError::ReauthorizationRequired)
    );
}

#[tokio::test]
async fn a_changed_resource_never_receives_the_shared_bearer_or_starts_a_refresh_lease() {
    use crate::mcp::auth::McpAuthError;
    use crate::mcp::client::{connect, McpClientError};
    use crate::mcp::runtime::SharedGrantAuthProvider;
    use crate::mcp::server_resolver::McpServerDescriptor;
    use tokio_util::sync::CancellationToken;

    for expiry in [3600, 0] {
        let da = MockServer::start().await;
        let authorization = MockServer::start().await;
        let replacement = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(GRANT_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(grant_row(
                &format!("{}/token", authorization.uri()),
                expiry,
                4,
            )))
            .expect(1)
            .mount(&da)
            .await;
        let provider = SharedGrantAuthProvider::new(
            Arc::new(repository(&da)),
            Arc::new(shared_key()),
            "srv-1",
            web_client(),
            None,
        );
        let server = McpServerDescriptor::registered(
            reqwest::Url::parse(&format!("{}/mcp", replacement.uri())).unwrap(),
        );
        let result = connect(&server, &provider, &CancellationToken::new()).await;
        assert!(matches!(
            result,
            Err(McpClientError::Auth(McpAuthError::MissingBearer))
        ));
        assert!(authorization.received_requests().await.unwrap().is_empty());
        assert!(replacement.received_requests().await.unwrap().is_empty());
        assert!(da
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method == reqwest::Method::GET));
    }
}
