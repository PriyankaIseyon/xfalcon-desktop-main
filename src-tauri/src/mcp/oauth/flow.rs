//! Signing in to one registered MCP server, and renewing that sign-in.
//!
//! The order of steps is the security argument, so it is worth stating plainly:
//!
//! 1. **Discover** the server's own Protected Resource Metadata — by operator
//!    override, well-known URL, or the server's own `WWW-Authenticate` pointer
//!    (`super::discovery`) — and refuse it if it names a resource other than the
//!    server being dialed.
//! 2. **Pick an authorization server** only from the list that metadata names.
//! 3. **Obtain a client identity** — a stored one, or a dynamic registration, or
//!    the one an operator issued. Never a client id from an untrusted document.
//! 4. **Listen on loopback first**, so the redirect URI in the authorization
//!    request is one this process is already serving.
//! 5. **Open the system browser**, never an in-app webview: the user must be able
//!    to see the authorization server's address bar and its certificate, and this
//!    app must not be in a position to read what they type
//!    (RFC 8252 §8.12).
//! 6. **Exchange** the code with PKCE and the `resource` indicator, then seal the
//!    grant to this device.
//!
//! # The loopback port is fixed, not ephemeral
//!
//! RFC 8252 §7.3 says an authorization server must accept any port on a loopback
//! redirect, and many do not — they compare the redirect URI literally against
//! what was registered. A port chosen per attempt would then work on the first
//! sign-in (which registers it) and fail on every subsequent one. So the port is
//! fixed, derived from the same configuration as the login flow's, and a port
//! already in use is reported rather than silently swapped.

use std::time::Duration;

use reqwest::{Client, Url};
use zeroize::Zeroizing;

use crate::auth::loopback::{LoopbackError, LoopbackWaiter};
use crate::auth::pkce::{generate_oauth_state, generate_pkce};

use super::discovery::{self, DiscoveryError};
use super::envelope::GrantKey;
use super::exchange::{self, ExchangeError, TokenRequest, TokenSet};
use super::registration::{self, OauthClient, RegistrationError};
use super::store::{DeviceGrants, GrantStoreError, StoredGrant};
use crate::clients::da::mcp_oauth_grants::{DaMcpGrantRepository, GrantUpsert};

/// How long the flow waits for the browser round-trip.
///
/// Generous because it spans a human: a first sign-in to a corporate identity
/// provider can involve a password, a second factor, and a consent screen.
const CALLBACK_DEADLINE: Duration = Duration::from_secs(300);

/// The loopback port an MCP sign-in listens on, given the port the app's own
/// login uses (`crate::auth::AuthConfig::loopback_port`).
///
/// Adjacent to it, and never the same one: an MCP sign-in and an app sign-in can
/// legitimately be in flight together, and sharing a port would make whichever
/// started second fail to bind. Derived rather than separately configured so
/// there is one value to register with an authorization server and no way for the
/// two to be set to the same number by accident.
#[must_use]
pub(crate) const fn sign_in_loopback_port(login_port: u16) -> u16 {
    match login_port.checked_add(1) {
        Some(port) => port,
        // The login port is the last one that exists; step down instead, so the
        // two flows still never share a listener.
        None => login_port - 1,
    }
}

/// Opens a URL in the user's browser.
///
/// A trait rather than a direct `tauri_plugin_opener` call so the flow is
/// testable without a windowing system, and so the one place that hands a URL to
/// the operating system is visible in this module's signature.
pub(crate) trait BrowserOpener: Send + Sync {
    fn open(&self, url: &Url) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SignInError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// No client identity could be obtained, and none was supplied. The
    /// recoverable case: the surface asks for the operator-issued client id.
    #[error("this server's authorization server does not register clients dynamically")]
    ClientRegistrationUnavailable,
    #[error("a dynamic client registration was attempted and failed")]
    ClientRegistrationFailed,
    /// The loopback redirect could not be served, so there is nowhere for the
    /// authorization server to send the user back to.
    #[error("the loopback redirect could not be started")]
    RedirectUnavailable,
    #[error("the browser could not be opened")]
    BrowserUnavailable,
    #[error("sign-in was declined or abandoned")]
    Declined,
    #[error("sign-in did not complete in time")]
    TimedOut,
    #[error("the authorization server refused to issue a grant")]
    Refused,
    #[error("the grant could not be stored on this device")]
    Storage,
}

