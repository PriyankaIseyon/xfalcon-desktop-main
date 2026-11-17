//! The `mcp_server_*` surface: what reaches DA, what stays on the device, and
//! what a user is told when something fails.
//!
//! The registry side is a real `wiremock` DA (same row/page shapes as
//! `crate::clients::da::tests::mcp_servers_wire`), and the connect side is a
//! real `rmcp` server (`super::support`), so the create/rollback ordering and
//! the connection-state mapping are both observed rather than asserted about
//! mocks.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use reqwest::{Method, Url};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use wiremock::{
    matchers::{method, path, path_regex},
    Mock, MockServer, ResponseTemplate,
};
use xfalcon_contracts::auth::{IdentityProjection, SessionProjection};
use xfalcon_contracts::mcp::{
    McpAuthType, McpConnectionState, McpPermissionMode, McpServerErrorCode, McpServerInput,
    McpServerUnavailableReason, BUNDLED_SERVER_ID,
};

use super::support::{
    spawn_raw_fixture, FixtureServer, FixtureState, RawBehavior, RawFixtureState,
};
use crate::auth::{AuthConfig, AuthSessionStore};
use crate::credentials::vault::CredentialVault;
use crate::mcp::auth::{McpAuthError, McpAuthProvider};
use crate::mcp::commands::{McpServerRuntime, OauthClientInput};
use crate::mcp::credential_store::{credential_db_path, CredentialOwner, McpCredentialStore};
use crate::mcp::oauth::envelope::GrantKey;
use crate::mcp::oauth::flow::BrowserOpener;
use crate::mcp::oauth::store::{DeviceGrants, DeviceOauthClients, StoredGrant};
use crate::mcp::registry::ENFORCE_PUBLIC_MCP_HOSTS;
use crate::mcp::runtime::SessionMcpAuthProvider;
use crate::mcp::server_resolver::McpServerResolver;
use crate::persistence::InMemorySecretStore;

const ROUTE: &str = "/api/v1/chat/mcp-servers";
const ROW_ID: &str = "00000000-0000-0000-0000-0000000000aa";
const ACCOUNT_ID: &str = "tenant-1:user-1";

fn identity() -> IdentityProjection {
    IdentityProjection::new("user-1", "tenant-1", "Registry User", None)
}

/// A login port whose MCP sign-in port (`login + 1`) is free right now.
///
/// The sign-in loopback port is deliberately fixed rather than ephemeral in
/// production (`oauth::flow` explains why: authorization servers compare the
/// redirect URI literally). That makes it shared state between tests, and two
/// concurrent sign-ins on one hard-coded port produce a `RedirectUnavailable`
/// in whichever bound second -- a failure that looks like a client-resolution
/// bug and is not one. Every runtime a test builds therefore gets its own pair.
fn free_login_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("an ephemeral port")
        .local_addr()
        .expect("its address")
        .port()
        - 1
}

fn owner() -> CredentialOwner {
    CredentialOwner {
        account_id: ACCOUNT_ID.to_owned(),
        tenant_id: "tenant-1".to_owned(),
        user_id: "user-1".to_owned(),
    }
}

struct UnexpectedBrowser;

impl BrowserOpener for UnexpectedBrowser {
    fn open(&self, _url: &Url) -> bool {
        panic!("this operation must not open a sign-in")
    }
}

struct RedirectingBrowser;

impl BrowserOpener for RedirectingBrowser {
    fn open(&self, url: &Url) -> bool {
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        let redirect = pairs.get("redirect_uri").expect("redirect uri").to_string();
        let state = pairs.get("state").expect("state").to_string();
        tokio::spawn(async move {
            let _ = reqwest::Client::new()
                .get(format!("{redirect}?code=the-code&state={state}"))
                .send()
                .await;
        });
        true
    }
}

struct RefusingBrowser;

impl BrowserOpener for RefusingBrowser {
    fn open(&self, _url: &Url) -> bool {
        false
    }
}

#[tokio::test]
async fn sign_in_does_not_offer_the_configured_client_to_an_unrelated_issuer() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth", "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(&remote)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": remote.uri(),
            "authorization_endpoint": format!("{}/authorize", remote.uri()),
            "token_endpoint": format!("{}/token", remote.uri()),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&remote)
        .await;
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(identity(), 120_000),
        "registry-bearer",
        None,
    );
    let directory = tempdir().unwrap();
    let runtime = McpServerRuntime::for_test(
        sessions,
        directory.path().to_owned(),
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        AuthConfig {
            da_base_url: Url::parse(&da.uri()).unwrap(),
            casdoor_issuer: Url::parse("https://tenant-auth.example").unwrap(),
            client_id: "login-client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: free_login_port(),
            mcp_client_id: Some("fallback-client".to_owned()),
            mcp_client_secret: Some("fallback-secret".to_owned()),
        },
    );
    let error = runtime
        .sign_in(ROW_ID.to_owned(), None, None, &UnexpectedBrowser)
        .await
        .unwrap_err();
    assert_eq!(
        error.code,
        McpServerErrorCode::ClientRegistrationUnavailable
    );
    assert!(remote
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| {
            request.method == Method::GET && request.headers.get("authorization").is_none()
        }));
}

#[tokio::test]
async fn explicit_client_overrides_a_configured_shared_binding_and_stays_on_device() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri()),
            "auth_config": {
                "client_id": "shared-client",
                "client_secret": "shared-secret"
            }
        }))],
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(&remote)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": remote.uri(),
            "authorization_endpoint": format!("{}/authorize", remote.uri()),
            "token_endpoint": format!("{}/token", remote.uri()),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&remote)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "issued-access-token",
            "refresh_token": "issued-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&remote)
        .await;

    let directory = tempdir().unwrap();
    let vault = Arc::new(CredentialVault::new(Arc::new(
        InMemorySecretStore::default(),
    )));
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(identity(), 120_000),
        "registry-bearer",
        None,
    );
    let runtime = McpServerRuntime::for_test(
        sessions,
        directory.path().to_path_buf(),
        vault,
        AuthConfig {
            da_base_url: Url::parse(&da.uri()).unwrap(),
            casdoor_issuer: Url::parse(&remote.uri()).unwrap(),
            client_id: "login-client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: free_login_port(),
            mcp_client_id: Some("fallback-client".to_owned()),
            mcp_client_secret: Some("fallback-secret".to_owned()),
        },
    )
    .with_test_grant_key(Arc::new(
        GrantKey::decode(&STANDARD.encode([0x5au8; 32])).unwrap(),
    ));

    let view = runtime
        .sign_in(
            ROW_ID.to_owned(),
            Some("explicit-client".to_owned()),
            Some("explicit-secret".to_owned()),
            &RedirectingBrowser,
        )
        .await
        .expect("explicit device sign-in");

    assert_eq!(
        view.sign_in,
        Some(xfalcon_contracts::mcp::McpSignInStatus::SignedIn)
    );
    let requests = remote.received_requests().await.unwrap();
    let token = requests
        .iter()
        .find(|request| request.url.path() == "/token")
        .expect("token exchange");
    assert_eq!(
        token.headers["authorization"],
        format!(
            "Basic {}",
            STANDARD.encode("explicit-client:explicit-secret")
        )
    );
    assert!(da
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method == Method::GET));
}

/// A runtime wired to a loopback DA, a real device vault over an in-memory
/// keyring, and a temporary local-data root.
struct Harness {
    runtime: McpServerRuntime,
    vault: Arc<CredentialVault>,
    local_data: TempDir,
}

impl Harness {
    fn signed_in(da_origin: &str) -> Self {
        Self::build(da_origin, true, false)
    }

    fn signed_out(da_origin: &str) -> Self {
        Self::build(da_origin, false, false)
    }

    /// As [`Self::signed_in`], but with a regular file where the per-account
    /// directory has to go, so every credential *write* fails while presence
    /// checks still answer honestly.
    fn signed_in_with_unwritable_store(da_origin: &str) -> Self {
        Self::build(da_origin, true, true)
    }

