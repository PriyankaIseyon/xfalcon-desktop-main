//! The protocol layer of OpenAI's "Sign in with ChatGPT" flow: discovery, the
//! authorize URL, code exchange, refresh and revocation. It stores no tokens and
//! opens no browser; the caller owns both, and generates the PKCE, state and
//! nonce values it passes in.

use std::fmt;
use std::time::Duration;

use chrono::DateTime;
use futures_util::StreamExt;
use reqwest::header::ACCEPT;
use reqwest::{redirect::Policy, Client, Response, StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub(crate) const ISSUER: &str = "https://auth.openai.com";
pub(crate) const RESOURCE: &str = "https://api.openai.com/v1";
pub(crate) const SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
pub(crate) const PLAN_USAGE_SCOPE: &str = "chatgpt.tokens.use.direct";
pub(crate) const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
pub(crate) const AGENT_NAME_HINT: &str = "xFalcon Desktop";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const REVOKE_TIMEOUT: Duration = Duration::from_secs(10);
const DISCOVERY_MAX_BYTES: usize = 64 * 1024;
const TOKEN_MAX_BYTES: usize = 256 * 1024;
const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
const CALLBACK_PATH: &str = "/auth/callback";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum OAuthError {
    #[error("network failure")]
    Network,
    #[error("unexpected HTTP status")]
    Status(u16),
    #[error("the refresh token can no longer be used")]
    RefreshTerminal,
    #[error("the client registration was refused")]
    InvalidClient,
    #[error("the authorization code was refused")]
    InvalidGrant,
    #[error("the server answer was malformed")]
    Malformed,
    #[error("the discovery document was rejected")]
    DiscoveryRejected,
    #[error("could not build the HTTP client")]
    ClientBuild,
}

#[derive(Clone, Debug)]
pub(crate) struct OAuthConfig {
    issuer: Url,
}

impl OAuthConfig {
    /// `None` only when the built-in issuer stops parsing. That is a defect in
    /// this file, so the caller must surface it; there is no safe default issuer.
    pub(crate) fn production() -> Option<Self> {
        Url::parse(ISSUER).ok().map(|issuer| Self { issuer })
    }

    #[cfg(test)]
    pub(crate) fn for_test(issuer: Url) -> Self {
        Self { issuer }
    }

    #[cfg(test)]
    pub(crate) fn issuer(&self) -> &Url {
        &self.issuer
    }
}

/// Every request this layer makes is bounded in time, and none follows a
/// redirect: a redirected token request would carry the refresh token to a host
/// the discovery document never named.
pub(crate) fn build_http_client(config: &OAuthConfig) -> Result<Client, OAuthError> {
    Client::builder()
        .use_rustls_tls()
        // A loopback test issuer is plain http; production never is.
        .https_only(config.issuer.scheme() == "https")
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| OAuthError::ClientBuild)
}

#[derive(Clone, Debug)]
pub(crate) struct Discovery {
    pub(crate) issuer: String,
    pub(crate) authorization_endpoint: Url,
    pub(crate) token_endpoint: Url,
    pub(crate) jwks_uri: Url,
    pub(crate) revocation_endpoint: Option<Url>,
}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    revocation_endpoint: Option<String>,
}