/// Where a completed sign-in is persisted.
///
/// The two are mutually exclusive on purpose. Writing both would create two
/// records of one grant that then refresh independently — and an authorization
/// server that rotates refresh tokens invalidates whichever copy renewed second,
/// so the "backup" copy is what breaks the working one.
pub(crate) enum GrantDestination<'a> {
    /// This device only. The default, and the only option when no shared
    /// key-encryption key is configured.
    Device(&'a DeviceGrants),
    /// DA's `mcp_oauth_grants`, in the format xfalcon-web reads — so the sign-in
    /// covers the account rather than the machine it was performed on.
    Shared {
        grants: &'a DaMcpGrantRepository,
        key: &'a GrantKey,
    },
}

/// What the caller supplies to start a sign-in.
pub(crate) struct SignIn<'a> {
    pub(crate) server_id: &'a str,
    /// The validated MCP endpoint from `crate::mcp::registry`.
    pub(crate) endpoint: &'a Url,
    /// An operator-issued client id, when the user has supplied one. Takes
    /// precedence over a stored client and over dynamic registration: it is the
    /// most explicit statement of intent available.
    pub(crate) client_id: Option<String>,
    pub(crate) client_secret: Option<Zeroizing<String>>,
    /// Configured fallback credentials belong only to this issuer. A mismatch
    /// skips that pair and continues stored-client or dynamic registration.
    /// Explicitly supplied and per-server shared clients have no fallback.
    pub(crate) client_issuer: Option<&'a Url>,
    /// An operator-configured Protected Resource Metadata URL, from the shared
    /// server row's `auth_config` — the same `resource_metadata_url` field
    /// xfalcon-web honours. `None` leaves discovery to find the document
    /// itself, which is the case for every server nobody has had to override.
    pub(crate) resource_metadata_url: Option<Url>,
    pub(crate) loopback_port: u16,
    pub(crate) destination: GrantDestination<'a>,
}