    fn build(da_origin: &str, signed_in: bool, poison_store: bool) -> Self {
        let local_data = tempdir().expect("tempdir");
        if poison_store {
            std::fs::write(local_data.path().join("accounts"), b"not a directory")
                .expect("poison the local data root");
        }
        let sessions = Arc::new(AuthSessionStore::default());
        if signed_in {
            sessions.set_authenticated(
                SessionProjection::authenticated(identity(), 120_000),
                "registry-bearer",
                None,
            );
        }
        let vault = Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        )));
        let runtime = McpServerRuntime::for_test(
            sessions,
            local_data.path().to_path_buf(),
            vault.clone(),
            AuthConfig {
                da_base_url: Url::parse(da_origin).expect("loopback da origin"),
                casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
                client_id: "client".to_owned(),
                organization: "org".to_owned(),
                loopback_port: free_login_port(),
                mcp_client_id: None,
                mcp_client_secret: None,
            },
        );
        Self {
            runtime,
            vault,
            local_data,
        }
    }

    /// The device credential this account holds for `server_id`, read straight
    /// from the store rather than through the runtime.
    fn stored_credential(&self, server_id: &str) -> Option<String> {
        let path = credential_db_path(self.local_data.path(), ACCOUNT_ID);
        if !path.exists() {
            return None;
        }
        McpCredentialStore::open(path, self.vault.clone())
            .expect("the store opens")
            .get(&owner(), server_id)
            .expect("the credential opens")
            .map(|value| value.to_string())
    }

    /// The operator-issued OAuth client this account holds for `server_id`, read
    /// straight from the store rather than through the runtime.
    ///
    /// Returns the identifier and its confidential half, so a test can tell
    /// "the pair is stored" from "only the identifier survived".
    fn stored_oauth_client(&self, server_id: &str) -> Option<(String, Option<String>)> {
        self.oauth_clients()
            .get(server_id)
            .expect("the stored client opens")
            .map(|client| {
                (
                    client.client_id,
                    client.client_secret.map(|secret| secret.to_string()),
                )
            })
    }

    fn oauth_clients(&self) -> DeviceOauthClients {
        DeviceOauthClients::new(
            credential_db_path(self.local_data.path(), ACCOUNT_ID),
            self.vault.clone(),
            owner(),
        )
    }

    fn reconstructed_runtime(&self, da_origin: &str) -> McpServerRuntime {
        runtime_for_identity(
            self.local_data.path(),
            self.vault.clone(),
            da_origin,
            identity(),
        )
    }

    fn seed_oauth_grant(&self, server_id: &str, resource: &str) {
        DeviceGrants::new(
            credential_db_path(self.local_data.path(), ACCOUNT_ID),
            self.vault.clone(),
            owner(),
        )
        .put(
            server_id,
            &StoredGrant {
                client_id: "stored-client".to_owned(),
                client_secret: None,
                issuer: "https://auth.example.test".to_owned(),
                token_endpoint: "https://auth.example.test/token".to_owned(),
                revocation_endpoint: None,
                resource: resource.to_owned(),
                scope: None,
                access_token: "old-access-token".to_owned(),
                refresh_token: None,
                expires_at: i64::MAX,
            },
        )
        .expect("seed oauth grant");
    }
}

fn runtime_for_identity(
    local_data: &std::path::Path,
    vault: Arc<CredentialVault>,
    da_origin: &str,
    identity: IdentityProjection,
) -> McpServerRuntime {
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(identity, 120_000),
        "registry-bearer",
        None,
    );
    McpServerRuntime::for_test(
        sessions,
        local_data.to_path_buf(),
        vault,
        AuthConfig {
            da_base_url: Url::parse(da_origin).expect("loopback da origin"),
            casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
            client_id: "client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: free_login_port(),
            mcp_client_id: None,
            mcp_client_secret: None,
        },
    )
}

fn row(overrides: Value) -> Value {
    let mut base = json!({
        "id": ROW_ID,
        "tenant_id": "tenant-1",
        "user_id": "user-1",
        "name": "crm",
        "display_name": "Acme CRM",
        "description": null,
        "transport": "http",
        "url": "https://mcp.example.test/mcp",
        "command": null,
        "args": null,
        "icon_url": null,
        "auth_type": "none",
        "auth_config": null,
        "is_enabled": true,
        "status": "connected",
        "status_message": null,
        "last_health_check": null,
        "permission_mode": "ask",
        "metadata": { "origin": "xfalcon-desktop" },
        "created_at": "1970-01-01T00:00:00Z",
        "updated_at": "1970-01-01T00:00:00Z",
        "visibility": "private"
    });
    let (Value::Object(base_map), Value::Object(override_map)) = (&mut base, overrides) else {
        panic!("row overrides must be a JSON object");
    };
    for (key, value) in override_map {
        base_map.insert(key, value);
    }
    base
}

fn page(items: Vec<Value>) -> Value {
    json!({ "items": items, "has_more": false, "next_cursor": null })
}

async fn mount_list(server: &MockServer, rows: Vec<Value>) {
    Mock::given(method(Method::GET))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(rows)))
        .mount(server)
        .await;
}

/// The client details a sheet submits: an identifier, and optionally the value
/// issued for it.
fn oauth_client(client_id: Option<&str>, client_secret: Option<&str>) -> OauthClientInput {
    OauthClientInput {
        client_id: client_id.map(str::to_owned),
        client_secret: client_secret.map(str::to_owned),
    }
}

fn input(name: &str, url: &str, auth_type: McpAuthType) -> McpServerInput {
    McpServerInput {
        name: name.to_owned(),
        display_name: None,
        description: None,
        url: url.to_owned(),
        auth_type,
        header_name: None,
        permission_mode: McpPermissionMode::Ask,
        enabled: true,
    }
}

// ---------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_bundled_server_is_listed_first_and_carries_no_da_row() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let listed = harness.runtime.list().await.expect("the registry lists");

    assert_eq!(listed.servers.len(), 2);
    let bundled = &listed.servers[0];
    assert!(bundled.builtin);
    assert!(bundled.id.is_bundled());
    assert_eq!(bundled.slug, BUNDLED_SERVER_ID);
    assert_eq!(
        bundled.url.as_deref(),
        Some(format!("{}/mcp", server.uri()).as_str()),
        "the bundled endpoint is derived from configuration, not from a row"
    );
    assert!(!listed.servers[1].builtin);
}

