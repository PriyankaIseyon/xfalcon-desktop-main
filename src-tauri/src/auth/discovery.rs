use reqwest::{redirect::Policy, Client, Url};
use serde::Deserialize;
use thiserror::Error;

use crate::clients::da::protected_resource::ProtectedResourceMetadata;

use super::config::AuthConfig;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredAuth {
    pub resource: String,
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub jwks_uri: Url,
    pub issuer: Url,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("protected resource metadata is unavailable")]
    ProtectedResourceUnavailable,
    #[error("authorization server is not allowlisted")]
    IssuerNotAllowlisted,
    #[error("authorization server metadata is invalid")]
    InvalidMetadata,
    #[error("discovery request failed")]
    RequestFailed,
}
#[derive(Debug, Deserialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    #[allow(dead_code)]
    registration_endpoint: Option<String>,
}

pub fn build_discovery_client() -> Result<Client, DiscoveryError> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(READ_TIMEOUT)
        .build()
        .map_err(|_| DiscoveryError::RequestFailed)
}

pub async fn discover_auth(
    client: &Client,
    config: &AuthConfig,
) -> Result<DiscoveredAuth, DiscoveryError> {
    // The authorization server URL comes from trusted configuration, not from
    // the protected resource document, so both fetches can run at once. The
    // results are still checked in the original order below, and a protected
    // resource failure returns without waiting for the metadata request.
    let prm_fetch = fetch_protected_resource_metadata(client, &config.da_base_url);
    let metadata_fetch = fetch_authorization_server_metadata(client, &config.casdoor_issuer);
    tokio::pin!(prm_fetch);
    tokio::pin!(metadata_fetch);
    let mut early_metadata = None;
    let prm = loop {
        tokio::select! {
            result = &mut prm_fetch => break result,
            result = &mut metadata_fetch, if early_metadata.is_none() => early_metadata = Some(result),
        }
    };
    let prm = prm?;
    let issuer_from_prm = prm
        .primary_authorization_server()
        .ok_or(DiscoveryError::InvalidMetadata)?;
    if !issuer_matches_allowlist(issuer_from_prm, config.casdoor_issuer.as_str()) {
        return Err(DiscoveryError::IssuerNotAllowlisted);
    }

    let metadata = match early_metadata {
        Some(result) => result,
        None => metadata_fetch.await,
    }?;

    if !issuer_matches_allowlist(&metadata.issuer, config.casdoor_issuer.as_str()) {
        return Err(DiscoveryError::IssuerNotAllowlisted);
    }

    let authorization_endpoint = Url::parse(&metadata.authorization_endpoint)
        .map_err(|_| DiscoveryError::InvalidMetadata)?;
    let token_endpoint =
        Url::parse(&metadata.token_endpoint).map_err(|_| DiscoveryError::InvalidMetadata)?;
    let jwks_uri = Url::parse(&metadata.jwks_uri).map_err(|_| DiscoveryError::InvalidMetadata)?;
    if !endpoint_scheme_allowed(authorization_endpoint.scheme())
        || !endpoint_scheme_allowed(token_endpoint.scheme())
        || !endpoint_scheme_allowed(jwks_uri.scheme())
        || !same_origin(&authorization_endpoint, &config.casdoor_issuer)
        || !same_origin(&token_endpoint, &config.casdoor_issuer)
        || !same_origin(&jwks_uri, &config.casdoor_issuer)
        || jwks_uri.fragment().is_some()
    {
        return Err(DiscoveryError::InvalidMetadata);
    }

    Ok(DiscoveredAuth {
        resource: prm.resource,
        authorization_endpoint,
        token_endpoint,
        jwks_uri,
        issuer: config.casdoor_issuer.clone(),
    })
}

pub async fn fetch_protected_resource_metadata(
    client: &Client,
    da_base: &Url,
) -> Result<ProtectedResourceMetadata, DiscoveryError> {
    let mut url = da_base.clone();
    url.set_path("/.well-known/oauth-protected-resource");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| DiscoveryError::RequestFailed)?;
    if !response.status().is_success() {
        return Err(DiscoveryError::ProtectedResourceUnavailable);
    }
    response
        .json::<ProtectedResourceMetadata>()
        .await
        .map_err(|_| DiscoveryError::InvalidMetadata)
}

async fn fetch_authorization_server_metadata(
    client: &Client,
    issuer: &Url,
) -> Result<AuthorizationServerMetadata, DiscoveryError> {
    let mut url = issuer.clone();
    let path = url.path().trim_end_matches('/');
    let well_known = if path.is_empty() {
        "/.well-known/openid-configuration".to_owned()
    } else {
        format!("{path}/.well-known/openid-configuration")
    };
    url.set_path(&well_known);
    url.set_query(None);
    url.set_fragment(None);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| DiscoveryError::RequestFailed)?;
    if !response.status().is_success() {
        return Err(DiscoveryError::InvalidMetadata);
    }
    response
        .json::<AuthorizationServerMetadata>()
        .await
        .map_err(|_| DiscoveryError::InvalidMetadata)
}

fn issuer_matches_allowlist(candidate: &str, allowlisted: &str) -> bool {
    normalize_issuer(candidate) == normalize_issuer(allowlisted)
}

fn normalize_issuer(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn endpoint_scheme_allowed(scheme: &str) -> bool {
    if cfg!(test) {
        scheme == "http" || scheme == "https"
    } else {
        scheme == "https"
    }
}

fn same_origin(candidate: &Url, issuer: &Url) -> bool {
    candidate.origin() == issuer.origin()
        && candidate.username().is_empty()
        && candidate.password().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_casdoor_openid_configuration_with_extra_fields() {
        let json = r#"{
            "issuer":"https://auth.mcp-staging.iseyon.com",
            "authorization_endpoint":"https://auth.mcp-staging.iseyon.com/login/oauth/authorize",
            "token_endpoint":"https://auth.mcp-staging.iseyon.com/api/login/oauth/access_token",
            "jwks_uri":"https://auth.mcp-staging.iseyon.com/.well-known/jwks"
        }"#;
        let parsed: AuthorizationServerMetadata = serde_json::from_str(json).expect("parse");
        assert_eq!(parsed.issuer, "https://auth.mcp-staging.iseyon.com");
    }
}
