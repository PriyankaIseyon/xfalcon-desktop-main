use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Url;
use serde::Serialize;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use xfalcon_contracts::auth::IdentityProjection;

use super::discovery::build_discovery_client;
use super::token::{
    ensure_identity_matches_token, unverified_issuer_and_audience, unverified_missing_claims,
    verify_access_token, TokenValidationError, VerifiedAccessToken, MAX_JWKS_BYTES, MAX_JWT_BYTES,
};

const KEY_ID: &str = "test-rsa-key";
const PRIVATE_KEY: &[u8] = br#"-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAofgWCuLjybRlzo0tZWJjNiuSfb4p4fAkd/wWJcyQoTbji9k0
l8W26mPddxHmfHQp+Vaw+4qPCJrcS2mJPMEzP1Pt0Bm4d4QlL+yRT+SFd2lZS+pC
gNMsD1W/YpRPEwOWvG6b32690r2jZ47soMZo9wGzjb/7OMg0LOL+bSf63kpaSHSX
ndS5z5rexMdbBYUsLA9e+KXBdQOS+UTo7WTBEMa2R2CapHg665xsmtdVMTBQY4uD
Zlxvb3qCo5ZwKh9kG4LT6/I5IhlJH7aGhyxXFvUK+DWNmoudF8NAco9/h9iaGNj8
q2ethFkMLs91kzk2PAcDTW9gb54h4FRWyuXpoQIDAQABAoIBABKucaRpzQorw35S
bEUAVx8dYXUdZOlJcHtiWQ+dC6V8ljxAHj/PLyzTveyI5QO/xkObCyjIL303l2cf
UhPu2MFaJdjVzqACXuOrLot/eSFvxjvqVidTtAZExqFRJ9mylUVAoLvhowVWmC1O
n95fZCXxTUtxNEG1Xcc7m0rtzJKs45J+N/V9DP1edYH6USyPSWGp6wuA+KgHRnKK
Vf9GRx80JQY7nVNkL17eHoTWEwga+lwi0FEoW9Y7lDtWXYmKBWhUE+U8PGxlJf8f
40493HDw1WRQ/aSLoS4QTp3rn7gYgeHEvfJdkkf0UMhlknlo53M09EFPdadQ4TlU
bjqKc50CgYEA4BzEEOtIpmVdVEZNCqS7baC4crd0pqnRH/5IB3jw3bcxGn6QLvnE
tfdUdiYrqBdss1l58BQ3KhooKeQTa9AB0Hw/Py5PJdTJNPY8cQn7ouZ2KKDcmnPG
BY5t7yLc1QlQ5xHdwW1VhvKn+nXqhJTBgIPgtldC+KDV5z+y2XDwGUcCgYEAuQPE
fgmVtjL0Uyyx88GZFF1fOunH3+7cepKmtH4pxhtCoHqpWmT8YAmZxaewHgHAjLYs
p1ZSe7zFYHj7C6ul7TjeLQeZD/YwD66t62wDmpe/HlB+TnBA+njbglfIsRLtXlnD
zQkv5dTltRJ11BKBBypeeF6689rjcJIDEz9RWdcCgYAHAp9XcCSrn8wVkMVkKdb7
DOX4IKjzdahm+ctDAJN4O/y7OW5FKebvUjdAIt2GuoTZ71iTG+7F0F+lP88jtjP4
U4qe7VHoewl4MKOfXZKTe+YCS1XbNvfgwJ3Ltyl1OH9hWvu2yza7q+d5PCsDzqtm
27kxuvULVeya+TEdAB1ijQKBgQCH/3r6YrVH/uCWGy6bzV1nGNOdjKc9tmkfOJmN
54dxdixdpozCQ6U4OxZrsj3FcOhHBsqAHvX2uuYjagqvo3cOj1TRqNocX40omfCC
Mx3bD1yPPf/6TI2XECva/ggqEY2mYzmIiA5LVVmc5nrybr+lssFKneeyxN2Wq93S
0iJMdQKBgCGHewxzoa1r8ZMD0LETNrToK423K377UCYqXfg5XMclbrjPbEC3YI1Z
NqMtuhdBJqUnBi6tjKMF+34Xf0CUN8ncuXGO2CAYvO8PdyCixHX52ybaDjy1FtCE
6yUXjoKNXKvUm7MWGsAYH6f4IegOetN5NvmUMFStCSkh7ixZLkN1
-----END RSA PRIVATE KEY-----"#;
const MODULUS: &str = "ofgWCuLjybRlzo0tZWJjNiuSfb4p4fAkd_wWJcyQoTbji9k0l8W26mPddxHmfHQp-Vaw-4qPCJrcS2mJPMEzP1Pt0Bm4d4QlL-yRT-SFd2lZS-pCgNMsD1W_YpRPEwOWvG6b32690r2jZ47soMZo9wGzjb_7OMg0LOL-bSf63kpaSHSXndS5z5rexMdbBYUsLA9e-KXBdQOS-UTo7WTBEMa2R2CapHg665xsmtdVMTBQY4uDZlxvb3qCo5ZwKh9kG4LT6_I5IhlJH7aGhyxXFvUK-DWNmoudF8NAco9_h9iaGNj8q2ethFkMLs91kzk2PAcDTW9gb54h4FRWyuXpoQ";