#[tokio::test]
async fn a_registered_alias_of_the_bundled_endpoint_is_neither_listed_nor_held() {
    let server = MockServer::start().await;
    mount_list(
        &server,
        vec![row(json!({"url": format!("{}/mcp", server.uri())}))],
    )
    .await;
    let harness = Harness::signed_in(&server.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&server.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let listed = harness.runtime.list().await.expect("the registry lists");
    let runtime = harness.runtime.runtime_set(&config).await;

    assert_eq!(
        listed.servers.len(),
        1,
        "Settings receives only the canonical bundled server"
    );
    assert!(listed.servers[0].builtin);
    assert_eq!(
        runtime.len(),
        1,
        "a run must not hold a second session to the same endpoint"
    );
    assert!(runtime.find(BUNDLED_SERVER_ID).is_some());
    assert!(runtime.find("crm").is_none());
}

#[tokio::test]
async fn a_row_needing_a_credential_this_device_lacks_is_listed_as_needing_one() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({"auth_type": "bearer"}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let listed = harness.runtime.list().await.expect("the registry lists");

    assert_eq!(
        listed.servers[1].unavailable,
        Some(McpServerUnavailableReason::MissingDeviceCredential)
    );
}

#[tokio::test]
async fn scoped_runtime_registers_and_dispatches_only_the_selected_server() {
    use crate::agent::tool_loop::ToolExecutorPort;
    use std::sync::atomic::Ordering;
    use tokio_util::sync::CancellationToken;
    use xfalcon_contracts::tools::ToolId;

    let da = MockServer::start().await;
    let selected_state = FixtureState::default();
    let (selected_url, _selected_guard) =
        FixtureServer::new(0, selected_state.clone()).spawn().await;
    let sibling_state = FixtureState::default();
    let (sibling_url, _sibling_guard) = FixtureServer::new(0, sibling_state.clone()).spawn().await;
    mount_list(
        &da,
        vec![
            row(json!({"url": selected_url, "name": "renamed-crm"})),
            row(json!({"id": "00000000-0000-0000-0000-0000000000bb", "name": "sibling", "url": sibling_url})),
        ],
    )
    .await;
    let harness = Harness::signed_in(&da.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let (servers, endpoint) = harness
        .runtime
        .runtime_set_for_server(&config, ROW_ID, None)
        .await
        .expect("selected server runtime");
    assert_eq!(servers.len(), 1);
    let selected_slug = format!("selected_{ROW_ID}");
    assert!(servers.find(&selected_slug).is_some());
    assert!(servers.find("crm").is_none());
    assert!(servers.find(BUNDLED_SERVER_ID).is_none());
    assert_eq!(endpoint, selected_url);

    let mut tools = crate::tools::registry::ToolRegistry::default();
    servers.register_tools(&mut tools).await;
    let selected_tool = format!("mcp.{selected_slug}.echo");
    assert!(tools.get(&ToolId::from(selected_tool.clone())).is_some());
    assert!(tools.get(&ToolId::from("mcp.crm.echo")).is_none());
    assert!(tools
        .get(&ToolId::from(format!("mcp.{BUNDLED_SERVER_ID}.echo")))
        .is_none());
    assert!(tools.get(&ToolId::from("mcp.sibling.echo")).is_none());

    let dispatcher = crate::mcp::dispatch::McpToolDispatcher::new(servers);
    dispatcher
        .execute(
            &ToolId::from(selected_tool),
            &json!({"text": "scoped"}),
            CancellationToken::new(),
        )
        .await
        .expect("selected call dispatches");
    assert!(dispatcher
        .execute(
            &ToolId::from("mcp.crm.echo"),
            &json!({"text": "old selected slug"}),
            CancellationToken::new(),
        )
        .await
        .is_err());
    assert!(dispatcher
        .execute(
            &ToolId::from("mcp.sibling.echo"),
            &json!({"text": "must not route"}),
            CancellationToken::new(),
        )
        .await
        .is_err());
    assert_eq!(selected_state.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(sibling_state.call_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scoped_runtime_rejects_invalid_or_changed_selection_before_provider_use() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![
            row(json!({"is_enabled": false})),
            row(json!({"id": "00000000-0000-0000-0000-0000000000bb", "name": "denied", "permission_mode": "deny"})),
            row(json!({"id": "00000000-0000-0000-0000-0000000000cc", "name": "no-credential", "auth_type": "bearer"})),
            row(json!({"id": "00000000-0000-0000-0000-0000000000dd", "name": "invalid-url", "url": "not a URL"})),
            row(json!({"id": "00000000-0000-0000-0000-0000000000ee", "name": "no-grant", "auth_type": "oauth"})),
            row(json!({"id": "00000000-0000-0000-0000-0000000000ff", "name": "tenant-alias", "url": format!("{}/mcp", da.uri())})),
            row(json!({"id": "00000000-0000-0000-0000-000000000011", "name": "healthy", "url": "https://HEALTHY.example.test:443/mcp"})),
        ],
    )
    .await;
    let harness = Harness::signed_in(&da.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let oversized = "x".repeat(257);
    for (id, expected_code) in [
        ("", McpServerErrorCode::InvalidRequest),
        (oversized.as_str(), McpServerErrorCode::InvalidRequest),
        ("missing", McpServerErrorCode::NotFound),
        (ROW_ID, McpServerErrorCode::NotFound),
        (
            "00000000-0000-0000-0000-0000000000bb",
            McpServerErrorCode::NotFound,
        ),
        (
            "00000000-0000-0000-0000-0000000000cc",
            McpServerErrorCode::ServiceUnavailable,
        ),
        (
            "00000000-0000-0000-0000-0000000000dd",
            McpServerErrorCode::ServiceUnavailable,
        ),
        (
            "00000000-0000-0000-0000-0000000000ee",
            McpServerErrorCode::ServiceUnavailable,
        ),
        (
            "00000000-0000-0000-0000-0000000000ff",
            McpServerErrorCode::NotEditable,
        ),
    ] {
        let error = match harness
            .runtime
            .runtime_set_for_server(&config, id, None)
            .await
        {
            Ok(_) => panic!("{id:?} must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code, expected_code, "{id:?}");
    }
    let normalized = harness
        .runtime
        .runtime_set_for_server(
            &config,
            "00000000-0000-0000-0000-000000000011",
            Some("https://healthy.example.test/mcp"),
        )
        .await
        .expect("normalized selected endpoint matches");
    assert_eq!(normalized.1, "https://healthy.example.test/mcp");
    let mismatch = harness
        .runtime
        .runtime_set_for_server(
            &config,
            "00000000-0000-0000-0000-000000000011",
            Some("https://changed.example/mcp"),
        )
        .await;
    assert!(mismatch.is_err(), "an endpoint change fails closed");
}

#[tokio::test]
async fn scoped_runtime_refuses_a_signed_out_selection_before_registry_access() {
    let da = MockServer::start().await;
    let harness = Harness::signed_out(&da.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let result = harness
        .runtime
        .runtime_set_for_server(&config, ROW_ID, None)
        .await;
    let error = match result {
        Ok(_) => panic!("signed-out selection must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, McpServerErrorCode::Unauthorized);
    assert!(da.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn scoped_runtime_fails_registry_errors_without_falling_back_to_bundled() {
    let da = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(503))
        .mount(&da)
        .await;
    let harness = Harness::signed_in(&da.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let result = harness
        .runtime
        .runtime_set_for_server(&config, ROW_ID, None)
        .await;
    let error = match result {
        Ok(_) => panic!("registry failure must fail selected-server resolution"),
        Err(error) => error,
    };
    assert_eq!(error.code, McpServerErrorCode::ServiceUnavailable);
}

#[tokio::test]
async fn scoped_runtime_fails_device_store_errors_without_falling_back() {
    let da = MockServer::start().await;
    mount_list(&da, vec![row(json!({}))]).await;
    let harness = Harness::signed_in(&da.uri());
    std::fs::create_dir_all(credential_db_path(harness.local_data.path(), ACCOUNT_ID))
        .expect("a directory where the device store database file belongs");
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let result = harness
        .runtime
        .runtime_set_for_server(&config, ROW_ID, None)
        .await;
    let error = match result {
        Ok(_) => panic!("device store failure must fail selected-server resolution"),
        Err(error) => error,
    };
    assert_eq!(error.code, McpServerErrorCode::StorageUnavailable);
}

#[tokio::test]
async fn scoped_bundled_runtime_uses_only_the_configured_tenant_endpoint() {
    let da = MockServer::start().await;
    let harness = Harness::signed_in(&da.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let expected = format!("{}/mcp", da.uri());
    let (servers, endpoint) = harness
        .runtime
        .runtime_set_for_server(&config, BUNDLED_SERVER_ID, Some(&expected))
        .await
        .expect("bundled selected runtime");
    assert_eq!(servers.len(), 1);
    assert!(servers
        .find(&format!("selected_{BUNDLED_SERVER_ID}"))
        .is_some());
    assert!(servers.find(BUNDLED_SERVER_ID).is_none());
    assert_eq!(endpoint, expected);
    assert!(
        da.received_requests().await.unwrap().is_empty(),
        "bundled selection does not read registered servers"
    );
}

#[tokio::test]
async fn scoped_bundled_runtime_rejects_a_removed_connection() {
    let da = MockServer::start().await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .runtime
        .delete(BUNDLED_SERVER_ID.to_owned())
        .await
        .expect("remove bundled connection");
    let config = AuthConfig {
        da_base_url: Url::parse(&da.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: free_login_port(),
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let result = harness
        .runtime
        .runtime_set_for_server(&config, BUNDLED_SERVER_ID, None)
        .await;
    assert!(result.is_err(), "removed bundled server cannot be selected");
}

#[tokio::test]
async fn selected_bundled_auth_provider_refuses_a_different_account() {
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(identity(), 120_000),
        "account-a-bearer",
        None,
    );
    let config = AuthConfig {
        da_base_url: Url::parse("https://da.example.test").expect("DA origin"),
        casdoor_issuer: Url::parse("https://auth.example.test").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 0,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    let descriptor = McpServerResolver::resolve(&config);
    sessions.set_authorized_resource(descriptor.endpoint().as_str().to_owned());
    let provider =
        SessionMcpAuthProvider::for_account(Arc::clone(&sessions), ACCOUNT_ID.to_owned());
    assert!(provider.current_auth(&descriptor).await.is_ok());

    sessions.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-2", "tenant-2", "Other user", None),
            120_000,
        ),
        "account-b-bearer",
        None,
    );
    sessions.set_authorized_resource(descriptor.endpoint().as_str().to_owned());
    assert!(matches!(
        provider.current_auth(&descriptor).await,
        Err(McpAuthError::MissingBearer)
    ));
}

#[tokio::test]
async fn a_signed_out_caller_is_refused_without_touching_the_registry() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({}))]).await;
    let harness = Harness::signed_out(&server.uri());

    let error = harness
        .runtime
        .list()
        .await
        .expect_err("a signed-out list must fail");

    assert_eq!(error.code, McpServerErrorCode::Unauthorized);
    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "no registry request may be made on behalf of nobody"
    );
}

// ---------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------

#[tokio::test]
async fn registering_a_server_seals_its_credential_under_the_id_da_assigned() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(row(json!({"name": "Acme", "auth_type": "bearer"}))),
        )
        .mount(&server)
        .await;
    mount_list(
        &server,
        vec![row(json!({"name": "Acme", "auth_type": "bearer"}))],
    )
    .await;
    let harness = Harness::signed_in(&server.uri());

    let view = harness
        .runtime
        .create(
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Bearer),
            Some("acme-token".to_owned()),
            OauthClientInput::default(),
        )
        .await
        .expect("the registration succeeds");

    assert_eq!(view.slug, "acme");
    assert_eq!(
        view.unavailable, None,
        "the sealed credential makes it usable"
    );
    assert_eq!(
        harness.stored_credential(ROW_ID),
        Some("acme-token".to_owned()),
        "the credential is sealed under DA's row id, not under the name"
    );
}

#[tokio::test]
async fn a_credential_that_cannot_be_sealed_rolls_the_registration_back() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(row(json!({"name": "Acme", "auth_type": "bearer"}))),
        )
        .mount(&server)
        .await;
    Mock::given(method(Method::DELETE))
        .and(path_regex(format!("^{ROUTE}/.+$")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deleted": true})))
        .mount(&server)
        .await;
    let harness = Harness::signed_in_with_unwritable_store(&server.uri());

    let error = harness
        .runtime
        .create(
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Bearer),
            Some("acme-token".to_owned()),
            OauthClientInput::default(),
        )
        .await
        .expect_err("a registration that cannot be authenticated must fail");

    assert_eq!(error.code, McpServerErrorCode::StorageUnavailable);
    let requests = server.received_requests().await.expect("recorded requests");
    assert!(
        requests
            .iter()
            .any(|request| request.method == Method::DELETE),
        "the row must be deleted again rather than left half-registered: {:?}",
        requests
            .iter()
            .map(|request| &request.method)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_registration_this_build_would_never_dial_is_refused_before_any_da_write() {
    let server = MockServer::start().await;
    let harness = Harness::signed_in(&server.uri());
    let mut cases = vec![
        // Reserved name: it would shadow the bundled server's namespace.
        (
            input("tenant", "https://mcp.example.test/mcp", McpAuthType::None),
            None,
            McpServerErrorCode::InvalidRequest,
        ),
        // Not a URL shape this build accepts at all.
        (
            input(
                "Acme",
                "https://user:pass@mcp.example.test/",
                McpAuthType::None,
            ),
            None,
            McpServerErrorCode::InvalidRequest,
        ),
        // A method DA named that this build has no implementation of, and no
        // wire spelling to register one under (plan D4). `oauth` is deliberately
        // absent from this list: it is registrable, and its material is obtained
        // by `mcp_server_sign_in` rather than pasted here.
        (
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Other),
            None,
            McpServerErrorCode::InvalidRequest,
        ),
        // An oauth server registered *with* a pasted credential: there is
        // nowhere to put it, and accepting it would imply a secret was stored.
        (
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Oauth),
            Some("stray".to_owned()),
            McpServerErrorCode::InvalidRequest,
        ),
        // A header server with no header to put the value in.
        (
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Header),
            Some("key".to_owned()),
            McpServerErrorCode::InvalidRequest,
        ),
        // A server that needs a credential, registered without one.
        (
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Bearer),
            None,
            McpServerErrorCode::InvalidRequest,
        ),
        // A server that authenticates with nothing has nowhere to put one.
        (
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::None),
            Some("stray".to_owned()),
            McpServerErrorCode::InvalidRequest,
        ),
    ];
    // Well formed, and a host this build refuses to dial. Refused only while the
    // public-hosts rules are enforced: the shipped policy dials private hosts.
    if ENFORCE_PUBLIC_MCP_HOSTS {
        cases.push((
            input("Acme", "https://10.0.0.5/mcp", McpAuthType::None),
            None,
            McpServerErrorCode::UrlRejected,
        ));
    }

    for (server_input, credential, expected) in cases {
        let url = server_input.url.clone();
        let auth_type = server_input.auth_type;
        let error = harness
            .runtime
            .create(server_input, credential, OauthClientInput::default())
            .await
            .expect_err("the registration must be refused");
        assert_eq!(error.code, expected, "{auth_type:?} at {url}");
    }

    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "a refused registration must leave nothing behind in the registry"
    );
}

