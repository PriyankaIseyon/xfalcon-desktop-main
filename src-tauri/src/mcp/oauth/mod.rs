//! Per-server OAuth 2.1 for registered MCP servers.
//!
//! `super::auth` explains why the *bundled* tenant server needs none of this:
//! it is the same resource server the desktop already authenticated against at
//! login, so its bearer is the session bearer. A **registered** server is a
//! different resource, with its own authorization server, its own consent, and
//! its own grant — and DA cannot supply any of it, because
//! `mcp_servers.auth_config` is encrypted under a key that never leaves
//! xfalcon-web (`crate::clients::da::mcp_servers`). So this module implements
//! the MCP specification's own client role instead of reading configuration it
//! has no way to decrypt:
//!
//! | Step | Specification |
//! |---|---|
//! | Protected Resource Metadata, both URL forms | RFC 9728 §3.1 |
//! | Authorization Server Metadata, RFC + OIDC forms | RFC 8414 §3, OIDC Discovery |
//! | Dynamic client registration | RFC 7591 |
//! | Authorization code + PKCE `S256` | RFC 7636, OAuth 2.1 §4.1 |
//! | Loopback redirect for a native app | RFC 8252 §7.3 |
//! | `resource` indicator on authorize *and* token | RFC 8707 §2 |
//! | Refresh, and revocation on sign-out | RFC 6749 §6, RFC 7009 |
//! | `WWW-Authenticate` challenge handling | RFC 6750 §3, RFC 9728 §5.1 |
//!
//! # Where the grant lives, and why not in DA
//!
//! In [`store`], sealed by `crate::credentials::CredentialVault` into this
//! device's envelope — the same model as a registered server's static
//! credential. xfalcon-web persists its grants in DA
//! (`/api/v1/chat/mcp-oauth-grants`) because a web session has nowhere else to
//! put them; this app deliberately does not, for the reason stated on the
//! Settings surface itself: a server definition is shared across the account,
//! a credential is not. Writing a refresh token to DA would make one device's
//! browser consent silently authorize every other device.
//!
//! The consequence is honest and visible: signing in on a laptop does not sign
//! in on a desktop, and each reports [`McpSignInStatus`] for itself.
//!
//! [`McpSignInStatus`]: xfalcon_contracts::mcp::McpSignInStatus

pub(crate) mod challenge;
pub(crate) mod discovery;
pub(crate) mod envelope;
pub(crate) mod exchange;
pub(crate) mod flow;
pub(crate) mod registration;
pub(crate) mod shared_grant;
pub(crate) mod store;

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{redirect::Policy, Client, Response, Url};

use crate::mcp::dns::PublicOnlyResolver;

/// Metadata and token requests get a short leash: they are all one-shot
/// request/response exchanges against an endpoint a user is waiting on.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// The `client_name` this app registers itself under (RFC 7591 §2), which is
/// what an authorization server shows on its consent screen and in its client
/// list. Fixed rather than derived from the device so an operator sees one
/// client name per product, not one per laptop.
pub(crate) const CLIENT_NAME: &str = "xFalcon Desktop";

/// A single JSON document read from an authorization server or resource server.
/// Metadata documents are a few hundred bytes; this is three orders of
/// magnitude of headroom, and exists so a hostile endpoint cannot stream the
/// process out of memory.
const MAX_JSON_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum HttpError {
    #[error("the request could not be made")]
    RequestFailed,
    #[error("the response was larger than this client will read")]
    TooLarge,
    #[error("the response was not the JSON document this step expects")]
    Malformed,
}

/// The client every step in this module uses.
///
/// Redirects are refused rather than followed: each endpoint here is either
/// discovered metadata or a token endpoint, and silently following a redirect
/// would move a request that carries a client secret, an authorization code, or
/// a refresh token to a host that was never validated.
///
/// Names resolve through [`PublicOnlyResolver`] only while
/// `ENFORCE_PUBLIC_MCP_HOSTS` is on. With it off, these endpoints belong to
/// user-registered servers that may sit on a private address, so names resolve
/// through the system resolver.
pub(crate) fn build_client() -> Result<Client, HttpError> {
    let mut builder = Client::builder()
        .user_agent(super::USER_AGENT)
        .redirect(Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT);
    if super::registry::ENFORCE_PUBLIC_MCP_HOSTS {
        builder = builder.dns_resolver(Arc::new(PublicOnlyResolver::new()));
    }
    builder.build().map_err(|_| HttpError::RequestFailed)
}

/// Reads at most [`MAX_JSON_BYTES`] and parses them.
///
/// Streamed with a running bound rather than `Response::json`, which would
/// buffer whatever the peer sends before any limit could apply.
pub(crate) async fn read_json<T: serde::de::DeserializeOwned>(
    response: Response,
) -> Result<T, HttpError> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| HttpError::RequestFailed)?;
        if body.len() + chunk.len() > MAX_JSON_BYTES {
            return Err(HttpError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| HttpError::Malformed)
}

/// Whether this build will send an OAuth request to `url`, under the public-hosts
/// policy the switch names (see [`endpoint_is_usable_with`]).
pub(crate) fn endpoint_is_usable(url: &Url) -> bool {
    endpoint_is_usable_with(url, super::registry::ENFORCE_PUBLIC_MCP_HOSTS)
}

