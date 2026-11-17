//! Discovery: what a registered MCP server says about how to authenticate to
//! it, and what its authorization server says about how to talk to it.
//!
//! This is the whole reason the desktop can sign in to a server whose
//! `auth_config` it cannot read. Nothing here is configured; every value is
//! read from the two documents the specifications define, and validated before
//! use.
//!
//! # The URL forms, and why both are tried
//!
//! RFC 9728 §3.1 inserts `/.well-known/oauth-protected-resource` *before* the
//! resource's path, so `https://host/mcp` publishes at
//! `https://host/.well-known/oauth-protected-resource/mcp`. Plenty of servers
//! (and the `mcp-remote` bridge) only serve the root form. DA serves both —
//! its `/.well-known/oauth-protected-resource/{path}` route exists precisely to
//! accept the inserted form — so the spec-correct URL is tried first and the
//! root form is the fallback. Trying only one would work against half the
//! servers in the ecosystem.
//!
//! The same shape applies to authorization server metadata (RFC 8414 §3
//! inserts, OIDC Discovery appends), with `openid-configuration` last because
//! it is the form that returns a superset document.
//!
//! # And why a well-known URL is not the only route
//!
//! RFC 9728 §5.1 lets a resource server point a client at its metadata from the
//! `WWW-Authenticate` challenge on a `401`, which is the only route available to
//! a server that publishes the document somewhere other than a well-known path.
//! [`resolve_protected_resource`] therefore ends with that challenge rather than
//! reporting the server undiscoverable, so "add any MCP server" means any
//! *conforming* server and not only the conventional ones.
//!
//! The pointer is not trusted for being a pointer. It is dropped unless it is
//! same-origin with the server being dialed ([`super::challenge::Challenge::metadata_url_for`]),
//! and the document it returns still has to name that same origin as its
//! `resource` ([`validate_protected_resource`]) — so the trust anchor stays what
//! it was: the `authorization_servers` list of a document that claims *this*
//! server.

use reqwest::{Client, Url};
use serde::Deserialize;

use super::{endpoint_is_usable, issuers_match, read_json, HttpError};

/// RFC 9728 §2, narrowed to the fields this client acts on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedResource {
    /// The audience value that must be sent as the RFC 8707 `resource`
    /// indicator, and that the server will check on the token it receives.
    pub(crate) resource: String,
    /// Issuer identifiers — not metadata document URLs (RFC 9728 §2).
    pub(crate) authorization_servers: Vec<String>,
    pub(crate) scopes_supported: Vec<String>,
}

/// RFC 8414 §2 / OIDC Discovery, narrowed the same way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizationServer {
    pub(crate) issuer: String,
    pub(crate) authorization_endpoint: Url,
    pub(crate) token_endpoint: Url,
    /// RFC 7591. Absent means this server issues no clients dynamically, which
    /// is the case for both hops of the Iseyon stack today.
    pub(crate) registration_endpoint: Option<Url>,
    /// RFC 7009, used on sign-out. Absent means a grant can only be forgotten
    /// locally.
    pub(crate) revocation_endpoint: Option<Url>,
    pub(crate) scopes_supported: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DiscoveryError {
    #[error("the server published no protected resource metadata")]
    ResourceMetadataUnavailable,
    #[error("the server's protected resource metadata is not usable")]
    ResourceMetadataInvalid,
    /// The metadata's `resource` names a different service than the one being
    /// dialed. Refused because that value becomes the audience of a token this
    /// app then attaches to *this* server's requests: honouring it would mean
    /// sending a token minted for somewhere else.
    #[error("the server's metadata names a different resource")]
    ResourceMismatch,
    #[error("the server named no authorization server this client can use")]
    NoUsableAuthorizationServer,
    #[error("the authorization server published no usable metadata")]
    AuthorizationServerUnavailable,
    #[error("the authorization server's metadata is not usable")]
    AuthorizationServerInvalid,
    /// OAuth 2.1 requires PKCE, and the MCP specification requires a client to
    /// verify `S256` support before starting a flow rather than discover the
    /// failure after sending the user to a browser.
    #[error("the authorization server does not advertise PKCE S256")]
    PkceUnsupported,
}

