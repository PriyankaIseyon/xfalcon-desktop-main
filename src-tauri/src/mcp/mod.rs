//! Real MCP protocol client for the bundled tenant DA MCP server and for the
//! servers a user registers.
//!
//! The layers are stacked so each is testable alone: resolve a server
//! ([`server_resolver`]), bind its credential ([`auth`]), negotiate a Streamable
//! HTTP session pinned to MCP protocol revision `2025-11-25` ([`client`]),
//! discover and validate its tool catalog ([`catalog`]), call tools with
//! bounded, cancellable results ([`executor`]), hold one session per server
//! across runs ([`runtime`]), and register/inspect servers from the UI
//! ([`commands`]).
//!
//! # Where a server may come from
//!
//! Exactly two places, and never from a tool argument, a model, or a skill:
//! [`server_resolver::McpServerResolver::resolve`] derives the bundled tenant
//! endpoint from trusted, environment-pinned configuration, and
//! [`registry::classify`] turns rows of DA's shared `mcp_servers` registry into
//! dialable descriptors — but only after [`registry::validate_endpoint`]
//! accepts the URL. That function is the single place every "may we dial that?"
//! rule lives (https, no embedded userinfo, no query or fragment, no private or
//! loopback host outside a debug build).
//!
//! `stdio` and `other` transports name no remote endpoint and are deliberately
//! not dialed: a row registered elsewhere with one of those is listed with an
//! [`xfalcon_contracts::mcp::McpServerUnavailableReason`] rather than hidden.
//! `oauth` authentication *is* implemented, per server and per device, in
//! [`oauth`] — the flow runs in the system browser and its grant never leaves
//! this machine. See
//! `docs/decisions.md`
//! (D2, D4).

// The `mcp_server_*` commands in `commands` are reached from the composition
// root (`src-tauri/src/lib.rs`) once its `invoke_handler` and capability list
// name them, which is the same plan task's React step; until then they and a
// few store/set accessors they expose are exercised only by this crate's tests.
#![allow(dead_code)]

pub(crate) mod auth;
pub(crate) mod catalog;
pub(crate) mod client;
pub(crate) mod commands;
pub(crate) mod credential_store;
pub(crate) mod dispatch;
mod dns;
pub(crate) mod executor;
pub(crate) mod oauth;
pub(crate) mod registry;
pub(crate) mod runtime;
mod server_resolver;

#[cfg(test)]
pub(crate) use server_resolver::McpServerDescriptor;
pub(crate) use server_resolver::McpServerResolver;

/// The `User-Agent` every request to an MCP server or its authorization server
/// carries. Not optional: a load balancer running the AWS WAF common rule set
/// refuses a request without one (`NoUserAgent_HEADER`) with a bare 403, which
/// the sign-in reported as "OAuth configuration is unavailable".
pub(crate) const USER_AGENT: &str = concat!("xFalcon-Desktop/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests;
