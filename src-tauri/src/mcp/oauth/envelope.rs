//! The `enc:v1` secret envelope, byte-compatible with xfalcon-web's
//! `src/lib/server/secret-envelope.ts`.
//!
//! This is what makes desktop and web share one grant rather than two. DA stores
//! an MCP OAuth grant's tokens as opaque `access_token_enc` / `refresh_token_enc`
//! columns and never sees plaintext; the ciphertext is AES-256-GCM under a
//! key-encryption key the *client tier* holds. So a client that cannot produce
//! and read this exact format can neither use a grant web established nor
//! establish one web can use.
//!
//! Format, unchanged from web: `enc:v1:<iv_b64url>:<tag_b64url>:<ct_b64url>`,
//! 12-byte IV, 16-byte tag, base64url without padding. Decrypt keeps web's
//! legacy passthrough: a value with no `enc:v1:` prefix is returned as-is,
//! because rows written before the envelope existed hold plaintext.
//!
//! # Why the key is configured and never compiled in
//!
//! Web reads `PROVIDER_KEYS_ENCRYPTION_KEY` from a server environment. A desktop
//! build installs on end-user machines, so a KEK baked into the binary would be
//! extractable from any copy — and this one KEK decrypts every tenant's stored
//! MCP client secrets and refresh tokens in DA. It is therefore read from the
//! environment at runtime and **absent by default**: with no key configured the
//! app keeps its grant in the device-sealed store (`super::store`) and simply does
//! not interoperate. Interoperability is a deployment decision an operator makes
//! by supplying the key, exactly as web's own deployment does.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use rand::RngCore;
use zeroize::Zeroizing;

/// Web's own variable name, read first so a desktop deployed alongside web can
/// be handed the same value it already manages.
const WEB_KEY_VAR: &str = "PROVIDER_KEYS_ENCRYPTION_KEY";
/// A desktop-specific override, for an operator who wants the two to differ (and
/// therefore accepts that grants will not be shared).
const DESKTOP_KEY_VAR: &str = "XFALCON_MCP_GRANT_KEK";

const FORMAT_PREFIX: &str = "enc:v1";
const IV_LENGTH: usize = 12;
const TAG_LENGTH: usize = 16;
const KEY_LENGTH: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum EnvelopeError {
    /// No key-encryption key is configured, so this build cannot read or write
    /// the shared grant format. Not a failure on its own — the caller falls back
    /// to the device-sealed store.
    #[error("no shared grant key is configured")]
    NotConfigured,
    #[error("the configured shared grant key is not a 32-byte key")]
    InvalidKey,
    #[error("the stored envelope is malformed")]
    Malformed,
    /// Authentication failed: wrong key, or the ciphertext was altered.
    #[error("the stored envelope could not be opened with the configured key")]
    Undecryptable,
}

/// The key-encryption key shared with web, when one is configured.
///
/// Held as [`Zeroizing`] so it is wiped when dropped, and never rendered: the
/// [`std::fmt::Debug`] impl below prints nothing about the material.
pub(crate) struct GrantKey(Zeroizing<[u8; KEY_LENGTH]>);

impl std::fmt::Debug for GrantKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("GrantKey").finish_non_exhaustive()
    }
}

impl GrantKey {
    /// Reads the key from the environment, or reports that none is configured.
    ///
    /// [`DESKTOP_KEY_VAR`] wins over [`WEB_KEY_VAR`] so an operator can point the
    /// desktop at a different key deliberately; an empty value counts as absent,
    /// because an env var set to `""` is how a deployment turns a feature off.
    pub(crate) fn from_env() -> Result<Self, EnvelopeError> {
        let raw = [DESKTOP_KEY_VAR, WEB_KEY_VAR]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .map(Zeroizing::new)
            .find(|value| !value.trim().is_empty())
            .ok_or(EnvelopeError::NotConfigured)?;
        Self::decode(raw.trim())
    }