#[tokio::test]
async fn a_duplicate_name_is_reported_as_a_conflict() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": { "code": "conflict", "message": "safe wire error", "request_id": "r-1" }
        })))
        .mount(&server)
        .await;
    let harness = Harness::signed_in(&server.uri());

    let error = harness
        .runtime
        .create(
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("a duplicate name must fail");

    assert_eq!(error.code, McpServerErrorCode::Conflict);
}

#[tokio::test]
async fn registering_the_bundled_endpoint_is_refused_before_any_da_write() {
    let server = MockServer::start().await;
    let harness = Harness::signed_in(&server.uri());

    let error = harness
        .runtime
        .create(
            input("Alias", &format!("{}/mcp", server.uri()), McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("the bundled endpoint is already registered by configuration");

    assert_eq!(error.code, McpServerErrorCode::NotEditable);
    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "the duplicate must be rejected before the registry is changed"
    );
}

// ---------------------------------------------------------------------
// Editing and unregistering
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_bundled_server_cannot_be_edited() {
    let server = MockServer::start().await;
    let harness = Harness::signed_in(&server.uri());

    let updated = harness
        .runtime
        .update(
            BUNDLED_SERVER_ID.to_owned(),
            input("tenant", "https://mcp.example.test/mcp", McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("the bundled server is not editable");
    assert_eq!(updated.code, McpServerErrorCode::NotEditable);
    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "the bundled server has no DA row, so nothing may be sent about it"
    );
}

#[tokio::test]
async fn bundled_removal_and_restore_persist_across_runtime_reconstruction() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let initial = harness.runtime.list().await.expect("initial list");
    assert!(initial.servers.iter().any(|server| server.builtin));
    let active_run = harness
        .runtime
        .runtime_set(&AuthConfig {
            da_base_url: Url::parse(&server.uri()).unwrap(),
            casdoor_issuer: Url::parse("https://auth.example").unwrap(),
            client_id: "client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: 19847,
            mcp_client_id: None,
            mcp_client_secret: None,
        })
        .await;

    harness
        .runtime
        .delete(BUNDLED_SERVER_ID.to_owned())
        .await
        .expect("remove bundled server");
    assert!(
        active_run.find(BUNDLED_SERVER_ID).is_some(),
        "an already-started run keeps its runtime set"
    );

    let reconstructed = harness.reconstructed_runtime(&server.uri());
    let removed = reconstructed.list().await.expect("list after restart");
    assert_eq!(removed.servers.len(), 1);
    assert!(removed.servers.iter().all(|server| !server.builtin));
    let next_run = reconstructed
        .runtime_set(&AuthConfig {
            da_base_url: Url::parse(&server.uri()).unwrap(),
            casdoor_issuer: Url::parse("https://auth.example").unwrap(),
            client_id: "client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: 19847,
            mcp_client_id: None,
            mcp_client_secret: None,
        })
        .await;
    assert!(next_run.find(BUNDLED_SERVER_ID).is_none());
    assert!(next_run.find("crm").is_some());

    let tested = reconstructed
        .test(BUNDLED_SERVER_ID.to_owned(), None)
        .await
        .expect_err("an explicitly removed server must not be dialed");
    assert_eq!(tested.code, McpServerErrorCode::NotFound);
    let restored = reconstructed
        .restore_bundled()
        .await
        .expect("restore bundled server");
    assert!(restored.builtin);
    assert!(reconstructed
        .list()
        .await
        .unwrap()
        .servers
        .iter()
        .any(|server| server.builtin));

    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method == Method::GET));
}

#[tokio::test]
async fn bundled_removal_is_isolated_by_account() {
    let server = MockServer::start().await;
    mount_list(&server, Vec::new()).await;
    let local_data = tempdir().unwrap();
    let vault = Arc::new(CredentialVault::new(Arc::new(
        InMemorySecretStore::default(),
    )));
    let first = runtime_for_identity(local_data.path(), vault.clone(), &server.uri(), identity());
    let second = runtime_for_identity(
        local_data.path(),
        vault,
        &server.uri(),
        IdentityProjection::new("user-2", "tenant-1", "Other User", None),
    );

    first
        .delete(BUNDLED_SERVER_ID.to_owned())
        .await
        .expect("remove for first account");
    assert!(first.list().await.unwrap().servers.is_empty());
    assert!(second.list().await.unwrap().servers[0].builtin);
}

#[tokio::test]
async fn bundled_preference_storage_failures_are_typed() {
    let server = MockServer::start().await;
    let harness = Harness::signed_in_with_unwritable_store(&server.uri());

    let error = harness
        .runtime
        .delete(BUNDLED_SERVER_ID.to_owned())
        .await
        .expect_err("the preference cannot be persisted");
    assert_eq!(error.code, McpServerErrorCode::StorageUnavailable);
}

