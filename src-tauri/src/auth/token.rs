use futures_util::StreamExt;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use xfalcon_contracts::auth::IdentityProjection;

pub(crate) const MAX_JWT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_JWKS_BYTES: usize = 256 * 1024;
const MAX_JWK_KEYS: usize = 32;
const MAX_JWK_KID_BYTES: usize = 256;
const MAX_JWK_MODULUS_BYTES: usize = 8 * 1024;
const MAX_JWK_EXPONENT_BYTES: usize = 32;
const MAX_JWK_FIELD_BYTES: usize = 16 * 1024;
const MAX_JWK_JSON_DEPTH: usize = 8;
const MAX_JWK_JSON_ITEMS: usize = 4_096;
/// Clock skew this app tolerates when reading a token's time claims.
///
/// Sixty seconds, not five. Five was tight enough that an ordinary workstation
/// broke sign-in: a Windows host five and a half seconds behind the tenant's
/// authorization server made every freshly minted token "not valid yet", and
/// the drift sitting near the boundary made it fail some attempts and not
/// others. Machines drift — after a resume, on a VM, and on a host whose WSL
/// clock and Windows clock disagree — and a person cannot be asked to hold
/// their clock to five seconds to sign in.
///
/// Sixty seconds is the conventional allowance (RFC 7519 §4.1.4 permits "some
/// small leeway, usually no more than a few minutes"), and it is what this
/// applies to `nbf`, `exp`, and `iat`.
///
/// What it costs: a token is accepted up to a minute before it becomes valid
/// and up to a minute after it expires. Nothing else moves — the signature, the
/// issuer, the audience, the algorithm, and the organization are checked
/// exactly as before, and a session is renewed from its refresh token long
/// before a minute of extra life on an access token could matter.
pub(crate) const CLOCK_TOLERANCE_SECONDS: u64 = 60;
const ALLOWED_ALGORITHMS: [Algorithm; 6] = [
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedAccessToken {
    pub(crate) owner: String,
    pub(crate) preferred_username: String,
    pub(crate) expires_at_ms: i64,
    /// The `nonce` the tenant echoed back, if its token format carries one.
    pub(crate) nonce: Option<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum TokenValidationError {
    #[error("access token is invalid")]
    InvalidToken,
    /// The token's time claims put it outside this device's idea of now, by
    /// more than [`CLOCK_TOLERANCE_SECONDS`]. Distinct because the remedy is
    /// this device's clock, and "contact your administrator" sends a person to
    /// someone who cannot fix it.
    #[error("this device's clock disagrees with the tenant's by more than a minute")]
    ClockSkew,
    #[error("access token owner does not match the configured tenant")]
    OwnerMismatch,
    #[error("access token identity does not match the protected resource")]
    IdentityMismatch,
    #[error("Casdoor signing keys are unavailable")]
    JwksUnavailable,
    #[error("Casdoor signing keys are invalid")]
    InvalidJwks,
}

#[derive(Debug, Deserialize)]
struct AccessTokenClaims {
    exp: u64,
    iat: u64,
    sub: String,
    owner: String,
    preferred_username: String,
    /// Optional: Casdoor echoes the sign-in request's nonce, but some tenant
    /// token formats omit it. Checked by `pkce::nonce_matches`, not here.
    #[serde(default)]
    nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RsaJwk {
    kty: String,
    kid: String,
    #[serde(default, rename = "use")]
    use_: Option<String>,
    #[serde(default)]
    alg: Option<String>,
    n: String,
    e: String,
}

/// The claims this app reads out of an access token, by name.
///
/// `owner` and `preferred_username` are Casdoor's, not OIDC's: an application
/// whose token format is set to carry only standard claims omits them, and the
/// payload then cannot be read at all. That is a tenant setting, and this is
/// how it is told apart from a signature or an audience.
const REQUIRED_CLAIM_NAMES: [&str; 5] = ["exp", "iat", "sub", "owner", "preferred_username"];

/// Which of [`REQUIRED_CLAIM_NAMES`] a token's payload does not carry.
///
/// Names, never values: a claim's value can name a person. Read without
/// verifying the signature, for a log line after verification already failed.
pub(crate) fn unverified_missing_claims(access_token: &str) -> String {
    let Some(claims) = unverified_payload(access_token) else {
        return "unreadable".to_owned();
    };
    let absent: Vec<&str> = REQUIRED_CLAIM_NAMES
        .into_iter()
        .filter(|name| claims.get(*name).is_none_or(serde_json::Value::is_null))
        .collect();
    if absent.is_empty() {
        return "none".to_owned();
    }
    absent.join(", ")
}

/// A token's payload as JSON, without verifying anything about it.
fn unverified_payload(access_token: &str) -> Option<Value> {
    use base64::Engine as _;

    let payload = access_token.split('.').nth(1)?;
    if payload.len() > MAX_JWT_BYTES {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()
}

/// The `iss` and `aud` a token claims, read without verifying its signature,
/// for a log line after verification has already failed.
///
/// Only those two, and only on the failure path. They are the tenant's own
/// URLs — the same pair `.env` and the tenant's metadata carry — and they are
/// what a verification failure is almost always about: an authorization server
/// that stopped honouring the resource indicator mints a token for its client
/// id instead of for the protected resource, and nothing else in the flow
/// notices. `sub` and `preferred_username` name a person and are never read
/// here, and no part of the token itself is returned.
pub(crate) fn unverified_issuer_and_audience(access_token: &str) -> (String, String) {
    let Some(claims) = unverified_payload(access_token) else {
        return ("unreadable".to_owned(), "unreadable".to_owned());
    };
    // `aud` is a string or an array of them (RFC 7519 §4.1.3), and rendering
    // the JSON covers both without a second shape to keep in step.
    let render = |value: Option<&Value>| match value {
        None => "absent".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    };
    (render(claims.get("iss")), render(claims.get("aud")))
}

pub(crate) async fn verify_access_token(
    client: &Client,
    issuer: &Url,
    jwks_uri: &Url,
    expected_audience: &str,
    expected_owner: &str,
    access_token: &str,
) -> Result<VerifiedAccessToken, TokenValidationError> {
    if access_token.is_empty() || access_token.len() > MAX_JWT_BYTES {
        return Err(TokenValidationError::InvalidToken);
    }
    validate_issuer_origin(issuer)?;
    validate_jwks_uri(issuer, jwks_uri)?;

    let header = decode_header(access_token).map_err(|_| TokenValidationError::InvalidToken)?;
    if !ALLOWED_ALGORITHMS.contains(&header.alg) {
        return Err(TokenValidationError::InvalidToken);
    }
    let kid = header
        .kid
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= MAX_JWK_KID_BYTES)
        .ok_or(TokenValidationError::InvalidToken)?;

    let jwks = fetch_jwks(client, jwks_uri).await?;
    let jwk = select_rsa_jwk(&jwks, kid, header.alg)?;
    let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|_| TokenValidationError::InvalidJwks)?;

    let audience_base = expected_audience.trim_end_matches('/');
    if audience_base.is_empty() || expected_owner.is_empty() {
        return Err(TokenValidationError::InvalidToken);
    }
    let expected_issuer = issuer.as_str().trim_end_matches('/').to_owned();
    let audience_with_slash = format!("{audience_base}/");
    let token = decode_for_audience(
        access_token,
        &decoding_key,
        header.alg,
        &expected_issuer,
        audience_base,
    )
    .or_else(|_| {
        decode_for_audience(
            access_token,
            &decoding_key,
            header.alg,
            &expected_issuer,
            &audience_with_slash,
        )
    })
    .map_err(|error| {
        // Which check refused it, by name. `InvalidToken` covers eight of them
        // and an operator cannot act on the difference between a signature and
        // an absent claim without being told which one it was.
        tracing::warn!(
            refused = %refusal(&error),
            "the authorization server's token was refused"
        );
        if is_clock_skew(&error) {
            return TokenValidationError::ClockSkew;
        }
        TokenValidationError::InvalidToken
    })?;
    let claims = token.claims;
    let now = u64::try_from(chrono::Utc::now().timestamp())
        .map_err(|_| TokenValidationError::InvalidToken)?;
    if claims.iat > now.saturating_add(CLOCK_TOLERANCE_SECONDS) {
        return Err(TokenValidationError::ClockSkew);
    }
    if claims.sub.trim().is_empty()
        || claims.owner.trim().is_empty()
        || claims.preferred_username.trim().is_empty()
    {
        return Err(TokenValidationError::InvalidToken);
    }
    if claims.owner != expected_owner {
        return Err(TokenValidationError::OwnerMismatch);
    }
    let expires_at_ms = i64::try_from(claims.exp)
        .ok()
        .and_then(|value| value.checked_mul(1_000))
        .ok_or(TokenValidationError::InvalidToken)?;

    Ok(VerifiedAccessToken {
        owner: claims.owner,
        preferred_username: claims.preferred_username,
        expires_at_ms,
        nonce: claims.nonce,
    })
}

/// Whether a refusal was about time rather than about the token.
///
/// Both directions: a token that has not started and one that has finished are
/// the same finding when a device's clock is wrong, and only the sign tells
/// which way it drifted.
fn is_clock_skew(error: &jsonwebtoken::errors::Error) -> bool {
    use jsonwebtoken::errors::ErrorKind;

    matches!(
        error.kind(),
        ErrorKind::ImmatureSignature | ErrorKind::ExpiredSignature
    )
}

/// Which of `decode`'s checks refused a token, as a short phrase.
///
/// The discriminant only. A type-mismatch error from the payload can quote the
/// value it read, and a claim's value can name a person — so the kinds that
/// carry one are reported by kind alone. A missing claim's *name* is carried,
/// because it is a name and it is the whole diagnosis.
fn refusal(error: &jsonwebtoken::errors::Error) -> String {
    use jsonwebtoken::errors::ErrorKind;

    match error.kind() {
        ErrorKind::InvalidToken => "the token is not a readable JWT".to_owned(),
        ErrorKind::InvalidSignature => "the signature does not match the tenant's key".to_owned(),
        ErrorKind::ExpiredSignature => "the token is already expired".to_owned(),
        ErrorKind::ImmatureSignature => "the token is not valid yet".to_owned(),
        ErrorKind::InvalidIssuer => "the issuer is not the configured one".to_owned(),
        ErrorKind::InvalidAudience => "the audience is not the protected resource".to_owned(),
        ErrorKind::InvalidSubject => "the subject is not the expected one".to_owned(),
        ErrorKind::InvalidAlgorithm => "the algorithm is not one of the accepted set".to_owned(),
        ErrorKind::MissingRequiredClaim(name) => format!("a required claim is absent: {name}"),
        ErrorKind::Json(_) => "the payload does not carry the claims this app reads".to_owned(),
        _ => "another check on the token".to_owned(),
    }
}

fn decode_for_audience(
    access_token: &str,
    decoding_key: &DecodingKey,
    algorithm: Algorithm,
    expected_issuer: &str,
    expected_audience: &str,
) -> jsonwebtoken::errors::Result<jsonwebtoken::TokenData<AccessTokenClaims>> {
    let mut validation = Validation::new(algorithm);
    validation.algorithms = ALLOWED_ALGORITHMS.to_vec();
    validation.leeway = CLOCK_TOLERANCE_SECONDS;
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["exp", "iat", "sub", "iss", "aud"]);
    validation.set_issuer(&[expected_issuer]);
    validation.set_audience(&[expected_audience]);
    decode::<AccessTokenClaims>(access_token, decoding_key, &validation)
}

pub(crate) fn ensure_identity_matches_token(
    token: &VerifiedAccessToken,
    identity: &IdentityProjection,
) -> Result<(), TokenValidationError> {
    if identity.user_id == token.preferred_username {
        Ok(())
    } else {
        Err(TokenValidationError::IdentityMismatch)
    }
}

async fn fetch_jwks(client: &Client, url: &Url) -> Result<Value, TokenValidationError> {
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|_| TokenValidationError::JwksUnavailable)?;
    if !response.status().is_success() {
        return Err(TokenValidationError::JwksUnavailable);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
    {
        return Err(TokenValidationError::InvalidJwks);
    }

    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| TokenValidationError::JwksUnavailable)?;
        if body
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > MAX_JWKS_BYTES)
        {
            return Err(TokenValidationError::InvalidJwks);
        }
        body.extend_from_slice(&chunk);
    }

    let jwks: Value =
        serde_json::from_slice(&body).map_err(|_| TokenValidationError::InvalidJwks)?;
    validate_jwks_shape(&jwks)?;
    Ok(jwks)
}

