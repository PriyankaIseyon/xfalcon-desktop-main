//! What the renewing auth provider does with a grant, and what it refuses to do.
//!
//! The flow itself (discovery, registration, the browser leg, the exchange) is
//! covered where it lives, against a `wiremock` authorization server. What is
//! covered here is the part with no HTTP in it at all: which grant a *connect*
//! ends up attaching, and what happens to a grant whose renewal is refused —
//! the two behaviours a user experiences as "it reconnected by itself" and
//! "it asked me to sign in again".

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use xfalcon_contracts::mcp::McpPermissionMode;

use crate::credentials::vault::CredentialVault;
use crate::mcp::auth::{McpAuthError, McpAuthProvider};
use crate::mcp::credential_store::CredentialOwner;
use crate::mcp::oauth::exchange::now_epoch_seconds;
use crate::mcp::oauth::store::{DeviceGrants, StoredGrant};
use crate::mcp::registry::{DialableServer, ServerAuthRequirement};
use crate::mcp::runtime::OauthGrantAuthProvider;
use crate::mcp::server_resolver::McpServerDescriptor;
use crate::persistence::InMemorySecretStore;

fn owner() -> CredentialOwner {
    CredentialOwner {
        account_id: "tenant-1__user-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        user_id: "user-1".to_owned(),
    }
}

fn grants(directory: &std::path::Path) -> DeviceGrants {
    DeviceGrants::new(
        directory
            .join("mcp-data")
            .join("server-credentials.sqlite3"),
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        owner(),
    )
}

fn descriptor() -> McpServerDescriptor {
    McpServerDescriptor::registered(
        reqwest::Url::parse("https://mcp.example.test/mcp").expect("url"),
    )
}

fn grant(token_endpoint: &str, expires_in: i64, refresh: Option<&str>) -> StoredGrant {
    StoredGrant {
        client_id: "issued-client".to_owned(),
        client_secret: None,
        issuer: "https://auth.example.test".to_owned(),
        token_endpoint: token_endpoint.to_owned(),
        revocation_endpoint: None,
        resource: "https://mcp.example.test/mcp".to_owned(),
        scope: Some("openid".to_owned()),
        access_token: "first-access-token".to_owned(),
        refresh_token: refresh.map(str::to_owned),
        expires_at: now_epoch_seconds() + expires_in,
    }
}

async fn token_endpoint(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_grant_with_life_left_is_attached_without_touching_the_network() {
    let remote = token_endpoint(ResponseTemplate::new(500)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put(
            "srv-1",
            &grant(&format!("{}/token", remote.uri()), 3600, Some("refresh")),
        )
        .expect("stored");

    let provider = OauthGrantAuthProvider::new(store, "srv-1");
    let auth = provider
        .current_auth(&descriptor())
        .await
        .expect("the stored grant binds");

    assert_eq!(auth.bearer_value(), Some("first-access-token"));
    assert!(
        remote
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "a grant that is not due for renewal must not cost a request"
    );
}

#[tokio::test]
async fn a_grant_close_to_expiry_is_renewed_and_the_renewal_is_persisted() {
    let remote = token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
        "access_token": "second-access-token",
        "refresh_token": "rotated-refresh-token",
        "expires_in": 3600,
    })))
    .await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put(
            "srv-1",
            &grant(&format!("{}/token", remote.uri()), 5, Some("first-refresh")),
        )
        .expect("stored");

    let provider = OauthGrantAuthProvider::new(store.clone(), "srv-1");
    let auth = provider
        .current_auth(&descriptor())
        .await
        .expect("the renewed grant binds");
    assert_eq!(auth.bearer_value(), Some("second-access-token"));

    let persisted = store
        .get("srv-1")
        .expect("readable")
        .expect("a stored grant");
    assert_eq!(persisted.access_token, "second-access-token");
    assert_eq!(
        persisted.refresh_token.as_deref(),
        Some("rotated-refresh-token"),
        "a rotated refresh token must replace the spent one, or the next renewal fails"
    );
}

/// Two connects racing on the same nearly-expired grant must spend the refresh
/// token once: a server that rotates refresh tokens invalidates the first
/// response when a second request arrives, so a double renewal would poison a
/// working grant.
#[tokio::test]
async fn concurrent_connects_renew_once_between_them() {
    let remote = token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
        "access_token": "second-access-token",
        "refresh_token": "rotated-refresh-token",
        "expires_in": 3600,
    })))
    .await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put(
            "srv-1",
            &grant(&format!("{}/token", remote.uri()), 5, Some("first-refresh")),
        )
        .expect("stored");

    let provider = Arc::new(OauthGrantAuthProvider::new(store, "srv-1"));
    let server = descriptor();
    let (first, second) = tokio::join!(
        provider.current_auth(&server),
        provider.current_auth(&server)
    );

    assert_eq!(
        first.expect("binds").bearer_value(),
        Some("second-access-token")
    );
    assert_eq!(
        second.expect("binds").bearer_value(),
        Some("second-access-token")
    );
    assert_eq!(
        remote.received_requests().await.expect("requests").len(),
        1,
        "the refresh token must be spent exactly once"
    );
}