pub(crate) async fn fetch_discovery(
    http: &Client,
    config: &OAuthConfig,
) -> Result<Discovery, OAuthError> {
    let mut url = config.issuer.clone();
    let path = format!(
        "{}{DISCOVERY_PATH}",
        config.issuer.path().trim_end_matches('/')
    );
    url.set_path(&path);
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|_| OAuthError::Network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(OAuthError::Status(status.as_u16()));
    }
    let body =
        read_capped_body(response, DISCOVERY_MAX_BYTES, OAuthError::DiscoveryRejected).await?;
    let document: DiscoveryDocument =
        serde_json::from_slice(&body).map_err(|_| OAuthError::DiscoveryRejected)?;

    // Compared as an origin because `Url` always serializes a bare issuer with a
    // trailing slash, and the document never carries one.
    let issuer_origin = config.issuer.origin();
    if document.issuer != issuer_origin.ascii_serialization() {
        return Err(OAuthError::DiscoveryRejected);
    }
    let authorization_endpoint = parse_endpoint(&document.authorization_endpoint)?;
    let token_endpoint = parse_endpoint(&document.token_endpoint)?;
    let jwks_uri = parse_endpoint(&document.jwks_uri)?;
    let revocation_endpoint = document
        .revocation_endpoint
        .as_deref()
        .map(parse_endpoint)
        .transpose()?;
    let same_origin = [&authorization_endpoint, &token_endpoint, &jwks_uri]
        .into_iter()
        .chain(revocation_endpoint.as_ref())
        .all(|endpoint| endpoint.origin() == issuer_origin);
    if !same_origin {
        return Err(OAuthError::DiscoveryRejected);
    }
    Ok(Discovery {
        issuer: document.issuer,
        authorization_endpoint,
        token_endpoint,
        jwks_uri,
        revocation_endpoint,
    })
}

fn parse_endpoint(raw: &str) -> Result<Url, OAuthError> {
    Url::parse(raw).map_err(|_| OAuthError::DiscoveryRejected)
}

pub(crate) fn new_host_id() -> String {
    format!("urn:uuid:{}", Uuid::new_v4())
}

#[derive(Clone)]
pub(crate) enum Registration {
    New,
    /// Hints and forced consent are only meaningful for an account this app has
    /// already registered. `id_token_hint` is a credential-bearing value.
    Returning {
        client_id: String,
        login_hint: Option<String>,
        id_token_hint: Option<String>,
        force_consent: bool,
    },
}

#[derive(Clone)]
pub(crate) struct AuthorizeRequest {
    pub(crate) registration: Registration,
    pub(crate) host_id: String,
    pub(crate) redirect_uri: String,
    pub(crate) state: String,
    pub(crate) nonce: String,
    pub(crate) code_challenge: String,
}

pub(crate) fn build_authorize_url(
    discovery: &Discovery,
    request: &AuthorizeRequest,
) -> Result<Url, OAuthError> {
    if !is_loopback_callback(&request.redirect_uri) {
        return Err(OAuthError::Malformed);
    }
    let mut url = discovery.authorization_endpoint.clone();
    {
        // The serializer writes the query back to `url` when it drops, so it is
        // scoped to finish before `url` is returned.
        let mut query = url.query_pairs_mut();
        match &request.registration {
            Registration::New => {
                query.append_pair("client_id", DYNAMIC_CLIENT_ID);
                query.append_pair("agent_name_hint", AGENT_NAME_HINT);
            }
            Registration::Returning {
                client_id,
                login_hint,
                id_token_hint,
                force_consent,
            } => {
                query.append_pair("client_id", client_id);
                if let Some(login_hint) = login_hint {
                    query.append_pair("login_hint", login_hint);
                }
                if let Some(id_token_hint) = id_token_hint {
                    query.append_pair("id_token_hint", id_token_hint);
                }
                if *force_consent {
                    query.append_pair("prompt", "consent");
                }
            }
        }
        query.append_pair("ext_agent_host_id", &request.host_id);
        query.append_pair("response_type", "code");
        query.append_pair("redirect_uri", &request.redirect_uri);
        query.append_pair("scope", SCOPE);
        query.append_pair("resource", RESOURCE);
        query.append_pair("state", &request.state);
        query.append_pair("nonce", &request.nonce);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("code_challenge", &request.code_challenge);
    }
    Ok(url)
}

/// The flow registers exactly one redirect, and the token exchange must echo it
/// byte for byte, so only the port may vary.
fn is_loopback_callback(redirect_uri: &str) -> bool {
    let Some(port) = redirect_uri
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix(CALLBACK_PATH))
    else {
        return false;
    };
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

