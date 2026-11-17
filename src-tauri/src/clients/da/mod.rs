mod audit;
mod auth;
pub(crate) mod chat_api;
pub(crate) mod generated;
pub(crate) mod identity;
pub(crate) mod kits_api;
pub(crate) mod mapping;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the shared-grant path is reachable only when an operator configures the key-encryption key xfalcon-web uses (crate::mcp::oauth::envelope); without it the device-sealed store is used instead"
    )
)]
pub(crate) mod mcp_oauth_grants;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the registry and command layers that consume this adapter land in a later change"
    )
)]
pub(crate) mod mcp_servers;
pub(crate) mod model_catalog;
mod models;
pub(crate) mod protected_resource;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Task 3 credential DA adapter remains internal until fixed-origin provider verification and DA wire tests are complete"
    )
)]
pub(crate) mod provider_keys;
mod secret;

#[cfg(test)]
mod tests;