fn validate_jwks_shape(jwks: &Value) -> Result<(), TokenValidationError> {
    let valid_key_count = jwks
        .as_object()
        .and_then(|object| object.get("keys"))
        .and_then(Value::as_array)
        .is_some_and(|keys| !keys.is_empty() && keys.len() <= MAX_JWK_KEYS);
    if !valid_key_count {
        return Err(TokenValidationError::InvalidJwks);
    }

    let mut stack = vec![(jwks, 0_usize)];
    let mut items = 0_usize;
    while let Some((value, depth)) = stack.pop() {
        items += 1;
        if depth > MAX_JWK_JSON_DEPTH || items > MAX_JWK_JSON_ITEMS {
            return Err(TokenValidationError::InvalidJwks);
        }
        match value {
            Value::String(value) if value.len() > MAX_JWK_FIELD_BYTES => {
                return Err(TokenValidationError::InvalidJwks);
            }
            Value::Array(values) => {
                stack.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) if values.keys().any(|key| key.len() > MAX_JWK_FIELD_BYTES) => {
                return Err(TokenValidationError::InvalidJwks);
            }
            Value::Object(values) => {
                stack.extend(values.values().map(|value| (value, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(())
}

fn select_rsa_jwk(
    jwks: &Value,
    expected_kid: &str,
    expected_alg: Algorithm,
) -> Result<RsaJwk, TokenValidationError> {
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(TokenValidationError::InvalidJwks)?;
    let mut matches = keys.iter().filter(|key| {
        key.get("kid")
            .and_then(Value::as_str)
            .is_some_and(|kid| kid == expected_kid)
    });
    let key = matches.next().ok_or(TokenValidationError::InvalidToken)?;
    if matches.next().is_some() {
        return Err(TokenValidationError::InvalidJwks);
    }

    let key: RsaJwk =
        serde_json::from_value(key.clone()).map_err(|_| TokenValidationError::InvalidJwks)?;
    if key.kty != "RSA"
        || key.kid.len() > MAX_JWK_KID_BYTES
        || key.n.is_empty()
        || key.n.len() > MAX_JWK_MODULUS_BYTES
        || key.e.is_empty()
        || key.e.len() > MAX_JWK_EXPONENT_BYTES
        || key.use_.as_deref().is_some_and(|value| value != "sig")
        || key
            .alg
            .as_deref()
            .is_some_and(|value| value != algorithm_name(expected_alg))
    {
        return Err(TokenValidationError::InvalidJwks);
    }
    Ok(key)
}

fn validate_issuer_origin(issuer: &Url) -> Result<(), TokenValidationError> {
    let scheme_allowed = issuer.scheme() == "https" || (cfg!(test) && issuer.scheme() == "http");
    let path_allowed = issuer.path().is_empty() || issuer.path() == "/";
    if !scheme_allowed
        || issuer.host_str().is_none()
        || !issuer.username().is_empty()
        || issuer.password().is_some()
        || !path_allowed
        || issuer.query().is_some()
        || issuer.fragment().is_some()
    {
        return Err(TokenValidationError::InvalidToken);
    }
    Ok(())
}

fn validate_jwks_uri(issuer: &Url, jwks_uri: &Url) -> Result<(), TokenValidationError> {
    let scheme_allowed =
        jwks_uri.scheme() == "https" || (cfg!(test) && jwks_uri.scheme() == "http");
    if !scheme_allowed
        || jwks_uri.origin() != issuer.origin()
        || !jwks_uri.username().is_empty()
        || jwks_uri.password().is_some()
        || jwks_uri.fragment().is_some()
    {
        return Err(TokenValidationError::InvalidToken);
    }
    Ok(())
}

fn algorithm_name(algorithm: Algorithm) -> &'static str {
    match algorithm {
        Algorithm::RS256 => "RS256",
        Algorithm::RS384 => "RS384",
        Algorithm::RS512 => "RS512",
        Algorithm::PS256 => "PS256",
        Algorithm::PS384 => "PS384",
        Algorithm::PS512 => "PS512",
        _ => "",
    }
}