#[derive(Clone, Serialize)]
struct TestClaims {
    iss: String,
    aud: serde_json::Value,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<i64>,
    /// Casdoor mints one, and it is what a device with a slow clock trips on.
    #[serde(skip_serializing_if = "Option::is_none")]
    nbf: Option<i64>,
    sub: String,
    owner: Option<String>,
    preferred_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
}

impl TestClaims {
    fn valid(issuer: &str) -> Self {
        let now = Utc::now().timestamp();
        Self {
            iss: issuer.trim_end_matches('/').to_owned(),
            aud: serde_json::json!("https://tenant.example/mcp/"),
            exp: now + 300,
            iat: Some(now),
            nbf: None,
            sub: "subject-123".to_owned(),
            owner: Some("tenant".to_owned()),
            preferred_username: Some("user-123".to_owned()),
            nonce: None,
        }
    }
}

fn sign(claims: &TestClaims, algorithm: Algorithm, kid: Option<&str>) -> String {
    let mut header = Header::new(algorithm);
    header.kid = kid.map(str::to_owned);
    let key = match algorithm {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
            EncodingKey::from_secret(b"not-an-rsa-key")
        }
        _ => EncodingKey::from_rsa_pem(PRIVATE_KEY).expect("test RSA key"),
    };
    encode(&header, claims, &key).expect("signed test token")
}

fn jwks() -> serde_json::Value {
    jwks_with_alg("RS256")
}

fn jwks_with_alg(algorithm: &str) -> serde_json::Value {
    serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "kid": KEY_ID,
            "use": "sig",
            "alg": algorithm,
            "n": MODULUS,
            "e": "AQAB"
        }]
    })
}

async fn issuer_with_jwks(body: impl Into<Vec<u8>>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    server
}

async fn validate(
    issuer: &MockServer,
    token: &str,
) -> Result<VerifiedAccessToken, TokenValidationError> {
    let client = build_discovery_client().expect("hardened auth client");
    let issuer_url = Url::parse(&issuer.uri()).expect("issuer URL");
    let jwks_uri = Url::parse(&format!("{}/.well-known/jwks", issuer.uri())).expect("JWKS URL");
    verify_access_token(
        &client,
        &issuer_url,
        &jwks_uri,
        "https://tenant.example/mcp",
        "tenant",
        token,
    )
    .await
}

