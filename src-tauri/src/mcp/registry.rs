//! Turns DA `mcp_servers` rows into things this build will actually dial.
//!
//! `crate::clients::da::mcp_servers` reads the shared registry faithfully and
//! decides nothing. This module is where every "may we connect to that?"
//! question is answered, in one place:
//!
//! - is the URL one this build is willing to dial ([`validate_endpoint`]);
//! - is the transport MCP Streamable HTTP;
//! - is the auth method one this build implements;
//! - does this device hold the credential the server needs;
//! - what namespace do its tools live under ([`slug_for`]).
//!
//! A row that fails any of those is still *listed* — with the reason — rather
//! than hidden. A user who registered a server on the web must be able to see
//! why the desktop is not using it, instead of wondering where it went.
//!
//! # Why the endpoint check is stricter than "is it a URL"
//!
//! A registered URL is data another product wrote into a shared table. It is
//! not attacker-controlled in the usual sense, but it is not compiled-in
//! configuration either, and it becomes the destination of an authenticated
//! outbound request carrying a credential. So it gets the same treatment as
//! any other request-shaping input: no embedded userinfo, no query or
//! fragment, and a web scheme. Those URL-shape rules hold in every mode.
//!
//! The public-hosts rules (https for a non-private host, and no host in a
//! range that would turn the desktop into a probe against the user's own
//! network, `docs/threat-model.md`, TM-005) are behind
//! [`ENFORCE_PUBLIC_MCP_HOSTS`]. They are switched off because on-prem and
//! private deployments run their MCP server on a private address.

use std::net::IpAddr;

use reqwest::Url;
use xfalcon_contracts::mcp::{
    McpAuthType, McpPermissionMode, McpServerUnavailableReason, McpServerView, McpSignInStatus,
    McpTransport, BUNDLED_SERVER_ID,
};

use crate::clients::da::mcp_servers::McpServerRecord;

use super::auth::RegisteredAuth;
use super::server_resolver::McpServerDescriptor;

/// Longest slug a server name may produce. Tool ids are `mcp.<slug>.<tool>`
/// and the wire name a provider sees is capped at 64 bytes
/// (`crate::agent::tool_session::wire_tool_name`), so a slug that ate the
/// whole budget would leave no room for the tool's own name.
const MAX_SLUG_BYTES: usize = 24;

/// A registered server this build is willing to connect to, plus what it needs
/// to authenticate. Produced only by [`classify`], so an unusable row cannot
/// reach the runtime by another route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DialableServer {
    pub(crate) id: String,
    pub(crate) slug: String,
    /// Human-facing name for this server, for a surface that has to say which
    /// one it means — see `super::runtime::McpServerIdentity`.
    pub(crate) label: String,
    pub(crate) descriptor: McpServerDescriptor,
    pub(crate) auth: ServerAuthRequirement,
    pub(crate) permission_mode: McpPermissionMode,
    /// A digest of the row's `auth_config` ciphertext, when it has one.
    ///
    /// Part of this type — and therefore of the runtime's cache key — so that an
    /// operator rotating a server's OAuth client on the web app invalidates the
    /// warm session built from the previous one. Never the value itself; see
    /// `crate::clients::da::mcp_servers::StoredAuthConfig::fingerprint`.
    pub(crate) auth_fingerprint: Option<String>,
}

/// What must be attached to this server's requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ServerAuthRequirement {
    /// Nothing. The server is open, or authenticates by network position.
    None,
    /// The device-held value as `Authorization: Bearer <value>`.
    Bearer,
    /// The device-held value in this custom header.
    Header { name: String },
    /// The access token of a grant this device holds (`super::oauth`), renewed
    /// on demand. Distinct from [`Self::Bearer`] because the recovery differs:
    /// a refused static token needs the user to paste a new one, a refused grant
    /// is renewed or re-established by a browser flow.
    Oauth,
}