/// Runs the whole flow and stores the grant.
pub(crate) async fn sign_in(
    http: &Client,
    browser: &dyn BrowserOpener,
    request: SignIn<'_>,
) -> Result<(), SignInError> {
    let resource = discovery::resolve_protected_resource(
        http,
        request.endpoint,
        request.resource_metadata_url.clone(),
    )
    .await?;
    // The first named server, having been verified as named: `authorization_server`
    // re-checks membership, so a caller cannot substitute another issuer here.
    let issuer = resource
        .authorization_servers
        .first()
        .ok_or(DiscoveryError::NoUsableAuthorizationServer)?
        .clone();
    let server = discovery::authorization_server(http, &resource, &issuer).await?;
    let scope = discovery::negotiated_scope(&resource, &server);

    // Bound before the client identity is settled, because a dynamic
    // registration has to declare the exact redirect URI this listener serves.
    let (waiter, redirect_uri) = LoopbackWaiter::bind_on_port(request.loopback_port)
        .await
        .map_err(|_| SignInError::RedirectUnavailable)?;

    // Only the device store can offer a previously-issued client to reuse; the
    // shared path takes its client from the server row, which the caller has
    // already decrypted into `request.client_id`.
    let existing = match &request.destination {
        GrantDestination::Device(grants) => grants
            .get(request.server_id)
            .map_err(|_| SignInError::Storage)?,
        GrantDestination::Shared { .. } => None,
    };
    let client = resolve_client(
        http,
        &request,
        &server,
        existing.as_ref(),
        &redirect_uri.value,
        scope.as_deref(),
    )
    .await?;

    let pkce = generate_pkce();
    let state = generate_oauth_state();
    let authorize = exchange::authorize_url(
        &server.authorization_endpoint,
        &client,
        &redirect_uri.value,
        &resource.resource,
        scope.as_deref(),
        &state.value,
        &pkce,
    );
    if !browser.open(&authorize) {
        // Nothing will arrive on the listener. Returning is enough to stop it:
        // dropping the waiter cancels, which is now how EVERY exit from this
        // function releases the port rather than only this one.
        return Err(SignInError::BrowserUnavailable);
    }

    let callback = waiter
        .wait_for_callback(&state.value, CALLBACK_DEADLINE)
        .await
        .map_err(|error| match error {
            LoopbackError::TimedOut => SignInError::TimedOut,
            // A user who closes the browser, an authorization server that
            // redirects with `error=access_denied`, and a callback whose `state`
            // does not match all arrive here. All three mean the same thing to
            // the caller: this attempt produced no grant.
            LoopbackError::Cancelled | LoopbackError::InvalidCallback => SignInError::Declined,
            LoopbackError::BindFailed | LoopbackError::Io(_) => SignInError::RedirectUnavailable,
        })?;

    let tokens = exchange::exchange_code(
        http,
        &TokenRequest {
            token_endpoint: &server.token_endpoint,
            client: &client,
            resource: &resource.resource,
            scope: scope.as_deref(),
        },
        &callback.code,
        &redirect_uri.value,
        &pkce,
    )
    .await
    .map_err(|error| match error {
        ExchangeError::Refused | ExchangeError::Invalid => SignInError::Refused,
        ExchangeError::RequestFailed => SignInError::Refused,
    })?;

    match &request.destination {
        GrantDestination::Device(grants) => {
            let grant = StoredGrant {
                client_id: client.client_id.clone(),
                client_secret: client
                    .client_secret
                    .as_ref()
                    .map(|secret| secret.to_string()),
                issuer: server.issuer.clone(),
                token_endpoint: server.token_endpoint.to_string(),
                revocation_endpoint: server.revocation_endpoint.as_ref().map(ToString::to_string),
                resource: resource.resource.clone(),
                scope: tokens.scope.clone().or(scope),
                access_token: tokens.access_token.to_string(),
                refresh_token: tokens.refresh_token.as_ref().map(|token| token.to_string()),
                expires_at: tokens.expires_at,
            };
            grants
                .put(request.server_id, &grant)
                .map_err(|_: GrantStoreError| SignInError::Storage)
        }
        GrantDestination::Shared { grants, key } => {
            upsert_shared(
                grants,
                key,
                request.server_id,
                &server,
                &resource,
                &tokens,
                scope,
            )
            .await
        }
    }
}

/// Writes the grant to DA in the format web reads.
///
/// The tokens are sealed here, before the request is built: DA stores opaque
/// ciphertext and never sees a usable token — the same division web maintains,
/// and the reason this write is safe to make from a client at all.
async fn upsert_shared(
    grants: &DaMcpGrantRepository,
    key: &GrantKey,
    server_id: &str,
    server: &discovery::AuthorizationServer,
    resource: &discovery::ProtectedResource,
    tokens: &TokenSet,
    scope: Option<String>,
) -> Result<(), SignInError> {
    let access_token_enc = key
        .seal(&tokens.access_token)
        .map_err(|_| SignInError::Storage)?;
    let refresh_token_enc = match tokens.refresh_token.as_deref() {
        Some(refresh) => Some(key.seal(refresh).map_err(|_| SignInError::Storage)?),
        None => None,
    };
    grants
        .upsert(
            server_id,
            &GrantUpsert {
                issuer: server.issuer.clone(),
                token_endpoint: server.token_endpoint.to_string(),
                resource: resource.resource.clone(),
                scope: tokens.scope.clone().or(scope),
                access_token_enc,
                refresh_token_enc,
                access_token_expires_at: super::shared_grant::to_iso8601(tokens.expires_at),
            },
        )
        .await
        .map(|_row| ())
        // A failed write must not report success: the user would be looking at a
        // server the whole account is supposedly signed in to, with no grant
        // behind it.
        .map_err(|_| SignInError::Storage)
}