#[tokio::test]
async fn accepts_discovered_non_default_jwks_path() {
    let issuer = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/keys/current"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(serde_json::to_vec(&jwks()).expect("JWKS")),
        )
        .mount(&issuer)
        .await;
    let token = sign(
        &TestClaims::valid(&issuer.uri()),
        Algorithm::RS256,
        Some(KEY_ID),
    );
    let client = build_discovery_client().expect("hardened auth client");

    let verified = verify_access_token(
        &client,
        &Url::parse(&issuer.uri()).expect("issuer URL"),
        &Url::parse(&format!("{}/keys/current", issuer.uri())).expect("JWKS URL"),
        "https://tenant.example/mcp",
        "tenant",
        &token,
    )
    .await
    .expect("advertised JWKS path");

    assert_eq!(verified.preferred_username, "user-123");
}

#[tokio::test]
async fn reads_the_nonce_claim_when_the_tenant_token_carries_one() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.nonce = Some("request-nonce".to_owned());
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));

    let verified = validate(&issuer, &token).await.expect("valid access token");

    assert_eq!(verified.nonce.as_deref(), Some("request-nonce"));
}

#[tokio::test]
async fn accepts_a_token_without_a_nonce_claim() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let token = sign(
        &TestClaims::valid(&issuer.uri()),
        Algorithm::RS256,
        Some(KEY_ID),
    );

    let verified = validate(&issuer, &token).await.expect("valid access token");

    assert_eq!(verified.nonce, None);
}

#[tokio::test]
async fn accepts_valid_access_token_and_trailing_slash_audience_equivalence() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let token = sign(
        &TestClaims::valid(&issuer.uri()),
        Algorithm::RS256,
        Some(KEY_ID),
    );

    let verified = validate(&issuer, &token).await.expect("valid access token");

    assert_eq!(verified.preferred_username, "user-123");
    assert_eq!(verified.owner, "tenant");
}

#[tokio::test]
async fn accepts_all_configured_rsa_algorithms_with_a_discovered_jwks_key() {
    for (algorithm, jwk_algorithm) in [
        (Algorithm::RS256, "RS256"),
        (Algorithm::RS384, "RS384"),
        (Algorithm::RS512, "RS512"),
        (Algorithm::PS256, "PS256"),
        (Algorithm::PS384, "PS384"),
        (Algorithm::PS512, "PS512"),
    ] {
        let issuer =
            issuer_with_jwks(serde_json::to_vec(&jwks_with_alg(jwk_algorithm)).expect("JWKS"))
                .await;
        let token = sign(&TestClaims::valid(&issuer.uri()), algorithm, Some(KEY_ID));

        validate(&issuer, &token)
            .await
            .unwrap_or_else(|error| panic!("{jwk_algorithm} token was refused: {error}"));
    }
}

#[tokio::test]
async fn accepts_matching_resource_in_audience_array() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.aud = serde_json::json!(["https://unrelated.example", "https://tenant.example/mcp"]);
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));

    validate(&issuer, &token)
        .await
        .expect("matching resource audience");
}

#[tokio::test]
async fn rejects_wrong_issuer() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let token = sign(
        &TestClaims::valid("https://wrong-issuer.example"),
        Algorithm::RS256,
        Some(KEY_ID),
    );
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_wrong_resource_audience() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.aud = serde_json::json!("https://other.example/mcp");
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_expired_token() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    // Past the skew allowance, not merely past `exp`: a token a few seconds
    // over is a clock, and a token an hour over is a token.
    claims.exp = Utc::now().timestamp() - 3_600;
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::ClockSkew),
        "an expired token is a time refusal, and its remedy is this device's clock"
    );
}

#[tokio::test]
async fn accepts_a_token_within_the_skew_allowance_in_either_direction() {
    // The failure this exists for: a Windows host five and a half seconds
    // behind its tenant made every freshly minted token read as "not valid
    // yet", and sign-in failed with a message about the administrator.
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;

    let mut ahead = TestClaims::valid(&issuer.uri());
    ahead.iat = Some(Utc::now().timestamp() + 30);
    ahead.nbf = Some(Utc::now().timestamp() + 30);
    assert!(
        validate(&issuer, &sign(&ahead, Algorithm::RS256, Some(KEY_ID)))
            .await
            .is_ok(),
        "a device half a minute behind its tenant can still sign in"
    );

    let mut behind = TestClaims::valid(&issuer.uri());
    behind.exp = Utc::now().timestamp() - 30;
    assert!(
        validate(&issuer, &sign(&behind, Algorithm::RS256, Some(KEY_ID)))
            .await
            .is_ok(),
        "a device half a minute ahead of its tenant can still sign in"
    );
}

