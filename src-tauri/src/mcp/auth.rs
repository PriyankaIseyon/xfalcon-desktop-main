//! Audience-bound bearer handling for the bundled DA MCP server.
//!
//! # Design decision: reuse the existing DA session bearer, do not mint a new one
//!
//! The plan's context section describes "the Rust-only DA/MCP-audience
//! bearer" and the MVP design spec (section 15.2) requires MCP bearer tokens
//! to be "short-lived, audience-bound, scope-minimized, validated by the
//! resource server, and never passed through to unrelated downstream
//! services." This module's job is to prove that requirement is met — not to
//! invent a second token-minting mechanism the rest of the codebase has no
//! evidence of.
//!
//! Inspecting the existing auth pipeline shows the audience binding already
//! happens at login time, before this module ever runs:
//!
//! - `crate::auth::oauth::build_authorization_url` and
//!   `exchange_authorization_code` both thread `discovered.resource` through
//!   as the RFC 8707 `resource` indicator, to the authorize endpoint AND the
//!   token endpoint. That is exactly how an OAuth 2.0 client asks an
//!   authorization server to bind an access token's audience to a specific
//!   protected resource — the token Casdoor returns is scoped to
//!   `discovered.resource`, not to "DA in general."
//! - `crate::auth::discovery::discover_auth` derives `resource` from DA's own
//!   RFC 9728 Protected Resource Metadata
//!   (`crate::clients::da::protected_resource::ProtectedResourceMetadata`),
//!   fetched from the same DA origin this module's bundled server lives
//!   under (see `super::server_resolver`). The pinned DA OpenAPI baseline
//!   (`contracts/da/openapi.json`) documents this metadata as "the per-tenant
//!   URL, read from `request.state.resource_uri`" — i.e. DA, not this
//!   desktop, decides what the resource identifier is, and the desktop must
//!   not second-guess it.
//! - The codebase's own naming already treats that value as the MCP
//!   resource: `crate::workspace::service::WorkspaceService::status` exposes
//!   it verbatim as `mcp_resource_url` (`discovered.resource`).
//! - `crate::auth::session::AuthSessionStore::access_token()` is the single
//!   bearer already reused for every authenticated DA REST call in
//!   `crate::clients::da::chat_api::DaChatSession`. There is exactly one
//!   session bearer in this codebase, and it is already audience-bound to
//!   the DA resource identifier at the moment it was minted.
//! - The pinned DA OpenAPI baseline exposes no token-exchange or
//!   token-downscoping endpoint the desktop could call to mint a second,
//!   MCP-specific token (its `mcp-oauth-grants*` endpoints are DA's
//!   *server-side* infrastructure for proxying OAuth to independently
//!   registered third-party/custom MCP servers — the entitlement-gated
//!   "custom MCP" feature this slice deliberately does not build, per
//!   `super::server_resolver`'s doc comment — not a mechanism for the
//!   desktop client to re-scope its own bearer).
//! - `source-examples/codex/codex-rs/rmcp-client`'s `oauth.rs`/
//!   `perform_oauth_login.rs` implement a full per-server OAuth flow (dynamic
//!   client registration, PKCE, refresh) because Codex connects to
//!   arbitrary, independently-authorizing third-party MCP servers — a
//!   fundamentally different problem from ours (exactly one server, already
//!   authenticated against the same resource server used for chat REST).
//!   That complexity is evidence for what the *future* custom-MCP slice may
//!   need, not for this one.
//!
//! Conclusion: the bundled MCP client reuses
//! `AuthSessionStore::access_token()` as-is. "No provider/control token is
//! reused" (per the plan) reads as: never attach a model-provider API key or
//! some other control-plane credential to MCP requests — not "mint a second
//! DA token." What this module still must do, because nothing upstream
//! proves it structurally, is verify *at bind time* that the resource this
//! particular bearer was authorized for actually matches the bundled MCP
//! server's origin, and make it structurally impossible for a tool
//! call/model/skill to substitute a different bearer. That is exactly
//! [`McpAuth::bind`]'s job.

use http::header::{HeaderName, HeaderValue};
use zeroize::Zeroizing;

