//! Identity of an MCP server this application will connect to.
//!
//! Two kinds, and the distinction is load-bearing:
//!
//! - The **bundled tenant server**, from [`McpServerResolver::resolve`]. Its
//!   endpoint is a pure function of trusted, environment-pinned [`AuthConfig`]
//!   (`<da_base_url>/mcp`) — never a caller, tool, or model argument — and its
//!   bearer is the signed-in session's, audience-bound to the DA resource by
//!   [`super::auth::McpAuth::bind`].
//! - A **user-registered server**, from [`McpServerDescriptor::registered`].
//!   Its endpoint comes from a row in DA's shared registry, so it is data
//!   another product wrote, and it is only ever constructed after
//!   [`super::registry::validate_endpoint`] has passed it. Every "may we dial
//!   that?" rule lives there, in one place, not here.
//!
//! Neither kind is ever built from a tool argument or model output: nothing in
//! `super::client`/`super::executor` accepts a URL, and the only constructors
//! are the two above.

use reqwest::Url;

use crate::auth::AuthConfig;

use super::registry::ENFORCE_PUBLIC_MCP_HOSTS;

/// One MCP server's identity: the Streamable HTTP endpoint to dial, plus the
/// origin whose authorized resource `super::auth` compares a bearer against —
/// never an arbitrary origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpServerDescriptor {
    endpoint: Url,
    da_origin: Url,
    /// Whether every DNS name this server is reached by must resolve only to
    /// public addresses. For a user-registered server this follows
    /// [`ENFORCE_PUBLIC_MCP_HOSTS`]; always false for the bundled one, whose
    /// on-prem origin may legitimately be a private name.
    public_hosts_only: bool,
}

impl McpServerDescriptor {
    /// The Streamable HTTP endpoint to connect to.
    #[must_use]
    pub(crate) fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// The origin this server belongs to: DA's for the bundled server, the
    /// endpoint's own for a registered one. Used only for same-origin audience
    /// comparisons in `super::auth`; never dialed directly.
    #[must_use]
    pub(crate) fn da_origin(&self) -> &Url {
        &self.da_origin
    }

    /// A stable, human/log-safe identity string for cache-keying
    /// (`super::catalog`) and diagnostics. Deliberately just the endpoint —
    /// no credentials or query/fragment ever reach this type.
    #[must_use]
    pub(crate) fn identity(&self) -> String {
        self.endpoint.as_str().to_owned()
    }

    /// Whether this server's DNS names must resolve only to public addresses.
    #[must_use]
    pub(crate) fn public_hosts_only(&self) -> bool {
        self.public_hosts_only
    }

    /// A user-registered server's descriptor.
    ///
    /// `endpoint` must already have come through
    /// [`super::registry::validate_endpoint`] — that is the only caller, and it
    /// is where every "may we dial that?" rule lives. `da_origin` is the
    /// endpoint's own origin: a registered server is its own resource, so the
    /// audience comparison in [`super::auth`] has nothing to do with DA here.
    /// It is deliberately not usable for the bundled server's session bearer,
    /// which [`McpAuth::bind`](super::auth::McpAuth::bind) still binds against
    /// the DA origin only.
    #[must_use]
    pub(crate) fn registered(endpoint: Url) -> Self {
        let mut origin = endpoint.clone();
        origin.set_path("/");
        Self {
            endpoint,
            da_origin: origin,
            // Follows the policy switch: while `ENFORCE_PUBLIC_MCP_HOSTS` is off,
            // a registered server may be on a private address, so no resolver
            // check applies to it.
            public_hosts_only: ENFORCE_PUBLIC_MCP_HOSTS,
        }
    }
}

/// Resolves the bundled DA MCP server. Stateless by design: its endpoint is a
/// pure function of trusted config, not a lookup against caller-supplied data.
pub(crate) struct McpServerResolver;

impl McpServerResolver {
    /// Returns the one bundled DA MCP server descriptor.
    ///
    /// `config.da_base_url` is already validated (https, no path/query/
    /// fragment — see `AuthConfig::from_env`) by the time it reaches here, so
    /// appending the fixed `/mcp` path cannot fail; resolution is infallible.
    #[must_use]
    pub(crate) fn resolve(config: &AuthConfig) -> McpServerDescriptor {
        let mut endpoint = config.da_base_url.clone();
        endpoint.set_path("/mcp");
        McpServerDescriptor {
            endpoint,
            da_origin: config.da_base_url.clone(),
            public_hosts_only: false,
        }
    }
}