#[derive(Clone)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: Option<String>,
    pub(crate) expires_in: u64,
    pub(crate) scope: String,
    /// Unix epoch seconds before which a refresh should not be attempted while the
    /// access token is still valid. The unit comes from the reference client
    /// (`value * 1000` to milliseconds), not from OpenAI's published documents.
    pub(crate) earliest_refresh_at: Option<i64>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("id_token", &self.id_token.as_ref().map(|_| "<redacted>"))
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .field("earliest_refresh_at", &self.earliest_refresh_at)
            .finish()
    }
}

#[derive(Deserialize)]
struct RawTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: u64,
    scope: String,
    earliest_refresh_at: Option<Value>,
}

pub(crate) async fn exchange_code(
    http: &Client,
    discovery: &Discovery,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, OAuthError> {
    post_token(
        http,
        &discovery.token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("resource", RESOURCE),
        ],
    )
    .await
}

pub(crate) async fn refresh_tokens(
    http: &Client,
    discovery: &Discovery,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, OAuthError> {
    post_token(
        http,
        &discovery.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", RESOURCE),
        ],
    )
    .await
    // `invalid_grant` is an expired code on exchange but a dead token on refresh.
    .map_err(|error| match error {
        OAuthError::InvalidGrant => OAuthError::RefreshTerminal,
        other => other,
    })
}

async fn post_token(
    http: &Client,
    endpoint: &Url,
    form: &[(&str, &str)],
) -> Result<TokenResponse, OAuthError> {
    let response = http
        .post(endpoint.clone())
        .header(ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .map_err(|_| OAuthError::Network)?;
    let status = response.status();
    if status.is_success() {
        let body = read_capped_body(response, TOKEN_MAX_BYTES, OAuthError::Malformed).await?;
        return parse_token_response(&body);
    }
    // An error body is read only for its code. When it cannot be read, the HTTP
    // status is still the answer the caller needs.
    let body = read_capped_body(response, TOKEN_MAX_BYTES, OAuthError::Malformed).await;
    Err(match body.ok().as_deref().and_then(error_code) {
        Some(code) => classify_token_error(&code, status.as_u16()),
        None => OAuthError::Status(status.as_u16()),
    })
}

fn parse_token_response(body: &[u8]) -> Result<TokenResponse, OAuthError> {
    let raw: RawTokenResponse = serde_json::from_slice(body).map_err(|_| OAuthError::Malformed)?;
    if raw.access_token.is_empty() {
        return Err(OAuthError::Malformed);
    }
    Ok(TokenResponse {
        earliest_refresh_at: earliest_refresh_seconds(raw.earliest_refresh_at.as_ref()),
        access_token: raw.access_token,
        refresh_token: raw.refresh_token,
        id_token: raw.id_token,
        expires_in: raw.expires_in,
        scope: raw.scope,
    })
}

/// The reference client's schema accepts this field as a number or an RFC 3339
/// string. Any other shape is dropped, not failed: a missing refresh floor only
/// lets a refresh happen sooner, and a failed sign-in over it helps no one.
fn earliest_refresh_seconds(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(seconds) => seconds.as_i64(),
        Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|time| time.timestamp()),
        _ => None,
    }
}

