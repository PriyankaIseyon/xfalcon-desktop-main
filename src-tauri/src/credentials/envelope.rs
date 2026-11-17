use std::fmt;

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use xfalcon_contracts::credentials::ProviderId;
use zeroize::{Zeroize, Zeroizing};

pub(crate) const ENVELOPE_PREFIX: &str = "xfdc:v1:";
const PRODUCT_ID: &str = "xfalcon-desktop";
const ALGORITHM: &str = "AES-256-GCM";
const MAX_CONTEXT_FIELD_BYTES: usize = 160;
const MAX_ENVELOPE_BYTES: usize = 32 * 1024;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// The additional authenticated data one sealed value is bound to.
///
/// `scope` says *what the value is for*. A model-provider credential uses the
/// provider's own wire name (`"openai"`, `"anthropic"`, `"google"`); a
/// registered MCP server's credential uses
/// [`MCP_SERVER_SCOPE_PREFIX`]`<server id>`. Because the scope is part of the
/// AEAD's AAD, a value sealed for one scope cannot be opened as another — an
/// envelope stolen from the provider-key row cannot be replayed as an MCP
/// server credential, or vice versa.
///
/// The AAD field is still serialized under the name `provider` (see
/// [`AuthenticatedContext`]) even though it now carries scopes that are not
/// providers. Renaming it would change the AAD bytes and make every already
/// sealed credential un-openable, which is a migration this generalization has
/// no reason to force.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialContext {
    tenant_id: String,
    user_id: String,
    scope: String,
    device_key_id: String,
}

impl CredentialContext {
    /// A model-provider credential's context. Produces AAD byte-identical to
    /// what this function produced before `scope` was generalized, so existing
    /// sealed provider keys keep opening.
    pub(crate) fn new(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        provider: ProviderId,
        device_key_id: impl Into<String>,
    ) -> Result<Self, EnvelopeError> {
        Self::for_scope(tenant_id, user_id, provider_name(provider), device_key_id)
    }

    /// A context for any other sealed value, named by `scope`.
    pub(crate) fn for_scope(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        scope: impl Into<String>,
        device_key_id: impl Into<String>,
    ) -> Result<Self, EnvelopeError> {
        let context = Self {
            tenant_id: tenant_id.into(),
            user_id: user_id.into(),
            scope: scope.into(),
            device_key_id: device_key_id.into(),
        };
        // `scope` is validated with the rest now that it is caller-supplied
        // rather than derived from a closed enum.
        for value in [
            context.tenant_id.as_str(),
            context.user_id.as_str(),
            context.scope.as_str(),
            context.device_key_id.as_str(),
        ] {
            if value.is_empty()
                || value.len() > MAX_CONTEXT_FIELD_BYTES
                || !value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                })
            {
                return Err(EnvelopeError::InvalidContext);
            }
        }
        Ok(context)
    }
}

pub(crate) struct EncryptedCredential {
    serialized: Zeroizing<String>,
    device_key_id: String,
}

impl EncryptedCredential {
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        self.serialized.as_str()
    }

    pub(crate) fn into_parts(self) -> (Zeroizing<String>, String) {
        (self.serialized, self.device_key_id)
    }
}

impl fmt::Debug for EncryptedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncryptedCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum EnvelopeError {
    #[error("credential context is invalid")]
    InvalidContext,
    #[error("credential envelope is unsupported")]
    UnsupportedEnvelope,
    #[error("credential envelope authentication failed")]
    AuthenticationFailed,
}

/// The AAD, exactly as it is serialized. The `provider` field name is load
/// bearing history — see [`CredentialContext`]'s doc comment. Its value is the
/// context's `scope`.
#[derive(Serialize)]
struct AuthenticatedContext<'a> {
    product: &'static str,
    tenant_id: &'a str,
    user_id: &'a str,
    provider: &'a str,
    device_key_id: &'a str,
    envelope_id: Uuid,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireEnvelope {
    version: u8,
    algorithm: String,
    vault_key_version: u32,
    envelope_id: Uuid,
    nonce: String,
    ciphertext: String,
    context_digest: String,
}

impl Drop for WireEnvelope {
    fn drop(&mut self) {
        self.nonce.zeroize();
        self.ciphertext.zeroize();
        self.context_digest.zeroize();
    }
}

pub(crate) fn encrypt(
    plaintext: Zeroizing<String>,
    key: &[u8; 32],
    vault_key_version: u32,
    context: &CredentialContext,
) -> Result<EncryptedCredential, EnvelopeError> {
    if plaintext.is_empty() || vault_key_version == 0 {
        return Err(EnvelopeError::InvalidContext);
    }
    let envelope_id = Uuid::new_v4();
    let aad = canonical_aad(context, envelope_id)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| EnvelopeError::InvalidContext)?;
    let ciphertext = Zeroizing::new(
        cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| EnvelopeError::AuthenticationFailed)?,
    );
    let wire = WireEnvelope {
        version: 1,
        algorithm: ALGORITHM.to_owned(),
        vault_key_version,
        envelope_id,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext.as_slice()),
        context_digest: URL_SAFE_NO_PAD.encode(Sha256::digest(&aad)),
    };
    let payload =
        Zeroizing::new(serde_json::to_vec(&wire).map_err(|_| EnvelopeError::UnsupportedEnvelope)?);
    let serialized = Zeroizing::new(format!(
        "{ENVELOPE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(payload.as_slice())
    ));
    if serialized.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::UnsupportedEnvelope);
    }
    Ok(EncryptedCredential {
        serialized,
        device_key_id: context.device_key_id.clone(),
    })
}