use super::server_resolver::McpServerDescriptor;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum McpAuthError {
    /// No usable bearer was supplied (e.g. the user is signed out). Never
    /// carries the token value.
    #[error("no active session bearer is available for the bundled MCP server")]
    MissingBearer,
    /// The token was authorized for a resource that is not this bundled MCP
    /// server's origin. This is the "audience mismatch" rejection the plan
    /// requires as an explicit test: a bearer scoped to the wrong resource
    /// must never be attached to an MCP request, no matter how it was
    /// obtained.
    #[error("session bearer is not authorized for the bundled MCP server's resource")]
    AudienceMismatch,
    /// A registered server named a header this build will not send — either not
    /// a valid HTTP header name, or one the transport owns.
    #[error("the registered server's header name is not usable")]
    InvalidHeader,
    /// The device holds a credential for this server but could not open it —
    /// the local store is unavailable, or the device key has rotated. Distinct
    /// from [`Self::MissingBearer`] because "unusable" and "absent" lead a user
    /// to different fixes.
    #[error("the registered server's device credential could not be opened")]
    CredentialUnavailable,
}

/// An access token proven, at construction time, to be authorized for
/// exactly the bundled DA MCP server this process is about to call.
///
/// This is the only type `super::client` accepts to attach an
/// `Authorization` header. There is no API anywhere in `super::client` or
/// `super::executor` that accepts a bearer sourced from tool arguments, model
/// output, or a skill — the only way to obtain an `McpAuth` is
/// [`McpAuth::bind`], which requires an already-authenticated session bearer
/// plus the resource DA itself said that bearer was minted for. A model,
/// tool, or skill cannot forge or widen that binding: it never sees this
/// type or any constructor for it.
pub(crate) struct McpAuth {
    scheme: McpAuthScheme,
}

/// How one server's requests are authenticated.
///
/// `Anonymous` exists because a registered server may legitimately require
/// nothing (`auth_type = none`); modelling that as an empty bearer would send
/// `Authorization: Bearer ` and let a misconfiguration look like a server
/// problem.
enum McpAuthScheme {
    Anonymous,
    Bearer(Zeroizing<String>),
    Header {
        name: HeaderName,
        value: Zeroizing<String>,
    },
}

impl McpAuth {
    /// Binds `bearer` to `server`, requiring proof (`authorized_resource`)
    /// that the token's audience is actually this server's origin.
    ///
    /// `authorized_resource` is the `resource` value DA's Protected Resource
    /// Metadata returned at discovery time (`DiscoveredAuth::resource`) —
    /// i.e. what the token was minted for — not anything derived from the
    /// server descriptor itself. Binding fails closed on any ambiguity:
    /// empty/whitespace bearer, unparsable resource, or a resource whose
    /// origin does not match `server`'s DA origin.
    pub(crate) fn bind(
        bearer: Zeroizing<String>,
        authorized_resource: &str,
        server: &McpServerDescriptor,
    ) -> Result<Self, McpAuthError> {
        if bearer.trim().is_empty() {
            return Err(McpAuthError::MissingBearer);
        }
        if !resource_matches_server(authorized_resource, server) {
            return Err(McpAuthError::AudienceMismatch);
        }
        Ok(Self {
            scheme: McpAuthScheme::Bearer(bearer),
        })
    }

    /// Binds a device-held credential to one *registered* server.
    ///
    /// There is no audience proof to check here, and deliberately so: a
    /// registered server is its own resource, authenticated by a value the user
    /// entered for that specific server and that
    /// `super::credential_store::McpCredentialStore` sealed under that server's
    /// id. The binding is enforced by the AEAD rather than by a URL comparison
    /// — a credential sealed for one server id cannot be opened for another.
    ///
    /// Never reachable with the signed-in session bearer: the only caller reads
    /// from the per-server credential store, and the session bearer is not in
    /// it.
    pub(crate) fn bind_registered(
        credential: Zeroizing<String>,
        requirement: &RegisteredAuth,
    ) -> Result<Self, McpAuthError> {
        let scheme = match requirement {
            RegisteredAuth::None => McpAuthScheme::Anonymous,
            RegisteredAuth::Bearer | RegisteredAuth::Oauth => {
                if credential.trim().is_empty() {
                    return Err(McpAuthError::MissingBearer);
                }
                McpAuthScheme::Bearer(credential)
            }
            RegisteredAuth::Header { name } => {
                if credential.trim().is_empty() {
                    return Err(McpAuthError::MissingBearer);
                }
                let name =
                    HeaderName::try_from(name.as_str()).map_err(|_| McpAuthError::InvalidHeader)?;
                if RESERVED_HEADERS.contains(&name) {
                    // A registered row must not be able to set the transport's
                    // own headers: doing so would let a registration override
                    // session id, protocol version, or content negotiation.
                    return Err(McpAuthError::InvalidHeader);
                }
                McpAuthScheme::Header {
                    name,
                    value: credential,
                }
            }
        };
        Ok(Self { scheme })
    }