impl From<&ServerAuthRequirement> for RegisteredAuth {
    /// `super::auth` sits below this module and so mirrors this vocabulary
    /// rather than importing it; this is the one place the two are tied
    /// together, so they cannot drift silently.
    fn from(requirement: &ServerAuthRequirement) -> Self {
        match requirement {
            ServerAuthRequirement::None => Self::None,
            ServerAuthRequirement::Bearer => Self::Bearer,
            ServerAuthRequirement::Header { name } => Self::Header { name: name.clone() },
            ServerAuthRequirement::Oauth => Self::Oauth,
        }
    }
}

/// One row's verdict: either dialable, or listed with a reason it is not.
///
/// The dialable payload is boxed because it is two orders of magnitude larger
/// than the reason, and most rows in a mixed registry are one or the other —
/// an unboxed enum would size every element of the classified list to the
/// larger arm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ServerVerdict {
    Dialable(Box<DialableServer>),
    Unavailable(McpServerUnavailableReason),
}

/// What this device holds as one `oauth` row's operator-issued client.
///
/// The identifier travels in the clear because it names a registration rather
/// than authenticating one; the value that does authenticate it is reported
/// only as [`Self::confidential`], and never leaves
/// `super::oauth::store::DeviceOauthClients`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceOauthClient {
    pub(crate) client_id: String,
    /// Whether a confidential value is sealed for that identifier.
    pub(crate) confidential: bool,
}

/// Whether registered MCP servers and their OAuth endpoints must reach only
/// public addresses. This is the one switch for every public-hosts rule.
///
/// `false` (the shipped policy): on-prem and private deployments run their MCP
/// server on `localhost`, a loopback or private address, or a private name,
/// often over plain `http` on the LAN, and cannot resolve to a public address.
/// With the switch off, [`validate_endpoint`] accepts those hosts. Cleartext
/// `http` is accepted for a host [`host_looks_private`] names, and for any host
/// in a debug build as before. The OAuth endpoint gate accepts the same hosts.
///
/// `true` restores every public-hosts rule at once. The rules are the ones that
/// shipped before the switch, with one difference that applies in both modes: the
/// address classification below now also judges NAT64/SIIT/6to4 addresses by the
/// IPv4 they carry and refuses the whole of `0.0.0.0/8`:
/// - the save-time host check (`host_is_dialable_with`), which refuses IP
///   literals and `localhost` that are not public;
/// - https only, outside debug builds, for every host;
/// - the connect-time `PublicOnlyResolver` in the `dns` module, installed for
///   registered servers (`McpServerDescriptor::registered`) and for the OAuth
///   client (`oauth::build_client`);
/// - the OAuth endpoint gate's public-host check (`oauth::endpoint_is_usable`).
///
/// The classification helpers (`address_is_dialable` and the range checks below
/// it) are not behind the switch. They are the policy when it is `true`, and
/// `uploads::da_repository` uses `address_is_dialable` whatever the switch says.
/// The `_with` functions take the policy as an argument so both modes are
/// tested without changing this constant.
pub(crate) const ENFORCE_PUBLIC_MCP_HOSTS: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum EndpointError {
    #[error("an MCP server URL must be an absolute https URL")]
    NotHttps,
    #[error("an MCP server URL must not embed credentials")]
    EmbeddedUserinfo,
    #[error("an MCP server URL must not carry a query string or fragment")]
    QueryOrFragment,
    #[error("this build will not dial that host")]
    HostRejected,
    #[error("an MCP server URL must name a host")]
    MissingHost,
}

impl From<EndpointError> for McpServerUnavailableReason {
    fn from(_: EndpointError) -> Self {
        // One reason for every endpoint rejection: which rule a URL broke is a
        // diagnostic, not something the row's author gets to learn in detail.
        Self::InvalidUrl
    }
}

/// Validates a registered server URL under [`ENFORCE_PUBLIC_MCP_HOSTS`] and
/// returns the endpoint to dial.
///
/// Unlike the bundled server (whose endpoint is always `<DA origin>/mcp`), a
/// registered URL is used exactly as given: the path is the server's own, and
/// this build does not guess at appending `/mcp`.
pub(crate) fn validate_endpoint(raw: &str) -> Result<Url, EndpointError> {
    validate_endpoint_with(raw, ENFORCE_PUBLIC_MCP_HOSTS)
}