#[tokio::test]
async fn renaming_through_an_update_is_refused_rather_than_silently_dropped() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({"name": "crm"}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let error = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("renamed", "https://mcp.example.test/mcp", McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("DA's patch surface carries no name, so a rename must be refused");

    assert_eq!(error.code, McpServerErrorCode::InvalidRequest);
}

#[tokio::test]
async fn updating_a_server_that_no_longer_exists_reports_not_found() {
    let server = MockServer::start().await;
    mount_list(&server, Vec::new()).await;
    let harness = Harness::signed_in(&server.uri());

    let error = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", "https://mcp.example.test/mcp", McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("an absent row cannot be updated");

    assert_eq!(error.code, McpServerErrorCode::NotFound);
}

#[tokio::test]
async fn changing_an_oauth_url_invalidates_the_device_grant_even_if_new_sign_in_is_cancelled() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": "https://old.example.test/mcp"
        }))],
    )
    .await;
    Mock::given(method(Method::PATCH))
        .and(path(format!("{ROUTE}/{ROW_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))))
        .mount(&da)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(&remote)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": remote.uri(),
            "authorization_endpoint": format!("{}/authorize", remote.uri()),
            "token_endpoint": format!("{}/token", remote.uri()),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&remote)
        .await;
    let harness = Harness::signed_in(&da.uri());
    harness.seed_oauth_grant(ROW_ID, "https://old.example.test/mcp");

    let updated = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", &format!("{}/mcp", remote.uri()), McpAuthType::Oauth),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect("update URL");
    assert_eq!(
        updated.sign_in,
        Some(xfalcon_contracts::mcp::McpSignInStatus::NotSignedIn)
    );

    da.reset().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;

    let sign_in = harness
        .runtime
        .sign_in(
            ROW_ID.to_owned(),
            Some("replacement-client".to_owned()),
            None,
            &RefusingBrowser,
        )
        .await
        .expect_err("cancelled sign-in");
    assert_eq!(sign_in.code, McpServerErrorCode::SignInFailed);
    let grants = DeviceGrants::new(
        credential_db_path(harness.local_data.path(), ACCOUNT_ID),
        harness.vault.clone(),
        owner(),
    );
    assert!(grants.get(ROW_ID).expect("grant store readable").is_none());
}

#[tokio::test]
async fn updating_an_additional_server_to_the_bundled_endpoint_changes_nothing() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let error = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", &format!("{}/mcp", server.uri()), McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("an update cannot turn an additional server into a tenant alias");

    assert_eq!(error.code, McpServerErrorCode::NotEditable);
    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(
        requests.len(),
        1,
        "only the read needed to authorize the edit is allowed"
    );
    assert_eq!(requests[0].method, Method::GET);
}

#[tokio::test]
async fn an_existing_bundled_alias_is_not_mutable_from_desktop() {
    let server = MockServer::start().await;
    mount_list(
        &server,
        vec![row(json!({"url": format!("{}/mcp", server.uri())}))],
    )
    .await;
    let harness = Harness::signed_in(&server.uri());

    let updated = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", "https://other.example/mcp", McpAuthType::None),
            None,
            OauthClientInput::default(),
        )
        .await
        .expect_err("the tenant alias is workspace-managed on desktop");
    let deleted = harness
        .runtime
        .delete(ROW_ID.to_owned())
        .await
        .expect_err("the tenant alias is workspace-managed on desktop");
    let tested = harness
        .runtime
        .test(ROW_ID.to_owned(), None)
        .await
        .expect_err("the tenant alias is tested through the bundled identity");
    let signed_in = harness
        .runtime
        .sign_in(ROW_ID.to_owned(), None, None, &UnexpectedBrowser)
        .await
        .expect_err("the tenant alias must not start a second browser sign-in");
    let signed_out = harness
        .runtime
        .sign_out(ROW_ID.to_owned())
        .await
        .expect_err("the tenant alias must not revoke a workspace grant");

    assert_eq!(updated.code, McpServerErrorCode::NotEditable);
    assert_eq!(deleted.code, McpServerErrorCode::NotEditable);
    assert_eq!(tested.code, McpServerErrorCode::NotEditable);
    assert_eq!(signed_in.code, McpServerErrorCode::NotEditable);
    assert_eq!(signed_out.code, McpServerErrorCode::NotEditable);
    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.iter().all(|request| request.method == Method::GET)),
        "no mutation may reach DA for the hidden tenant alias"
    );
}