/// This device's own client, the client a previous grant used, a configured
/// fallback, or a dynamically registered one, in that order.
///
/// The caller has already collapsed "pasted just now" and "sealed on an earlier
/// attempt" into one value (`crate::mcp::commands::device_path_client`), and
/// signals which kind it is with [`SignIn::client_issuer`]: absent means this
/// device's own client, which wins outright, and present means the packaged
/// fallback, which is offered only to the issuer it belongs to. So the same
/// field carries both and they are never confused for each other.
///
/// The previous grant's client comes second, because a per-server client that
/// completed a sign-in must remain selected across retry and restart — and it is
/// reused only when the issuer has not changed, since an authorization server
/// change means that client id was issued by a different party and would not be
/// recognised.
async fn resolve_client(
    http: &Client,
    request: &SignIn<'_>,
    server: &discovery::AuthorizationServer,
    existing: Option<&StoredGrant>,
    redirect_uri: &str,
    scope: Option<&str>,
) -> Result<OauthClient, SignInError> {
    let requested_client = request
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(client_id) = requested_client.filter(|_| request.client_issuer.is_none()) {
        return Ok(OauthClient {
            client_id: client_id.to_owned(),
            client_secret: request.client_secret.clone(),
        });
    }
    if let Some(grant) =
        existing.filter(|grant| super::issuers_match(&grant.issuer, &server.issuer))
    {
        return Ok(grant.client());
    }
    if let Some(client_id) = requested_client.filter(|_| {
        request
            .client_issuer
            .is_some_and(|issuer| super::issuers_match(issuer.as_str(), &server.issuer))
    }) {
        return Ok(OauthClient {
            client_id: client_id.to_owned(),
            client_secret: request.client_secret.clone(),
        });
    }
    let Some(endpoint) = server.registration_endpoint.as_ref() else {
        return Err(SignInError::ClientRegistrationUnavailable);
    };
    registration::register(http, endpoint, redirect_uri, scope)
        .await
        .map_err(|error| match error {
            RegistrationError::Unavailable => SignInError::ClientRegistrationUnavailable,
            RegistrationError::Invalid | RegistrationError::RequestFailed => {
                SignInError::ClientRegistrationFailed
            }
        })
}

/// Renews a stored grant, returning the grant to store in its place.
///
/// Separate from [`sign_in`] and deliberately narrower: it involves no browser,
/// no discovery, and no user. Everything it needs was stored when the grant was
/// established, which is what lets a reconnect renew a token without the
/// authorization server's metadata endpoint being reachable.
pub(crate) async fn renew(
    http: &Client,
    grant: &StoredGrant,
) -> Result<StoredGrant, ExchangeError> {
    let refresh_token = grant
        .refresh_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or(ExchangeError::Refused)?;
    let token_endpoint = grant
        .token_endpoint_url()
        .ok_or(ExchangeError::RequestFailed)?;
    let client = grant.client();
    let tokens = exchange::refresh(
        http,
        &TokenRequest {
            token_endpoint: &token_endpoint,
            client: &client,
            resource: &grant.resource,
            scope: grant.scope.as_deref(),
        },
        refresh_token,
    )
    .await?;
    Ok(grant.renewed_with(&tokens))
}

/// Forgets a grant, revoking it upstream first when the authorization server
/// published a revocation endpoint.
///
/// The local deletion happens regardless of what the remote endpoint says: the
/// user asked for the material to be gone from this device, and a refusal or an
/// outage upstream must not leave it there. The returned flag is whether the
/// upstream revocation was acknowledged, which is what lets the caller be honest
/// about a grant that may still be live elsewhere.
pub(crate) async fn sign_out_shared(
    grants: &DaMcpGrantRepository,
    server_id: &str,
) -> Result<(), ()> {
    // No upstream revocation here: the shared grant's client secret lives in the
    // server row, and RFC 7009 revocation of an account-wide grant is web's
    // `evictGrantByServerViaApi` behaviour too — it deletes the row and lets the
    // authorization server's own lifetime apply.
    grants.evict(server_id).await.map_err(|_| ())
}

