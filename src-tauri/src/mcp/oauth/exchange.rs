//! The token endpoint: authorization-code exchange, refresh, and revocation.
//!
//! Three requests, one set of rules:
//!
//! - **PKCE is mandatory** (OAuth 2.1 §4.1.1). The verifier is sent on the
//!   exchange, never stored, and never leaves the flow that generated it.
//! - **`resource` is sent on both legs** (RFC 8707 §2). The authorization server
//!   cross-checks the value against the one used on `/authorize`, and the token
//!   it mints carries that audience — which is the only reason the resource
//!   server will accept it. Casdoor in particular honours `resource` on
//!   `authorization_code` and `refresh_token` and *not* on `client_credentials`,
//!   which is why this app never uses that grant for MCP.
//! - **Client authentication follows what the server issued.** A public client
//!   sends `client_id` in the body; one that was issued a secret sends HTTP Basic
//!   (RFC 6749 §2.3.1 — "MUST support" Basic, "MAY support" body), so the secret
//!   stays out of any access log that records request bodies.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{Client, Url};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::registration::OauthClient;
use super::{endpoint_is_usable, read_json};
use crate::auth::pkce::PkceChallenge;

/// Renewed this many seconds before nominal expiry, so a request that takes a
/// moment to reach the server does not arrive with a token that expired in
/// flight.
pub(crate) const RENEWAL_LEEWAY_SECONDS: i64 = 60;

/// Assumed lifetime when a token response omits `expires_in` (RFC 6749 §5.1
/// makes it optional). Short on purpose: guessing long would keep using a token
/// the server may already have retired, and a renewal is one cheap request.
const ASSUMED_LIFETIME_SECONDS: i64 = 300;

/// What the token endpoint returned.
#[derive(Clone)]
pub(crate) struct TokenSet {
    pub(crate) access_token: Zeroizing<String>,
    pub(crate) refresh_token: Option<Zeroizing<String>>,
    /// Unix seconds. Always populated — from `expires_in` when the server sent
    /// one, from [`ASSUMED_LIFETIME_SECONDS`] when it did not.
    pub(crate) expires_at: i64,
    /// The scope the server actually granted, which may be narrower than the
    /// scope requested (RFC 6749 §5.1).
    pub(crate) scope: Option<String>,
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenSet")
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ExchangeError {
    /// The authorization server refused the grant itself: a code that was
    /// already used or expired, a refresh token that was revoked, a redirect URI
    /// it does not recognise for this client. Not retryable without involving
    /// the user again.
    #[error("the authorization server refused this grant")]
    Refused,
    #[error("the token endpoint could not be reached")]
    RequestFailed,
    #[error("the token response was not usable")]
    Invalid,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Where a code exchange or refresh is sent, and as whom.
pub(crate) struct TokenRequest<'a> {
    pub(crate) token_endpoint: &'a Url,
    pub(crate) client: &'a OauthClient,
    /// RFC 8707 audience. Never derived here — it is the `resource` value the
    /// resource server itself published (`super::discovery`).
    pub(crate) resource: &'a str,
    pub(crate) scope: Option<&'a str>,
}

/// Builds the `/authorize` URL the system browser opens.
///
/// A pure builder: the caller owns the lifetime of `state` and the PKCE
/// verifier, and the loopback listener that will receive the redirect.
#[must_use]
pub(crate) fn authorize_url(
    authorization_endpoint: &Url,
    client: &OauthClient,
    redirect_uri: &str,
    resource: &str,
    scope: Option<&str>,
    state: &str,
    pkce: &PkceChallenge,
) -> Url {
    let mut url = authorization_endpoint.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", &client.client_id);
        pairs.append_pair("redirect_uri", redirect_uri);
        pairs.append_pair("state", state);
        pairs.append_pair("code_challenge", &pkce.challenge);
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("resource", resource);
        if let Some(scope) = scope {
            pairs.append_pair("scope", scope);
        }
    }
    url
}

/// Exchanges an authorization code for a grant (RFC 6749 §4.1.3).
pub(crate) async fn exchange_code(
    http: &Client,
    request: &TokenRequest<'_>,
    code: &str,
    redirect_uri: &str,
    pkce: &PkceChallenge,
) -> Result<TokenSet, ExchangeError> {
    let form = vec![
        ("grant_type", "authorization_code".to_owned()),
        ("code", code.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("code_verifier", pkce.verifier.clone()),
        ("resource", request.resource.to_owned()),
    ];
    post_token(http, request, form).await
}

/// Renews a grant (RFC 6749 §6), carrying the same audience so the renewed token
/// is accepted by the same resource server.
pub(crate) async fn refresh(
    http: &Client,
    request: &TokenRequest<'_>,
    refresh_token: &str,
) -> Result<TokenSet, ExchangeError> {
    let form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token.to_owned()),
        ("resource", request.resource.to_owned()),
    ];
    post_token(http, request, form).await
}