#[derive(Debug, Deserialize)]
struct ProtectedResourceDocument {
    resource: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AuthorizationServerDocument {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    revocation_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

/// Reads a server's Protected Resource Metadata and checks it describes the
/// server actually being dialed.
///
/// `override_url` is the `resource_metadata` value from a `401` challenge, which
/// `super::challenge` has already confined to the server's own origin. When
/// present it is used alone: the server has just said where its current
/// metadata is, and probing the well-known paths anyway would be second-guessing
/// a live statement with a stale convention.
pub(crate) async fn protected_resource(
    client: &Client,
    endpoint: &Url,
    override_url: Option<Url>,
) -> Result<ProtectedResource, DiscoveryError> {
    let candidates = match override_url {
        Some(url) => vec![url],
        None => metadata_urls(endpoint),
    };

    let mut last = DiscoveryError::ResourceMetadataUnavailable;
    for url in candidates {
        match fetch::<ProtectedResourceDocument>(client, &url).await {
            Ok(document) => return validate_protected_resource(document, endpoint),
            // A 404 on the inserted form is the expected outcome for a server
            // that only serves the root form, so a failure here is only fatal
            // once every candidate has failed.
            Err(_) => last = DiscoveryError::ResourceMetadataUnavailable,
        }
    }
    Err(last)
}

/// The server's Protected Resource Metadata, by every route the specifications
/// define, in order of how much the client can rely on each.
///
/// 1. `override_url`, when an operator configured one on the shared server row.
///    Configuration beats discovery: it is a human statement about a server that
///    discovery cannot be expected to get right.
/// 2. The two RFC 9728 §3.1 well-known forms.
/// 3. The `resource_metadata` pointer in the server's own `WWW-Authenticate`
///    challenge (RFC 9728 §5.1).
///
/// The challenge is consulted only when the earlier routes found *nothing*. A
/// document that was found and then refused — because it named a different
/// resource, or listed no authorization server — is a definitive answer, and
/// re-asking through a channel the server also controls would let a bad document
/// be retried until one passed.
pub(crate) async fn resolve_protected_resource(
    client: &Client,
    endpoint: &Url,
    override_url: Option<Url>,
) -> Result<ProtectedResource, DiscoveryError> {
    let configured = override_url.is_some();
    match protected_resource(client, endpoint, override_url).await {
        Ok(resource) => Ok(resource),
        // An override that resolves to nothing is reported as configured, not
        // silently replaced by discovery: an operator who set the wrong URL has
        // to be able to see that it is the wrong URL.
        Err(error) if configured => Err(error),
        Err(DiscoveryError::ResourceMetadataUnavailable) => {
            let hint = challenge_metadata_url(client, endpoint).await;
            match hint {
                None => Err(DiscoveryError::ResourceMetadataUnavailable),
                Some(url) => {
                    tracing::debug!(
                        "mcp sign-in: taking protected resource metadata from the server's challenge"
                    );
                    protected_resource(client, endpoint, Some(url)).await
                }
            }
        }
        Err(error) => Err(error),
    }
}

/// Asks the MCP endpoint itself where its metadata lives, by making an
/// unauthenticated request and reading the `WWW-Authenticate` challenge.
///
/// Two probes, because auth is enforced at two different depths in real servers:
/// middleware refuses any method and answers the `GET` a Streamable HTTP client
/// would open its stream with, while a server that dispatches first may answer
/// `405` to that and only challenge the `POST` an MCP session actually starts
/// with. Both are unauthenticated and neither creates anything: the `initialize`
/// body is the same first request a successful session sends, and this one is
/// about to be refused.
///
/// Every failure is `None`. This runs only after discovery has already found
/// nothing, so there is no state to report beyond "still nothing".
async fn challenge_metadata_url(client: &Client, endpoint: &Url) -> Option<Url> {
    let probes = [
        client.get(endpoint.clone()).header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        ),
        client
            .post(endpoint.clone())
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": crate::mcp::client::PINNED_PROTOCOL_VERSION.as_str(),
                    "capabilities": {},
                    "clientInfo": { "name": super::CLIENT_NAME, "version": "0" }
                }
            })),
    ];
    for probe in probes {
        let Ok(response) = probe.send().await else {
            continue;
        };
        let header = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // The body is never read: only the header is wanted, and the response
        // came from an endpoint that has not authenticated anything yet.
        if let Some(url) = super::challenge::parse(header.as_deref()).metadata_url_for(endpoint) {
            return Some(url);
        }
    }
    None
}