/// [`validate_endpoint`] with the public-hosts policy passed in. `enforce` is
/// [`ENFORCE_PUBLIC_MCP_HOSTS`] for the public function.
///
/// Refused in both modes, because they are malformed-URL guards and not
/// deployment policy: a non-web scheme, embedded userinfo, a query or fragment,
/// and a missing host. Cleartext `http` is refused in a release build unless
/// `enforce` is off and the host is one [`host_looks_private`] names.
pub(crate) fn validate_endpoint_with(raw: &str, enforce: bool) -> Result<Url, EndpointError> {
    let url = Url::parse(raw).map_err(|_| EndpointError::NotHttps)?;
    let cleartext_allowed =
        cfg!(debug_assertions) || (!enforce && url.host_str().is_some_and(host_looks_private));
    let scheme_allowed = match url.scheme() {
        "https" => true,
        "http" => cleartext_allowed,
        _ => false,
    };
    if !scheme_allowed {
        return Err(EndpointError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EndpointError::EmbeddedUserinfo);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(EndpointError::QueryOrFragment);
    }
    let host = url.host_str().ok_or(EndpointError::MissingHost)?;
    if !host_is_dialable_with(host, enforce) {
        return Err(EndpointError::HostRejected);
    }
    Ok(url)
}

/// Whether `host` may be dialed under `enforce`. With `enforce` off every host
/// is dialable. With it on, this rejects hosts that would point an authenticated
/// request at the user's own network rather than at a tenant service. Loopback
/// is permitted in debug builds only, so local MCP development works without
/// shipping that allowance.
pub(crate) fn host_is_dialable_with(host: &str, enforce: bool) -> bool {
    if !enforce {
        return true;
    }
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    // A name with a trailing dot is the same host, so `localhost.` and `127.0.0.1.`
    // must not slip past the checks below.
    let literal = literal.strip_suffix('.').unwrap_or(literal);
    let Ok(address) = literal.parse::<IpAddr>() else {
        // A DNS name. The addresses it resolves to are checked at connection
        // time by `PublicOnlyResolver` in the `dns` module, which is the only
        // resolution the transport performs, so the checked addresses are the
        // dialled ones.
        return !literal.eq_ignore_ascii_case("localhost") || cfg!(debug_assertions);
    };
    address_is_dialable(address, cfg!(debug_assertions))
}

/// Name suffixes that stay on the user's own network. Matched against the
/// lowercased name, so each carries its leading dot.
const PRIVATE_NAME_SUFFIXES: &[&str] = &[
    ".localhost",
    ".local",
    ".internal",
    ".lan",
    ".corp",
    ".intranet",
    ".localdomain",
    ".home.arpa",
];

/// Whether `host` names a destination on the user's own network, judged by its
/// spelling alone and never by resolving it: a loopback, private, link-local,
/// CGNAT or otherwise non-public IP literal, `localhost`, a single-label name,
/// or a name under one of [`PRIVATE_NAME_SUFFIXES`].
///
/// This decides only whether cleartext `http` is accepted while the public-hosts
/// rules are off. A public-looking name such as `mcp.example.com` is not
/// private, whatever it resolves to.
pub(crate) fn host_looks_private(host: &str) -> bool {
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    let name = literal.strip_suffix('.').unwrap_or(literal);
    if let Ok(address) = name.parse::<IpAddr>() {
        return !address_is_dialable(address, false);
    }
    let name = name.to_ascii_lowercase();
    name == "localhost"
        || !name.contains('.')
        || PRIVATE_NAME_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(*suffix))
}

/// Classifies one resolved or literal address. Loopback is dialable only when
/// `allow_loopback` is set; every other non-public range is refused regardless.
pub(crate) fn address_is_dialable(address: IpAddr, allow_loopback: bool) -> bool {
    // Folded before anything is classified, so one set of range rules covers
    // both spellings of an IPv4 destination.
    let address = ipv4_mapped_folded(address);
    if address.is_loopback() {
        return allow_loopback;
    }
    if address.is_unspecified() || address.is_multicast() {
        return false;
    }
    match address {
        IpAddr::V4(v4) => ipv4_is_dialable(v4),
        IpAddr::V6(v6) => ipv6_is_dialable(v6),
    }
}