    /// The value for `Authorization: Bearer <value>`, when this scheme uses one.
    /// Not exposed outside the `mcp` module.
    pub(super) fn bearer_value(&self) -> Option<&str> {
        match &self.scheme {
            McpAuthScheme::Bearer(bearer) => Some(bearer),
            McpAuthScheme::Anonymous | McpAuthScheme::Header { .. } => None,
        }
    }

    /// The custom header this scheme attaches, when it uses one.
    pub(super) fn custom_header(&self) -> Option<(HeaderName, HeaderValue)> {
        match &self.scheme {
            McpAuthScheme::Header { name, value } => {
                let mut header = HeaderValue::from_str(value).ok()?;
                header.set_sensitive(true);
                Some((name.clone(), header))
            }
            McpAuthScheme::Anonymous | McpAuthScheme::Bearer(_) => None,
        }
    }
}

/// What a registered server expects, mirroring
/// `super::registry::ServerAuthRequirement` without importing it (this module
/// is below `registry` in the dependency order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RegisteredAuth {
    None,
    Bearer,
    Header {
        name: String,
    },
    /// The access token of a grant this device obtained through
    /// `super::oauth`, attached as a bearer. Indistinguishable from
    /// [`Self::Bearer`] on the wire — and deliberately distinct here, because
    /// where the value comes from decides what happens when the server refuses
    /// it: a static token needs the user to paste a new one, a grant is renewed
    /// without involving them.
    Oauth,
}

/// Headers a registered server may never claim: the Streamable HTTP transport
/// owns them, and letting a registration set one would let it redirect or
/// reshape the session rather than merely authenticate to it.
static RESERVED_HEADERS: &[HeaderName] = &[
    http::header::AUTHORIZATION,
    http::header::ACCEPT,
    http::header::CONTENT_TYPE,
    http::header::HOST,
    http::header::COOKIE,
];

impl std::fmt::Debug for McpAuth {
    /// Never format the bearer itself.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("McpAuth").finish_non_exhaustive()
    }
}

/// Supplies a freshly bound [`McpAuth`] on demand.
///
/// `super::client` calls this once to establish a session and again on every
/// reconnect (e.g. after the bundled server responds `401`, indicating the
/// previous bearer expired) — never caching a bearer itself beyond the
/// lifetime of one connection attempt. Implementations typically wrap
/// `crate::auth::AuthSessionStore::access_token()` plus the last discovered
/// resource; tests use a narrower in-memory fake.
///
/// Known follow-up (out of Task 2's scope — no production implementation of
/// this trait exists yet): `AuthSessionStore` does not currently persist
/// `DiscoveredAuth::resource` alongside the bearer. Whichever task wires a
/// real implementation must either have `AuthSessionStore` retain that
/// value from the `discover_auth` call that authenticated the session, or
/// re-run `discover_auth` at connect time — see the decision record
/// (`docs/decisions.md`) for the full note.
#[async_trait::async_trait]
pub(crate) trait McpAuthProvider: Send + Sync {
    /// Async because one implementation renews an OAuth grant here
    /// (`super::runtime::OauthGrantAuthProvider`): the access token a connect
    /// needs may be minutes from expiry, and the renewal is an HTTP request. The
    /// alternative — renewing on a timer elsewhere — would have every reconnect
    /// depend on that timer having run, and would attach an expired token
    /// whenever it had not.
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError>;