#[tokio::test]
async fn rejects_a_token_that_is_not_valid_yet_beyond_the_allowance() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.nbf = Some(Utc::now().timestamp() + 3_600);
    assert_eq!(
        validate(&issuer, &sign(&claims, Algorithm::RS256, Some(KEY_ID))).await,
        Err(TokenValidationError::ClockSkew)
    );
}

#[tokio::test]
async fn rejects_missing_required_identity_claim() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.preferred_username = None;
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_missing_required_iat_claim() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.iat = None;
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_negative_iat_claim() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.iat = Some(-1);
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_iat_beyond_clock_tolerance() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.iat = Some(Utc::now().timestamp() + 3_600);
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::ClockSkew)
    );
}

#[tokio::test]
async fn rejects_whitespace_only_sub() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.sub = " \t ".to_owned();
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_owner_mismatch() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.owner = Some("other-tenant".to_owned());
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::OwnerMismatch)
    );
}

#[tokio::test]
async fn rejects_whitespace_only_owner() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.owner = Some(" \t ".to_owned());
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_whitespace_only_preferred_username() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.preferred_username = Some(" \t ".to_owned());
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn preserves_raw_preferred_username_for_da_identity_binding() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let mut claims = TestClaims::valid(&issuer.uri());
    claims.preferred_username = Some(" user-123 ".to_owned());
    let token = sign(&claims, Algorithm::RS256, Some(KEY_ID));

    let verified = validate(&issuer, &token)
        .await
        .expect("nonempty signed preferred_username");

    assert_eq!(verified.preferred_username, " user-123 ");
    let identity = IdentityProjection::new("user-123", "tenant", "Display Name", None);
    assert_eq!(
        ensure_identity_matches_token(&verified, &identity),
        Err(TokenValidationError::IdentityMismatch)
    );
}

#[test]
fn rejects_preferred_username_mismatch_with_da_identity() {
    let token = VerifiedAccessToken {
        owner: "tenant".to_owned(),
        preferred_username: "verified-user".to_owned(),
        expires_at_ms: 123_000,
        nonce: None,
    };
    let identity = IdentityProjection::new("different-user", "tenant", "Display Name", None);
    assert_eq!(
        ensure_identity_matches_token(&token, &identity),
        Err(TokenValidationError::IdentityMismatch)
    );
}

#[tokio::test]
async fn rejects_unsupported_algorithm() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let token = sign(
        &TestClaims::valid(&issuer.uri()),
        Algorithm::HS256,
        Some(KEY_ID),
    );
    assert_eq!(
        validate(&issuer, &token).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_unknown_or_missing_kid() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    let claims = TestClaims::valid(&issuer.uri());
    for kid in [Some("unknown-key"), None] {
        let token = sign(&claims, Algorithm::RS256, kid);
        assert_eq!(
            validate(&issuer, &token).await,
            Err(TokenValidationError::InvalidToken)
        );
    }
}