/// [`endpoint_is_usable`] with the public-hosts policy passed in. `enforce` is
/// `registry::ENFORCE_PUBLIC_MCP_HOSTS` for the public function.
///
/// Every URL reaching this function came from a discovery document, so it is
/// data a third party wrote — and it is about to receive an authorization code,
/// a client secret, or a refresh token. With `enforce` on it gets the same
/// treatment as a registered MCP endpoint (`super::registry`): https, and no
/// host that would point the request at the user's own network. Cleartext
/// `http` is also allowed in a debug build, and, with `enforce` off, to a host
/// on the user's own network. Embedded userinfo and fragments are refused in
/// both modes.
///
/// Unlike an MCP endpoint a query string is *allowed*: RFC 6749 §3.1 permits
/// the authorization and token endpoints to carry one, and refusing it would
/// reject a conforming authorization server.
pub(crate) fn endpoint_is_usable_with(url: &Url, enforce: bool) -> bool {
    let cleartext_allowed = cfg!(debug_assertions)
        || (!enforce
            && url
                .host_str()
                .is_some_and(super::registry::host_looks_private));
    let scheme_allowed = match url.scheme() {
        "https" => true,
        "http" => cleartext_allowed,
        _ => false,
    };
    if !scheme_allowed {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return false;
    }
    url.host_str()
        .is_some_and(|host| super::registry::host_is_dialable_with(host, enforce))
}

/// Compares issuer URLs with the existing trailing-slash tolerance. URL parsing
/// normalizes the scheme and host, while issuer paths remain case-sensitive.
pub(crate) fn issuers_match(left: &str, right: &str) -> bool {
    fn normalize(value: &str) -> Option<Url> {
        let mut issuer = Url::parse(value.trim()).ok()?;
        let path = issuer.path().trim_end_matches('/').to_owned();
        issuer.set_path(&path);
        Some(issuer)
    }
    match (normalize(left), normalize(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_request_names_the_app_in_its_user_agent() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Stands in for an AWS WAF common rule set, which answers a request
        // with no User-Agent with a bare 403.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-protected-resource"))
            .and(header("user-agent", crate::mcp::USER_AGENT))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let response = build_client()
            .expect("client")
            .get(format!(
                "{}/.well-known/oauth-protected-resource",
                server.uri()
            ))
            .send()
            .await
            .expect("response");
        assert_eq!(response.status(), 200);
        assert!(crate::mcp::USER_AGENT.starts_with("xFalcon-Desktop/"));
    }

    #[test]
    fn a_discovered_endpoint_must_be_https_without_userinfo_or_fragment() {
        for rejected in [
            "https://user:pass@as.example/token",
            "https://as.example/token#fragment",
            "ftp://as.example/token",
        ] {
            let url = Url::parse(rejected).expect("a parseable url");
            assert!(
                !endpoint_is_usable_with(&url, true),
                "{rejected} must be refused"
            );
            assert!(
                !endpoint_is_usable_with(&url, false),
                "{rejected} must be refused"
            );
        }
        // A query string is legal on an OAuth endpoint, unlike on an MCP one.
        let query = Url::parse("https://as.example/token?tenant=acme").expect("url");
        assert!(endpoint_is_usable_with(&query, true));
        assert!(endpoint_is_usable_with(&query, false));
        // Cleartext is accepted only where a developer's local authorization
        // server is the point, matching `super::registry`'s rule for MCP
        // endpoints. A release build refuses it under the enforcing policy.
        assert_eq!(
            endpoint_is_usable_with(&Url::parse("http://as.example/token").expect("url"), true),
            cfg!(debug_assertions)
        );
        // Permissive: cleartext to a private host is accepted in every build,
        // and a public-looking name still needs a debug build.
        assert!(endpoint_is_usable_with(
            &Url::parse("http://10.0.0.5/token").expect("url"),
            false
        ));
        assert!(endpoint_is_usable_with(
            &Url::parse("https://10.0.0.5/token").expect("url"),
            false
        ));
        assert_eq!(
            endpoint_is_usable_with(&Url::parse("http://as.example/token").expect("url"), false),
            cfg!(debug_assertions)
        );
    }

    #[test]
    fn a_discovered_endpoint_may_not_point_at_the_users_own_network() {
        for rejected in [
            "https://10.0.0.5/token",
            "https://192.168.1.1/token",
            "https://[fd00::1]/token",
            "https://169.254.169.254/token",
        ] {
            let url = Url::parse(rejected).expect("a parseable url");
            assert!(
                !endpoint_is_usable_with(&url, true),
                "{rejected} must be refused"
            );
        }
    }

    #[test]
    fn a_discovered_endpoint_may_point_at_the_users_own_network_when_not_enforced() {
        for accepted in [
            "https://10.0.0.5/token",
            "https://192.168.1.1/token",
            "https://[fd00::1]/token",
            "https://169.254.169.254/token",
        ] {
            let url = Url::parse(accepted).expect("a parseable url");
            assert!(
                endpoint_is_usable_with(&url, false),
                "{accepted} must be usable"
            );
        }
    }

    #[test]
    fn the_oauth_gate_follows_the_policy_switch() {
        let private = Url::parse("https://10.0.0.5/token").expect("url");
        assert_eq!(
            endpoint_is_usable(&private),
            !crate::mcp::registry::ENFORCE_PUBLIC_MCP_HOSTS
        );
    }

    #[test]
    fn issuer_comparison_ignores_a_trailing_slash_and_host_case() {
        assert!(issuers_match(
            "https://auth.example.com",
            "https://auth.example.com/"
        ));
        assert!(issuers_match(
            "https://AUTH.example.com/",
            "https://auth.example.com"
        ));
        assert!(!issuers_match(
            "https://auth.example.com/a",
            "https://auth.example.com/b"
        ));
        assert!(!issuers_match(
            "https://auth.example.com/Tenant",
            "https://auth.example.com/tenant"
        ));
        assert!(issuers_match(
            "HTTPS://AUTH.example.com/Tenant/",
            "https://auth.example.com/Tenant"
        ));
    }
}