#[tokio::test]
async fn unregistering_a_server_also_forgets_its_device_credential() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({}))]).await;
    Mock::given(method(Method::DELETE))
        .and(path_regex(format!("^{ROUTE}/.+$")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deleted": true})))
        .mount(&server)
        .await;
    let harness = Harness::signed_in(&server.uri());
    McpCredentialStore::open(
        credential_db_path(harness.local_data.path(), ACCOUNT_ID),
        harness.vault.clone(),
    )
    .expect("the store opens")
    .put(
        &owner(),
        ROW_ID,
        zeroize::Zeroizing::new("acme-token".to_owned()),
    )
    .expect("seed a credential");
    assert!(harness.stored_credential(ROW_ID).is_some());

    harness
        .runtime
        .delete(ROW_ID.to_owned())
        .await
        .expect("the row is unregistered");

    assert_eq!(
        harness.stored_credential(ROW_ID),
        None,
        "a secret for a server that no longer exists is one nobody can ever remove"
    );
}

// ---------------------------------------------------------------------
// Connection checks
// ---------------------------------------------------------------------

#[tokio::test]
async fn testing_a_reachable_server_reports_its_tools_and_the_review_they_will_get() {
    let (mcp_url, _guard) = FixtureServer::new(0, FixtureState::default()).spawn().await;
    let server = MockServer::start().await;
    mount_list(
        &server,
        vec![row(json!({"url": mcp_url, "permission_mode": "allow"}))],
    )
    .await;
    let harness = Harness::signed_in(&server.uri());

    let report = harness
        .runtime
        .test(ROW_ID.to_owned(), None)
        .await
        .expect("the check runs");

    assert_eq!(report.state, McpConnectionState::Connected);
    assert!(report.message.is_none());
    let echo = report
        .tools
        .iter()
        .find(|tool| tool.name == "echo")
        .expect("the fixture publishes echo");
    assert_eq!(echo.tool_id, "mcp.crm.echo");
    assert!(
        echo.requires_approval,
        "the fixture annotates nothing, so even an `allow` server's tools pause"
    );
}

#[tokio::test]
async fn testing_a_server_this_build_will_not_dial_reports_that_it_never_tried() {
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({"transport": "stdio"}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let report = harness
        .runtime
        .test(ROW_ID.to_owned(), None)
        .await
        .expect("the check runs");

    assert_eq!(report.state, McpConnectionState::NotAttempted);
    assert!(report.tools.is_empty());
    assert_eq!(
        report.message.as_deref(),
        Some("This app connects MCP servers over Streamable HTTP only.")
    );
}

#[tokio::test]
async fn testing_a_server_that_answers_on_the_wrong_protocol_reports_a_refusal() {
    let (mcp_url, _guard) = spawn_raw_fixture(
        RawBehavior::WrongProtocolVersion("2025-06-18"),
        RawFixtureState::default(),
    )
    .await;
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({"url": mcp_url}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let report = harness
        .runtime
        .test(ROW_ID.to_owned(), None)
        .await
        .expect("the check runs");

    // Reached, and unusable on MCP terms — a different fix from "unreachable".
    assert_eq!(report.state, McpConnectionState::Rejected);
    assert!(report.message.is_some());
}

#[tokio::test]
async fn testing_a_dead_endpoint_reports_it_unreachable() {
    let dead = {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        drop(listener);
        format!("http://{address}/mcp")
    };
    let server = MockServer::start().await;
    mount_list(&server, vec![row(json!({"url": dead}))]).await;
    let harness = Harness::signed_in(&server.uri());

    let report = harness
        .runtime
        .test(ROW_ID.to_owned(), None)
        .await
        .expect("the check runs");

    assert_eq!(report.state, McpConnectionState::Unreachable);
    assert!(report.tools.is_empty());
}

#[tokio::test]
async fn testing_a_server_that_is_not_registered_reports_not_found() {
    let server = MockServer::start().await;
    mount_list(&server, Vec::new()).await;
    let harness = Harness::signed_in(&server.uri());

    let error = harness
        .runtime
        .test(ROW_ID.to_owned(), None)
        .await
        .expect_err("an absent row cannot be tested");

    assert_eq!(error.code, McpServerErrorCode::NotFound);
}

// ---------------------------------------------------------------------
// The run's view of the registry
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_disabled_server_is_listed_but_never_held_for_a_run() {
    let (mcp_url, _guard) = FixtureServer::new(0, FixtureState::default()).spawn().await;
    let server = MockServer::start().await;
    mount_list(
        &server,
        vec![row(json!({"url": mcp_url, "is_enabled": false}))],
    )
    .await;
    let harness = Harness::signed_in(&server.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&server.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let listed = harness.runtime.list().await.expect("the registry lists");
    let servers = harness.runtime.runtime_set(&config).await;

    assert!(!listed.servers[1].enabled);
    assert_eq!(listed.servers[1].unavailable, None);
    assert_eq!(
        servers.len(),
        1,
        "only the bundled server is held: a disabled row is off, not broken"
    );
    assert!(servers.find("crm").is_none());
}

#[tokio::test]
async fn an_unreadable_registry_still_leaves_a_run_the_tenant_workspace() {
    let server = MockServer::start().await;
    Mock::given(method(Method::GET))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let harness = Harness::signed_in(&server.uri());
    let config = AuthConfig {
        da_base_url: Url::parse(&server.uri()).expect("origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let servers = harness.runtime.runtime_set(&config).await;

    assert_eq!(
        servers.len(),
        1,
        "a registry this device cannot read must not take MCP away from the run"
    );
    assert!(servers.find(BUNDLED_SERVER_ID).is_some());
}

#[tokio::test]
async fn an_unreadable_registry_does_not_restore_a_removed_bundled_server() {
    let server = MockServer::start().await;
    Mock::given(method(Method::GET))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let harness = Harness::signed_in(&server.uri());
    harness
        .runtime
        .delete(BUNDLED_SERVER_ID.to_owned())
        .await
        .expect("remove bundled server");
    let config = AuthConfig {
        da_base_url: Url::parse(&server.uri()).unwrap(),
        casdoor_issuer: Url::parse("https://auth.example").unwrap(),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };

    let servers = harness.runtime.runtime_set(&config).await;

    assert_eq!(servers.len(), 0);
    assert!(servers.find(BUNDLED_SERVER_ID).is_none());
}

#[tokio::test]
async fn registry_excludes_missing_grants_and_explicit_checks_offer_sign_in() {
    use crate::mcp::dispatch::ReauthNotifier;
    use xfalcon_contracts::mcp::McpReauthRequired;
    #[derive(Default)]
    struct Prompts(std::sync::Mutex<Vec<McpReauthRequired>>);
    impl ReauthNotifier for Prompts {
        fn reauth_required(&self, prompt: McpReauthRequired) {
            self.0.lock().unwrap().push(prompt);
        }
    }
    for (enabled, permission) in [(true, "ask"), (false, "ask"), (true, "deny")] {
        let server = MockServer::start().await;
        mount_list(
            &server,
            vec![row(
                json!({"auth_type": "oauth", "is_enabled": enabled, "permission_mode": permission}),
            )],
        )
        .await;
        let harness = Harness::signed_in(&server.uri());
        let config = AuthConfig {
            da_base_url: Url::parse(&server.uri()).unwrap(),
            casdoor_issuer: Url::parse("https://auth.example").unwrap(),
            client_id: "client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: 19847,
            mcp_client_id: None,
            mcp_client_secret: None,
        };
        let servers = harness.runtime.runtime_set(&config).await;
        assert_eq!(servers.len(), 1);
        let prompts = Prompts::default();
        if enabled && permission == "ask" {
            let report = harness
                .runtime
                .test(ROW_ID.to_owned(), Some(&prompts))
                .await
                .unwrap();
            assert_eq!(report.state, McpConnectionState::NotAttempted);
            let raised = prompts.0.lock().unwrap();
            assert_eq!(raised.len(), 1);
            assert_eq!(raised[0].id.as_str(), ROW_ID);
        }
    }
}

// ---------------------------------------------------------------------
// The operator-issued OAuth client, stored per device
// ---------------------------------------------------------------------

/// How a token request identified its client.
///
/// A confidential client authenticates with HTTP Basic (RFC 6749 §2.3.1) and a
/// public one puts its id in the body, so a test that only read one of the two
/// would pass for the wrong reason.
fn token_client_credentials(request: &wiremock::Request) -> String {
    let body = String::from_utf8_lossy(&request.body).into_owned();
    match request.headers.get("authorization") {
        None => body,
        Some(header) => {
            let encoded = header
                .to_str()
                .expect("an ASCII authorization header")
                .strip_prefix("Basic ")
                .expect("basic client authentication");
            let decoded = STANDARD.decode(encoded).expect("base64 credentials");
            format!("{body}\n{}", String::from_utf8_lossy(&decoded))
        }
    }
}

/// Mounts the two documents a sign-in discovers on the server itself: RFC 9728
/// Protected Resource Metadata naming the resource and its issuer, and that
/// issuer's own metadata.
///
/// Deliberately publishes no `registration_endpoint`, which is the Casdoor
/// posture every tenant here has: dynamic registration is refused, so the
/// client has to come from somewhere else.
async fn mount_oauth_discovery(remote: &MockServer) {
    Mock::given(method(Method::GET))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(remote)
        .await;
    Mock::given(method(Method::GET))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": remote.uri(),
            "authorization_endpoint": format!("{}/authorize", remote.uri()),
            "token_endpoint": format!("{}/token", remote.uri()),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(remote)
        .await;
}

/// Mounts a create that answers with an `oauth` row, plus the list the create
/// re-reads afterwards to learn its slug.
async fn mount_oauth_create(da: &MockServer, url: &str) {
    let row = row(json!({"name": "Acme", "auth_type": "oauth", "url": url}));
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(201).set_body_json(row.clone()))
        .mount(da)
        .await;
    mount_list(da, vec![row]).await;
}

/// Mounts a patch that answers with an `oauth` row at `url`.
async fn mount_oauth_patch(da: &MockServer, url: &str) {
    Mock::given(method(Method::PATCH))
        .and(path(format!("{ROUTE}/{ROW_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(row(json!({"auth_type": "oauth", "url": url}))),
        )
        .mount(da)
        .await;
}

#[tokio::test]
async fn registering_an_oauth_server_seals_its_client_and_reports_it_back() {
    let da = MockServer::start().await;
    mount_oauth_create(&da, "https://mcp.example.test/mcp").await;
    let harness = Harness::signed_in(&da.uri());

    let view = harness
        .runtime
        .create(
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Oauth),
            None,
            oauth_client(Some("  issued-client  "), Some("  issued-secret  ")),
        )
        .await
        .expect("the registration succeeds");

    // Sealed by the create, not by a later sign-in. This is the defect: a
    // browser flow that is declined used to take the pair with it.
    assert_eq!(
        harness.stored_oauth_client(ROW_ID),
        Some(("issued-client".to_owned(), Some("issued-secret".to_owned()))),
        "both halves are trimmed and sealed under DA's row id"
    );
    // ...and reported back, so the edit sheet can show it instead of a blank
    // field over a saved value.
    assert_eq!(view.oauth_client_id.as_deref(), Some("issued-client"));
    assert!(view.oauth_client_confidential);

    // The pair must not be in anything sent to the shared registry.
    let bodies = da
        .received_requests()
        .await
        .expect("recorded requests")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!bodies.contains("issued-secret"), "{bodies}");
    assert!(!bodies.contains("issued-client"), "{bodies}");
}

#[tokio::test]
async fn a_client_that_cannot_be_sealed_rolls_the_registration_back() {
    let da = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(row(json!({"name": "Acme", "auth_type": "oauth"}))),
        )
        .mount(&da)
        .await;
    Mock::given(method(Method::DELETE))
        .and(path_regex(format!("^{ROUTE}/.+$")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deleted": true})))
        .mount(&da)
        .await;
    let harness = Harness::signed_in_with_unwritable_store(&da.uri());

    let error = harness
        .runtime
        .create(
            input("Acme", "https://mcp.example.test/mcp", McpAuthType::Oauth),
            None,
            oauth_client(Some("issued-client"), Some("issued-secret")),
        )
        .await
        .expect_err("a client that cannot be sealed must not leave a registration behind");

    assert_eq!(error.code, McpServerErrorCode::StorageUnavailable);
    assert!(
        da.received_requests()
            .await
            .expect("recorded requests")
            .iter()
            .any(|request| request.method == Method::DELETE),
        "the row must be deleted again rather than left with a client nobody can read"
    );
}

#[tokio::test]
async fn a_client_is_refused_for_a_server_that_never_signs_in() {
    let da = MockServer::start().await;
    mount_list(&da, Vec::new()).await;
    let harness = Harness::signed_in(&da.uri());

    for auth_type in [McpAuthType::None, McpAuthType::Bearer] {
        let credential = auth_type
            .needs_device_credential()
            .then(|| "token".to_owned());
        let error = harness
            .runtime
            .create(
                input("Acme", "https://mcp.example.test/mcp", auth_type),
                credential,
                oauth_client(Some("issued-client"), None),
            )
            .await
            .expect_err("a method with nowhere to keep a client must refuse one");
        assert_eq!(
            error.code,
            McpServerErrorCode::InvalidRequest,
            "{auth_type:?}"
        );
    }

    assert!(
        da.received_requests()
            .await
            .is_some_and(|requests| requests.iter().all(|r| r.method == Method::GET)),
        "the refusal happens before any registry write"
    );
}

#[tokio::test]
async fn an_edit_keeps_a_sealed_client_value_it_was_not_asked_to_change() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    mount_oauth_patch(&da, "https://mcp.example.test/mcp").await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    // The sheet submits the id it was seeded with and a blank secret, which is
    // what an edit of some unrelated field looks like.
    let view = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", "https://mcp.example.test/mcp", McpAuthType::Oauth),
            None,
            oauth_client(Some("issued-client"), None),
        )
        .await
        .expect("the update succeeds");

    assert_eq!(
        harness.stored_oauth_client(ROW_ID),
        Some(("issued-client".to_owned(), Some("issued-secret".to_owned()))),
        "a blank secret beside an unchanged id keeps what is sealed"
    );
    assert!(view.oauth_client_confidential);
}

#[tokio::test]
async fn a_reissued_client_id_does_not_inherit_the_previous_sealed_value() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    mount_oauth_patch(&da, "https://mcp.example.test/mcp").await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    let view = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", "https://mcp.example.test/mcp", McpAuthType::Oauth),
            None,
            oauth_client(Some("reissued-client"), None),
        )
        .await
        .expect("the update succeeds");

    // A value issued for one client does not authenticate another: sending it
    // anyway is answered `invalid_client`, which reads as a wrong secret.
    assert_eq!(
        harness.stored_oauth_client(ROW_ID),
        Some(("reissued-client".to_owned(), None)),
    );
    assert!(!view.oauth_client_confidential);
}

#[tokio::test]
async fn clearing_the_client_id_removes_the_stored_client_entirely() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    mount_oauth_patch(&da, "https://mcp.example.test/mcp").await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    let view = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", "https://mcp.example.test/mcp", McpAuthType::Oauth),
            None,
            oauth_client(None, None),
        )
        .await
        .expect("the update succeeds");

    assert_eq!(harness.stored_oauth_client(ROW_ID), None);
    assert_eq!(view.oauth_client_id, None);
    assert!(!view.oauth_client_confidential);
}

