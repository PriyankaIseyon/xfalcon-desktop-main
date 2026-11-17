mod account;
mod commands;
mod config;
mod discovery;
pub(crate) mod loopback;
mod oauth;
pub(crate) mod pkce;
pub(crate) mod renewal;
mod service;
mod session;
mod token;

pub(crate) use account::canonical_account_id;
pub use commands::{auth_cancel, auth_login, auth_logout, auth_status};
pub(crate) use config::{
    init_bundled_tenant_config, tenant_config_source, AuthConfig, AuthConfigError,
};
pub(crate) use discovery::{build_discovery_client, discover_auth, DiscoveryError};
pub(crate) use renewal::ensure_fresh_session;
pub use service::AuthService;
pub use session::AuthSessionStore;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod token_tests;