/// Revokes a grant on sign-out (RFC 7009), best effort.
///
/// Returns whether the server acknowledged. A failure is deliberately not fatal:
/// the local grant is deleted either way, and refusing to sign out because a
/// remote endpoint is unreachable would leave the material on the device — the
/// opposite of what the user asked for.
pub(crate) async fn revoke(
    http: &Client,
    endpoint: &Url,
    client: &OauthClient,
    token: &str,
    is_refresh_token: bool,
) -> bool {
    if !endpoint_is_usable(endpoint) {
        return false;
    }
    let mut form = vec![
        ("token", token.to_owned()),
        (
            "token_type_hint",
            if is_refresh_token {
                "refresh_token".to_owned()
            } else {
                "access_token".to_owned()
            },
        ),
    ];
    let mut builder = http.post(endpoint.clone());
    builder = match &client.client_secret {
        Some(secret) => builder.header(
            reqwest::header::AUTHORIZATION,
            basic_auth(&client.client_id, secret),
        ),
        None => {
            form.push(("client_id", client.client_id.clone()));
            builder
        }
    };
    builder
        .form(&form)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

async fn post_token(
    http: &Client,
    request: &TokenRequest<'_>,
    mut form: Vec<(&'static str, String)>,
) -> Result<TokenSet, ExchangeError> {
    if !endpoint_is_usable(request.token_endpoint) {
        return Err(ExchangeError::RequestFailed);
    }
    if let Some(scope) = request.scope {
        form.push(("scope", scope.to_owned()));
    }

    let mut builder = http
        .post(request.token_endpoint.clone())
        .header(reqwest::header::ACCEPT, "application/json");
    builder = match &request.client.client_secret {
        Some(secret) => builder.header(
            reqwest::header::AUTHORIZATION,
            basic_auth(&request.client.client_id, secret),
        ),
        None => {
            form.push(("client_id", request.client.client_id.clone()));
            builder
        }
    };

    let response = builder
        .form(&form)
        .send()
        .await
        .map_err(|_| ExchangeError::RequestFailed)?;

    let status = response.status();
    let body: TokenResponse = read_json(response)
        .await
        // A 4xx with an unparseable body is still a refusal, and reporting it as
        // a transport failure would invite a retry that cannot succeed.
        .unwrap_or(TokenResponse {
            access_token: None,
            refresh_token: None,
            token_type: None,
            expires_in: None,
            scope: None,
            error: None,
        });

    if !status.is_success() {
        return Err(if status.is_client_error() {
            ExchangeError::Refused
        } else {
            ExchangeError::RequestFailed
        });
    }
    // An HTTP 200 carrying an `error` member is how several authorization
    // servers report a refused grant. Trusting the status alone would store a
    // grant with no token in it.
    if body.error.is_some() {
        return Err(ExchangeError::Refused);
    }
    let access_token = body
        .access_token
        .map(|value| Zeroizing::new(value.trim().to_owned()))
        .filter(|value| !value.is_empty())
        .ok_or(ExchangeError::Invalid)?;
    // Anything other than a bearer would be attached to requests as one, which
    // would silently send a DPoP or MAC token in a header it does not belong in.
    if body
        .token_type
        .as_ref()
        .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
    {
        return Err(ExchangeError::Invalid);
    }

    let lifetime_seconds = body
        .expires_in
        .filter(|seconds| *seconds > 0)
        .unwrap_or(ASSUMED_LIFETIME_SECONDS);
    Ok(TokenSet {
        access_token,
        refresh_token: body
            .refresh_token
            .map(|value| Zeroizing::new(value.trim().to_owned()))
            .filter(|value| !value.is_empty()),
        expires_at: now_epoch_seconds().saturating_add(lifetime_seconds),
        scope: body.scope.filter(|scope| !scope.trim().is_empty()),
    })
}

fn basic_auth(client_id: &str, secret: &str) -> String {
    format!(
        "Basic {}",
        STANDARD.encode(format!(
            "{}:{}",
            form_encode(client_id),
            form_encode(secret)
        ))
    )
}

/// `application/x-www-form-urlencoded` encoding of one value.
///
/// RFC 6749 §2.3.1 requires the client id and secret to be encoded this way
/// *before* they are base64'd for Basic auth. Skipping it is invisible for the
/// hex-and-alphanumeric credentials most authorization servers issue and wrong
/// for any that contains a `:`, a `+`, or a non-ASCII character — the exact case
/// where a silent authentication failure is hardest to diagnose. Hand-written
/// because the crate that would provide it is only a transitive dependency here.
fn form_encode(value: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => {
                // Infallible: writing to a String never fails.
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// Unix seconds now.
///
/// A clock read rather than a monotonic instant because the value is persisted
/// and compared across process restarts, which an `Instant` cannot survive.
#[must_use]
pub(crate) fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::pkce::generate_pkce;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn public_client() -> OauthClient {
        OauthClient {
            client_id: "desktop-client".to_owned(),
            client_secret: None,
        }
    }

    fn confidential_client() -> OauthClient {
        OauthClient {
            client_id: "desktop-client".to_owned(),
            client_secret: Some(Zeroizing::new("s3cr3t".to_owned())),
        }
    }

    #[test]
    fn the_authorize_url_carries_pkce_and_the_resource_indicator() {
        let pkce = generate_pkce();
        let url = authorize_url(
            &Url::parse("https://auth.example.test/login/oauth/authorize").expect("url"),
            &public_client(),
            "http://127.0.0.1:19848/callback",
            "https://mcp.example.test/mcp",
            Some("openid offline_access"),
            "state-value",
            &pkce,
        );
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();

        assert_eq!(pairs["response_type"], "code");
        assert_eq!(pairs["client_id"], "desktop-client");
        assert_eq!(pairs["redirect_uri"], "http://127.0.0.1:19848/callback");
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pairs["code_challenge"], pkce.challenge.as_str());
        assert_eq!(pairs["resource"], "https://mcp.example.test/mcp");
        assert_eq!(pairs["scope"], "openid offline_access");
        assert_eq!(pairs["state"], "state-value");
        assert!(
            !url.query().expect("a query").contains(&pkce.verifier),
            "the verifier must never appear in the browser URL"
        );
    }

    #[test]
    fn no_negotiated_scope_means_no_scope_parameter() {
        let url = authorize_url(
            &Url::parse("https://auth.example.test/authorize").expect("url"),
            &public_client(),
            "http://127.0.0.1:19848/callback",
            "https://mcp.example.test/mcp",
            None,
            "state-value",
            &generate_pkce(),
        );
        assert!(url.query_pairs().all(|(key, _)| key != "scope"));
    }

    async fn token_server(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    fn endpoint(server: &MockServer) -> Url {
        Url::parse(&format!("{}/token", server.uri())).expect("url")
    }

    #[tokio::test]
    async fn a_public_client_exchange_sends_the_verifier_the_resource_and_its_client_id() {
        let server = token_server(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "issued-access-token",
            "refresh_token": "issued-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "openid"
        })))
        .await;
        let pkce = generate_pkce();
        let http = super::super::build_client().expect("client");
        let client = public_client();
        let token_endpoint = endpoint(&server);

        let tokens = exchange_code(
            &http,
            &TokenRequest {
                token_endpoint: &token_endpoint,
                client: &client,
                resource: "https://mcp.example.test/mcp",
                scope: None,
            },
            "the-code",
            "http://127.0.0.1:19848/callback",
            &pkce,
        )
        .await
        .expect("the exchange succeeds");

        assert_eq!(tokens.access_token.as_str(), "issued-access-token");
        assert_eq!(
            tokens.refresh_token.as_deref().map(String::as_str),
            Some("issued-refresh-token")
        );
        assert!(tokens.expires_at > now_epoch_seconds() + 3000);
        assert_eq!(tokens.scope.as_deref(), Some("openid"));
        assert!(
            !format!("{tokens:?}").contains("issued-access-token"),
            "a grant must never reach a debug rendering"
        );

        let body = String::from_utf8(
            server.received_requests().await.expect("requests")[0]
                .body
                .clone(),
        )
        .expect("a form body");
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains(&format!("code_verifier={}", pkce.verifier)));
        assert!(body.contains("resource=https%3A%2F%2Fmcp.example.test%2Fmcp"));
        assert!(
            body.contains("client_id=desktop-client"),
            "a public client identifies itself in the body"
        );
    }

    #[tokio::test]
    async fn a_client_issued_a_secret_authenticates_with_basic_and_never_in_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header_exists("authorization"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "issued", "expires_in": 60 })),
            )
            .mount(&server)
            .await;

        let http = super::super::build_client().expect("client");
        let client = confidential_client();
        let token_endpoint = endpoint(&server);
        let tokens = refresh(
            &http,
            &TokenRequest {
                token_endpoint: &token_endpoint,
                client: &client,
                resource: "https://mcp.example.test/mcp",
                scope: None,
            },
            "stored-refresh-token",
        )
        .await
        .expect("the refresh succeeds");
        assert_eq!(tokens.access_token.as_str(), "issued");

        let request = &server.received_requests().await.expect("requests")[0];
        let body = String::from_utf8(request.body.clone()).expect("a form body");
        assert!(body.contains("grant_type=refresh_token"));
        assert!(
            !body.contains("s3cr3t") && !body.contains("client_secret"),
            "the secret must travel in the Authorization header, not the body"
        );
        let header = request
            .headers
            .get("authorization")
            .expect("basic credentials")
            .to_str()
            .expect("ascii");
        assert_eq!(header, "Basic ZGVza3RvcC1jbGllbnQ6czNjcjN0");
    }

    #[tokio::test]
    async fn a_refused_grant_is_distinguished_from_an_unreachable_endpoint() {
        let refused = token_server(
            ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })),
        )
        .await;
        let broken = token_server(ResponseTemplate::new(503)).await;
        let http = super::super::build_client().expect("client");
        let client = public_client();

        for (server, expected) in [
            (&refused, ExchangeError::Refused),
            (&broken, ExchangeError::RequestFailed),
        ] {
            let token_endpoint = endpoint(server);
            let outcome = refresh(
                &http,
                &TokenRequest {
                    token_endpoint: &token_endpoint,
                    client: &client,
                    resource: "https://mcp.example.test/mcp",
                    scope: None,
                },
                "stored-refresh-token",
            )
            .await;
            assert_eq!(outcome.err(), Some(expected));
        }
    }

    #[tokio::test]
    async fn a_two_hundred_carrying_an_error_member_is_still_a_refusal() {
        let server = token_server(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "error": "invalid_grant", "error_description": "expired" })),
        )
        .await;
        let http = super::super::build_client().expect("client");
        let client = public_client();
        let token_endpoint = endpoint(&server);

        let outcome = refresh(
            &http,
            &TokenRequest {
                token_endpoint: &token_endpoint,
                client: &client,
                resource: "https://mcp.example.test/mcp",
                scope: None,
            },
            "stored-refresh-token",
        )
        .await;
        assert_eq!(outcome.err(), Some(ExchangeError::Refused));
    }

    #[tokio::test]
    async fn a_grant_of_a_type_this_client_cannot_attach_is_refused() {
        let server = token_server(ResponseTemplate::new(200).set_body_json(
            json!({ "access_token": "issued", "token_type": "DPoP", "expires_in": 60 }),
        ))
        .await;
        let http = super::super::build_client().expect("client");
        let client = public_client();
        let token_endpoint = endpoint(&server);

        let outcome = refresh(
            &http,
            &TokenRequest {
                token_endpoint: &token_endpoint,
                client: &client,
                resource: "https://mcp.example.test/mcp",
                scope: None,
            },
            "stored-refresh-token",
        )
        .await;
        assert_eq!(outcome.err(), Some(ExchangeError::Invalid));
    }

    #[tokio::test]
    async fn a_response_without_expires_in_still_gets_a_bounded_lifetime() {
        let server =
            token_server(ResponseTemplate::new(200).set_body_json(json!({ "access_token": "x" })))
                .await;
        let http = super::super::build_client().expect("client");
        let client = public_client();
        let token_endpoint = endpoint(&server);

        let tokens = refresh(
            &http,
            &TokenRequest {
                token_endpoint: &token_endpoint,
                client: &client,
                resource: "https://mcp.example.test/mcp",
                scope: None,
            },
            "stored-refresh-token",
        )
        .await
        .expect("the refresh succeeds");

        let assumed = now_epoch_seconds() + ASSUMED_LIFETIME_SECONDS;
        assert!((tokens.expires_at - assumed).abs() <= 5);
    }

    #[tokio::test]
    async fn revocation_reports_failure_without_preventing_a_sign_out() {
        let server = token_server(ResponseTemplate::new(503)).await;
        let http = super::super::build_client().expect("client");
        let revoked = revoke(
            &http,
            &endpoint(&server),
            &public_client(),
            "stored-refresh-token",
            true,
        )
        .await;
        assert!(!revoked);

        let sent = String::from_utf8(
            server.received_requests().await.expect("requests")[0]
                .body
                .clone(),
        )
        .expect("a form body");
        assert!(sent.contains("token_type_hint=refresh_token"));
    }

    #[tokio::test]
    async fn a_body_string_match_pins_the_resource_indicator_on_the_refresh_leg() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains(
                "resource=https%3A%2F%2Fmcp.example.test%2Fmcp",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "issued", "expires_in": 60 })),
            )
            .mount(&server)
            .await;

        let http = super::super::build_client().expect("client");
        let client = public_client();
        let token_endpoint = endpoint(&server);
        assert!(refresh(
            &http,
            &TokenRequest {
                token_endpoint: &token_endpoint,
                client: &client,
                resource: "https://mcp.example.test/mcp",
                scope: None,
            },
            "stored-refresh-token",
        )
        .await
        .is_ok());
    }
}