pub(crate) fn classify_token_error(code: &str, http_status: u16) -> OAuthError {
    match code {
        "invalid_grant" => OAuthError::InvalidGrant,
        "invalid_refresh_token"
        | "token_expired"
        | "refresh_token_expired"
        | "refresh_token_invalidated"
        | "refresh_token_reused" => OAuthError::RefreshTerminal,
        "invalid_client" => OAuthError::InvalidClient,
        _ => OAuthError::Status(http_status),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ErrorField {
    Code(String),
    Detailed { code: String },
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorField,
}

fn error_code(body: &[u8]) -> Option<String> {
    match serde_json::from_slice::<ErrorBody>(body).ok()?.error {
        ErrorField::Code(code) | ErrorField::Detailed { code } => Some(code),
    }
}

async fn read_capped_body(
    response: Response,
    limit: usize,
    oversized: OAuthError,
) -> Result<Vec<u8>, OAuthError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| OAuthError::Network)?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(oversized);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RevokeOutcome {
    Confirmed,
    Unconfirmed,
}

/// A dropped connection and a 5xx both leave open whether the server acted on the
/// revocation, so both retry. Any other answer is final.
pub(crate) async fn revoke_refresh_token(
    http: &Client,
    discovery: &Discovery,
    client_id: &str,
    refresh_token: &str,
    delays: &[Duration],
) -> RevokeOutcome {
    let Some(endpoint) = discovery.revocation_endpoint.as_ref() else {
        return RevokeOutcome::Unconfirmed;
    };
    let form = [
        ("token", refresh_token),
        ("token_type_hint", "refresh_token"),
        ("client_id", client_id),
    ];
    for attempt in 0..=delays.len() {
        if attempt > 0 {
            tokio::time::sleep(delays[attempt - 1]).await;
        }
        match http
            .post(endpoint.clone())
            .timeout(REVOKE_TIMEOUT)
            .form(&form)
            .send()
            .await
        {
            Ok(response) if response.status() == StatusCode::OK => {
                return RevokeOutcome::Confirmed;
            }
            Ok(response) if response.status().is_server_error() => {}
            Err(_) => {}
            Ok(_) => return RevokeOutcome::Unconfirmed,
        }
    }
    RevokeOutcome::Unconfirmed
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    const LOOPBACK: &str = "http://127.0.0.1:53682/auth/callback";
    const TOKEN_PATH: &str = "/api/accounts/oauth/token";
    const REVOKE_PATH: &str = "/api/accounts/oauth/revoke";
    const HOST_ID: &str = "urn:uuid:00000000-0000-4000-8000-000000000001";
    const TOKEN_RESPONSE: &str = r#"{"access_token":"at-value","token_type":"Bearer","expires_in":3600,"scope":"openid profile","refresh_token":"rt-new-value","id_token":"id-value","earliest_refresh_at":1800000000}"#;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("test url")
    }

    fn discovery_at(base: &str, revocation: bool) -> Discovery {
        Discovery {
            issuer: base.to_owned(),
            authorization_endpoint: url(&format!("{base}/api/accounts/authorize")),
            token_endpoint: url(&format!("{base}{TOKEN_PATH}")),
            jwks_uri: url(&format!("{base}/.well-known/jwks.json")),
            revocation_endpoint: revocation.then(|| url(&format!("{base}{REVOKE_PATH}"))),
        }
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn query_of(url: &Url) -> Vec<(String, String)> {
        url.query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    fn form_pairs(body: &[u8]) -> Vec<(String, String)> {
        let text = String::from_utf8_lossy(body);
        let parsed = url(&format!("http://form.invalid/?{text}"));
        query_of(&parsed)
    }

    /// Answers with `status` and `body` only when the form is exactly `expected`,
    /// so a stray or missing field fails the call instead of passing silently.
    struct FormReply {
        expected: Vec<(String, String)>,
        status: u16,
        body: String,
    }

    impl Respond for FormReply {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            if form_pairs(&request.body) == self.expected {
                ResponseTemplate::new(self.status).set_body_string(self.body.clone())
            } else {
                ResponseTemplate::new(400).set_body_string(r#"{"error":"form_mismatch"}"#)
            }
        }
    }

    fn request(registration: Registration) -> AuthorizeRequest {
        AuthorizeRequest {
            registration,
            host_id: HOST_ID.to_owned(),
            redirect_uri: LOOPBACK.to_owned(),
            state: "state-value".to_owned(),
            nonce: "nonce-value".to_owned(),
            code_challenge: "challenge-value".to_owned(),
        }
    }

    fn discovery_document(base: &str) -> Value {
        json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/api/accounts/authorize"),
            "token_endpoint": format!("{base}{TOKEN_PATH}"),
            "jwks_uri": format!("{base}/.well-known/jwks.json"),
            "revocation_endpoint": format!("{base}{REVOKE_PATH}"),
        })
    }

    async fn fetch_with(response: ResponseTemplate) -> Result<Discovery, OAuthError> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(DISCOVERY_PATH))
            .respond_with(response)
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config)?;
        fetch_discovery(&http, &config).await
    }

    async fn refresh_against(status: u16, body: String) -> Result<TokenResponse, OAuthError> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .and(header("accept", "application/json"))
            .respond_with(FormReply {
                expected: pairs(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", "oaiapp_test"),
                    ("refresh_token", "rt-old-value"),
                    ("resource", RESOURCE),
                ]),
                status,
                body,
            })
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config)?;
        let discovery = discovery_at(&server.uri(), false);
        refresh_tokens(&http, &discovery, "oaiapp_test", "rt-old-value").await
    }

    #[test]
    fn production_config_is_the_openai_issuer_and_builds_a_client() {
        let config = OAuthConfig::production().expect("the built-in issuer parses");
        assert_eq!(config.issuer().origin().ascii_serialization(), ISSUER);
        assert!(build_http_client(&config).is_ok());
    }

    #[tokio::test]
    async fn discovery_accepts_a_same_origin_document() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path(DISCOVERY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery_document(&base)))
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&base));
        let http = build_http_client(&config).expect("client");
        let discovery = fetch_discovery(&http, &config).await.expect("discovery");
        assert_eq!(discovery.issuer, base);
        assert_eq!(
            discovery.token_endpoint,
            url(&format!("{base}{TOKEN_PATH}"))
        );
        assert_eq!(
            discovery.revocation_endpoint,
            Some(url(&format!("{base}{REVOKE_PATH}")))
        );
    }

    #[tokio::test]
    async fn discovery_rejects_a_cross_origin_endpoint() {
        let server = MockServer::start().await;
        let mut document = discovery_document(&server.uri());
        document["token_endpoint"] = json!("https://evil.example/api/accounts/oauth/token");
        Mock::given(method("GET"))
            .and(path(DISCOVERY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(document))
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let outcome = fetch_discovery(&http, &config).await;
        assert_eq!(outcome.err(), Some(OAuthError::DiscoveryRejected));
    }

    #[tokio::test]
    async fn discovery_rejects_a_wrong_issuer() {
        let server = MockServer::start().await;
        let mut document = discovery_document(&server.uri());
        document["issuer"] = json!("https://auth.example");
        Mock::given(method("GET"))
            .and(path(DISCOVERY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(document))
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let outcome = fetch_discovery(&http, &config).await;
        assert_eq!(outcome.err(), Some(OAuthError::DiscoveryRejected));
    }

    #[tokio::test]
    async fn discovery_rejects_an_oversized_body() {
        let server = MockServer::start().await;
        let mut document = discovery_document(&server.uri());
        document["pad"] = json!("a".repeat(DISCOVERY_MAX_BYTES));
        Mock::given(method("GET"))
            .and(path(DISCOVERY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(document))
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let outcome = fetch_discovery(&http, &config).await;
        assert_eq!(outcome.err(), Some(OAuthError::DiscoveryRejected));
    }

    #[tokio::test]
    async fn discovery_reports_an_unsuccessful_status() {
        let outcome = fetch_with(ResponseTemplate::new(503)).await;
        assert_eq!(outcome.err(), Some(OAuthError::Status(503)));
    }

    #[test]
    fn new_registration_sends_the_agent_name_and_no_hints() {
        let url = build_authorize_url(
            &discovery_at("https://auth.openai.com", true),
            &request(Registration::New),
        )
        .expect("authorize url");
        assert_eq!(url.path(), "/api/accounts/authorize");
        assert_eq!(
            query_of(&url),
            pairs(&[
                ("client_id", "dynamic_agent_client"),
                ("agent_name_hint", "xFalcon Desktop"),
                ("ext_agent_host_id", HOST_ID),
                ("response_type", "code"),
                ("redirect_uri", LOOPBACK),
                (
                    "scope",
                    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
                ),
                ("resource", "https://api.openai.com/v1"),
                ("state", "state-value"),
                ("nonce", "nonce-value"),
                ("code_challenge_method", "S256"),
                ("code_challenge", "challenge-value"),
            ])
        );
    }

    #[test]
    fn returning_account_sends_its_client_and_hints_but_no_agent_name() {
        let registration = Registration::Returning {
            client_id: "oaiapp_saved".to_owned(),
            login_hint: Some("person@example.com".to_owned()),
            id_token_hint: Some("id.token.value".to_owned()),
            force_consent: true,
        };
        let url = build_authorize_url(
            &discovery_at("https://auth.openai.com", true),
            &request(registration),
        )
        .expect("authorize url");
        assert_eq!(
            query_of(&url),
            pairs(&[
                ("client_id", "oaiapp_saved"),
                ("login_hint", "person@example.com"),
                ("id_token_hint", "id.token.value"),
                ("prompt", "consent"),
                ("ext_agent_host_id", HOST_ID),
                ("response_type", "code"),
                ("redirect_uri", LOOPBACK),
                (
                    "scope",
                    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
                ),
                ("resource", "https://api.openai.com/v1"),
                ("state", "state-value"),
                ("nonce", "nonce-value"),
                ("code_challenge_method", "S256"),
                ("code_challenge", "challenge-value"),
            ])
        );
    }

    #[test]
    fn returning_account_asks_for_consent_only_when_forced() {
        let registration = Registration::Returning {
            client_id: "oaiapp_saved".to_owned(),
            login_hint: None,
            id_token_hint: Some("id.token.value".to_owned()),
            force_consent: false,
        };
        let url = build_authorize_url(
            &discovery_at("https://auth.openai.com", true),
            &request(registration),
        )
        .expect("authorize url");
        let keys: Vec<String> = query_of(&url).into_iter().map(|(key, _)| key).collect();
        assert_eq!(
            keys,
            [
                "client_id",
                "id_token_hint",
                "ext_agent_host_id",
                "response_type",
                "redirect_uri",
                "scope",
                "resource",
                "state",
                "nonce",
                "code_challenge_method",
                "code_challenge",
            ]
        );
    }

    #[test]
    fn a_redirect_other_than_the_loopback_callback_is_malformed() {
        for bad in [
            "http://localhost:53682/auth/callback",
            "http://127.0.0.1/auth/callback",
            "http://127.0.0.1:0/auth/callback",
            "http://127.0.0.1:99999/auth/callback",
            "http://127.0.0.1:+53682/auth/callback",
            "https://127.0.0.1:53682/auth/callback",
            "http://127.0.0.1:53682/other",
            "http://127.0.0.1:53682/auth/callback?extra=1",
        ] {
            let mut bad_request = request(Registration::New);
            bad_request.redirect_uri = bad.to_owned();
            let outcome =
                build_authorize_url(&discovery_at("https://auth.openai.com", true), &bad_request);
            assert_eq!(outcome, Err(OAuthError::Malformed), "{bad}");
        }
    }

    #[tokio::test]
    async fn exchange_posts_exactly_the_listed_fields_and_parses_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .and(header("accept", "application/json"))
            .respond_with(FormReply {
                expected: pairs(&[
                    ("grant_type", "authorization_code"),
                    ("client_id", "oaiapp_test"),
                    ("code", "code-value"),
                    ("code_verifier", "verifier-value"),
                    ("redirect_uri", LOOPBACK),
                    ("resource", RESOURCE),
                ]),
                status: 200,
                body: TOKEN_RESPONSE.to_owned(),
            })
            .expect(1)
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&server.uri(), true);
        let tokens = exchange_code(
            &http,
            &discovery,
            "oaiapp_test",
            "code-value",
            "verifier-value",
            LOOPBACK,
        )
        .await
        .expect("tokens");
        assert_eq!(tokens.access_token, "at-value");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-new-value"));
        assert_eq!(tokens.id_token.as_deref(), Some("id-value"));
        assert_eq!(tokens.expires_in, 3600);
        assert_eq!(tokens.scope, "openid profile");
        assert_eq!(tokens.earliest_refresh_at, Some(1_800_000_000));
    }

    #[tokio::test]
    async fn exchange_rejects_an_oversized_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("x".repeat(TOKEN_MAX_BYTES + 1)),
            )
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&server.uri(), true);
        let outcome = exchange_code(&http, &discovery, "oaiapp_test", "c", "v", LOOPBACK).await;
        assert_eq!(outcome.err(), Some(OAuthError::Malformed));
    }

    #[tokio::test]
    async fn refresh_posts_exactly_the_listed_fields_and_parses_the_answer() {
        let body = TOKEN_RESPONSE.to_owned();
        let tokens = refresh_against(200, body).await.expect("tokens");
        assert_eq!(tokens.access_token, "at-value");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-new-value"));
    }

    #[tokio::test]
    async fn each_terminal_refresh_code_is_refresh_terminal() {
        for code in [
            "invalid_grant",
            "invalid_refresh_token",
            "token_expired",
            "refresh_token_expired",
            "refresh_token_invalidated",
            "refresh_token_reused",
        ] {
            let outcome = refresh_against(400, format!(r#"{{"error":"{code}"}}"#)).await;
            assert_eq!(outcome.err(), Some(OAuthError::RefreshTerminal), "{code}");
        }
    }

    #[tokio::test]
    async fn an_invalid_client_refresh_is_a_configuration_fault() {
        let outcome =
            refresh_against(401, r#"{"error":{"code":"invalid_client"}}"#.to_owned()).await;
        assert_eq!(outcome.err(), Some(OAuthError::InvalidClient));
    }

    #[tokio::test]
    async fn a_503_refresh_is_status_503() {
        let outcome = refresh_against(503, String::new()).await;
        assert_eq!(outcome.err(), Some(OAuthError::Status(503)));
    }

    #[tokio::test]
    async fn a_dropped_connection_is_a_network_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let config = OAuthConfig::for_test(url(&base));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&base, false);
        let outcome = refresh_tokens(&http, &discovery, "oaiapp_test", "rt-old-value").await;
        assert_eq!(outcome.err(), Some(OAuthError::Network));
    }

    fn revoke_form() -> Vec<(String, String)> {
        pairs(&[
            ("token", "rt-old-value"),
            ("token_type_hint", "refresh_token"),
            ("client_id", "oaiapp_test"),
        ])
    }

    #[tokio::test]
    async fn revocation_is_confirmed_by_a_200() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(REVOKE_PATH))
            .respond_with(FormReply {
                expected: revoke_form(),
                status: 200,
                body: String::new(),
            })
            .expect(1)
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&server.uri(), true);
        let outcome =
            revoke_refresh_token(&http, &discovery, "oaiapp_test", "rt-old-value", &[]).await;
        assert_eq!(outcome, RevokeOutcome::Confirmed);
    }

    #[tokio::test]
    async fn revocation_retries_a_503_once_per_delay_then_gives_up() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(REVOKE_PATH))
            .respond_with(FormReply {
                expected: revoke_form(),
                status: 503,
                body: String::new(),
            })
            .expect(3)
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&server.uri(), true);
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let outcome =
            revoke_refresh_token(&http, &discovery, "oaiapp_test", "rt-old-value", &delays).await;
        assert_eq!(outcome, RevokeOutcome::Unconfirmed);
    }

    #[tokio::test]
    async fn revocation_does_not_retry_a_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(REVOKE_PATH))
            .respond_with(FormReply {
                expected: revoke_form(),
                status: 400,
                body: r#"{"error":"invalid_request"}"#.to_owned(),
            })
            .expect(1)
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&server.uri(), true);
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let outcome =
            revoke_refresh_token(&http, &discovery, "oaiapp_test", "rt-old-value", &delays).await;
        assert_eq!(outcome, RevokeOutcome::Unconfirmed);
    }

    #[tokio::test]
    async fn revocation_without_an_endpoint_is_unconfirmed_and_sends_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let config = OAuthConfig::for_test(url(&server.uri()));
        let http = build_http_client(&config).expect("client");
        let discovery = discovery_at(&server.uri(), false);
        let delays = [Duration::from_millis(1)];
        let outcome =
            revoke_refresh_token(&http, &discovery, "oaiapp_test", "rt-old-value", &delays).await;
        assert_eq!(outcome, RevokeOutcome::Unconfirmed);
    }

    #[test]
    fn token_debug_output_contains_no_token_text() {
        let tokens = TokenResponse {
            access_token: "access-secret".to_owned(),
            refresh_token: Some("refresh-secret".to_owned()),
            id_token: Some("id-secret".to_owned()),
            expires_in: 3600,
            scope: "openid".to_owned(),
            earliest_refresh_at: Some(1),
        };
        let debug = format!("{tokens:?}");
        for secret in ["access-secret", "refresh-secret", "id-secret"] {
            assert!(!debug.contains(secret), "{secret} leaked into Debug");
        }
        assert!(debug.contains("3600"));
    }

    #[test]
    fn earliest_refresh_at_is_epoch_seconds_or_rfc3339_and_other_shapes_are_dropped() {
        let parse = |value: &str| {
            parse_token_response(
                format!(
                    r#"{{"access_token":"a","expires_in":1,"scope":"s","earliest_refresh_at":{value}}}"#
                )
                .as_bytes(),
            )
            .expect("token response")
            .earliest_refresh_at
        };
        assert_eq!(parse("1800000000"), Some(1_800_000_000));
        assert_eq!(parse(r#""2030-01-01T00:00:00Z""#), Some(1_893_456_000));
        assert_eq!(parse("1800000000.5"), None);
        assert_eq!(parse(r#""not a time""#), None);
        assert_eq!(parse(r#"{"at":1}"#), None);
        assert_eq!(parse("null"), None);
    }

    #[test]
    fn both_error_body_shapes_yield_the_code() {
        assert_eq!(
            error_code(br#"{"error":"invalid_client"}"#).as_deref(),
            Some("invalid_client")
        );
        assert_eq!(
            error_code(br#"{"error":{"code":"invalid_client"}}"#).as_deref(),
            Some("invalid_client")
        );
        assert_eq!(error_code(b"<html>"), None);
    }

    #[test]
    fn classify_maps_each_code_and_keeps_the_status_for_unknown_ones() {
        assert_eq!(
            classify_token_error("invalid_grant", 400),
            OAuthError::InvalidGrant
        );
        assert_eq!(
            classify_token_error("refresh_token_reused", 400),
            OAuthError::RefreshTerminal
        );
        assert_eq!(
            classify_token_error("invalid_client", 401),
            OAuthError::InvalidClient
        );
        assert_eq!(
            classify_token_error("server_error", 500),
            OAuthError::Status(500)
        );
    }

    #[test]
    fn host_id_is_a_urn_uuid() {
        let id = new_host_id();
        let uuid = id.strip_prefix("urn:uuid:").expect("urn prefix");
        assert!(Uuid::parse_str(uuid).is_ok(), "{id}");
    }
}