#[tokio::test]
async fn rejects_malformed_or_oversized_jwt() {
    let issuer = issuer_with_jwks(serde_json::to_vec(&jwks()).expect("JWKS")).await;
    assert_eq!(
        validate(&issuer, "not-a-jwt").await,
        Err(TokenValidationError::InvalidToken)
    );
    assert_eq!(
        validate(&issuer, &"x".repeat(MAX_JWT_BYTES + 1)).await,
        Err(TokenValidationError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_malformed_or_oversized_jwks() {
    let malformed = issuer_with_jwks(b"{not-json".to_vec()).await;
    let malformed_token = sign(
        &TestClaims::valid(&malformed.uri()),
        Algorithm::RS256,
        Some(KEY_ID),
    );
    assert_eq!(
        validate(&malformed, &malformed_token).await,
        Err(TokenValidationError::InvalidJwks)
    );

    let oversized = issuer_with_jwks(vec![b'x'; MAX_JWKS_BYTES + 1]).await;
    let oversized_token = sign(
        &TestClaims::valid(&oversized.uri()),
        Algorithm::RS256,
        Some(KEY_ID),
    );
    assert_eq!(
        validate(&oversized, &oversized_token).await,
        Err(TokenValidationError::InvalidJwks)
    );
}

#[test]
fn the_coordinates_of_a_rejected_token_are_readable_without_verifying_it() {
    use base64::Engine as _;

    // An authorization server that stops honouring the resource indicator mints
    // a token for its own client id, and nothing before verification notices.
    // Reading the pair it claims is what tells an operator that, so the log line
    // has to survive a token that never verified.
    let payload = serde_json::json!({
        "iss": "https://auth.example",
        "aud": "a-client-id",
        "sub": "user-1",
        "preferred_username": "someone",
    })
    .to_string();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
    let token = format!("header.{encoded}.signature");

    let (issuer, audience) = unverified_issuer_and_audience(&token);
    assert_eq!(issuer, "https://auth.example");
    assert_eq!(audience, "a-client-id");
}

#[test]
fn an_array_audience_is_rendered_rather_than_dropped() {
    use base64::Engine as _;

    let payload = serde_json::json!({ "aud": ["one", "two"] }).to_string();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
    let (issuer, audience) = unverified_issuer_and_audience(&format!("h.{encoded}.s"));
    assert_eq!(issuer, "absent", "a claim the token omits says so");
    assert_eq!(audience, "[\"one\",\"two\"]");
}

#[test]
fn a_token_that_is_not_a_token_reads_as_unreadable_rather_than_panicking() {
    // Reached only after verification already failed, so the input here is
    // whatever the authorization server sent — including nothing usable.
    assert_eq!(
        unverified_issuer_and_audience("not-a-jwt"),
        ("unreadable".to_owned(), "unreadable".to_owned())
    );
    assert_eq!(
        unverified_issuer_and_audience("h.!!!not-base64!!!.s"),
        ("unreadable".to_owned(), "unreadable".to_owned())
    );
    assert_eq!(
        unverified_issuer_and_audience(""),
        ("unreadable".to_owned(), "unreadable".to_owned())
    );
}

#[test]
fn the_claims_a_payload_does_not_carry_are_named() {
    use base64::Engine as _;

    // `owner` and `preferred_username` are Casdoor's, not OIDC's. An
    // application set to mint only standard claims omits them, the payload
    // cannot be read at all, and every other check on the token passes — which
    // is indistinguishable from a signature problem without this.
    let encode = |value: serde_json::Value| {
        format!(
            "h.{}.s",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
        )
    };

    let standard_only = encode(serde_json::json!({
        "exp": 1_800_000_000,
        "iat": 1_700_000_000,
        "sub": "user-1",
    }));
    assert_eq!(
        unverified_missing_claims(&standard_only),
        "owner, preferred_username"
    );

    let complete = encode(serde_json::json!({
        "exp": 1_800_000_000,
        "iat": 1_700_000_000,
        "sub": "user-1",
        "owner": "org",
        "preferred_username": "someone",
    }));
    assert_eq!(unverified_missing_claims(&complete), "none");

    // A null claim is an absent one: serde reads neither.
    let nulled = encode(serde_json::json!({
        "exp": 1_800_000_000,
        "iat": 1_700_000_000,
        "sub": "user-1",
        "owner": serde_json::Value::Null,
        "preferred_username": "someone",
    }));
    assert_eq!(unverified_missing_claims(&nulled), "owner");

    assert_eq!(unverified_missing_claims("not-a-jwt"), "unreadable");
}
