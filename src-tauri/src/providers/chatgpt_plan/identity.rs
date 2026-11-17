//! Verifies the ID token from OpenAI's sign-in.
//!
//! The ID token proves who signed in. It does not authorize plan usage, which
//! takes the `chatgpt.tokens.use.direct` scope on the token response; that is
//! checked on the stored scopes by the session.

use std::{fmt, time::Duration};

use futures_util::StreamExt;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::auth::pkce::constant_time_eq;

/// Tolerance on `exp`, in seconds. The ID token is read at the moment sign-in
/// completes, so there is no reason to accept one long after it expired; the
/// tolerance only absorbs ordinary drift between this device and the issuer.
const CLOCK_LEEWAY_SECONDS: i64 = 5;
/// How far `iat` may lie in the future, in seconds. Anything further is a clock
/// that cannot be trusted, or a token that was not issued by this sign-in.
const MAX_ISSUED_AT_SKEW_SECONDS: i64 = 300;
/// A key set is a few kilobytes. The cap bounds how much an unreachable or
/// hostile endpoint can make this process buffer.
const MAX_JWKS_BYTES: usize = 256 * 1024;
const JWKS_TIMEOUT: Duration = Duration::from_secs(20);

/// Who signed in. The caller keeps the client id with `subject`: neither `sub`
/// nor `email` identifies a workspace on its own.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct Identity {
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) expires_at: i64,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("subject", &"<redacted>")
            .field("email", &self.email.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum IdentityError {
    #[error("could not fetch the signing keys")]
    Network,
    #[error("the ID token is malformed")]
    Malformed,
    #[error("no signing key matches the ID token")]
    NoMatchingKey,
    #[error("the ID token signature is invalid")]
    BadSignature,
    #[error("the ID token claims were rejected: {0}")]
    BadClaims(&'static str),
}

#[derive(Deserialize)]
struct IdTokenClaims {
    iss: Option<String>,
    aud: Option<Audience>,
    azp: Option<String>,
    exp: Option<i64>,
    iat: Option<i64>,
    sub: Option<String>,
    nonce: Option<String>,
    email: Option<String>,
}

/// The key set as received: each key is left unparsed until it is read one at a time.
#[derive(Deserialize)]
struct RawKeySet {
    keys: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

/// Fetches the key set at the discovery document's `jwks_uri`.
pub(crate) async fn fetch_jwks(
    http: &reqwest::Client,
    jwks_uri: &reqwest::Url,
) -> Result<JwkSet, IdentityError> {
    let response = http
        .get(jwks_uri.clone())
        .timeout(JWKS_TIMEOUT)
        .send()
        .await
        .map_err(|_| IdentityError::Network)?;
    if !response.status().is_success() {
        return Err(IdentityError::Network);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
    {
        return Err(IdentityError::Malformed);
    }

    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| IdentityError::Network)?;
        if body.len() + chunk.len() > MAX_JWKS_BYTES {
            return Err(IdentityError::Malformed);
        }
        body.extend_from_slice(&chunk);
    }
    let raw: RawKeySet = serde_json::from_slice(&body).map_err(|_| IdentityError::Malformed)?;
    // A key this code cannot read, such as an unknown `kty`, is dropped alone. The
    // other keys in the set still verify tokens.
    let keys = raw
        .keys
        .into_iter()
        .filter_map(|key| serde_json::from_value::<Jwk>(key).ok())
        .collect();
    Ok(JwkSet { keys })
}

/// Verifies an ID token's signature and claims, and returns the identity it names.
///
/// `expected_nonce` is the nonce saved for this sign-in attempt. A token that
/// does not echo it was issued for some other request.
pub(crate) fn verify_id_token(
    id_token: &str,
    jwks: &JwkSet,
    issuer: &str,
    client_id: &str,
    expected_nonce: &str,
    now_unix: i64,
) -> Result<Identity, IdentityError> {
    let header = decode_header(id_token).map_err(|_| IdentityError::Malformed)?;
    // Refused before any key is looked up, so the token never chooses how it is verified.
    if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err(IdentityError::Malformed);
    }
    let jwk = select_key(jwks, header.kid.as_deref(), header.alg)?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| IdentityError::NoMatchingKey)?;

    // The library's claim checks read the system clock, so they are off here and
    // every claim is checked below against `now_unix` and the values required.
    let mut validation = Validation::new(header.alg);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_aud = false;
    let claims = decode::<IdTokenClaims>(id_token, &key, &validation)
        .map_err(|error| match error.kind() {
            ErrorKind::InvalidSignature => IdentityError::BadSignature,
            _ => IdentityError::Malformed,
        })?
        .claims;

    if claims.iss.as_deref() != Some(issuer) {
        return Err(IdentityError::BadClaims("issuer"));
    }
    if !audience_matches(claims.aud.as_ref(), claims.azp.as_deref(), client_id) {
        return Err(IdentityError::BadClaims("audience"));
    }
    let expires_at = claims.exp.ok_or(IdentityError::BadClaims("expiry"))?;
    if now_unix.saturating_sub(CLOCK_LEEWAY_SECONDS) > expires_at {
        return Err(IdentityError::BadClaims("expiry"));
    }
    if claims
        .iat
        .is_some_and(|iat| iat > now_unix.saturating_add(MAX_ISSUED_AT_SKEW_SECONDS))
    {
        return Err(IdentityError::BadClaims("issued-at"));
    }
    let subject = claims
        .sub
        .filter(|sub| !sub.trim().is_empty())
        .ok_or(IdentityError::BadClaims("subject"))?;
    if !echoes_nonce(claims.nonce.as_deref(), expected_nonce) {
        return Err(IdentityError::BadClaims("nonce"));
    }

    Ok(Identity {
        subject,
        email: claims.email,
        expires_at,
    })
}