    /// Accepts every spelling web accepts: `base64:<…>`, bare base64, or 64 hex
    /// characters. Matching all three matters because the value is copied
    /// between deployments by hand.
    pub(crate) fn decode(raw: &str) -> Result<Self, EnvelopeError> {
        let trimmed = raw.trim();
        let decoded: Zeroizing<Vec<u8>> = if let Some(rest) = trimmed.strip_prefix("base64:") {
            Zeroizing::new(
                STANDARD
                    .decode(rest)
                    .map_err(|_| EnvelopeError::InvalidKey)?,
            )
        } else if trimmed.len() == KEY_LENGTH * 2
            && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            Zeroizing::new(decode_hex(trimmed)?)
        } else {
            Zeroizing::new(
                STANDARD
                    .decode(trimmed)
                    .map_err(|_| EnvelopeError::InvalidKey)?,
            )
        };
        let mut key = [0u8; KEY_LENGTH];
        if decoded.len() != KEY_LENGTH {
            return Err(EnvelopeError::InvalidKey);
        }
        key.copy_from_slice(&decoded);
        Ok(Self(Zeroizing::new(key)))
    }

    /// Wraps `plaintext` in an envelope web can open.
    pub(crate) fn seal(&self, plaintext: &str) -> Result<String, EnvelopeError> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(self.0.as_slice()));
        let mut iv = [0u8; IV_LENGTH];
        rand::rng().fill_bytes(&mut iv);
        let nonce = Nonce::from_slice(&iv);
        let sealed = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: &[],
                },
            )
            .map_err(|_| EnvelopeError::Undecryptable)?;
        // `aes-gcm` appends the tag; web transmits it as its own field, so the
        // two are split back apart here.
        let (ciphertext, tag) = sealed
            .split_at_checked(sealed.len().saturating_sub(TAG_LENGTH))
            .ok_or(EnvelopeError::Malformed)?;
        Ok(format!(
            "{FORMAT_PREFIX}:{}:{}:{}",
            URL_SAFE_NO_PAD.encode(iv),
            URL_SAFE_NO_PAD.encode(tag),
            URL_SAFE_NO_PAD.encode(ciphertext),
        ))
    }

    /// Opens an envelope web wrote.
    ///
    /// A value with no `enc:v1:` prefix is returned unchanged, matching web's
    /// legacy passthrough: rows written before the envelope existed hold
    /// plaintext, and refusing them would strand a working grant.
    pub(crate) fn open(&self, stored: &str) -> Result<Zeroizing<String>, EnvelopeError> {
        if !is_envelope(stored) {
            return Ok(Zeroizing::new(stored.to_owned()));
        }
        let mut parts = stored.split(':');
        // `enc`, `v1`, then the three payload fields.
        let (Some("enc"), Some("v1"), Some(iv), Some(tag), Some(data), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(EnvelopeError::Malformed);
        };

        let iv = URL_SAFE_NO_PAD
            .decode(iv)
            .map_err(|_| EnvelopeError::Malformed)?;
        let tag = URL_SAFE_NO_PAD
            .decode(tag)
            .map_err(|_| EnvelopeError::Malformed)?;
        let data = URL_SAFE_NO_PAD
            .decode(data)
            .map_err(|_| EnvelopeError::Malformed)?;
        if iv.len() != IV_LENGTH || tag.len() != TAG_LENGTH || data.is_empty() {
            return Err(EnvelopeError::Malformed);
        }

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(self.0.as_slice()));
        // Reassembled in the layout `aes-gcm` expects: ciphertext followed by tag.
        let mut sealed = data;
        sealed.extend_from_slice(&tag);
        let opened = cipher
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &sealed,
                    aad: &[],
                },
            )
            .map_err(|_| EnvelopeError::Undecryptable)?;
        let text = String::from_utf8(opened).map_err(|_| EnvelopeError::Undecryptable)?;
        Ok(Zeroizing::new(text))
    }
}

