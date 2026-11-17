//! Test-only support: a real, spec-compliant MCP test server built from
//! `rmcp`'s own server transport (dev-only features — see `Cargo.toml`), plus
//! raw axum handlers for adversarial cases a well-behaved `ServerHandler`
//! can't easily produce. See
//! `docs/decisions.md` for
//! why this approach was chosen over extending `wiremock`.

mod support;

mod commands;
mod credential_store;
mod multi_server;
mod oauth;
mod reauth;
mod registry;
mod runtime;
mod security;
mod session;
mod shared_grant;