    /// Forgets the credential this provider holds, after the server refused it.
    ///
    /// Best-effort and infallible by design: it runs *after* the refusal is
    /// already being reported, so a failure to evict must not mask the fault
    /// that caused it.
    ///
    /// Defaults to doing nothing, which is the correct behavior for every
    /// provider whose material the user typed. Deleting a pasted bearer because
    /// a server rejected it would destroy something only the user can restore,
    /// and the sign-in prompt cannot mint a replacement for it. Only a grant
    /// this app can re-obtain through OAuth is worth dropping — see
    /// `super::runtime::OauthGrantAuthProvider`.
    fn forget(&self) {}
}

pub(crate) fn resource_matches_server(
    authorized_resource: &str,
    server: &McpServerDescriptor,
) -> bool {
    let Ok(resource_url) = reqwest::Url::parse(authorized_resource.trim()) else {
        return false;
    };
    if !resource_url.username().is_empty() || resource_url.password().is_some() {
        return false;
    }
    resource_url.origin() == server.da_origin().origin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthConfig;
    use crate::mcp::server_resolver::McpServerResolver;

    fn server() -> McpServerDescriptor {
        let config = AuthConfig {
            da_base_url: reqwest::Url::parse("https://tenant.example").unwrap(),
            casdoor_issuer: reqwest::Url::parse("https://auth.example").unwrap(),
            client_id: "client".to_owned(),
            organization: "org".to_owned(),
            loopback_port: 19847,
            mcp_client_id: None,
            mcp_client_secret: None,
        };
        McpServerResolver::resolve(&config)
    }

    #[test]
    fn binds_when_resource_matches_server_origin() {
        let server = server();
        let bound = McpAuth::bind(
            Zeroizing::new("token-value".to_owned()),
            "https://tenant.example/mcp",
            &server,
        );
        assert!(bound.is_ok());
        assert_eq!(bound.unwrap().bearer_value(), Some("token-value"));
    }

    #[test]
    fn binds_when_resource_is_bare_origin() {
        // DA's PRM may return just the tenant origin rather than a path
        // suffixed with `/mcp`; either is the same resource server.
        let server = server();
        let bound = McpAuth::bind(
            Zeroizing::new("token-value".to_owned()),
            "https://tenant.example",
            &server,
        );
        assert!(bound.is_ok());
    }

    #[test]
    fn rejects_audience_mismatch_different_origin() {
        let server = server();
        let err = McpAuth::bind(
            Zeroizing::new("token-value".to_owned()),
            "https://attacker.example/mcp",
            &server,
        )
        .unwrap_err();
        assert_eq!(err, McpAuthError::AudienceMismatch);
    }

    #[test]
    fn rejects_audience_mismatch_same_host_different_port() {
        let server = server();
        let err = McpAuth::bind(
            Zeroizing::new("token-value".to_owned()),
            "https://tenant.example:8443/mcp",
            &server,
        )
        .unwrap_err();
        assert_eq!(err, McpAuthError::AudienceMismatch);
    }

    #[test]
    fn rejects_unparsable_resource() {
        let server = server();
        let err = McpAuth::bind(
            Zeroizing::new("token-value".to_owned()),
            "not a url",
            &server,
        )
        .unwrap_err();
        assert_eq!(err, McpAuthError::AudienceMismatch);
    }

    #[test]
    fn rejects_resource_with_embedded_credentials() {
        let server = server();
        let err = McpAuth::bind(
            Zeroizing::new("token-value".to_owned()),
            "https://user:pass@tenant.example/mcp",
            &server,
        )
        .unwrap_err();
        assert_eq!(err, McpAuthError::AudienceMismatch);
    }

    #[test]
    fn rejects_empty_bearer() {
        let server = server();
        let err = McpAuth::bind(
            Zeroizing::new(String::new()),
            "https://tenant.example/mcp",
            &server,
        )
        .unwrap_err();
        assert_eq!(err, McpAuthError::MissingBearer);
    }

    #[test]
    fn rejects_whitespace_only_bearer() {
        let server = server();
        let err = McpAuth::bind(
            Zeroizing::new("   ".to_owned()),
            "https://tenant.example/mcp",
            &server,
        )
        .unwrap_err();
        assert_eq!(err, McpAuthError::MissingBearer);
    }

    #[test]
    fn debug_never_prints_bearer_value() {
        let server = server();
        let bound = McpAuth::bind(
            Zeroizing::new("super-secret-token".to_owned()),
            "https://tenant.example/mcp",
            &server,
        )
        .unwrap();
        let rendered = format!("{bound:?}");
        assert!(!rendered.contains("super-secret-token"));
    }
}