/// Reads an authorization server's metadata, having first confirmed the
/// resource server actually named it.
///
/// The listing check is the trust anchor of the whole flow: without it, any
/// value in `authorization_servers` — or any issuer a caller passed in — could
/// send the user to an authorization server of an attacker's choosing and have
/// the resulting token attached to this server's requests.
pub(crate) async fn authorization_server(
    client: &Client,
    resource: &ProtectedResource,
    issuer: &str,
) -> Result<AuthorizationServer, DiscoveryError> {
    if !resource
        .authorization_servers
        .iter()
        .any(|listed| issuers_match(listed, issuer))
    {
        return Err(DiscoveryError::NoUsableAuthorizationServer);
    }
    let issuer_url = Url::parse(issuer).map_err(|_| DiscoveryError::NoUsableAuthorizationServer)?;
    if !endpoint_is_usable(&issuer_url) {
        return Err(DiscoveryError::NoUsableAuthorizationServer);
    }

    let mut last = DiscoveryError::AuthorizationServerUnavailable;
    for url in metadata_candidates(&issuer_url) {
        match fetch::<AuthorizationServerDocument>(client, &url).await {
            Ok(document) => return validate_authorization_server(document, issuer),
            Err(_) => last = DiscoveryError::AuthorizationServerUnavailable,
        }
    }
    Err(last)
}

/// The two Protected Resource Metadata URLs, spec-correct form first.
fn metadata_urls(endpoint: &Url) -> Vec<Url> {
    let path = endpoint
        .path()
        .trim_start_matches('/')
        .trim_end_matches('/');
    let mut root = endpoint.clone();
    root.set_path("/.well-known/oauth-protected-resource");
    root.set_query(None);
    root.set_fragment(None);
    if path.is_empty() {
        return vec![root];
    }
    let mut inserted = root.clone();
    inserted.set_path(&format!("/.well-known/oauth-protected-resource/{path}"));
    vec![inserted, root]
}

/// Authorization server metadata URLs: RFC 8414's inserted form, then the
/// appended form, then OIDC discovery.
fn metadata_candidates(issuer: &Url) -> Vec<Url> {
    let path = issuer.path().trim_start_matches('/').trim_end_matches('/');
    let mut base = issuer.clone();
    base.set_query(None);
    base.set_fragment(None);

    let build = |path: String| {
        let mut url = base.clone();
        url.set_path(&path);
        url
    };
    if path.is_empty() {
        return vec![
            build("/.well-known/oauth-authorization-server".to_owned()),
            build("/.well-known/openid-configuration".to_owned()),
        ];
    }
    vec![
        build(format!("/.well-known/oauth-authorization-server/{path}")),
        build(format!("/{path}/.well-known/oauth-authorization-server")),
        build(format!("/{path}/.well-known/openid-configuration")),
    ]
}

async fn fetch<T: serde::de::DeserializeOwned>(client: &Client, url: &Url) -> Result<T, HttpError> {
    if !endpoint_is_usable(url) {
        return Err(HttpError::RequestFailed);
    }
    let response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| HttpError::RequestFailed)?;
    if !response.status().is_success() {
        return Err(HttpError::RequestFailed);
    }
    read_json(response).await
}

fn validate_protected_resource(
    document: ProtectedResourceDocument,
    endpoint: &Url,
) -> Result<ProtectedResource, DiscoveryError> {
    let resource = Url::parse(document.resource.trim())
        .map_err(|_| DiscoveryError::ResourceMetadataInvalid)?;
    if resource.origin() != endpoint.origin() {
        return Err(DiscoveryError::ResourceMismatch);
    }
    if document.authorization_servers.is_empty() {
        return Err(DiscoveryError::ResourceMetadataInvalid);
    }
    Ok(ProtectedResource {
        resource: document.resource.trim().to_owned(),
        authorization_servers: document.authorization_servers,
        scopes_supported: document.scopes_supported,
    })
}

fn validate_authorization_server(
    document: AuthorizationServerDocument,
    requested_issuer: &str,
) -> Result<AuthorizationServer, DiscoveryError> {
    // RFC 8414 §3.3: the document must claim the issuer it was fetched for.
    // Without this a metadata document served from anywhere could redirect the
    // flow to endpoints it does not own.
    if !issuers_match(&document.issuer, requested_issuer) {
        return Err(DiscoveryError::AuthorizationServerInvalid);
    }
    // Verified before the browser opens, per the MCP specification: a flow that
    // discovers mid-way that PKCE is unavailable has already sent the user
    // somewhere, and OAuth 2.1 has no acceptable fallback to fall back *to*.
    if !document
        .code_challenge_methods_supported
        .iter()
        .any(|method| method.eq_ignore_ascii_case("S256"))
    {
        return Err(DiscoveryError::PkceUnsupported);
    }

    let authorization_endpoint = usable_url(&document.authorization_endpoint)
        .ok_or(DiscoveryError::AuthorizationServerInvalid)?;
    let token_endpoint =
        usable_url(&document.token_endpoint).ok_or(DiscoveryError::AuthorizationServerInvalid)?;

    Ok(AuthorizationServer {
        issuer: document.issuer,
        authorization_endpoint,
        token_endpoint,
        // An unusable optional endpoint is dropped rather than fatal: neither
        // registration nor revocation is required to sign in, and refusing the
        // whole server over one malformed optional field would deny a flow that
        // works.
        registration_endpoint: document
            .registration_endpoint
            .as_deref()
            .and_then(usable_url),
        revocation_endpoint: document.revocation_endpoint.as_deref().and_then(usable_url),
        scopes_supported: document.scopes_supported,
    })
}