pub(crate) fn decrypt(
    serialized: &str,
    key: &[u8; 32],
    context: &CredentialContext,
) -> Result<Zeroizing<String>, EnvelopeError> {
    let wire = parse_wire(serialized)?;
    let aad = canonical_aad(context, wire.envelope_id)?;
    let expected_digest = Sha256::digest(&aad);
    let supplied_digest = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(wire.context_digest.as_bytes())
            .map_err(|_| EnvelopeError::UnsupportedEnvelope)?,
    );
    if supplied_digest.len() != expected_digest.len()
        || !bool::from(supplied_digest.ct_eq(expected_digest.as_slice()))
    {
        return Err(EnvelopeError::AuthenticationFailed);
    }
    let nonce = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(wire.nonce.as_bytes())
            .map_err(|_| EnvelopeError::UnsupportedEnvelope)?,
    );
    let ciphertext = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(wire.ciphertext.as_bytes())
            .map_err(|_| EnvelopeError::UnsupportedEnvelope)?,
    );
    if nonce.len() != NONCE_BYTES || ciphertext.len() < TAG_BYTES {
        return Err(EnvelopeError::AuthenticationFailed);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| EnvelopeError::InvalidContext)?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| EnvelopeError::AuthenticationFailed)?;
    match String::from_utf8(plaintext) {
        Ok(plaintext) => Ok(Zeroizing::new(plaintext)),
        Err(error) => {
            let mut plaintext = error.into_bytes();
            plaintext.zeroize();
            Err(EnvelopeError::AuthenticationFailed)
        }
    }
}

pub(crate) fn envelope_key_version(serialized: &str) -> Result<u32, EnvelopeError> {
    Ok(parse_wire(serialized)?.vault_key_version)
}

fn parse_wire(serialized: &str) -> Result<WireEnvelope, EnvelopeError> {
    if serialized.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::UnsupportedEnvelope);
    }
    let encoded = serialized
        .strip_prefix(ENVELOPE_PREFIX)
        .ok_or(EnvelopeError::UnsupportedEnvelope)?;
    let payload = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| EnvelopeError::UnsupportedEnvelope)?,
    );
    let wire: WireEnvelope =
        serde_json::from_slice(&payload).map_err(|_| EnvelopeError::UnsupportedEnvelope)?;
    if wire.version != 1
        || wire.algorithm != ALGORITHM
        || wire.vault_key_version == 0
        || wire.envelope_id.is_nil()
    {
        return Err(EnvelopeError::UnsupportedEnvelope);
    }
    Ok(wire)
}

fn canonical_aad(context: &CredentialContext, envelope_id: Uuid) -> Result<Vec<u8>, EnvelopeError> {
    serde_json::to_vec(&AuthenticatedContext {
        product: PRODUCT_ID,
        tenant_id: &context.tenant_id,
        user_id: &context.user_id,
        provider: &context.scope,
        device_key_id: &context.device_key_id,
        envelope_id,
    })
    .map_err(|_| EnvelopeError::InvalidContext)
}

/// Scope prefix for a registered MCP server's credential. The `:` separator is
/// inside [`CredentialContext::for_scope`]'s permitted character set, and no
/// provider wire name can collide with it.
pub(crate) const MCP_SERVER_SCOPE_PREFIX: &str = "mcp-server:";

/// The scope naming one registered server's credential.
pub(crate) fn mcp_server_scope(server_id: &str) -> String {
    format!("{MCP_SERVER_SCOPE_PREFIX}{server_id}")
}

/// Scope prefix for a registered MCP server's OAuth grant
/// (`crate::mcp::oauth::store`).
///
/// Distinct from [`MCP_SERVER_SCOPE_PREFIX`] rather than sharing it: the two
/// hold different things for the same server id — a value the user pasted versus
/// a grant a browser flow produced — and a shared scope would let one open as
/// the other. Since the scope is part of the AEAD's additional authenticated
/// data, separating the prefixes makes that confusion a decryption failure
/// instead of a type error nobody wrote.
pub(crate) const MCP_OAUTH_SCOPE_PREFIX: &str = "mcp-oauth:";

/// The scope naming one registered server's OAuth grant.
pub(crate) fn mcp_oauth_scope(server_id: &str) -> String {
    format!("{MCP_OAUTH_SCOPE_PREFIX}{server_id}")
}

/// Scope prefix for the operator-issued OAuth client an account holds for one
/// registered MCP server (`crate::mcp::oauth::store::DeviceOauthClients`).
///
/// A third prefix for the same reason the second one exists: the client this
/// device was told to authenticate as outlives any single grant — signing out
/// keeps it — so it is stored apart from the grant, and the AAD must make that
/// separation enforceable rather than conventional.
pub(crate) const MCP_OAUTH_CLIENT_SCOPE_PREFIX: &str = "mcp-oauth-client:";

/// The scope naming one registered server's operator-issued OAuth client.
pub(crate) fn mcp_oauth_client_scope(server_id: &str) -> String {
    format!("{MCP_OAUTH_CLIENT_SCOPE_PREFIX}{server_id}")
}

/// A model provider's scope spelling, as it appears in the AAD. Public within
/// the crate so `super::vault` never has to restate the mapping.
pub(crate) const fn provider_scope(provider: ProviderId) -> &'static str {
    provider_name(provider)
}

const fn provider_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::OpenAi => "openai",
        ProviderId::Anthropic => "anthropic",
        ProviderId::Google => "google",
        ProviderId::OpenRouter => "openrouter",
        ProviderId::CustomOpenAi => "custom_openai",
        ProviderId::ClaudeCode => "claude_code",
        ProviderId::ChatgptPlan => "chatgpt_plan",
    }
}