#[cfg(test)]
mod registered_auth_tests {
    use super::{McpAuth, McpAuthError, RegisteredAuth};
    use zeroize::Zeroizing;

    fn credential() -> Zeroizing<String> {
        Zeroizing::new("registered-server-credential".to_owned())
    }

    #[test]
    fn an_open_server_attaches_nothing() {
        let auth = McpAuth::bind_registered(Zeroizing::new(String::new()), &RegisteredAuth::None)
            .expect("an open server needs no credential");
        assert_eq!(auth.bearer_value(), None);
        assert!(auth.custom_header().is_none());
    }

    #[test]
    fn a_bearer_server_attaches_only_the_authorization_header() {
        let auth = McpAuth::bind_registered(credential(), &RegisteredAuth::Bearer)
            .expect("a bearer server binds");
        assert_eq!(auth.bearer_value(), Some("registered-server-credential"));
        assert!(auth.custom_header().is_none());
    }

    #[test]
    fn a_header_server_attaches_only_its_custom_header_and_marks_it_sensitive() {
        let auth = McpAuth::bind_registered(
            credential(),
            &RegisteredAuth::Header {
                name: "X-Acme-Key".to_owned(),
            },
        )
        .expect("a header server binds");
        assert_eq!(auth.bearer_value(), None);
        let (name, value) = auth.custom_header().expect("a custom header");
        assert_eq!(name.as_str(), "x-acme-key");
        assert!(
            value.is_sensitive(),
            "a credential header must be marked sensitive so it is redacted in logs"
        );
    }

    /// A grant's access token is attached exactly as a bearer, and nothing else
    /// is: whatever an authorization server returned, it reaches the wire the way
    /// RFC 6750 §2.1 says a bearer does.
    #[test]
    fn an_oauth_server_attaches_its_grant_as_a_bearer_and_nothing_else() {
        let auth = McpAuth::bind_registered(
            Zeroizing::new("granted-access-token".to_owned()),
            &RegisteredAuth::Oauth,
        )
        .expect("a grant binds");
        assert_eq!(auth.bearer_value(), Some("granted-access-token"));
        assert!(auth.custom_header().is_none());
        assert!(!format!("{auth:?}").contains("granted-access-token"));
    }

    #[test]
    fn an_empty_credential_is_refused_for_any_scheme_that_needs_one() {
        for requirement in [
            RegisteredAuth::Bearer,
            RegisteredAuth::Oauth,
            RegisteredAuth::Header {
                name: "X-Acme-Key".to_owned(),
            },
        ] {
            assert_eq!(
                McpAuth::bind_registered(Zeroizing::new("   ".to_owned()), &requirement).err(),
                Some(McpAuthError::MissingBearer),
                "{requirement:?} must refuse an empty credential"
            );
        }
    }

    #[test]
    fn a_registered_server_cannot_claim_a_header_the_transport_owns() {
        for reserved in [
            "authorization",
            "Authorization",
            "accept",
            "content-type",
            "host",
            "cookie",
        ] {
            assert_eq!(
                McpAuth::bind_registered(
                    credential(),
                    &RegisteredAuth::Header {
                        name: reserved.to_owned()
                    }
                )
                .err(),
                Some(McpAuthError::InvalidHeader),
                "{reserved} must not be settable by a registration"
            );
        }
    }

    #[test]
    fn a_malformed_header_name_is_refused_rather_than_sanitized() {
        for malformed in ["X Bad", "X-Bad\r\nInjected: 1", "", "X:Bad"] {
            assert_eq!(
                McpAuth::bind_registered(
                    credential(),
                    &RegisteredAuth::Header {
                        name: malformed.to_owned()
                    }
                )
                .err(),
                Some(McpAuthError::InvalidHeader),
                "{malformed:?} must be refused"
            );
        }
    }

    #[test]
    fn a_credential_never_appears_in_debug_output() {
        let auth = McpAuth::bind_registered(credential(), &RegisteredAuth::Bearer)
            .expect("a bearer server binds");
        let rendered = format!("{auth:?}");
        assert!(
            !rendered.contains("registered-server-credential"),
            "{rendered}"
        );
    }
}