/// Rewrites an IPv6 address that carries an IPv4 destination as that IPv4
/// address, so one set of range rules covers every spelling of the destination.
///
/// Every `Ipv6Addr` range predicate — `is_loopback` included — treats the mapped
/// block as ordinary global unicast, because at the IPv6 layer that is what it
/// is. So `[::ffff:127.0.0.1]` and `[::ffff:10.0.0.5]` would pass an IPv6-shaped
/// check while naming exactly the destinations this gate exists to refuse, and
/// the transport would dial the IPv4 address with the device credential
/// attached. Folding first is what makes the IPv4 rules authoritative for both
/// spellings instead of only the short one.
fn ipv4_mapped_folded(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => embedded_ipv4(v6).map_or(address, IpAddr::V4),
        IpAddr::V4(_) => address,
    }
}

/// The IPv4 destination an IPv6 address stands for, when its prefix says so:
/// IPv4-mapped (`::ffff:0:0/96`), the well-known NAT64 prefix (`64:ff9b::/96`,
/// RFC 6052), SIIT (`::ffff:0:0:0/96`, RFC 2765) and 6to4 (`2002::/16`).
///
/// The well-known NAT64 prefix matters in both directions. A DNS64 resolver on
/// an IPv6-only network answers a public IPv4-only server with
/// `64:ff9b::<public v4>`, and refusing the prefix outright would make every
/// such server unreachable; a NAT64 gateway also dials the embedded address, so
/// `64:ff9b::a00:5` is a way to name `10.0.0.5`. Judging the embedded address
/// handles both.
fn embedded_ipv4(address: std::net::Ipv6Addr) -> Option<std::net::Ipv4Addr> {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return Some(mapped);
    }
    let segments = address.segments();
    let from = |high: u16, low: u16| {
        let [a, b] = high.to_be_bytes();
        let [c, d] = low.to_be_bytes();
        std::net::Ipv4Addr::new(a, b, c, d)
    };
    if segments[..6] == [0x0064, 0xff9b, 0, 0, 0, 0] || segments[..6] == [0, 0, 0, 0, 0xffff, 0] {
        return Some(from(segments[6], segments[7]));
    }
    if segments[0] == 0x2002 {
        return Some(from(segments[1], segments[2]));
    }
    None
}

/// `is_shared` is still unstable for `Ipv4Addr`, so its range is matched
/// directly.
///
/// The non-routable ranges (documentation, benchmarking, reserved) are
/// deliberately *not* refused: a request there cannot reach anything, so
/// refusing it buys no containment, and they are the conventional literals for
/// fixtures and docs.
fn ipv4_is_dialable(address: std::net::Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    // 100.64.0.0/10 (RFC 6598). On a consumer ISP this is the subscriber's own
    // side of the carrier NAT — the same thing 10/8 is on a corporate network,
    // and reachable from the desktop for exactly the same reason.
    let shared = first == 100 && (64..128).contains(&second);
    // 0.0.0.0/8 ("this network"): some stacks route it to the local host.
    let this_network = first == 0;
    !(address.is_private()
        || address.is_link_local()
        || address.is_broadcast()
        || shared
        || this_network)
}