fn usable_url(raw: &str) -> Option<Url> {
    let url = Url::parse(raw.trim()).ok()?;
    endpoint_is_usable(&url).then_some(url)
}

/// What to ask for, given what both parties said they support.
///
/// The intersection, not the resource server's list: a resource may advertise
/// scopes its authorization server has never heard of (Iseyon's DA advertises
/// `mcp:admin*`, which its Casdoor does not issue), and requesting one of those
/// gets the whole authorization request rejected. Intersecting also keeps this
/// client from asking for administrative scopes merely because they exist —
/// least privilege falls out of the rule rather than needing a deny-list.
///
/// `offline_access` is added when the authorization server offers it, because a
/// grant with no refresh token makes the user re-consent in the browser every
/// time its access token ages out.
///
/// `None` when the intersection is empty: an omitted `scope` asks the
/// authorization server for its own default, which is the correct request to
/// make when the two documents share no vocabulary.
#[must_use]
pub(crate) fn negotiated_scope(
    resource: &ProtectedResource,
    server: &AuthorizationServer,
) -> Option<String> {
    const OFFLINE_ACCESS: &str = "offline_access";

    let mut scopes: Vec<&str> = resource
        .scopes_supported
        .iter()
        .filter(|wanted| {
            server
                .scopes_supported
                .iter()
                .any(|offered| offered == *wanted)
        })
        .map(String::as_str)
        .collect();

    let offers_offline_access = server
        .scopes_supported
        .iter()
        .any(|offered| offered == OFFLINE_ACCESS);
    if offers_offline_access && !scopes.contains(&OFFLINE_ACCESS) {
        scopes.push(OFFLINE_ACCESS);
    }
    if scopes.is_empty() {
        return None;
    }
    Some(scopes.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(resource: &str, servers: &[&str]) -> ProtectedResourceDocument {
        ProtectedResourceDocument {
            resource: resource.to_owned(),
            authorization_servers: servers.iter().map(|value| (*value).to_owned()).collect(),
            scopes_supported: Vec::new(),
        }
    }

    fn as_document() -> AuthorizationServerDocument {
        AuthorizationServerDocument {
            issuer: "https://auth.example.test".to_owned(),
            authorization_endpoint: "https://auth.example.test/login/oauth/authorize".to_owned(),
            token_endpoint: "https://auth.example.test/api/login/oauth/access_token".to_owned(),
            registration_endpoint: Some("https://auth.example.test/api/oauth/register".to_owned()),
            revocation_endpoint: None,
            scopes_supported: vec!["openid".to_owned(), "offline_access".to_owned()],
            code_challenge_methods_supported: vec!["S256".to_owned()],
        }
    }

    #[test]
    fn the_spec_correct_metadata_url_is_tried_before_the_root_form() {
        let endpoint = Url::parse("https://mcp.example.test/mcp").expect("url");
        let urls = metadata_urls(&endpoint);
        assert_eq!(
            urls.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec![
                "https://mcp.example.test/.well-known/oauth-protected-resource/mcp",
                "https://mcp.example.test/.well-known/oauth-protected-resource",
            ]
        );
    }

    #[test]
    fn a_root_endpoint_has_only_the_root_metadata_url() {
        let endpoint = Url::parse("https://mcp.example.test/").expect("url");
        let urls = metadata_urls(&endpoint);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0].as_str(),
            "https://mcp.example.test/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn an_issuer_with_a_path_is_probed_in_all_three_documented_forms() {
        let issuer = Url::parse("https://auth.example.test/realms/acme").expect("url");
        let urls = metadata_candidates(&issuer);
        assert_eq!(
            urls.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec![
                "https://auth.example.test/.well-known/oauth-authorization-server/realms/acme",
                "https://auth.example.test/realms/acme/.well-known/oauth-authorization-server",
                "https://auth.example.test/realms/acme/.well-known/openid-configuration",
            ]
        );
    }

    #[test]
    fn metadata_naming_another_service_as_the_resource_is_refused() {
        let endpoint = Url::parse("https://mcp.example.test/mcp").expect("url");
        assert_eq!(
            validate_protected_resource(
                document(
                    "https://elsewhere.example/mcp",
                    &["https://auth.example.test"]
                ),
                &endpoint,
            ),
            Err(DiscoveryError::ResourceMismatch)
        );
        // The same origin with a different path is the same resource server, and
        // is the shape DA actually publishes (`https://<tenant>/mcp`).
        assert!(validate_protected_resource(
            document(
                "https://mcp.example.test/mcp",
                &["https://auth.example.test"]
            ),
            &endpoint,
        )
        .is_ok());
    }

    #[test]
    fn metadata_with_no_authorization_server_is_refused() {
        let endpoint = Url::parse("https://mcp.example.test/mcp").expect("url");
        assert_eq!(
            validate_protected_resource(document("https://mcp.example.test/mcp", &[]), &endpoint),
            Err(DiscoveryError::ResourceMetadataInvalid)
        );
    }

    #[test]
    fn an_authorization_server_document_must_claim_the_issuer_it_was_fetched_for() {
        let mut document = as_document();
        document.issuer = "https://attacker.example".to_owned();
        assert_eq!(
            validate_authorization_server(document, "https://auth.example.test"),
            Err(DiscoveryError::AuthorizationServerInvalid)
        );
        let mut document = as_document();
        document.issuer = "https://auth.example.test/tenant".to_owned();
        assert_eq!(
            validate_authorization_server(document, "https://auth.example.test/Tenant"),
            Err(DiscoveryError::AuthorizationServerInvalid)
        );
    }

    #[test]
    fn a_server_that_does_not_advertise_s256_is_refused_before_a_browser_opens() {
        let mut document = as_document();
        document.code_challenge_methods_supported = vec!["plain".to_owned()];
        assert_eq!(
            validate_authorization_server(document, "https://auth.example.test"),
            Err(DiscoveryError::PkceUnsupported)
        );

        let mut silent = as_document();
        silent.code_challenge_methods_supported = Vec::new();
        assert_eq!(
            validate_authorization_server(silent, "https://auth.example.test"),
            Err(DiscoveryError::PkceUnsupported),
            "an absent advertisement is not an implicit yes"
        );
    }

    #[test]
    fn a_malformed_optional_endpoint_is_dropped_rather_than_failing_the_server() {
        let mut document = as_document();
        document.registration_endpoint = Some("not a url".to_owned());
        // Refused under either host policy (a private address is policy-dependent,
        // see `ENFORCE_PUBLIC_MCP_HOSTS`; embedded credentials never are).
        document.revocation_endpoint = Some("https://user:pw@auth.example.test/revoke".to_owned());
        let server = validate_authorization_server(document, "https://auth.example.test")
            .expect("the server is still usable");
        assert!(server.registration_endpoint.is_none());
        assert!(server.revocation_endpoint.is_none());
    }

    #[test]
    fn the_requested_scope_is_the_intersection_plus_offline_access() {
        let resource = ProtectedResource {
            resource: "https://mcp.example.test/mcp".to_owned(),
            authorization_servers: vec!["https://auth.example.test".to_owned()],
            scopes_supported: vec![
                "openid".to_owned(),
                "profile".to_owned(),
                // Advertised by the resource, unknown to the authorization
                // server below: requesting it would fail the whole flow.
                "mcp:admin".to_owned(),
            ],
        };
        let server = validate_authorization_server(as_document(), "https://auth.example.test")
            .expect("valid server");

        assert_eq!(
            negotiated_scope(&resource, &server).as_deref(),
            Some("openid offline_access")
        );
    }

    #[test]
    fn no_shared_vocabulary_means_no_scope_parameter_at_all() {
        let resource = ProtectedResource {
            resource: "https://mcp.example.test/mcp".to_owned(),
            authorization_servers: vec!["https://auth.example.test".to_owned()],
            scopes_supported: vec!["mcp:admin".to_owned()],
        };
        let mut document = as_document();
        document.scopes_supported = Vec::new();
        let server = validate_authorization_server(document, "https://auth.example.test")
            .expect("valid server");

        assert_eq!(negotiated_scope(&resource, &server), None);
    }

    #[test]
    fn an_authorization_server_the_resource_never_named_is_refused() {
        let resource = ProtectedResource {
            resource: "https://mcp.example.test/mcp".to_owned(),
            authorization_servers: vec!["https://auth.example.test".to_owned()],
            scopes_supported: Vec::new(),
        };
        let client = super::super::build_client().expect("client");
        let refused = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(authorization_server(
                &client,
                &resource,
                "https://attacker.example",
            ));
        assert_eq!(refused, Err(DiscoveryError::NoUsableAuthorizationServer));
    }
}