#[tokio::test]
async fn a_refused_renewal_drops_the_grant_so_the_user_is_asked_to_sign_in_again() {
    let remote = token_endpoint(
        ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })),
    )
    .await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put(
            "srv-1",
            &grant(
                &format!("{}/token", remote.uri()),
                5,
                Some("revoked-refresh"),
            ),
        )
        .expect("stored");

    let provider = OauthGrantAuthProvider::new(store.clone(), "srv-1");
    let outcome = provider.current_auth(&descriptor()).await;

    assert_eq!(outcome.err(), Some(McpAuthError::MissingBearer));
    assert!(
        store.get("srv-1").expect("readable").is_none(),
        "a dead refresh token must not be retried on every reconnect"
    );
}

/// An unreachable token endpoint is not evidence that the grant is dead. The
/// stored access token is attached and the server gets to decide.
#[tokio::test]
async fn an_unreachable_renewal_keeps_the_grant_and_lets_the_server_judge_it() {
    let remote = token_endpoint(ResponseTemplate::new(503)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put(
            "srv-1",
            &grant(&format!("{}/token", remote.uri()), 5, Some("first-refresh")),
        )
        .expect("stored");

    let provider = OauthGrantAuthProvider::new(store.clone(), "srv-1");
    let auth = provider
        .current_auth(&descriptor())
        .await
        .expect("the stored grant still binds");

    assert_eq!(auth.bearer_value(), Some("first-access-token"));
    assert!(store.get("srv-1").expect("readable").is_some());
}

#[tokio::test]
async fn an_expired_grant_with_no_refresh_token_is_still_offered_before_it_is_discarded() {
    let remote = token_endpoint(ResponseTemplate::new(500)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put("srv-1", &grant(&format!("{}/token", remote.uri()), 5, None))
        .expect("stored");

    let provider = OauthGrantAuthProvider::new(store.clone(), "srv-1");
    let auth = provider
        .current_auth(&descriptor())
        .await
        .expect("the grant binds");

    assert_eq!(auth.bearer_value(), Some("first-access-token"));
    assert!(
        remote
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "there is nothing to renew with, so nothing may be sent"
    );
}

#[tokio::test]
async fn a_server_this_device_never_signed_in_to_has_no_bearer_to_attach() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let provider = OauthGrantAuthProvider::new(grants(directory.path()), "srv-1");
    assert_eq!(
        provider.current_auth(&descriptor()).await.err(),
        Some(McpAuthError::MissingBearer)
    );
}

/// The runtime picks its provider from the classification, so an `oauth` row
/// authenticates from the grant store and a `bearer` row from the credential
/// store. Getting this backwards would attach a pasted API key to a server
/// expecting a grant, or vice versa.
#[tokio::test]
async fn the_runtime_authenticates_an_oauth_row_from_the_grant_store() {
    use crate::mcp::credential_store::DeviceCredentials;
    use crate::mcp::runtime::McpRuntime;

    let remote = token_endpoint(ResponseTemplate::new(500)).await;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = grants(directory.path());
    store
        .put(
            "srv-1",
            &grant(&format!("{}/token", remote.uri()), 3600, Some("refresh")),
        )
        .expect("stored");

    let vault = Arc::new(CredentialVault::new(Arc::new(
        InMemorySecretStore::default(),
    )));
    let credentials = DeviceCredentials::new(
        directory
            .path()
            .join("mcp-data")
            .join("server-credentials.sqlite3"),
        vault,
        owner(),
    );
    // Deliberately populated with a different value: if the runtime read this
    // store for an oauth row, the assertion below would see it.
    credentials
        .put(
            "srv-1",
            zeroize::Zeroizing::new("pasted-api-key".to_owned()),
        )
        .expect("stored");

    let server = DialableServer {
        id: "srv-1".to_owned(),
        slug: "acme".to_owned(),
        label: "Acme".to_owned(),
        descriptor: descriptor(),
        auth: ServerAuthRequirement::Oauth,
        permission_mode: McpPermissionMode::Ask,
        auth_fingerprint: None,
    };
    let runtime = McpRuntime::for_dialable(&server, credentials, store, None);

    assert_eq!(
        runtime
            .auth_for_test()
            .current_auth(&descriptor())
            .await
            .expect("binds")
            .bearer_value(),
        Some("first-access-token")
    );
}

#[tokio::test]
async fn a_changed_resource_never_receives_the_old_device_bearer_or_triggers_renewal() {
    use crate::mcp::client::{connect, McpClientError};
    use tokio_util::sync::CancellationToken;

    for expiry in [3600, 0] {
        let authorization = token_endpoint(ResponseTemplate::new(500)).await;
        let replacement = MockServer::start().await;
        let directory = tempfile::tempdir().expect("directory");
        let store = grants(directory.path());
        store
            .put(
                "srv-1",
                &grant(
                    &format!("{}/token", authorization.uri()),
                    expiry,
                    Some("refresh"),
                ),
            )
            .expect("grant");
        let provider = OauthGrantAuthProvider::new(store, "srv-1");
        let server = McpServerDescriptor::registered(
            reqwest::Url::parse(&format!("{}/mcp", replacement.uri())).unwrap(),
        );
        let result = connect(&server, &provider, &CancellationToken::new()).await;
        assert!(matches!(
            result,
            Err(McpClientError::Auth(McpAuthError::AudienceMismatch))
        ));
        assert!(authorization.received_requests().await.unwrap().is_empty());
        assert!(replacement.received_requests().await.unwrap().is_empty());
    }
}