/// The key the token's `kid` names. Without a `kid`, a set of exactly one key is
/// unambiguous. A `kid` that two keys share names neither, so it is refused.
fn select_key<'a>(
    jwks: &'a JwkSet,
    kid: Option<&str>,
    alg: Algorithm,
) -> Result<&'a Jwk, IdentityError> {
    let mut candidates = jwks.keys.iter().filter(|jwk| match kid {
        Some(kid) => jwk.common.key_id.as_deref() == Some(kid),
        None => true,
    });
    let jwk = candidates.next().ok_or(IdentityError::NoMatchingKey)?;
    if candidates.next().is_some() || !key_fits(jwk, alg) {
        return Err(IdentityError::NoMatchingKey);
    }
    Ok(jwk)
}

/// A key is used only for the algorithm its type and curve define, and only when
/// the key does not declare a different one.
fn key_fits(jwk: &Jwk, alg: Algorithm) -> bool {
    let type_fits = match (&jwk.algorithm, alg) {
        (AlgorithmParameters::RSA(_), Algorithm::RS256) => true,
        (AlgorithmParameters::EllipticCurve(params), Algorithm::ES256) => {
            params.curve == EllipticCurve::P256
        }
        _ => false,
    };
    let declared_fits = match jwk.common.key_algorithm {
        None => true,
        Some(KeyAlgorithm::RS256) => alg == Algorithm::RS256,
        Some(KeyAlgorithm::ES256) => alg == Algorithm::ES256,
        Some(_) => false,
    };
    // A key published for encryption, or for any other use, does not verify signatures.
    let use_fits = matches!(
        jwk.common.public_key_use,
        None | Some(PublicKeyUse::Signature)
    );
    type_fits && declared_fits && use_fits
}

