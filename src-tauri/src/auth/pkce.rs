use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

const VERIFIER_LEN: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuthState {
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuthNonce {
    pub value: String,
}

pub fn generate_pkce() -> PkceChallenge {
    let mut verifier_bytes = [0u8; VERIFIER_LEN];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    PkceChallenge {
        verifier,
        challenge,
    }
}

pub fn generate_oauth_state() -> OAuthState {
    OAuthState {
        value: random_url_safe(24),
    }
}

pub fn generate_oauth_nonce() -> OAuthNonce {
    OAuthNonce {
        value: random_url_safe(24),
    }
}

fn random_url_safe(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Constant-time comparison for OAuth state/nonce values (loopback callback).
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    use subtle::ConstantTimeEq;
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

/// Whether the token's `nonce` claim, if it has one, is the sign-in request's.
///
/// An absent claim passes: some tenant token formats omit it. A present claim
/// must match exactly, and an empty one never does.
pub fn nonce_matches(claimed: Option<&str>, expected: &str) -> bool {
    match claimed {
        None => true,
        Some(claimed) => !claimed.is_empty() && constant_time_eq(claimed, expected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let pkce = generate_pkce();
        let digest = Sha256::digest(pkce.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(pkce.challenge, expected);
        assert!(pkce.verifier.len() >= 43);
    }

    #[test]
    fn oauth_state_and_nonce_are_unique() {
        let a = generate_oauth_state();
        let b = generate_oauth_state();
        assert_ne!(a.value, b.value);
        assert_ne!(generate_oauth_nonce().value, generate_oauth_nonce().value);
    }

    #[test]
    fn constant_time_eq_matches_only_equal_strings() {
        assert!(constant_time_eq("alpha", "alpha"));
        assert!(!constant_time_eq("alpha", "beta"));
    }

    #[test]
    fn nonce_matches_truth_table() {
        // Absent claim: accepted, because some tenant token formats omit it.
        assert!(nonce_matches(None, "expected-nonce"));
        // Present and equal.
        assert!(nonce_matches(Some("expected-nonce"), "expected-nonce"));
        // Present and different, same length.
        assert!(!nonce_matches(Some("expected-nonce"), "expected-nonc3"));
        // Present and different length, including a strict prefix.
        assert!(!nonce_matches(Some("expected"), "expected-nonce"));
        assert!(!nonce_matches(
            Some("expected-nonce-extra"),
            "expected-nonce"
        ));
        // Present but empty never matches, even against an empty expectation.
        assert!(!nonce_matches(Some(""), "expected-nonce"));
        assert!(!nonce_matches(Some(""), ""));
    }
}