/// `is_unique_local`/`is_unicast_link_local` are still unstable for `Ipv6Addr`,
/// so the prefixes are matched directly.
fn ipv6_is_dialable(address: std::net::Ipv6Addr) -> bool {
    let segments = address.segments();
    // fc00::/7 (unique local) and fe80::/10 (link local).
    if (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    // 64:ff9b::/96 never reaches here: `embedded_ipv4` has folded it to its
    // IPv4 address. What is left of the prefix, 64:ff9b:1::/48 (RFC 8215), is
    // local-use space whose embedded address a translator dials on our behalf,
    // so the IPv4 rules never see the real destination.
    if segments[0] == 0x0064 && segments[1] == 0xff9b {
        return false;
    }
    // The rest of ::/80 — it includes the deprecated IPv4-compatible form
    // (`::a.b.c.d`, ::/96), which resolvers and translators handle
    // inconsistently. `::` and `::1` are already refused above; nothing
    // legitimate is reachable in the remainder, and the IPv4-mapped and SIIT
    // blocks that share this prefix have been folded away.
    segments[..5] != [0, 0, 0, 0, 0]
}

/// Derives a server's tool-namespace segment from its DA name.
///
/// Lowercased, non-alphanumerics collapsed to `-`, bounded. Two servers whose
/// names collapse to the same slug are disambiguated by [`classify`], not
/// here, so this stays a pure function of one name.
pub(crate) fn slug_for(name: &str) -> String {
    let mut slug = String::with_capacity(name.len().min(MAX_SLUG_BYTES));
    let mut last_was_dash = false;
    for character in name.chars() {
        if slug.len() >= MAX_SLUG_BYTES {
            break;
        }
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_was_dash = false;
        } else if !slug.is_empty() && !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_end_matches('-');
    if trimmed.is_empty() {
        // A name of only punctuation still needs a namespace. `classify`
        // uniquifies it against the rest.
        return "server".to_owned();
    }
    trimmed.to_owned()
}

/// Whether a name may be registered. Rejects the bundled server's reserved
/// slug so a DA row can never shadow the tenant server's tool namespace.
pub(crate) fn name_is_registrable(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 100
        && slug_for(trimmed) != BUNDLED_SERVER_ID
        && !trimmed.eq_ignore_ascii_case(BUNDLED_SERVER_ID)
}

/// Classifies every additional registered row, in DA's order, against what this
/// build and this device can do. A row that aliases the bundled endpoint is not
/// additional and is omitted.
///
/// `has_device_credential` answers "is there a credential sealed on this device
/// for this server id?". It is a callback rather than a store so this module
/// stays pure and testable, and so the credential store can land without
/// touching this policy. `oauth_client` answers the same shape of question for
/// the operator-issued OAuth client this device holds, and is reported for
/// `oauth` rows only — it changes no verdict, it is what lets the edit sheet
/// show a saved client instead of an empty field.
///
/// Slugs are uniquified across the whole set (and against the bundled server's
/// reserved slug) by appending `-2`, `-3`, … — two servers named `Acme CRM` and
/// `acme-crm` must not both claim `acme-crm`, or one would silently shadow the
/// other's tools.
pub(crate) fn classify(
    records: &[McpServerRecord],
    bundled_endpoint: &Url,
    has_device_credential: &dyn Fn(&str) -> bool,
    sign_in_state: &dyn Fn(&str) -> McpSignInStatus,
    oauth_client: &dyn Fn(&str) -> Option<DeviceOauthClient>,
) -> Vec<(McpServerView, ServerVerdict)> {
    let mut taken = vec![BUNDLED_SERVER_ID.to_owned()];
    let mut classified = Vec::with_capacity(records.len());

    for record in records {
        // The shared registry is also consumed by xFalcon Web, where a row may
        // describe the tenant MCP endpoint itself. On desktop that endpoint is
        // already the bundled server and authenticates with the app session.
        // Treating the DA row as another identity creates a second session and,
        // when it has no separate OAuth grant, a false sign-in warning for an
        // endpoint the app is already using.
        if is_bundled_alias(record, bundled_endpoint) {
            continue;
        }
        let slug = unique_slug(&slug_for(&record.name), &mut taken);
        let verdict = verdict_for(record, &slug, has_device_credential, sign_in_state);
        // Looked up once. Same gate as `sign_in`: a client this device holds is
        // only meaningful for the method that uses one, and reporting it on a
        // bearer row would invite an edit that could never take effect.
        let held_client = record
            .auth_type
            .needs_sign_in()
            .then(|| oauth_client(&record.id))
            .flatten();
        let view = McpServerView {
            version: xfalcon_contracts::CONTRACT_VERSION,
            id: record.id.clone().into(),
            name: record.name.clone(),
            display_name: record.display_name.clone(),
            description: record.description.clone(),
            transport: record.transport,
            url: record.url.clone(),
            auth_type: record.auth_type,
            permission_mode: record.permission_mode,
            enabled: record.enabled,
            builtin: false,
            slug,
            unavailable: match &verdict {
                ServerVerdict::Dialable(_) => None,
                ServerVerdict::Unavailable(reason) => Some(*reason),
            },
            // Reported for every `oauth` row and no other, so the surface can
            // offer "Sign in" or "Sign out" without inferring which from the
            // absence of a failure reason — and can say which clients a sign-out
            // would affect.
            sign_in: record
                .auth_type
                .needs_sign_in()
                .then(|| sign_in_state(&record.id)),
            oauth_client_confidential: held_client
                .as_ref()
                .is_some_and(|client| client.confidential),
            oauth_client_id: held_client.map(|client| client.client_id),
        };
        classified.push((view, verdict));
    }
    classified
}

/// Whether a shared registry row names the endpoint this desktop already owns
/// as its bundled tenant connection.
///
/// `Url` equality supplies the safe normalization: host case and default ports
/// compare consistently, while distinct paths such as `/mcp` and `/mcp/` stay
/// distinct because a server may route them differently. Only remote HTTP rows
/// can alias the bundled Streamable HTTP connection; a `stdio` row carrying a
/// stray URL still describes a different transport and keeps its own verdict.
#[must_use]
pub(crate) fn is_bundled_alias(record: &McpServerRecord, bundled_endpoint: &Url) -> bool {
    matches!(record.transport, McpTransport::Http | McpTransport::Sse)
        && record
            .url
            .as_deref()
            .and_then(|raw| validate_endpoint(raw).ok())
            .is_some_and(|endpoint| endpoint == *bundled_endpoint)
}

fn verdict_for(
    record: &McpServerRecord,
    slug: &str,
    has_device_credential: &dyn Fn(&str) -> bool,
    sign_in_state: &dyn Fn(&str) -> McpSignInStatus,
) -> ServerVerdict {
    match record.transport {
        // Both name a remote HTTP endpoint, and the distinction DA records is
        // not one its writers make: xFalcon Web stamps `transport: 'sse'` on
        // every server it registers (`src/components/mcp/MCPPanel.tsx`) and then
        // speaks Streamable HTTP to it anyway — its client never branches on the
        // column except to refuse `stdio`. Honouring the label therefore refused
        // *every* server registered from the web app, working ones included, and
        // no client writes a label that would distinguish them.
        //
        // So the label is not treated as authority on the wire protocol; the
        // connection attempt is. A server that really only speaks the MCP spec's
        // superseded HTTP+SSE transport fails its `initialize` and is reported
        // as unreachable, which is a truthful answer the user can act on —
        // unlike refusing to try.
        McpTransport::Http | McpTransport::Sse => {}
        // A local-process server has no URL to dial, and `Other` is a transport
        // this build has no name for, let alone an implementation.
        McpTransport::Stdio | McpTransport::Other => {
            return ServerVerdict::Unavailable(McpServerUnavailableReason::UnsupportedTransport)
        }
    }
    if !record.auth_type.is_supported() {
        return ServerVerdict::Unavailable(McpServerUnavailableReason::UnsupportedAuthMethod);
    }
    let Some(raw_url) = record.url.as_deref() else {
        return ServerVerdict::Unavailable(McpServerUnavailableReason::InvalidUrl);
    };
    let endpoint = match validate_endpoint(raw_url) {
        Ok(endpoint) => endpoint,
        Err(error) => return ServerVerdict::Unavailable(error.into()),
    };
    let auth = match record.auth_type {
        McpAuthType::None => ServerAuthRequirement::None,
        McpAuthType::Bearer => ServerAuthRequirement::Bearer,
        McpAuthType::Header => match record.header_name.clone() {
            Some(name) => ServerAuthRequirement::Header { name },
            // `api_key` with no header to put it in is not a usable
            // registration, whoever wrote the row.
            None => {
                return ServerVerdict::Unavailable(
                    McpServerUnavailableReason::UnsupportedAuthMethod,
                )
            }
        },
        // Dialable exactly when this device holds a grant. The check is below,
        // with the credential one, so both "this device is not set up for that
        // server" answers are decided in one place.
        McpAuthType::Oauth => ServerAuthRequirement::Oauth,
        // Already rejected by `is_supported` above; kept exhaustive so a new
        // arm cannot silently become dialable.
        McpAuthType::Other => {
            return ServerVerdict::Unavailable(McpServerUnavailableReason::UnsupportedAuthMethod)
        }
    };
    if record.auth_type.needs_device_credential() && !has_device_credential(&record.id) {
        return ServerVerdict::Unavailable(McpServerUnavailableReason::MissingDeviceCredential);
    }
    if record.auth_type.needs_sign_in() && !sign_in_state(&record.id).is_usable() {
        // Distinct from `MissingDeviceCredential`: there is nothing for the user
        // to paste, and the fix is a browser flow this app runs for them.
        return ServerVerdict::Unavailable(McpServerUnavailableReason::NeedsSignIn);
    }
    ServerVerdict::Dialable(Box::new(DialableServer {
        id: record.id.clone(),
        slug: slug.to_owned(),
        // What a reconnect prompt calls this server. The row's own display name
        // when it has one, so the prompt and the Settings panel name it the same
        // way; the slug otherwise, which is derived from the row's name.
        label: record
            .display_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| slug.to_owned()),
        descriptor: McpServerDescriptor::registered(endpoint),
        auth,
        permission_mode: record.permission_mode,
        auth_fingerprint: record
            .auth_config
            .as_ref()
            .map(crate::clients::da::mcp_servers::StoredAuthConfig::fingerprint),
    }))
}