/// `aud` is a string or an array of them (RFC 7519 section 4.1.3). The token is
/// for this client when the client id is one of them. With several audiences the
/// token must also name this client in `azp`, the party it was issued to.
fn audience_matches(audience: Option<&Audience>, azp: Option<&str>, client_id: &str) -> bool {
    if client_id.is_empty() {
        return false;
    }
    match audience {
        Some(Audience::One(aud)) => aud == client_id,
        Some(Audience::Many(auds)) => {
            auds.iter().any(|aud| aud == client_id) && (auds.len() <= 1 || azp == Some(client_id))
        }
        None => false,
    }
}

/// An absent nonce never matches. An empty expectation never matches either, so
/// a caller that failed to save one cannot accept every token.
fn echoes_nonce(claimed: Option<&str>, expected: &str) -> bool {
    !expected.is_empty() && claimed.is_some_and(|claimed| constant_time_eq(claimed, expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ISSUER: &str = "https://auth.openai.com";
    const CLIENT_ID: &str = "app_test_client";
    const NONCE: &str = "nonce-of-this-attempt";
    const NOW: i64 = 1_800_000_000;
    const RSA_KID: &str = "rsa-test";
    const OTHER_RSA_KID: &str = "rsa-other";
    const EC_KID: &str = "ec-test";

    // Generated for these tests only. They sign nothing outside this module.
    const RSA_PRIVATE_PEM: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAoSm0iPW8QHXs1t6cVRmec32HpZKecwoUXhXFXSqCOj5tc3JK
C5b7rPAKFvdRIvn3z8eTwF2GzWxhJUU3bYj84DyGoNEysV5Z1C9SCmIeo79EO0BX
uE1uCwyWPmYAgfUXA+WJv0b+zdEZCROH6RYXiGfkV1negHdhUKV6jtdytlU8Oemq
UFTZO19wKMeKrcr5Ry2UjJPjmyAGGkfkVRWP2Ga6uDxvjnMDlzjKjO8Y5/qDrlrQ
ADg6cG8u+uetbTnf4yOaj/2wrNExNP1r8mSdkbx9ut3mLPN639I7wPyPWGa4ifNc
CUUk9xaJiKA58ZafB5Kalcgqikk+WBD4hvzcJQIDAQABAoIBAAnW60ISaL4xxPZA
bKCIeiNYdmvuNfTVmHU/FIfx5LYMIjASegFXpEGEfUU2L9K4GsBpkbSEsUqA87Rs
E6yBGNlS8LBIXXU1Srw48GQYZgD+2Dpa3RAwUAfmKpUgMqc8kL1o6f5O5hcoTUQ/
2EQrmF+4YhlQMBS9VShwlfk7H7KQjEsfHi9fW6hilz7y2siOZwLwVD2AFYXmXCnx
e5XmxV9X/nJAYUARm03+shqgK6QaXiWQaa6bonl5AXhXmv0kBvZ2c7+TcKmu5ke9
ihlCbwQhxDgjqacqa00EL2aq3wuavlyIuHsSt9uq8RGCIdWN6WrxXM6kH0Q2KEUs
o16NkOECgYEA2UhDWd3NZ+Q0xZujpv1PBwdl690DApz6ZoX2UDIHL5iCpIDvJaW9
40I4g1ooy5xWJUwySaBn8JImbcCprq/vn+hQS8mySbVwHo4VIeYK7bAcnTDdYfPq
fWIJnpxQzpd+T81C7T6LYe2ckw7EOFvss4ZN4Kca2ap1zPrsaoKCqUUCgYEAveFz
0HzsTibk0tsweocoPXfBsoSExOgU0xRx+5FqM6Xufzk3aE2EZcq1UbfucsUwAUwA
XJgQU8dArlTF5kBWCBgJqUXDPMutRn+y/Psvhcc/Jht77mSBQj3y0F5hI34Dppqd
VXP41aWBJTuLyQvd+C6rGCSFTWXmo1O2K6LQ5WECgYBmnmJwxviD4Jur2F3qOChi
wXO5eaA6U7lPQmWaaoZKTFUnLF3GAgk0k8ns/1/VCfkFlH8Llt6AGAZeP2fkQOOD
Okzx/khd13skcyY35FW0/EAHXVSxNmnA3dR9PIZNi+7z8ZVyRt+9WGNicM7rbz7T
atLNDx73PjJ0pO2TQCBFoQKBgAuwDvLsXZdJ6ZfZChjgrOdrsa6NXabJKk772TBE
+SaIti96lCCFdPrXCWYoTjMeVCAgT31jls2xXBfe+/d6SlFhOWcD9kqNtvK59YiA
5aH0a7yqwGwiH5q2OXX9+Ti4Dv14Uurpoo10DIkAK1CKvHUEWl+4D8R92z5+bp4s
/XLBAoGASce8fdfKPzyvhYljCsBTmTQjjtY7dQh0MrBZynwiaoYQ0CCCcoZ3ftIf
S/hjoedDMjrV/6w6egWFRYhvP+ofiu6jRzvE3rTN9kFLDqKAF5RanYODA0PgLN+H
i1yytNYlOMX+CYCaLrGFGrUXj9XB4fMVgm2n+Nz6tVTOhASPTuU=
-----END RSA PRIVATE KEY-----"#;
    const RSA_MODULUS: &str = "oSm0iPW8QHXs1t6cVRmec32HpZKecwoUXhXFXSqCOj5tc3JKC5b7rPAKFvdRIvn3z8eTwF2GzWxhJUU3bYj84DyGoNEysV5Z1C9SCmIeo79EO0BXuE1uCwyWPmYAgfUXA-WJv0b-zdEZCROH6RYXiGfkV1negHdhUKV6jtdytlU8OemqUFTZO19wKMeKrcr5Ry2UjJPjmyAGGkfkVRWP2Ga6uDxvjnMDlzjKjO8Y5_qDrlrQADg6cG8u-uetbTnf4yOaj_2wrNExNP1r8mSdkbx9ut3mLPN639I7wPyPWGa4ifNcCUUk9xaJiKA58ZafB5Kalcgqikk-WBD4hvzcJQ";
    const OTHER_RSA_PRIVATE_PEM: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA4uAHhcTQIksmxrOzMMGgcZ66XlBodo+dwYoq1eT/cCJmw5oQ
5xOrj6Vm9re7jnEanLBYRlCyexfHDEiPQlvf5uZvTiDTpaRJtK9KL2FZ5YuHS2bu
+21JCTGTotwDnGBjo+ig3kRFOf5ek+9RwaUtC0oaj8DQTK0dbaolTxjW+9AhlX0E
J9GDIV2i7JPz0ea2V9oABdNp06rmQVH2xuAb0D/20vz+Vb5T4WChvaG9BMeGL5Qw
Xz9z78t9EegNQuczScFT1AGb6y04HLqoyM7adzT3zT2a8D1HC7z6T533eeEjQf+b
v2Tzwpkfj2ptrFna5D0zjxB6fEgKgm53LrARnwIDAQABAoIBAE1f2FDv2YgIl1Xn
DXLQZna5ipgF1kqxGj1acNd3/QHySZfmIGOemsEnyODVspqWylur7L82YFiIuHLh
ADLCgK8P4eAEZAo26Ig9zBAypGUb2+2EUnbfQ6xmAxLk9DpNyU2qVcrX/8ASl5De
asSfH6idAkgKmz+7PrEa+RRrxU4lfhuqBnUm1YV9RGiY/I1dRSvtxTBbe3hiJk3+
HaZ7fbtX5N4JJyxgnPOjm6CYIkyUgmUOC9y+XuUlDZ+TAOaZa1u098m92m6JC9Lu
7QO/wsiSNLKzD9iaQDVHCl9n4AB5QtDqVBXsF+CSknFP4sSD4I7sqOdkFVerSfk6
2vse3TkCgYEA8k8P7Xk55w1E/inFclPdthPN6IP6BggAL+iYaThVzLM316e7WpVW
ztFL3AcD0eXj/hJVlGWUVWbAm9EnBl+ENx287pvz9q8DYdrSNx6ULdNoXGfifLtt
SV5GGv7O22WgLPtzaJXUIw31yMCboeBzHnwHQezbQXNV3ZYikW8G8FMCgYEA77G4
4DfNptF8AUYA4I2r4H29ZIIsdBh6repC3+PbUW3OlniCyRWWYl3szryKh1cGdD9H
mfBdjEod6yFXSwTZuau1vNevxQC/fGR9ReSWhr9YaCo5Pk2eslo9TIhGrk22rddA
SQNU2KtydHCQSAGoQW6H1tq+Pt4gB0fNLBARIAUCgYEAwBLn/0hvG7YrEIYwG5Zb
U2+Tc1LaqFlDPLbVyeJ8MM2H4Iz3OLDUpRDHX1y4LHRdLBOAfyeeU6wl1d4PXz/r
4HzUPRimZM8KkVER/dthfonsWgOHR3STBuuSCugdNVDzjXStYSQTis4QHNiBBeqT
loCeiCWnD9TLJeFS+ZHXID0CgYBeOpvXaZirUAdT6NVHcepz4nqmHJalPUpdPI71
vXnm1Sg190JTEu1/1jTWQUCkXA3pfUcEuX3NPQXt/bl3gHynhV1408IBzH16xXaK
F4clieuTZ6YBXDpbGyamA5zlwR9j0A8xpOGiHCRuqiHObPhcYLt+N5g61s/4jL2X
6OkSkQKBgFFd0tMK8AYi4yGY31NaaBO577qgzmKQodjroQKO3VvgWV2cYmr0O1qR
aYFcsPOBx7iut94UYHIhxzTsY8hHVfokh3HHyDAIkYbnFmweFbwNnc+bbsOCsY6x
SBaSJzUecXnSPTOqKOKcufPsvE/XGU7bzxz3B41FpNogxmak+8Y1
-----END RSA PRIVATE KEY-----"#;
    const OTHER_RSA_MODULUS: &str = "4uAHhcTQIksmxrOzMMGgcZ66XlBodo-dwYoq1eT_cCJmw5oQ5xOrj6Vm9re7jnEanLBYRlCyexfHDEiPQlvf5uZvTiDTpaRJtK9KL2FZ5YuHS2bu-21JCTGTotwDnGBjo-ig3kRFOf5ek-9RwaUtC0oaj8DQTK0dbaolTxjW-9AhlX0EJ9GDIV2i7JPz0ea2V9oABdNp06rmQVH2xuAb0D_20vz-Vb5T4WChvaG9BMeGL5QwXz9z78t9EegNQuczScFT1AGb6y04HLqoyM7adzT3zT2a8D1HC7z6T533eeEjQf-bv2Tzwpkfj2ptrFna5D0zjxB6fEgKgm53LrARnw";
    const EC_PRIVATE_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgatiOW+fLuaw9j4B8
fzUyV1AT6qCrbhejXGKshxdBtZOhRANCAAS6jl7hBR1H3oV/kXgdG+bLumwITyyP
XId5+bhp3uP69GzvkDt5rtB8YoY7ji/2wkWsmdf4H1wQQ9Y2+Zrf3pYX
-----END PRIVATE KEY-----"#;
    const EC_X: &str = "uo5e4QUdR96Ff5F4HRvmy7psCE8sj1yHefm4ad7j-vQ";
    const EC_Y: &str = "bO-QO3mu0HxihjuOL_bCRayZ1_gfXBBD1jb5mt_elhc";

    fn rsa_key() -> EncodingKey {
        EncodingKey::from_rsa_pem(RSA_PRIVATE_PEM.as_bytes()).expect("test RSA key")
    }

    fn other_rsa_key() -> EncodingKey {
        EncodingKey::from_rsa_pem(OTHER_RSA_PRIVATE_PEM.as_bytes()).expect("test RSA key")
    }

    fn ec_key() -> EncodingKey {
        EncodingKey::from_ec_pem(EC_PRIVATE_PEM.as_bytes()).expect("test EC key")
    }

    fn sign(claims: &Value, algorithm: Algorithm, kid: Option<&str>, key: &EncodingKey) -> String {
        let mut header = Header::new(algorithm);
        header.kid = kid.map(str::to_owned);
        encode(&header, claims, key).expect("signed test token")
    }

    fn rs256(claims: &Value) -> String {
        sign(claims, Algorithm::RS256, Some(RSA_KID), &rsa_key())
    }

    /// A token whose header names `alg: none` and whose signature is empty.
    fn unsigned_token(claims: &Value) -> String {
        let segment =
            |value: &Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).expect("JSON"));
        format!(
            "{}.{}.",
            segment(&json!({"alg": "none", "typ": "JWT"})),
            segment(claims)
        )
    }

    fn claims() -> Value {
        json!({
            "iss": ISSUER,
            "aud": CLIENT_ID,
            "exp": NOW + 600,
            "sub": "user-123",
            "nonce": NONCE,
            "email": "person@example.com",
        })
    }

    fn with(mut claims: Value, name: &str, value: Value) -> Value {
        claims[name] = value;
        claims
    }

    fn without(mut claims: Value, name: &str) -> Value {
        claims.as_object_mut().expect("claims object").remove(name);
        claims
    }

    fn rsa_jwk(kid: &str, modulus: &str) -> Value {
        json!({"kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256", "n": modulus, "e": "AQAB"})
    }

    fn ec_jwk() -> Value {
        json!({
            "kty": "EC",
            "crv": "P-256",
            "kid": EC_KID,
            "use": "sig",
            "alg": "ES256",
            "x": EC_X,
            "y": EC_Y,
        })
    }

    fn jwk_set(keys: Vec<Value>) -> JwkSet {
        serde_json::from_value(json!({"keys": keys})).expect("test JWK set")
    }

    fn trusted_jwks() -> JwkSet {
        jwk_set(vec![rsa_jwk(RSA_KID, RSA_MODULUS)])
    }

    fn verify(token: &str, jwks: &JwkSet) -> Result<Identity, IdentityError> {
        verify_id_token(token, jwks, ISSUER, CLIENT_ID, NONCE, NOW)
    }

    #[test]
    fn rs256_token_verifies_to_its_identity() {
        assert_eq!(
            verify(&rs256(&claims()), &trusted_jwks()),
            Ok(Identity {
                subject: "user-123".to_owned(),
                email: Some("person@example.com".to_owned()),
                expires_at: NOW + 600,
            })
        );
    }

    #[test]
    fn es256_token_verifies_against_a_p256_key() {
        let jwks = jwk_set(vec![ec_jwk()]);
        let token = sign(&claims(), Algorithm::ES256, Some(EC_KID), &ec_key());
        assert_eq!(
            verify(&token, &jwks).map(|identity| identity.subject),
            Ok("user-123".to_owned())
        );
    }

    #[test]
    fn audience_array_containing_the_client_is_accepted_when_azp_names_it() {
        let claims = with(
            with(claims(), "aud", json!(["app_other", CLIENT_ID])),
            "azp",
            json!(CLIENT_ID),
        );
        assert!(verify(&rs256(&claims), &trusted_jwks()).is_ok());
    }

    #[test]
    fn audience_array_of_several_entries_without_azp_is_rejected() {
        let claims = with(claims(), "aud", json!(["app_other", CLIENT_ID]));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("audience"))
        );
    }

    #[test]
    fn audience_array_whose_azp_names_another_client_is_rejected() {
        let claims = with(
            with(claims(), "aud", json!([CLIENT_ID, "app_other"])),
            "azp",
            json!("app_other"),
        );
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("audience"))
        );
    }

    #[test]
    fn single_entry_audience_array_needs_no_azp() {
        let claims = with(claims(), "aud", json!([CLIENT_ID]));
        assert!(verify(&rs256(&claims), &trusted_jwks()).is_ok());
    }

    #[test]
    fn issued_at_more_than_five_minutes_ahead_is_rejected() {
        let ahead = with(claims(), "iat", json!(NOW + 301));
        assert_eq!(
            verify(&rs256(&ahead), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("issued-at"))
        );

        let edge = with(claims(), "iat", json!(NOW + 300));
        assert!(verify(&rs256(&edge), &trusted_jwks()).is_ok());
        let absent = without(claims(), "iat");
        assert!(verify(&rs256(&absent), &trusted_jwks()).is_ok());
    }

    #[test]
    fn a_key_published_for_encryption_does_not_verify_signatures() {
        let jwks = jwk_set(vec![json!({
            "kty": "RSA",
            "kid": RSA_KID,
            "use": "enc",
            "alg": "RS256",
            "n": RSA_MODULUS,
            "e": "AQAB",
        })]);
        assert_eq!(
            verify(&rs256(&claims()), &jwks).err(),
            Some(IdentityError::NoMatchingKey)
        );
    }

    #[tokio::test]
    async fn fetch_jwks_skips_a_key_it_cannot_read_and_keeps_the_rest() {
        let body = json!({
            "keys": [
                {"kty": "UNKNOWN-TYPE", "kid": "unreadable"},
                rsa_jwk(RSA_KID, RSA_MODULUS),
            ]
        });
        let (_server, url) = serve_jwks(ResponseTemplate::new(200).set_body_json(body)).await;
        let jwks = fetch_jwks(&reqwest::Client::new(), &url)
            .await
            .expect("fetched key set");
        assert_eq!(jwks.keys.len(), 1);
        assert!(verify(&rs256(&claims()), &jwks).is_ok());
    }

    #[test]
    fn identity_debug_shows_no_subject_or_email() {
        let identity = Identity {
            subject: "user-123".to_owned(),
            email: Some("person@example.com".to_owned()),
            expires_at: NOW + 600,
        };
        let debug = format!("{identity:?}");

        assert!(!debug.contains("user-123"), "{debug}");
        assert!(!debug.contains("person@example.com"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let claims = with(claims(), "iss", json!("https://evil.example"));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("issuer"))
        );
    }

    #[test]
    fn wrong_audience_string_is_rejected() {
        let claims = with(claims(), "aud", json!("app_other"));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("audience"))
        );
    }

    #[test]
    fn wrong_audience_array_is_rejected() {
        let claims = with(claims(), "aud", json!(["app_other", "app_another"]));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("audience"))
        );
    }

    #[test]
    fn token_expired_beyond_the_leeway_is_rejected() {
        let claims = with(claims(), "exp", json!(NOW - 6));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("expiry"))
        );
    }

    #[test]
    fn token_expired_within_the_leeway_is_accepted() {
        let claims = with(claims(), "exp", json!(NOW - 5));
        assert!(verify(&rs256(&claims), &trusted_jwks()).is_ok());
    }

    #[test]
    fn missing_nonce_is_rejected() {
        let claims = without(claims(), "nonce");
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("nonce"))
        );
    }

    #[test]
    fn wrong_nonce_is_rejected() {
        let claims = with(claims(), "nonce", json!("nonce-of-another-attempt"));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("nonce"))
        );
    }

    #[test]
    fn empty_subject_is_rejected() {
        let claims = with(claims(), "sub", json!(""));
        assert_eq!(
            verify(&rs256(&claims), &trusted_jwks()).err(),
            Some(IdentityError::BadClaims("subject"))
        );
    }

    #[test]
    fn token_signed_by_another_key_is_rejected() {
        let token = sign(&claims(), Algorithm::RS256, Some(RSA_KID), &other_rsa_key());
        assert_eq!(
            verify(&token, &trusted_jwks()).err(),
            Some(IdentityError::BadSignature)
        );
    }

    #[test]
    fn alg_none_token_is_rejected() {
        assert_eq!(
            verify(&unsigned_token(&claims()), &trusted_jwks()).err(),
            Some(IdentityError::Malformed)
        );
    }

    #[test]
    fn hs256_token_is_rejected_even_with_a_known_kid() {
        let key = EncodingKey::from_secret(b"not-a-public-key");
        let token = sign(&claims(), Algorithm::HS256, Some(RSA_KID), &key);
        assert_eq!(
            verify(&token, &trusted_jwks()).err(),
            Some(IdentityError::Malformed)
        );
    }

    #[test]
    fn unknown_kid_has_no_matching_key() {
        let token = sign(
            &claims(),
            Algorithm::RS256,
            Some("rotated-away"),
            &rsa_key(),
        );
        assert_eq!(
            verify(&token, &trusted_jwks()).err(),
            Some(IdentityError::NoMatchingKey)
        );
    }

    #[test]
    fn missing_kid_with_one_key_in_the_set_is_accepted() {
        let token = sign(&claims(), Algorithm::RS256, None, &rsa_key());
        assert!(verify(&token, &trusted_jwks()).is_ok());
    }

    #[test]
    fn missing_kid_with_two_keys_in_the_set_is_rejected() {
        let jwks = jwk_set(vec![
            rsa_jwk(RSA_KID, RSA_MODULUS),
            rsa_jwk(OTHER_RSA_KID, OTHER_RSA_MODULUS),
        ]);
        let token = sign(&claims(), Algorithm::RS256, None, &rsa_key());
        assert_eq!(
            verify(&token, &jwks).err(),
            Some(IdentityError::NoMatchingKey)
        );
    }

    async fn serve_jwks(response: ResponseTemplate) -> (MockServer, reqwest::Url) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(response)
            .mount(&server)
            .await;
        let url = reqwest::Url::parse(&format!("{}/.well-known/jwks.json", server.uri()))
            .expect("JWKS URL");
        (server, url)
    }

    #[tokio::test]
    async fn fetch_jwks_returns_a_set_that_verifies_tokens() {
        let body = json!({"keys": [rsa_jwk(RSA_KID, RSA_MODULUS)]});
        let (_server, url) = serve_jwks(ResponseTemplate::new(200).set_body_json(body)).await;
        let jwks = fetch_jwks(&reqwest::Client::new(), &url)
            .await
            .expect("fetched key set");
        assert!(verify(&rs256(&claims()), &jwks).is_ok());
    }

    #[tokio::test]
    async fn fetch_jwks_refuses_a_non_success_status() {
        let (_server, url) = serve_jwks(ResponseTemplate::new(503)).await;
        assert_eq!(
            fetch_jwks(&reqwest::Client::new(), &url).await.err(),
            Some(IdentityError::Network)
        );
    }

    #[tokio::test]
    async fn fetch_jwks_refuses_a_body_that_is_not_a_key_set() {
        let (_server, url) =
            serve_jwks(ResponseTemplate::new(200).set_body_string("<html>not keys</html>")).await;
        assert_eq!(
            fetch_jwks(&reqwest::Client::new(), &url).await.err(),
            Some(IdentityError::Malformed)
        );
    }

    #[tokio::test]
    async fn fetch_jwks_refuses_a_body_over_the_cap() {
        // Valid JSON, so only the size can make this body fail.
        let body = json!({
            "keys": [rsa_jwk(RSA_KID, RSA_MODULUS)],
            "padding": "x".repeat(MAX_JWKS_BYTES),
        });
        let (_server, url) = serve_jwks(ResponseTemplate::new(200).set_body_json(body)).await;
        assert_eq!(
            fetch_jwks(&reqwest::Client::new(), &url).await.err(),
            Some(IdentityError::Malformed)
        );
    }
}