pub(crate) async fn sign_out(
    http: &Client,
    grants: &DeviceGrants,
    server_id: &str,
) -> Result<bool, GrantStoreError> {
    let stored = grants.get(server_id)?;
    let mut revoked = false;
    if let Some(grant) = stored.as_ref() {
        if let Some(endpoint) = grant.revocation_endpoint_url() {
            let client = grant.client();
            // The refresh token is revoked in preference to the access token: RFC
            // 7009 §2.1 says revoking a refresh token invalidates the access
            // tokens issued from it, so one request retires the whole grant.
            let (token, is_refresh) = match grant.refresh_token.as_deref() {
                Some(refresh) if !refresh.trim().is_empty() => (refresh, true),
                _ => (grant.access_token.as_str(), false),
            };
            revoked = exchange::revoke(http, &endpoint, &client, token, is_refresh).await;
        }
    }
    grants.delete(server_id)?;
    Ok(revoked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::vault::CredentialVault;
    use crate::mcp::credential_store::CredentialOwner;
    use crate::persistence::InMemorySecretStore;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Completes the browser leg by fetching the authorization URL's redirect
    /// target itself, which is what a real browser does after consent.
    struct RedirectingBrowser {
        code: String,
        seen: Arc<Mutex<Vec<Url>>>,
    }

    impl BrowserOpener for RedirectingBrowser {
        fn open(&self, url: &Url) -> bool {
            self.seen.lock().expect("lock").push(url.clone());
            let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
            let redirect = pairs
                .get("redirect_uri")
                .expect("a redirect uri")
                .to_string();
            let state = pairs.get("state").expect("a state").to_string();
            let code = self.code.clone();
            tokio::spawn(async move {
                let _ = reqwest::Client::new()
                    .get(format!("{redirect}?code={code}&state={state}"))
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

    fn store(directory: &std::path::Path) -> DeviceGrants {
        DeviceGrants::new(
            directory
                .join("mcp-data")
                .join("server-credentials.sqlite3"),
            Arc::new(CredentialVault::new(Arc::new(
                InMemorySecretStore::default(),
            ))),
            CredentialOwner {
                account_id: "tenant-1__user-1".to_owned(),
                tenant_id: "tenant-1".to_owned(),
                user_id: "user-1".to_owned(),
            },
        )
    }

    /// A resource server and an authorization server on one mock host, wired the
    /// way DA and its Casdoor are.
    async fn stack(register: bool) -> MockServer {
        stack_at_issuer_path(register, "").await
    }

    async fn stack_at_issuer_path(register: bool, issuer_path: &str) -> MockServer {
        let server = MockServer::start().await;
        let base = server.uri();
        let issuer = format!("{base}{issuer_path}");

        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-protected-resource/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": format!("{base}/mcp"),
                "authorization_servers": [issuer.clone()],
                "scopes_supported": ["openid", "mcp:admin"],
            })))
            .mount(&server)
            .await;

        let mut metadata = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{base}/login/oauth/authorize"),
            "token_endpoint": format!("{base}/api/login/oauth/access_token"),
            "revocation_endpoint": format!("{base}/api/login/oauth/revoke"),
            "scopes_supported": ["openid", "offline_access"],
            "code_challenge_methods_supported": ["S256"],
        });
        if register {
            metadata["registration_endpoint"] = json!(format!("{base}/api/oauth/register"));
        }
        Mock::given(method("GET"))
            .and(path(format!(
                "/.well-known/oauth-authorization-server{issuer_path}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/oauth/register"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "client_id": "issued-client" })),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/login/oauth/access_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "granted-access-token",
                "refresh_token": "granted-refresh-token",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": "openid offline_access",
            })))
            .mount(&server)
            .await;

        server
    }

    #[tokio::test]
    async fn a_server_that_registers_clients_needs_no_configuration_at_all() {
        let remote = stack(true).await;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).expect("url");
        let seen = Arc::new(Mutex::new(Vec::new()));

        sign_in(
            &http,
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: Arc::clone(&seen),
            },
            SignIn {
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: None,
                client_secret: None,
                client_issuer: None,
                resource_metadata_url: None,
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await
        .expect("the sign-in completes");

        let stored = grants
            .get("srv-1")
            .expect("readable")
            .expect("a stored grant");
        assert_eq!(stored.access_token, "granted-access-token");
        assert_eq!(stored.client_id, "issued-client");
        assert_eq!(stored.resource, format!("{}/mcp", remote.uri()));
        assert!(stored.is_renewable());

        // The scope actually requested is the intersection the two documents
        // share, plus offline_access — never the resource's `mcp:admin`.
        let authorize = seen.lock().expect("lock")[0].clone();
        let pairs: std::collections::HashMap<_, _> = authorize.query_pairs().collect();
        assert_eq!(pairs["scope"], "openid offline_access");
        assert_eq!(pairs["resource"], format!("{}/mcp", remote.uri()));
        assert_eq!(pairs["code_challenge_method"], "S256");
    }

    #[tokio::test]
    async fn a_server_that_refuses_registration_asks_for_an_operator_issued_client_id() {
        let remote = stack(false).await;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).expect("url");

        let outcome = sign_in(
            &http,
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: Arc::new(Mutex::new(Vec::new())),
            },
            SignIn {
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: None,
                client_secret: None,
                client_issuer: None,
                resource_metadata_url: None,
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await;

        assert!(matches!(
            outcome,
            Err(SignInError::ClientRegistrationUnavailable)
        ));
        assert!(
            grants.get("srv-1").expect("readable").is_none(),
            "a failed sign-in must store nothing"
        );
    }

    #[tokio::test]
    async fn an_operator_issued_client_id_signs_in_where_registration_is_disabled() {
        let remote = stack(false).await;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).expect("url");
        let seen = Arc::new(Mutex::new(Vec::new()));

        sign_in(
            &http,
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: Arc::clone(&seen),
            },
            SignIn {
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: Some("  connector-config-client  ".to_owned()),
                client_secret: Some(Zeroizing::new("connector-secret".to_owned())),
                client_issuer: None,
                resource_metadata_url: None,
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await
        .expect("the sign-in completes");

        let stored = grants
            .get("srv-1")
            .expect("readable")
            .expect("a stored grant");
        assert_eq!(
            stored.client_id, "connector-config-client",
            "a pasted client id is trimmed"
        );
        assert_eq!(stored.client_secret.as_deref(), Some("connector-secret"));

        let authorize = seen.lock().expect("lock")[0].clone();
        let pairs: std::collections::HashMap<_, _> = authorize.query_pairs().collect();
        assert_eq!(pairs["client_id"], "connector-config-client");
    }

    #[tokio::test]
    async fn a_configured_fallback_client_is_sent_only_to_its_own_issuer() {
        use base64::Engine as _;

        for (matching_issuer, registration) in [(true, false), (false, true), (false, false)] {
            let remote = stack(registration).await;
            let directory = tempfile::tempdir().expect("directory");
            let grants = store(directory.path());
            let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).unwrap();
            let issuer = if matching_issuer {
                Url::parse(&remote.uri()).unwrap()
            } else {
                Url::parse("https://tenant-auth.example").unwrap()
            };
            let seen = Arc::new(Mutex::new(Vec::new()));
            let outcome = sign_in(
                &super::super::build_client().unwrap(),
                &RedirectingBrowser {
                    code: "the-code".to_owned(),
                    seen: seen.clone(),
                },
                SignIn {
                    resource_metadata_url: None,
                    server_id: "srv-1",
                    endpoint: &endpoint,
                    client_id: Some("fallback-client".to_owned()),
                    client_secret: Some(Zeroizing::new("fallback-secret".to_owned())),
                    client_issuer: Some(&issuer),
                    loopback_port: 0,
                    destination: GrantDestination::Device(&grants),
                },
            )
            .await;

            let requests = remote.received_requests().await.unwrap();
            let token = requests
                .iter()
                .find(|request| request.url.path() == "/api/login/oauth/access_token");
            let fallback_auth = format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("fallback-client:fallback-secret")
            );
            if matching_issuer {
                outcome.expect("the tenant issuer accepts its configured client");
                assert_eq!(token.unwrap().headers["authorization"], fallback_auth);
                assert_eq!(
                    grants.get("srv-1").unwrap().unwrap().client_id,
                    "fallback-client"
                );
            } else {
                assert!(requests.iter().all(|request| {
                    !request
                        .body
                        .windows(b"fallback-client".len())
                        .any(|value| value == b"fallback-client")
                        && request.headers.get("authorization").is_none()
                }));
                assert!(seen.lock().unwrap().iter().all(|url| {
                    !url.query_pairs()
                        .any(|(key, value)| key == "client_id" && value == "fallback-client")
                }));
                if registration {
                    outcome.expect("the unrelated issuer registers its own client");
                    assert!(token.is_some());
                    assert_eq!(
                        grants.get("srv-1").unwrap().unwrap().client_id,
                        "issued-client"
                    );
                } else {
                    assert!(matches!(
                        outcome,
                        Err(SignInError::ClientRegistrationUnavailable)
                    ));
                    assert!(token.is_none());
                    assert!(seen.lock().unwrap().is_empty());
                    assert!(grants.get("srv-1").unwrap().is_none());
                }
            }
        }
    }

    #[tokio::test]
    async fn a_matching_fallback_preserves_the_issuers_stored_client() {
        use base64::Engine as _;

        let remote = stack(false).await;
        let directory = tempfile::tempdir().unwrap();
        let grants = store(directory.path());
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).unwrap();
        grants
            .put(
                "srv-1",
                &StoredGrant {
                    client_id: "stored-client".to_owned(),
                    client_secret: Some("stored-secret".to_owned()),
                    issuer: remote.uri(),
                    token_endpoint: format!("{}/api/login/oauth/access_token", remote.uri()),
                    revocation_endpoint: None,
                    resource: endpoint.to_string(),
                    scope: None,
                    access_token: "old-access-token".to_owned(),
                    refresh_token: None,
                    expires_at: 0,
                },
            )
            .unwrap();
        let issuer = Url::parse(&remote.uri()).unwrap();
        sign_in(
            &super::super::build_client().unwrap(),
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: Arc::new(Mutex::new(Vec::new())),
            },
            SignIn {
                resource_metadata_url: None,
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: Some("fallback-client".to_owned()),
                client_secret: Some(Zeroizing::new("fallback-secret".to_owned())),
                client_issuer: Some(&issuer),
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await
        .expect("the stored client still signs in");
        let requests = remote.received_requests().await.unwrap();
        let token = requests
            .iter()
            .find(|request| request.url.path() == "/api/login/oauth/access_token")
            .unwrap();
        assert_eq!(
            token.headers["authorization"],
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("stored-client:stored-secret")
            )
        );
        assert_eq!(
            grants.get("srv-1").unwrap().unwrap().client_id,
            "stored-client"
        );
    }

    #[tokio::test]
    async fn configured_fallback_rejects_an_issuer_with_different_path_case() {
        let remote = stack_at_issuer_path(false, "/tenant").await;
        let directory = tempfile::tempdir().unwrap();
        let grants = store(directory.path());
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).unwrap();
        let issuer = Url::parse(&format!("{}/Tenant", remote.uri())).unwrap();
        let outcome = sign_in(
            &super::super::build_client().unwrap(),
            &RefusingBrowser,
            SignIn {
                resource_metadata_url: None,
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: Some("fallback-client".to_owned()),
                client_secret: Some(Zeroizing::new("fallback-secret".to_owned())),
                client_issuer: Some(&issuer),
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await;
        assert!(matches!(
            outcome,
            Err(SignInError::ClientRegistrationUnavailable)
        ));
        assert!(remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| {
                request.method == reqwest::Method::GET
                    && request.headers.get("authorization").is_none()
            }));
    }

    #[tokio::test]
    async fn issuer_metadata_cannot_redirect_a_configured_client_to_another_host() {
        let remote = stack(false).await;
        let issuer = MockServer::start().await;
        let issuer_url = Url::parse(&issuer.uri()).unwrap();
        // A resource may name the trusted issuer, but the metadata must still
        // be fetched from that issuer without following a redirect elsewhere.
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-protected-resource/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": format!("{}/mcp", remote.uri()),
                "authorization_servers": [issuer.uri()],
            })))
            .with_priority(1)
            .mount(&remote)
            .await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/untrusted-metadata", remote.uri())),
            )
            .mount(&issuer)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let grants = store(directory.path());
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let outcome = sign_in(
            &super::super::build_client().unwrap(),
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: seen.clone(),
            },
            SignIn {
                resource_metadata_url: None,
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: Some("fallback-client".to_owned()),
                client_secret: Some(Zeroizing::new("fallback-secret".to_owned())),
                client_issuer: Some(&issuer_url),
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await;
        assert!(matches!(
            outcome,
            Err(SignInError::Discovery(
                DiscoveryError::AuthorizationServerUnavailable
            ))
        ));
        assert!(seen.lock().unwrap().is_empty());
        assert!(remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| {
                request.url.path() == "/.well-known/oauth-protected-resource/mcp"
                    && request.headers.get("authorization").is_none()
            }));
    }

    #[tokio::test]
    async fn a_browser_that_will_not_open_fails_without_waiting_out_the_deadline() {
        let remote = stack(true).await;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).expect("url");

        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            sign_in(
                &http,
                &RefusingBrowser,
                SignIn {
                    server_id: "srv-1",
                    endpoint: &endpoint,
                    client_id: None,
                    client_secret: None,
                    client_issuer: None,
                    resource_metadata_url: None,
                    loopback_port: 0,
                    destination: GrantDestination::Device(&grants),
                },
            ),
        )
        .await
        .expect("the flow must not wait for a callback that cannot arrive");

        assert!(matches!(outcome, Err(SignInError::BrowserUnavailable)));
    }

    #[tokio::test]
    async fn a_renewal_uses_the_stored_coordinates_and_keeps_the_grant_current() {
        let remote = stack(true).await;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).expect("url");

        sign_in(
            &http,
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: Arc::new(Mutex::new(Vec::new())),
            },
            SignIn {
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: None,
                client_secret: None,
                client_issuer: None,
                resource_metadata_url: None,
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await
        .expect("the sign-in completes");

        let stored = grants.get("srv-1").expect("readable").expect("a grant");
        let renewed = renew(&http, &stored).await.expect("the renewal succeeds");
        assert_eq!(renewed.access_token, "granted-access-token");
        assert_eq!(renewed.token_endpoint, stored.token_endpoint);
        assert_eq!(renewed.resource, stored.resource);
    }

    #[tokio::test]
    async fn signing_out_revokes_upstream_and_deletes_locally_even_when_it_cannot() {
        let remote = stack(true).await;
        let unreachable = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/login/oauth/revoke"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&unreachable)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/login/oauth/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&remote)
            .await;

        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        let endpoint = Url::parse(&format!("{}/mcp", remote.uri())).expect("url");

        sign_in(
            &http,
            &RedirectingBrowser {
                code: "the-code".to_owned(),
                seen: Arc::new(Mutex::new(Vec::new())),
            },
            SignIn {
                server_id: "srv-1",
                endpoint: &endpoint,
                client_id: None,
                client_secret: None,
                client_issuer: None,
                resource_metadata_url: None,
                loopback_port: 0,
                destination: GrantDestination::Device(&grants),
            },
        )
        .await
        .expect("the sign-in completes");

        assert!(
            sign_out(&http, &grants, "srv-1")
                .await
                .expect("the sign-out completes"),
            "an acknowledged revocation is reported"
        );
        assert!(grants.get("srv-1").expect("readable").is_none());

        // A server whose revocation endpoint is failing must still leave the
        // device with nothing stored.
        let stored = StoredGrant {
            client_id: "issued-client".to_owned(),
            client_secret: None,
            issuer: unreachable.uri(),
            token_endpoint: format!("{}/api/login/oauth/access_token", unreachable.uri()),
            revocation_endpoint: Some(format!("{}/api/login/oauth/revoke", unreachable.uri())),
            resource: format!("{}/mcp", unreachable.uri()),
            scope: None,
            access_token: "granted-access-token".to_owned(),
            refresh_token: Some("granted-refresh-token".to_owned()),
            expires_at: exchange::now_epoch_seconds() + 3600,
        };
        grants.put("srv-2", &stored).expect("stored");
        assert!(
            !sign_out(&http, &grants, "srv-2")
                .await
                .expect("the sign-out completes"),
            "an unacknowledged revocation is reported as such"
        );
        assert!(grants.get("srv-2").expect("readable").is_none());
    }

    #[tokio::test]
    async fn signing_out_of_a_server_that_was_never_signed_in_to_is_success() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let grants = store(directory.path());
        let http = super::super::build_client().expect("client");
        assert!(!sign_out(&http, &grants, "srv-unknown")
            .await
            .expect("the sign-out completes"));
    }
}