fn unique_slug(candidate: &str, taken: &mut Vec<String>) -> String {
    if !taken.iter().any(|existing| existing == candidate) {
        taken.push(candidate.to_owned());
        return candidate.to_owned();
    }
    for suffix in 2..=u16::MAX {
        let attempt = format!("{candidate}-{suffix}");
        if !taken.iter().any(|existing| existing == &attempt) {
            taken.push(attempt.clone());
            return attempt;
        }
    }
    // Unreachable in practice — the DA adapter's own page cap admits far fewer
    // rows than 65k — but returning a duplicate would let one server shadow
    // another's tools. The fallback uses `_`, which neither `slug_for` nor the
    // suffix loop above can emit, so it cannot collide with either; and it
    // stays free of `.`, which `super::runtime::mcp_tool_route` relies on to
    // split a namespaced tool id back into (slug, tool).
    let fallback = format!("{candidate}_{}", taken.len());
    taken.push(fallback.clone());
    fallback
}

/// The bundled tenant server's list entry. It has no DA row, so it is built
/// rather than classified, and is always usable when the user is signed in.
#[must_use]
pub(crate) fn bundled_view(endpoint: &str) -> McpServerView {
    McpServerView {
        version: xfalcon_contracts::CONTRACT_VERSION,
        id: BUNDLED_SERVER_ID.into(),
        name: BUNDLED_SERVER_ID.to_owned(),
        display_name: Some("Tenant Data Analyst".to_owned()),
        description: Some(
            "Your tenant's bundled analytical workspace. Connected automatically after sign-in."
                .to_owned(),
        ),
        transport: McpTransport::Http,
        url: Some(endpoint.to_owned()),
        auth_type: McpAuthType::Bearer,
        permission_mode: McpPermissionMode::Ask,
        enabled: true,
        builtin: true,
        slug: BUNDLED_SERVER_ID.to_owned(),
        unavailable: None,
        // The bundled server rides the signed-in session, so there is no
        // per-server sign-in state to report for it -- and therefore no client
        // of its own either.
        sign_in: None,
        oauth_client_id: None,
        oauth_client_confidential: false,
    }
}