#[tokio::test]
async fn moving_a_server_off_browser_sign_in_forgets_the_client_it_no_longer_uses() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    Mock::given(method(Method::PATCH))
        .and(path(format!("{ROUTE}/{ROW_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(row(json!({
            "auth_type": "bearer",
            "url": "https://mcp.example.test/mcp"
        }))))
        .mount(&da)
        .await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    let view = harness
        .runtime
        .update(
            ROW_ID.to_owned(),
            input("crm", "https://mcp.example.test/mcp", McpAuthType::Bearer),
            Some("acme-token".to_owned()),
            oauth_client(None, None),
        )
        .await
        .expect("the update succeeds");

    // Dead weight otherwise: a client nobody can use and nothing would remove.
    assert_eq!(harness.stored_oauth_client(ROW_ID), None);
    assert_eq!(view.oauth_client_id, None);
}

#[tokio::test]
async fn unregistering_a_server_forgets_the_client_a_sign_out_would_have_kept() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    Mock::given(method(Method::DELETE))
        .and(path(format!("{ROUTE}/{ROW_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deleted": true})))
        .mount(&da)
        .await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    harness
        .runtime
        .delete(ROW_ID.to_owned())
        .await
        .expect("the server unregisters");

    assert_eq!(
        harness.stored_oauth_client(ROW_ID),
        None,
        "removing the server is the one moment the client must go too"
    );
}

#[tokio::test]
async fn a_declined_sign_in_keeps_the_client_it_was_given_for_the_next_attempt() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    mount_oauth_discovery(&remote).await;
    let harness = Harness::signed_in(&da.uri());

    // A browser that never opens is the cheapest way to reach every exit this
    // flow has that writes no grant.
    let error = harness
        .runtime
        .sign_in(
            ROW_ID.to_owned(),
            Some("issued-client".to_owned()),
            Some("issued-secret".to_owned()),
            &RefusingBrowser,
        )
        .await
        .expect_err("a browser that will not open produces no grant");
    assert_eq!(error.code, McpServerErrorCode::SignInFailed);

    // The regression: the pair used to live only inside a grant that was never
    // written, so the retry dialog asked for it again.
    assert_eq!(
        harness.stored_oauth_client(ROW_ID),
        Some(("issued-client".to_owned(), Some("issued-secret".to_owned()))),
    );
    let listed = harness.runtime.list().await.expect("the registry lists");
    let view = listed
        .servers
        .iter()
        .find(|server| server.id.as_str() == ROW_ID)
        .expect("the row is listed");
    assert_eq!(view.oauth_client_id.as_deref(), Some("issued-client"));
    assert!(view.oauth_client_confidential);
}

#[tokio::test]
async fn a_retry_reuses_the_stored_client_without_being_handed_it_again() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    mount_oauth_discovery(&remote).await;
    Mock::given(method(Method::POST))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "issued-access-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&remote)
        .await;
    let harness = Harness::signed_in(&da.uri());
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    // Nothing passed in: this is the "Sign in" button on the row, after the
    // client was supplied on an earlier attempt.
    let view = harness
        .runtime
        .sign_in(ROW_ID.to_owned(), None, None, &RedirectingBrowser)
        .await
        .expect("the stored client completes the sign-in");
    assert!(view.sign_in.is_some_and(|status| status.is_usable()));

    // The authorization server was asked to register nothing, and the token
    // request authenticated as the stored client.
    let requests = remote.received_requests().await.expect("recorded requests");
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() != "/register"),
        "a stored client must not trigger dynamic registration"
    );
    let token_credentials = requests
        .iter()
        .find(|request| request.url.path() == "/token")
        .map(token_client_credentials)
        .expect("a token request");
    assert!(
        token_credentials.contains("issued-client:issued-secret"),
        "the stored pair authenticates the exchange whole: {token_credentials}"
    );
}

#[tokio::test]
async fn signing_out_keeps_the_client_so_signing_back_in_needs_no_paste() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    let harness = Harness::signed_in(&da.uri());
    harness.seed_oauth_grant(ROW_ID, "https://mcp.example.test/mcp");
    harness
        .oauth_clients()
        .put(
            ROW_ID,
            "issued-client",
            Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
        )
        .expect("seed the stored client");

    let view = harness
        .runtime
        .sign_out(ROW_ID.to_owned())
        .await
        .expect("the sign-out succeeds");

    assert_eq!(
        view.sign_in,
        Some(xfalcon_contracts::mcp::McpSignInStatus::NotSignedIn),
        "the grant is gone"
    );
    assert_eq!(
        harness.stored_oauth_client(ROW_ID),
        Some(("issued-client".to_owned(), Some("issued-secret".to_owned()))),
        "the client says which application this device is, not that it is signed in"
    );
    assert_eq!(view.oauth_client_id.as_deref(), Some("issued-client"));
}

#[tokio::test]
async fn a_stored_client_is_preferred_over_the_packaged_one() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    mount_oauth_discovery(&remote).await;
    Mock::given(method(Method::POST))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "issued-access-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&remote)
        .await;
    let local_data = tempdir().expect("tempdir");
    let vault = Arc::new(CredentialVault::new(Arc::new(
        InMemorySecretStore::default(),
    )));
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(identity(), 120_000),
        "registry-bearer",
        None,
    );
    // A build that packages a client, against a server whose issuer is its own
    // rather than the tenant's.
    let runtime = McpServerRuntime::for_test(
        sessions,
        local_data.path().to_path_buf(),
        vault.clone(),
        AuthConfig {
            da_base_url: Url::parse(&da.uri()).expect("da origin"),
            casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
            client_id: "client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: free_login_port(),
            mcp_client_id: Some("packaged-client".to_owned()),
            mcp_client_secret: Some("packaged-secret".to_owned()),
        },
    );
    DeviceOauthClients::new(
        credential_db_path(local_data.path(), ACCOUNT_ID),
        vault,
        owner(),
    )
    .put(
        ROW_ID,
        "issued-client",
        Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
    )
    .expect("seed the stored client");

    runtime
        .sign_in(ROW_ID.to_owned(), None, None, &RedirectingBrowser)
        .await
        .expect("the stored client completes the sign-in");

    let token_credentials = remote
        .received_requests()
        .await
        .expect("recorded requests")
        .iter()
        .find(|request| request.url.path() == "/token")
        .map(token_client_credentials)
        .expect("a token request");
    assert!(
        token_credentials.contains("issued-client:issued-secret"),
        "{token_credentials}"
    );
    assert!(
        !token_credentials.contains("packaged-client"),
        "a client this device was told to use wins over the packaged fallback: {token_credentials}"
    );
    assert!(
        !token_credentials.contains("packaged-secret"),
        "and its halves never interleave: {token_credentials}"
    );
}

// ---------------------------------------------------------------------
// Finding a server's OAuth description (RFC 9728, all three routes)
// ---------------------------------------------------------------------

/// Mounts issuer metadata for `remote`, but no Protected Resource Metadata at
/// either well-known URL — the shape a server takes when it publishes that
/// document somewhere else and says so in its challenge.
async fn mount_issuer_metadata_only(remote: &MockServer) {
    Mock::given(method(Method::GET))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": remote.uri(),
            "authorization_endpoint": format!("{}/authorize", remote.uri()),
            "token_endpoint": format!("{}/token", remote.uri()),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(remote)
        .await;
}

