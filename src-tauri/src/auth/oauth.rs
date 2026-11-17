use reqwest::Url;
use serde::Deserialize;
use thiserror::Error;

use super::config::AuthConfig;
use super::discovery::DiscoveredAuth;
use super::loopback::LoopbackRedirectUri;
use super::pkce::PkceChallenge;

#[derive(Debug, Error)]
pub enum OAuthError {
    #[error(
        "token exchange was rejected (check the Casdoor redirect URI for this desktop client)"
    )]
    RedirectRejected,
    #[error("token exchange failed")]
    ExchangeFailed,
    #[error("token response was invalid")]
    InvalidResponse,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    error_description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TokenBundle {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

pub fn build_authorization_url(
    discovered: &DiscoveredAuth,
    config: &AuthConfig,
    pkce: &PkceChallenge,
    state: &str,
    nonce: &str,
    redirect_uri: &LoopbackRedirectUri,
) -> Result<Url, OAuthError> {
    let mut url = discovered.authorization_endpoint.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", &config.client_id);
        pairs.append_pair("redirect_uri", &redirect_uri.value);
        pairs.append_pair("scope", "openid profile email");
        pairs.append_pair("state", state);
        pairs.append_pair("nonce", nonce);
        pairs.append_pair("code_challenge", &pkce.challenge);
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("resource", &discovered.resource);
        pairs.append_pair("organization", &config.organization);
    }
    Ok(url)
}

/// Trades a refresh token for a fresh access token (RFC 6749 §6).
///
/// `resource` is sent for the same reason the authorization-code exchange sends
/// it (RFC 8707): the renewed token must carry the same audience as the one it
/// replaces, or `crate::mcp::auth::McpAuth::bind` will refuse to attach it to
/// the bundled MCP server and every tool call would start failing mid-session.
///
/// No `scope` is sent, which asks the authorization server for the same scope as
/// the original grant. Requesting one explicitly risks *narrowing* it against a
/// server that treats the parameter as authoritative.
///
/// A rejection here is reported as [`OAuthError::RedirectRejected`] only for the
/// shapes that mean "this grant is finished" (`invalid_grant`) — the caller
/// distinguishes that from a transport failure, because one means "sign in
/// again" and the other means "try again later".
pub async fn exchange_refresh_token(
    client: &reqwest::Client,
    discovered: &DiscoveredAuth,
    config: &AuthConfig,
    refresh_token: &str,
) -> Result<TokenBundle, OAuthError> {
    let response = client
        .post(discovered.token_endpoint.clone())
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", config.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("resource", discovered.resource.as_str()),
        ])
        .send()
        .await
        .map_err(|_| OAuthError::ExchangeFailed)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 400 || status.as_u16() == 401 {
            if let Ok(parsed) = serde_json::from_str::<OAuthErrorResponse>(&body) {
                if matches!(parsed.error.as_deref(), Some("invalid_grant")) {
                    return Err(OAuthError::RedirectRejected);
                }
            }
            if body.contains("invalid_grant") {
                return Err(OAuthError::RedirectRejected);
            }
        }
        return Err(OAuthError::ExchangeFailed);
    }

    let token: TokenResponse = response
        .json()
        .await
        .map_err(|_| OAuthError::InvalidResponse)?;
    if token.access_token.trim().is_empty() {
        return Err(OAuthError::InvalidResponse);
    }
    if token
        .token_type
        .as_ref()
        .is_some_and(|value| !value.eq_ignore_ascii_case("bearer"))
    {
        return Err(OAuthError::InvalidResponse);
    }

    Ok(TokenBundle {
        access_token: token.access_token,
        // An authorization server that rotates refresh tokens returns the new
        // one here; one that does not omits the field, and the caller must keep
        // the token it already holds rather than dropping the session's only
        // way to renew again.
        refresh_token: token.refresh_token,
    })
}

pub async fn exchange_authorization_code(
    client: &reqwest::Client,
    discovered: &DiscoveredAuth,
    config: &AuthConfig,
    code: &str,
    redirect_uri: &LoopbackRedirectUri,
    pkce: &PkceChallenge,
) -> Result<TokenBundle, OAuthError> {
    let response = client
        .post(discovered.token_endpoint.clone())
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", config.client_id.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri.value.as_str()),
            ("code_verifier", pkce.verifier.as_str()),
            ("resource", discovered.resource.as_str()),
        ])
        .send()
        .await
        .map_err(|_| OAuthError::ExchangeFailed)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 400 {
            if let Ok(parsed) = serde_json::from_str::<OAuthErrorResponse>(&body) {
                if matches!(
                    parsed.error.as_deref(),
                    Some("invalid_grant") | Some("invalid_request")
                ) {
                    return Err(OAuthError::RedirectRejected);
                }
            }
            if body.contains("invalid_grant") || body.contains("redirect_uri") {
                return Err(OAuthError::RedirectRejected);
            }
        }
        return Err(OAuthError::ExchangeFailed);
    }

    let token: TokenResponse = response
        .json()
        .await
        .map_err(|_| OAuthError::InvalidResponse)?;
    if token.access_token.trim().is_empty() {
        return Err(OAuthError::InvalidResponse);
    }
    if token
        .token_type
        .as_ref()
        .is_some_and(|value| !value.eq_ignore_ascii_case("bearer"))
    {
        return Err(OAuthError::InvalidResponse);
    }

    Ok(TokenBundle {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
    })
}
