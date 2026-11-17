use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use xfalcon_contracts::credentials::ProviderId;
use zeroize::Zeroizing;

use super::envelope::{
    decrypt, encrypt, envelope_key_version, mcp_oauth_client_scope, mcp_oauth_scope,
    mcp_server_scope, provider_scope, CredentialContext, EncryptedCredential, EnvelopeError,
};
use crate::persistence::SecretStore;

pub(crate) struct CredentialVault {
    secrets: Arc<dyn SecretStore>,
}

impl CredentialVault {
    pub(crate) const fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }

    pub(crate) fn seal(
        &self,
        account_id: &str,
        tenant_id: &str,
        user_id: &str,
        provider: ProviderId,
        plaintext: Zeroizing<String>,
    ) -> Result<EncryptedCredential, VaultError> {
        self.seal_scoped(
            account_id,
            tenant_id,
            user_id,
            provider_scope(provider),
            plaintext,
        )
    }

    /// Seals a value bound to `scope` rather than to a model provider — see
    /// [`CredentialContext`]'s doc comment. Used for a registered MCP server's
    /// credential, which never leaves the device.
    pub(crate) fn seal_scoped(
        &self,
        account_id: &str,
        tenant_id: &str,
        user_id: &str,
        scope: &str,
        plaintext: Zeroizing<String>,
    ) -> Result<EncryptedCredential, VaultError> {
        let (key_version, key) = self
            .secrets
            .load_or_create_active_key(account_id)
            .map_err(|_| VaultError::DeviceKeyUnavailable)?;
        let context = CredentialContext::for_scope(
            tenant_id,
            user_id,
            scope,
            device_key_id(&key, key_version),
        )?;
        encrypt(plaintext, &key, key_version, &context).map_err(Into::into)
    }

    pub(crate) fn open(
        &self,
        account_id: &str,
        tenant_id: &str,
        user_id: &str,
        provider: ProviderId,
        serialized: &str,
    ) -> Result<Zeroizing<String>, VaultError> {
        self.open_scoped(
            account_id,
            tenant_id,
            user_id,
            provider_scope(provider),
            serialized,
        )
    }

    /// Opens a value sealed by [`Self::seal_scoped`]. A mismatched scope fails
    /// authentication rather than returning the wrong plaintext: the scope is
    /// part of the AEAD's additional authenticated data.
    pub(crate) fn open_scoped(
        &self,
        account_id: &str,
        tenant_id: &str,
        user_id: &str,
        scope: &str,
        serialized: &str,
    ) -> Result<Zeroizing<String>, VaultError> {
        let key_version = envelope_key_version(serialized)?;
        let key = self
            .secrets
            .account_key(account_id, key_version)
            .map_err(|_| VaultError::DeviceKeyUnavailable)?;
        let context = CredentialContext::for_scope(
            tenant_id,
            user_id,
            scope,
            device_key_id(&key, key_version),
        )?;
        decrypt(serialized, &key, &context).map_err(Into::into)
    }

    /// The scope naming one registered MCP server's credential.
    #[must_use]
    pub(crate) fn mcp_server_scope(server_id: &str) -> String {
        mcp_server_scope(server_id)
    }

    /// The scope naming one registered MCP server's OAuth grant.
    #[must_use]
    pub(crate) fn mcp_oauth_scope(server_id: &str) -> String {
        mcp_oauth_scope(server_id)
    }

    /// The scope naming one registered MCP server's operator-issued OAuth
    /// client.
    #[must_use]
    pub(crate) fn mcp_oauth_client_scope(server_id: &str) -> String {
        mcp_oauth_client_scope(server_id)
    }

    pub(crate) fn active_device_key_id(&self, account_id: &str) -> Result<String, VaultError> {
        let key_version = self
            .secrets
            .active_key_version(account_id)
            .map_err(|_| VaultError::DeviceKeyUnavailable)?;
        let key = self
            .secrets
            .account_key(account_id, key_version)
            .map_err(|_| VaultError::DeviceKeyUnavailable)?;
        Ok(device_key_id(&key, key_version))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum VaultError {
    #[error("credential device key is unavailable")]
    DeviceKeyUnavailable,
    #[error("credential envelope is invalid")]
    InvalidEnvelope,
}

impl From<EnvelopeError> for VaultError {
    fn from(_: EnvelopeError) -> Self {
        Self::InvalidEnvelope
    }
}

fn device_key_id(key: &[u8; 32], key_version: u32) -> String {
    let mut digest = Sha256::new();
    digest.update(b"xfalcon-desktop:provider-device-key:v1:");
    digest.update(key_version.to_be_bytes());
    digest.update(key);
    let digest = digest.finalize();
    format!("dk-{}", URL_SAFE_NO_PAD.encode(&digest[..18]))
}