#[tokio::test]
async fn a_server_that_points_at_its_metadata_from_a_challenge_can_be_signed_in_to() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    mount_issuer_metadata_only(&remote).await;
    // RFC 9728 §5.1: the only route a server has to a client when its document
    // is not at a well-known path. Answered on the MCP endpoint itself.
    Mock::given(path("/mcp"))
        .respond_with(
            ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                format!(
                    r#"Bearer resource_metadata="{}/auth/resource.json""#,
                    remote.uri()
                )
                .as_str(),
            ),
        )
        .mount(&remote)
        .await;
    Mock::given(method(Method::GET))
        .and(path("/auth/resource.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(&remote)
        .await;
    Mock::given(method(Method::POST))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "issued-access-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&remote)
        .await;
    let harness = Harness::signed_in(&da.uri());
    // Seeded so that client resolution is not what this test is about: the
    // issuer here publishes no registration endpoint.
    harness
        .oauth_clients()
        .put(ROW_ID, "issued-client", None)
        .expect("seed the stored client");

    let view = harness
        .runtime
        .sign_in(ROW_ID.to_owned(), None, None, &RedirectingBrowser)
        .await
        .expect("the challenge names where the metadata lives");

    assert!(view.sign_in.is_some_and(|status| status.is_usable()));
    assert!(
        remote
            .received_requests()
            .await
            .expect("recorded requests")
            .iter()
            .any(|request| request.url.path() == "/auth/resource.json"),
        "the document named by the challenge must actually be read"
    );
}

#[tokio::test]
async fn a_challenge_pointing_off_origin_is_not_followed() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    mount_issuer_metadata_only(&remote).await;
    // A 401 is the least authenticated moment in the exchange. A pointer to
    // another host there would let a hostile or compromised server aim
    // discovery -- and the authorization request after it -- wherever it liked.
    Mock::given(path("/mcp"))
        .respond_with(
            ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                format!(
                    r#"Bearer resource_metadata="{}/resource.json""#,
                    elsewhere.uri()
                )
                .as_str(),
            ),
        )
        .mount(&remote)
        .await;
    Mock::given(path("/resource.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(&elsewhere)
        .await;
    let harness = Harness::signed_in(&da.uri());

    let error = harness
        .runtime
        .sign_in(ROW_ID.to_owned(), None, None, &UnexpectedBrowser)
        .await
        .expect_err("an off-origin metadata pointer is not a discovery route");

    assert_eq!(error.code, McpServerErrorCode::DiscoveryFailed);
    assert!(
        elsewhere
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty()),
        "the other host must never be contacted at all"
    );
}

#[tokio::test]
async fn a_well_known_document_that_names_another_resource_is_not_retried_via_the_challenge() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri())
        }))],
    )
    .await;
    mount_issuer_metadata_only(&remote).await;
    // Found, and refused: it names a resource on another origin, so a token
    // minted for it would be minted for somewhere else.
    Mock::given(method(Method::GET))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": "https://elsewhere.example.test/mcp",
            "authorization_servers": [remote.uri()],
        })))
        .mount(&remote)
        .await;
    Mock::given(path("/mcp"))
        .respond_with(
            ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                format!(
                    r#"Bearer resource_metadata="{}/auth/resource.json""#,
                    remote.uri()
                )
                .as_str(),
            ),
        )
        .mount(&remote)
        .await;
    Mock::given(method(Method::GET))
        .and(path("/auth/resource.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", remote.uri()),
            "authorization_servers": [remote.uri()],
        })))
        .mount(&remote)
        .await;
    let harness = Harness::signed_in(&da.uri());

    let error = harness
        .runtime
        .sign_in(ROW_ID.to_owned(), None, None, &UnexpectedBrowser)
        .await
        .expect_err("a refused document is a definitive answer");

    assert_eq!(error.code, McpServerErrorCode::DiscoveryFailed);
    assert!(
        remote
            .received_requests()
            .await
            .expect("recorded requests")
            .iter()
            .all(|request| request.url.path() != "/auth/resource.json"),
        "retrying through a channel the server also controls would let a bad \
         document be re-asked until one passed"
    );
}

#[tokio::test]
async fn a_client_saved_on_this_device_keeps_a_shared_row_on_the_device_path() {
    let da = MockServer::start().await;
    let remote = MockServer::start().await;
    // A row the web app registered its own OAuth client into, in a deployment
    // that also configures the shared key -- the account-wide sign-in path.
    mount_list(
        &da,
        vec![row(json!({
            "auth_type": "oauth",
            "url": format!("{}/mcp", remote.uri()),
            "auth_config": {
                "client_id": "shared-client",
                "client_secret": "shared-secret"
            }
        }))],
    )
    .await;
    mount_oauth_discovery(&remote).await;
    Mock::given(method(Method::POST))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "issued-access-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&remote)
        .await;
    let local_data = tempdir().expect("tempdir");
    let vault = Arc::new(CredentialVault::new(Arc::new(
        InMemorySecretStore::default(),
    )));
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        SessionProjection::authenticated(identity(), 120_000),
        "registry-bearer",
        None,
    );
    let runtime = McpServerRuntime::for_test(
        sessions,
        local_data.path().to_path_buf(),
        vault.clone(),
        AuthConfig {
            da_base_url: Url::parse(&da.uri()).expect("da origin"),
            casdoor_issuer: Url::parse(&remote.uri()).expect("issuer"),
            client_id: "login-client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: free_login_port(),
            mcp_client_id: None,
            mcp_client_secret: None,
        },
    )
    .with_test_grant_key(Arc::new(
        GrantKey::decode(&STANDARD.encode([0x5au8; 32])).expect("a shared key"),
    ));
    // Supplied on an earlier attempt and still held, with nothing passed in now.
    DeviceOauthClients::new(
        credential_db_path(local_data.path(), ACCOUNT_ID),
        vault,
        owner(),
    )
    .put(
        ROW_ID,
        "issued-client",
        Some(zeroize::Zeroizing::new("issued-secret".to_owned())),
    )
    .expect("seed the stored client");

    let view = runtime
        .sign_in(ROW_ID.to_owned(), None, None, &RedirectingBrowser)
        .await
        .expect("the stored client completes the sign-in");

    // Device-scoped, not account-wide: a client this device was told to use is
    // a deliberate statement about this device. It is visible in Settings and
    // clearing it returns the row to the shared path.
    assert_eq!(
        view.sign_in,
        Some(xfalcon_contracts::mcp::McpSignInStatus::SignedIn)
    );
    let token_credentials = remote
        .received_requests()
        .await
        .expect("recorded requests")
        .iter()
        .find(|request| request.url.path() == "/token")
        .map(token_client_credentials)
        .expect("a token request");
    assert!(
        token_credentials.contains("issued-client:issued-secret"),
        "{token_credentials}"
    );
    assert!(
        !token_credentials.contains("shared-client"),
        "the row's shared client must not be used while this device holds its own: {token_credentials}"
    );
    // Nothing was written to the account-wide grant store.
    assert!(
        da.received_requests()
            .await
            .expect("recorded requests")
            .iter()
            .all(|request| request.method == Method::GET),
        "a device-path sign-in writes no shared grant"
    );
}

/// Listing reads three device stores and a preference, and a failure in any one
/// of them empties the Settings panel identically — which is the macOS-versus-
/// Windows report this path is logged for. A store that exists and cannot be
/// opened must be reported, never rendered as "this account has no servers".
///
/// The database file is replaced by a DIRECTORY rather than by poisoning an
/// ancestor path, and that distinction is the whole reason this test reads the
/// way it does. Every `stored_ids` reader early-returns an empty set when
/// `path.exists()` is false, and a path whose ancestor is a regular file
/// reports "absent" on both platforms — so an ancestor poison never reaches a
/// store at all. It reached only `bundled_enabled`, whose `try_exists` is `Err`
/// on unix and `Ok(false)` on Windows, which made an earlier version of this
/// test pass on Linux and fail on Windows for reasons unrelated to what it
/// claimed to cover. A directory at the file's own path makes `exists()` true
/// and `Connection::open` fail on every platform.
#[tokio::test]
async fn a_store_that_cannot_be_opened_is_reported_rather_than_shown_as_an_empty_registry() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    let harness = Harness::signed_in(&da.uri());
    std::fs::create_dir_all(credential_db_path(harness.local_data.path(), ACCOUNT_ID))
        .expect("a directory where the store's database file belongs");

    let error = harness
        .runtime
        .list()
        .await
        .expect_err("a device store that cannot be opened is not an empty registry");

    assert_eq!(error.code, McpServerErrorCode::StorageUnavailable);
}

/// ...and the readable case still lists, with the client store simply empty.
/// An account that has never supplied a client has no rows in that table, which
/// must read as "none held" and not as a failure.
#[tokio::test]
async fn a_registry_with_no_stored_clients_lists_normally() {
    let da = MockServer::start().await;
    mount_list(
        &da,
        vec![row(
            json!({"auth_type": "oauth", "url": "https://mcp.example.test/mcp"}),
        )],
    )
    .await;
    let harness = Harness::signed_in(&da.uri());

    let listed = harness.runtime.list().await.expect("the registry lists");

    let view = listed
        .servers
        .iter()
        .find(|server| server.id.as_str() == ROW_ID)
        .expect("the row is listed");
    assert_eq!(view.oauth_client_id, None);
    assert!(!view.oauth_client_confidential);
    assert_eq!(harness.stored_oauth_client(ROW_ID), None);
}