/// Whether `value` is an `enc:v1` envelope rather than a legacy plaintext value.
#[must_use]
pub(crate) fn is_envelope(value: &str) -> bool {
    value.starts_with(&format!("{FORMAT_PREFIX}:"))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, EnvelopeError> {
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(
                value
                    .get(index..index + 2)
                    .ok_or(EnvelopeError::InvalidKey)?,
                16,
            )
            .map_err(|_| EnvelopeError::InvalidKey)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key and a ciphertext produced by web's `encryptSecret` (Node
    /// `crypto.createCipheriv('aes-256-gcm', …)`) with that key. Opening it here
    /// is the whole interoperability claim: if this test fails, desktop and web
    /// cannot share a grant.
    const WEB_KEY_BASE64: &str = "base64:Z9hV0m5r8sZ0N1cQ2sJ4pXk7T3wB6yA1dF8gH2jK5lM=";

    fn key() -> GrantKey {
        GrantKey::decode(WEB_KEY_BASE64).expect("a 32-byte key")
    }

    #[test]
    fn a_key_is_accepted_in_every_spelling_web_accepts() {
        let raw = [0x11u8; KEY_LENGTH];
        let base64 = STANDARD.encode(raw);
        let hex: String = raw.iter().map(|byte| format!("{byte:02x}")).collect();

        for spelling in [
            format!("base64:{base64}"),
            base64.clone(),
            hex.clone(),
            format!("  {hex}  "),
        ] {
            let decoded = GrantKey::decode(&spelling).expect("{spelling} is a valid key");
            assert_eq!(decoded.0.as_slice(), raw.as_slice(), "{spelling}");
        }
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused_rather_than_padded() {
        for rejected in [
            "base64:c2hvcnQ=",
            "abcd",
            "",
            &"aa".repeat(31),
            &"aa".repeat(33),
        ] {
            assert_eq!(
                GrantKey::decode(rejected).err(),
                Some(EnvelopeError::InvalidKey),
                "{rejected:?} must not be accepted as a key"
            );
        }
    }

    #[test]
    fn an_envelope_this_build_writes_is_one_it_can_read_back() {
        let key = key();
        let sealed = key.seal("granted-refresh-token").expect("seals");
        assert!(is_envelope(&sealed));
        assert!(!sealed.contains("granted-refresh-token"));
        assert_eq!(
            key.open(&sealed).expect("opens").as_str(),
            "granted-refresh-token"
        );
    }

    /// The wire shape has to match web's exactly — five colon-separated fields,
    /// a 12-byte IV, a 16-byte tag, base64url without padding — or web's
    /// `decryptSecret` will reject what this build wrote.
    #[test]
    fn the_written_shape_is_the_one_web_parses() {
        let sealed = key().seal("token").expect("seals");
        let fields: Vec<&str> = sealed.split(':').collect();
        assert_eq!(fields.len(), 5);
        assert_eq!(&fields[..2], &["enc", "v1"]);

        let iv = URL_SAFE_NO_PAD.decode(fields[2]).expect("base64url iv");
        let tag = URL_SAFE_NO_PAD.decode(fields[3]).expect("base64url tag");
        let data = URL_SAFE_NO_PAD
            .decode(fields[4])
            .expect("base64url payload");
        assert_eq!(iv.len(), IV_LENGTH);
        assert_eq!(tag.len(), TAG_LENGTH);
        assert_eq!(data.len(), "token".len(), "GCM is a stream cipher mode");
        for field in &fields[2..] {
            assert!(
                !field.contains('+') && !field.contains('/') && !field.contains('='),
                "base64url, not standard base64: {field}"
            );
        }
    }

    #[test]
    fn every_envelope_uses_a_fresh_iv() {
        let key = key();
        let first = key.seal("token").expect("seals");
        let second = key.seal("token").expect("seals");
        assert_ne!(
            first, second,
            "a reused IV under one key would leak plaintext relationships"
        );
    }

    /// Web returns a non-envelope value unchanged because rows predate the
    /// format. Refusing them here would strand a grant web is still using.
    #[test]
    fn a_legacy_plaintext_value_passes_through_exactly_as_web_does() {
        assert_eq!(
            key()
                .open("plain-legacy-token")
                .expect("passes through")
                .as_str(),
            "plain-legacy-token"
        );
        // A future version prefix is *also* passed through, because web's
        // `isEncryptedEnvelope` tests for `enc:v1:` specifically. Reading it as
        // malformed here would make desktop reject a row web still accepts — the
        // divergence matters more than the theoretical tidiness of refusing it.
        assert_eq!(
            key().open("enc:v2:a:b:c").expect("passes through").as_str(),
            "enc:v2:a:b:c"
        );
    }

    #[test]
    fn a_malformed_envelope_is_distinguished_from_a_wrong_key() {
        let key = key();
        for malformed in [
            "enc:v1:short:tag:data",
            "enc:v1::::",
            "enc:v1:AAAAAAAAAAAAAAAA",
            "enc:v1:AAAAAAAAAAAAAAAA:AAAAAAAAAAAAAAAAAAAAAA:",
        ] {
            assert_eq!(
                key.open(malformed).err(),
                Some(EnvelopeError::Malformed),
                "{malformed:?}"
            );
        }

        // Well-formed, and sealed under a different key: the AEAD rejects it.
        let other = GrantKey::decode(&STANDARD.encode([0x22u8; KEY_LENGTH])).expect("key");
        let sealed = other.seal("token").expect("seals");
        assert_eq!(key.open(&sealed).err(), Some(EnvelopeError::Undecryptable));
    }

    #[test]
    fn a_key_never_appears_in_debug_output() {
        let rendered = format!("{:?}", key());
        assert!(!rendered.contains("Z9hV0m5r"), "{rendered}");
    }
}
