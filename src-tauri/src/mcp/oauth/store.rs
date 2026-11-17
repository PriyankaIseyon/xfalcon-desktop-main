//! Device-local storage for one registered server's OAuth grant.
//!
//! Same file, same vault, same account scoping as
//! `crate::mcp::credential_store` — a second table rather than a second
//! database, so the account clear removes both with one directory and no new
//! cleanup handler can be forgotten.
//!
//! # Why a whole record and not just a token
//!
//! Renewing a grant needs more than the refresh token: it needs the token
//! endpoint to send it to, the client identity to authenticate as, and the RFC
//! 8707 `resource` the renewed token must be bound to. Re-running discovery at
//! renewal time would make every reconnect depend on the authorization server's
//! metadata endpoint being reachable, and would let a changed document silently
//! move where a stored refresh token gets sent. So the coordinates the grant was
//! established with are stored *with* it, and discovery re-runs only when the
//! user signs in again.
//!
//! The whole record is sealed as one envelope, so the client secret and both
//! tokens are ciphertext at rest under this device's key, bound to this account
//! and this server id.
//!
//! # Why the operator-issued client is a separate table
//!
//! [`StoredGrant`] carries the client a *completed* sign-in used, which is all a
//! renewal needs. It is the wrong home for the client an operator issued and a
//! user pasted, for three reasons, each of which was a defect:
//!
//! - A sign-in that is declined, times out, or is refused writes no grant, so
//!   the pasted pair was discarded and had to be typed again on every retry.
//! - Signing out deletes the grant, so the pair was lost again on the next
//!   sign-in.
//! - Nothing could report the client id back to Settings, so the edit sheet
//!   showed an empty field over a value the user had already supplied.
//!
//! [`DeviceOauthClients`] therefore holds that pair for as long as the server is
//! registered, independent of any grant. The identifier is a column in the clear
//! — it names a registration rather than authenticating one, and listing the
//! whole registry must not need the device key — while the confidential half is
//! sealed exactly like a grant, under its own scope.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use reqwest::Url;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::credentials::vault::{CredentialVault, VaultError};
use crate::mcp::credential_store::CredentialOwner;

use super::exchange::{now_epoch_seconds, TokenSet, RENEWAL_LEEWAY_SECONDS};
use super::registration::OauthClient;

/// Ceiling on one sealed grant's plaintext. Two JWTs, a client secret, and the
/// endpoint coordinates fit in a small fraction of this; the envelope layer's own
/// 32 KiB bound is the backstop behind it.
const MAX_GRANT_BYTES: usize = 16 * 1024;

/// Everything needed to authenticate to one server, and to renew that
/// authentication without re-running discovery.
///
/// `Zeroize` on drop covers the two tokens and the client secret: this struct is
/// the decrypted form, and it exists in memory for as long as one connect or one
/// renewal takes.
#[derive(Clone, Deserialize, Serialize, Zeroize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredGrant {
    pub(crate) client_id: String,
    pub(crate) client_secret: Option<String>,
    /// The issuer this grant was established with. Compared on re-sign-in: an
    /// authorization server change means the stored client is no longer valid.
    pub(crate) issuer: String,
    pub(crate) token_endpoint: String,
    pub(crate) revocation_endpoint: Option<String>,
    /// The RFC 8707 audience. Persisted because a renewal must request the same
    /// one, or the renewed token will be refused by this server.
    pub(crate) resource: String,
    pub(crate) scope: Option<String>,
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    /// Unix seconds.
    pub(crate) expires_at: i64,
}

impl Drop for StoredGrant {
    /// Written out rather than derived: `#[zeroize(drop)]` is deprecated, and
    /// the `ZeroizeOnDrop` derive's field bounds differ across zeroize releases.
    /// One line here is stable against both.
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl std::fmt::Debug for StoredGrant {
    /// Neither token nor the client secret appears; the client id and issuer do,
    /// because diagnosing "signed in as the wrong client" needs them and neither
    /// is confidential.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredGrant")
            .field("client_id", &self.client_id)
            .field("issuer", &self.issuer)
            .field("resource", &self.resource)
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl StoredGrant {
    /// Whether the access token should be renewed before it is used.
    #[must_use]
    pub(crate) fn needs_renewal(&self) -> bool {
        self.expires_at.saturating_sub(RENEWAL_LEEWAY_SECONDS) <= now_epoch_seconds()
    }

    /// Whether a renewal is even possible. A grant with no refresh token is
    /// usable until it expires and then needs the user again.
    #[must_use]
    pub(crate) fn is_renewable(&self) -> bool {
        self.refresh_token
            .as_ref()
            .is_some_and(|token| !token.trim().is_empty())
    }

    #[must_use]
    pub(crate) fn client(&self) -> OauthClient {
        OauthClient {
            client_id: self.client_id.clone(),
            client_secret: self
                .client_secret
                .as_ref()
                .map(|secret| Zeroizing::new(secret.clone())),
        }
    }

    #[must_use]
    pub(crate) fn token_endpoint_url(&self) -> Option<Url> {
        Url::parse(&self.token_endpoint).ok()
    }

    #[must_use]
    pub(crate) fn revocation_endpoint_url(&self) -> Option<Url> {
        self.revocation_endpoint
            .as_deref()
            .and_then(|raw| Url::parse(raw).ok())
    }

    /// Replaces the token material after a renewal, keeping the coordinates.
    ///
    /// A refresh response that omits `refresh_token` keeps the existing one
    /// (RFC 6749 §6 permits omission); one that includes a new value replaces it,
    /// which is what a server doing refresh-token rotation requires — dropping
    /// the rotated value would invalidate the grant on its next use.
    pub(crate) fn renewed_with(&self, tokens: &TokenSet) -> Self {
        Self {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            issuer: self.issuer.clone(),
            token_endpoint: self.token_endpoint.clone(),
            revocation_endpoint: self.revocation_endpoint.clone(),
            resource: self.resource.clone(),
            scope: tokens.scope.clone().or_else(|| self.scope.clone()),
            access_token: tokens.access_token.to_string(),
            refresh_token: tokens
                .refresh_token
                .as_ref()
                .map(|token| token.to_string())
                .or_else(|| self.refresh_token.clone()),
            expires_at: tokens.expires_at,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GrantStoreError {
    #[error("the local grant store is unavailable")]
    Storage,
    #[error("the grant is larger than this store will hold")]
    TooLarge,
    /// A client identifier that is blank, or a pair whose confidential half is
    /// blank or over-length.
    ///
    /// Distinct from [`Self::TooLarge`] so the guard reports what is actually
    /// wrong. The command surface refuses both shapes before reaching here, so
    /// this is the store's own backstop rather than the message a user sees.
    #[error("the supplied client is empty or too large")]
    InvalidClient,
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// One account's grant store, opened per operation.
///
/// Not a held-open connection, for the same reason as
/// `crate::mcp::credential_store::DeviceCredentials`: on Windows an open SQLite
/// handle would defeat the account clear while the app is running.
#[derive(Clone)]
pub(crate) struct DeviceGrants {
    path: PathBuf,
    vault: Arc<CredentialVault>,
    owner: CredentialOwner,
}

/// One account's device-local MCP preferences, stored beside its grants.
///
/// This keeps the bundled-server choice inside the account directory that
/// the account clear (run by Reset xFalcon) already owns. The connection is opened per
/// operation so Windows can remove the account directory while the app is running.
#[derive(Clone, Debug)]
pub(crate) struct DevicePreferences {
    path: PathBuf,
    account_id: String,
}

impl DevicePreferences {
    #[must_use]
    pub(crate) fn new(path: PathBuf, account_id: String) -> Self {
        Self { path, account_id }
    }

    /// Whether the configured tenant server is available to subsequent runs.
    ///
    /// No row is the upgrade/default state and means enabled. A database that
    /// exists but cannot be opened or read is an error: treating that as the
    /// default would silently undo a user's removal.
    pub(crate) fn bundled_enabled(&self) -> Result<bool, GrantStoreError> {
        if !self
            .path
            .try_exists()
            .map_err(|_| GrantStoreError::Storage)?
        {
            return Ok(true);
        }
        let stored = self
            .connection()?
            .query_row(
                "SELECT bundled_server_enabled FROM device_mcp_preferences
                 WHERE account_id = ?1",
                [&self.account_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|_| GrantStoreError::Storage)?;
        match stored {
            None | Some(1) => Ok(true),
            Some(0) => Ok(false),
            Some(_) => Err(GrantStoreError::Storage),
        }
    }

    pub(crate) fn set_bundled_enabled(&self, enabled: bool) -> Result<(), GrantStoreError> {
        self.connection()?
            .execute(
                "INSERT INTO device_mcp_preferences (account_id, bundled_server_enabled)
                 VALUES (?1, ?2)
                 ON CONFLICT (account_id) DO UPDATE SET
                     bundled_server_enabled = excluded.bundled_server_enabled",
                (&self.account_id, enabled),
            )
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(())
    }

    fn connection(&self) -> Result<Connection, GrantStoreError> {
        open_store(&self.path)
    }
}

impl DeviceGrants {
    #[must_use]
    pub(crate) fn new(path: PathBuf, vault: Arc<CredentialVault>, owner: CredentialOwner) -> Self {
        Self { path, vault, owner }
    }

    /// The grant for `server_id`, or `None` when this device holds none.
    ///
    /// A stored row that will not open is an error, not `None`: "signed in, but
    /// this device can no longer read it" and "never signed in" lead the user to
    /// different actions.
    pub(crate) fn get(&self, server_id: &str) -> Result<Option<StoredGrant>, GrantStoreError> {
        let Some(envelope) = self.stored_envelope(server_id)? else {
            return Ok(None);
        };
        let opened = self.vault.open_scoped(
            &self.owner.account_id,
            &self.owner.tenant_id,
            &self.owner.user_id,
            &CredentialVault::mcp_oauth_scope(server_id),
            &envelope,
        )?;
        let grant =
            serde_json::from_str::<StoredGrant>(&opened).map_err(|_| GrantStoreError::Storage)?;
        Ok(Some(grant))
    }

    pub(crate) fn put(&self, server_id: &str, grant: &StoredGrant) -> Result<(), GrantStoreError> {
        let mut serialized =
            Zeroizing::new(serde_json::to_string(grant).map_err(|_| GrantStoreError::Storage)?);
        if serialized.len() > MAX_GRANT_BYTES {
            serialized.zeroize();
            return Err(GrantStoreError::TooLarge);
        }
        let sealed = self.vault.seal_scoped(
            &self.owner.account_id,
            &self.owner.tenant_id,
            &self.owner.user_id,
            &CredentialVault::mcp_oauth_scope(server_id),
            serialized,
        )?;
        let (envelope, _device_key_id) = sealed.into_parts();
        self.connection()?
            .execute(
                "INSERT INTO server_oauth_grants (account_id, server_id, envelope)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (account_id, server_id) DO UPDATE SET envelope = excluded.envelope",
                (&self.owner.account_id, server_id, envelope.as_str()),
            )
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(())
    }

    /// Forgets `server_id`'s grant. Idempotent, so signing out twice — or
    /// unregistering a server that was never signed in to — is success.
    pub(crate) fn delete(&self, server_id: &str) -> Result<(), GrantStoreError> {
        self.connection()?
            .execute(
                "DELETE FROM server_oauth_grants WHERE account_id = ?1 AND server_id = ?2",
                (&self.owner.account_id, server_id),
            )
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(())
    }

    /// Every server id this account holds a grant for.
    ///
    /// Answers the whole registry's "is this one signed in?" in one query, and
    /// without the device key — so a locked keychain still yields a correct list
    /// instead of failing it.
    pub(crate) fn stored_ids(&self) -> Result<Vec<String>, GrantStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT server_id FROM server_oauth_grants WHERE account_id = ?1")
            .map_err(|_| GrantStoreError::Storage)?;
        let ids = statement
            .query_map([&self.owner.account_id], |row| row.get::<_, String>(0))
            .map_err(|_| GrantStoreError::Storage)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(ids)
    }

    fn stored_envelope(&self, server_id: &str) -> Result<Option<String>, GrantStoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        self.connection()?
            .query_row(
                "SELECT envelope FROM server_oauth_grants
                 WHERE account_id = ?1 AND server_id = ?2",
                (&self.owner.account_id, server_id),
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| GrantStoreError::Storage)
    }

    fn connection(&self) -> Result<Connection, GrantStoreError> {
        open_store(&self.path)
    }
}

impl std::fmt::Debug for DeviceGrants {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceGrants")
            .finish_non_exhaustive()
    }
}

/// Ceiling on one stored client's confidential half. The same bound the command
/// surface refuses a pasted value at, restated here so a store write cannot
/// exceed what the boundary admitted.
const MAX_CLIENT_SECRET_BYTES: usize = 4 * 1024;

/// Ceiling on a stored client identifier, matching the command surface's bound.
const MAX_CLIENT_ID_BYTES: usize = 4 * 1024;

/// What this device holds as one server's operator-issued OAuth client, without
/// opening anything.
///
/// Answers "which client is configured, and does it have a confidential half?"
/// for a whole registry in one query and with no device key — so an account
/// whose keychain is locked still gets a correct Settings list, exactly as
/// [`DeviceGrants::stored_ids`] does for grants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredOauthClientSummary {
    pub(crate) server_id: String,
    pub(crate) client_id: String,
    pub(crate) confidential: bool,
}

/// One account's operator-issued OAuth clients, opened per operation.
///
/// Same file, same vault and same per-operation connection as [`DeviceGrants`];
/// see this module's comment for why the two are separate tables.
#[derive(Clone)]
pub(crate) struct DeviceOauthClients {
    path: PathBuf,
    vault: Arc<CredentialVault>,
    owner: CredentialOwner,
}

impl DeviceOauthClients {
    #[must_use]
    pub(crate) fn new(path: PathBuf, vault: Arc<CredentialVault>, owner: CredentialOwner) -> Self {
        Self { path, vault, owner }
    }

    /// The client for `server_id`, ready to authenticate with, or `None` when
    /// this device holds none.
    ///
    /// A row whose sealed half will not open is an error rather than a public
    /// client: sending the identifier without the value it was issued with is
    /// refused as `invalid_client`, which reads like a wrong client instead of
    /// an unreadable one.
    pub(crate) fn get(&self, server_id: &str) -> Result<Option<OauthClient>, GrantStoreError> {
        let Some((client_id, envelope)) = self.stored_row(server_id)? else {
            return Ok(None);
        };
        let client_secret = match envelope {
            None => None,
            Some(envelope) => Some(self.vault.open_scoped(
                &self.owner.account_id,
                &self.owner.tenant_id,
                &self.owner.user_id,
                &CredentialVault::mcp_oauth_client_scope(server_id),
                &envelope,
            )?),
        };
        Ok(Some(OauthClient {
            client_id,
            client_secret,
        }))
    }

    /// What is configured for `server_id`, without the device key.
    pub(crate) fn summary(
        &self,
        server_id: &str,
    ) -> Result<Option<StoredOauthClientSummary>, GrantStoreError> {
        Ok(self
            .stored_row(server_id)?
            .map(|(client_id, envelope)| StoredOauthClientSummary {
                server_id: server_id.to_owned(),
                client_id,
                confidential: envelope.is_some(),
            }))
    }

    /// What is configured for every server this account holds a client for.
    pub(crate) fn summaries(&self) -> Result<Vec<StoredOauthClientSummary>, GrantStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT server_id, client_id, envelope IS NOT NULL
                 FROM server_oauth_clients WHERE account_id = ?1",
            )
            .map_err(|_| GrantStoreError::Storage)?;
        let rows = statement
            .query_map([&self.owner.account_id], |row| {
                Ok(StoredOauthClientSummary {
                    server_id: row.get(0)?,
                    client_id: row.get(1)?,
                    confidential: row.get::<_, i64>(2)? == 1,
                })
            })
            .map_err(|_| GrantStoreError::Storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(rows)
    }

    /// Stores `client_id` and, when one was issued, the value that
    /// authenticates it. Replaces whatever was there, so re-pasting a rotated
    /// pair is one action rather than a delete-then-add with a window in which
    /// the server cannot be signed in to.
    ///
    /// `client_secret` of `None` stores a public client. "Keep the value
    /// already sealed" is not expressible here on purpose: it is a decision
    /// about what the user typed, so it belongs to the command surface, which
    /// resolves it before calling this.
    pub(crate) fn put(
        &self,
        server_id: &str,
        client_id: &str,
        client_secret: Option<Zeroizing<String>>,
    ) -> Result<(), GrantStoreError> {
        let client_id = client_id.trim();
        if client_id.is_empty() || client_id.len() > MAX_CLIENT_ID_BYTES {
            return Err(GrantStoreError::InvalidClient);
        }
        let envelope = match client_secret {
            None => None,
            Some(secret) => {
                if secret.trim().is_empty() || secret.len() > MAX_CLIENT_SECRET_BYTES {
                    return Err(GrantStoreError::InvalidClient);
                }
                let sealed = self.vault.seal_scoped(
                    &self.owner.account_id,
                    &self.owner.tenant_id,
                    &self.owner.user_id,
                    &CredentialVault::mcp_oauth_client_scope(server_id),
                    secret,
                )?;
                let (envelope, _device_key_id) = sealed.into_parts();
                Some(envelope)
            }
        };
        self.connection()?
            .execute(
                "INSERT INTO server_oauth_clients (account_id, server_id, client_id, envelope)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (account_id, server_id) DO UPDATE SET
                     client_id = excluded.client_id,
                     envelope  = excluded.envelope",
                (
                    &self.owner.account_id,
                    server_id,
                    client_id,
                    envelope.as_deref(),
                ),
            )
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(())
    }

    /// Forgets `server_id`'s client. Idempotent, so unregistering a server or
    /// switching it away from browser sign-in never fails on cleanup.
    pub(crate) fn delete(&self, server_id: &str) -> Result<(), GrantStoreError> {
        self.connection()?
            .execute(
                "DELETE FROM server_oauth_clients WHERE account_id = ?1 AND server_id = ?2",
                (&self.owner.account_id, server_id),
            )
            .map_err(|_| GrantStoreError::Storage)?;
        Ok(())
    }

    fn stored_row(
        &self,
        server_id: &str,
    ) -> Result<Option<(String, Option<String>)>, GrantStoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        self.connection()?
            .query_row(
                "SELECT client_id, envelope FROM server_oauth_clients
                 WHERE account_id = ?1 AND server_id = ?2",
                (&self.owner.account_id, server_id),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|_| GrantStoreError::Storage)
    }

    fn connection(&self) -> Result<Connection, GrantStoreError> {
        open_store(&self.path)
    }
}

impl std::fmt::Debug for DeviceOauthClients {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceOauthClients")
            .finish_non_exhaustive()
    }
}

/// Opens (creating if needed) the account's MCP store and ensures this table
/// exists.
///
/// `CREATE TABLE IF NOT EXISTS` on every open rather than a migration step: the
/// credential table in the same file is created the same way, and a store whose
/// schema is established by opening it cannot be left half-migrated by a crash
/// between two runs.
fn open_store(path: &Path) -> Result<Connection, GrantStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| GrantStoreError::Storage)?;
    }
    crate::persistence::open_local_sqlite_store(
        path,
        crate::persistence::LocalSqliteSynchronous::Full,
        |connection| {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS server_oauth_grants (
                 account_id TEXT NOT NULL,
                 server_id  TEXT NOT NULL,
                 envelope   TEXT NOT NULL,
                 PRIMARY KEY (account_id, server_id)
             );
             CREATE TABLE IF NOT EXISTS server_oauth_clients (
                 account_id TEXT NOT NULL,
                 server_id  TEXT NOT NULL,
                 client_id  TEXT NOT NULL,
                 envelope   TEXT,
                 PRIMARY KEY (account_id, server_id)
             );
             CREATE TABLE IF NOT EXISTS device_mcp_preferences (
                 account_id              TEXT PRIMARY KEY,
                 bundled_server_enabled  INTEGER NOT NULL
                     CHECK (bundled_server_enabled IN (0, 1))
             );",
            )?;
            Ok(())
        },
    )
    .map_err(|_| GrantStoreError::Storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::vault::CredentialVault;
    use crate::persistence::InMemorySecretStore;
    use zeroize::Zeroizing;

    fn owner() -> CredentialOwner {
        CredentialOwner {
            account_id: "tenant-1__user-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            user_id: "user-1".to_owned(),
        }
    }

    fn grants(directory: &Path) -> DeviceGrants {
        DeviceGrants::new(
            directory
                .join("mcp-data")
                .join("server-credentials.sqlite3"),
            Arc::new(CredentialVault::new(Arc::new(
                InMemorySecretStore::default(),
            ))),
            owner(),
        )
    }

    fn oauth_clients(directory: &Path) -> DeviceOauthClients {
        DeviceOauthClients::new(
            directory
                .join("mcp-data")
                .join("server-credentials.sqlite3"),
            Arc::new(CredentialVault::new(Arc::new(
                InMemorySecretStore::default(),
            ))),
            owner(),
        )
    }

    fn grant() -> StoredGrant {
        StoredGrant {
            client_id: "issued-client".to_owned(),
            client_secret: Some("issued-secret".to_owned()),
            issuer: "https://auth.example.test".to_owned(),
            token_endpoint: "https://auth.example.test/token".to_owned(),
            revocation_endpoint: Some("https://auth.example.test/revoke".to_owned()),
            resource: "https://mcp.example.test/mcp".to_owned(),
            scope: Some("openid offline_access".to_owned()),
            access_token: "issued-access-token".to_owned(),
            refresh_token: Some("issued-refresh-token".to_owned()),
            expires_at: now_epoch_seconds() + 3600,
        }
    }

    #[test]
    fn a_stored_grant_round_trips_and_is_ciphertext_on_disk() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = grants(directory.path());
        store.put("srv-1", &grant()).expect("the grant is stored");

        let recovered = store
            .get("srv-1")
            .expect("the store is readable")
            .expect("a grant is present");
        assert_eq!(recovered.access_token, "issued-access-token");
        assert_eq!(recovered.resource, "https://mcp.example.test/mcp");
        assert_eq!(recovered.client().client_id, "issued-client");

        let bytes = std::fs::read(
            directory
                .path()
                .join("mcp-data")
                .join("server-credentials.sqlite3"),
        )
        .expect("the database file");
        let raw = String::from_utf8_lossy(&bytes);
        for plaintext in [
            "issued-access-token",
            "issued-refresh-token",
            "issued-secret",
        ] {
            assert!(
                !raw.contains(plaintext),
                "{plaintext} must not be readable on disk"
            );
        }
    }

    #[test]
    fn an_absent_store_is_an_empty_set_rather_than_a_failure() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = grants(directory.path());
        assert!(store.stored_ids().expect("an empty list").is_empty());
        assert!(store.get("srv-1").expect("no grant").is_none());
    }

    #[test]
    fn signing_out_is_idempotent() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = grants(directory.path());
        store.put("srv-1", &grant()).expect("stored");
        store.delete("srv-1").expect("first sign-out");
        store.delete("srv-1").expect("second sign-out");
        assert!(store.get("srv-1").expect("readable").is_none());
        assert!(store.stored_ids().expect("readable").is_empty());
    }

    /// The scope is part of the AEAD, so a row moved to another server id must
    /// fail to open rather than authenticate that other server.
    #[test]
    fn a_grant_cannot_be_opened_under_another_servers_id() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = grants(directory.path());
        store.put("srv-1", &grant()).expect("stored");

        let path = directory
            .path()
            .join("mcp-data")
            .join("server-credentials.sqlite3");
        let connection = Connection::open(&path).expect("open");
        connection
            .execute(
                "UPDATE server_oauth_grants SET server_id = 'srv-2' WHERE server_id = 'srv-1'",
                (),
            )
            .expect("move the row");
        drop(connection);

        assert!(store.get("srv-2").is_err(), "the AEAD must refuse the move");
    }

    #[test]
    fn a_renewal_keeps_the_coordinates_and_takes_a_rotated_refresh_token() {
        let existing = grant();
        let rotated = existing.renewed_with(&TokenSet {
            access_token: Zeroizing::new("second-access-token".to_owned()),
            refresh_token: Some(Zeroizing::new("rotated-refresh-token".to_owned())),
            expires_at: now_epoch_seconds() + 7200,
            scope: None,
        });

        assert_eq!(rotated.access_token, "second-access-token");
        assert_eq!(
            rotated.refresh_token.as_deref(),
            Some("rotated-refresh-token")
        );
        assert_eq!(rotated.token_endpoint, existing.token_endpoint);
        assert_eq!(rotated.resource, existing.resource);
        assert_eq!(
            rotated.scope.as_deref(),
            Some("openid offline_access"),
            "a response that omits scope keeps what was granted before"
        );
    }

    #[test]
    fn bundled_preference_defaults_enabled_and_is_account_scoped() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("server-credentials.sqlite3");
        let first = DevicePreferences::new(path.clone(), "account-1".to_owned());
        let second = DevicePreferences::new(path, "account-2".to_owned());

        assert!(first.bundled_enabled().expect("default preference"));
        first
            .set_bundled_enabled(false)
            .expect("remove bundled server");
        assert!(!first.bundled_enabled().expect("stored preference"));
        assert!(second.bundled_enabled().expect("other account default"));

        first
            .set_bundled_enabled(true)
            .expect("restore bundled server");
        assert!(first.bundled_enabled().expect("restored preference"));
    }

    #[test]
    fn bundled_preference_read_failure_is_not_treated_as_enabled() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("server-credentials.sqlite3");
        std::fs::write(&path, b"not sqlite").expect("poison the database");
        let preferences = DevicePreferences::new(path, "account-1".to_owned());

        assert!(matches!(
            preferences.bundled_enabled(),
            Err(GrantStoreError::Storage)
        ));
    }

    #[test]
    fn a_renewal_that_omits_the_refresh_token_keeps_the_existing_one() {
        let existing = grant();
        let renewed = existing.renewed_with(&TokenSet {
            access_token: Zeroizing::new("second-access-token".to_owned()),
            refresh_token: None,
            expires_at: now_epoch_seconds() + 7200,
            scope: Some("openid".to_owned()),
        });
        assert_eq!(
            renewed.refresh_token.as_deref(),
            Some("issued-refresh-token")
        );
        assert_eq!(renewed.scope.as_deref(), Some("openid"));
    }

    #[test]
    fn renewal_is_due_before_expiry_and_only_possible_with_a_refresh_token() {
        let mut nearly_expired = grant();
        nearly_expired.expires_at = now_epoch_seconds() + RENEWAL_LEEWAY_SECONDS - 1;
        assert!(nearly_expired.needs_renewal());
        assert!(nearly_expired.is_renewable());

        let fresh = grant();
        assert!(!fresh.needs_renewal());

        let mut without_refresh = grant();
        without_refresh.refresh_token = None;
        assert!(!without_refresh.is_renewable());
    }

    #[test]
    fn a_stored_client_round_trips_with_its_value_as_ciphertext_on_disk() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = oauth_clients(directory.path());
        store
            .put(
                "srv-1",
                "issued-client",
                Some(Zeroizing::new("issued-secret".to_owned())),
            )
            .expect("the client is stored");

        let recovered = store
            .get("srv-1")
            .expect("the store is readable")
            .expect("a client is present");
        assert_eq!(recovered.client_id, "issued-client");
        assert_eq!(
            recovered.client_secret.as_deref().map(String::as_str),
            Some("issued-secret")
        );

        let bytes = std::fs::read(
            directory
                .path()
                .join("mcp-data")
                .join("server-credentials.sqlite3"),
        )
        .expect("the database file exists");
        let raw = String::from_utf8_lossy(&bytes);
        assert!(
            !raw.contains("issued-secret"),
            "the value that authenticates the client must be ciphertext at rest"
        );
        // The identifier is deliberately NOT ciphertext: it names a registration
        // rather than authenticating one, and listing a whole registry must not
        // need the device key.
        assert!(raw.contains("issued-client"));
    }

    #[test]
    fn a_summary_answers_without_the_device_key() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = oauth_clients(directory.path());
        store
            .put("srv-1", "issued-client", None)
            .expect("a public client is stored");
        store
            .put(
                "srv-2",
                "confidential-client",
                Some(Zeroizing::new("issued-secret".to_owned())),
            )
            .expect("a confidential client is stored");

        let mut summaries = store.summaries().expect("the store is readable");
        summaries.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        assert_eq!(
            summaries,
            vec![
                StoredOauthClientSummary {
                    server_id: "srv-1".to_owned(),
                    client_id: "issued-client".to_owned(),
                    confidential: false,
                },
                StoredOauthClientSummary {
                    server_id: "srv-2".to_owned(),
                    client_id: "confidential-client".to_owned(),
                    confidential: true,
                },
            ]
        );
        assert_eq!(
            store.summary("srv-2").expect("readable"),
            Some(StoredOauthClientSummary {
                server_id: "srv-2".to_owned(),
                client_id: "confidential-client".to_owned(),
                confidential: true,
            })
        );
        assert_eq!(store.summary("srv-3").expect("readable"), None);
    }

    #[test]
    fn a_replacement_client_leaves_nothing_of_the_one_it_replaced() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = oauth_clients(directory.path());
        store
            .put(
                "srv-1",
                "issued-client",
                Some(Zeroizing::new("issued-secret".to_owned())),
            )
            .expect("the client is stored");
        // Rotating to a public client must drop the sealed value, not orphan it:
        // a later read must not resurrect a secret for a different client.
        store
            .put("srv-1", "reissued-client", None)
            .expect("the client is replaced");

        let recovered = store
            .get("srv-1")
            .expect("the store is readable")
            .expect("a client is present");
        assert_eq!(recovered.client_id, "reissued-client");
        assert!(recovered.client_secret.is_none());
        assert!(
            !store
                .summary("srv-1")
                .expect("readable")
                .expect("present")
                .confidential
        );
    }

    #[test]
    fn an_absent_store_is_no_client_rather_than_a_failure() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = oauth_clients(directory.path());
        assert!(store.get("srv-1").expect("readable").is_none());
        assert_eq!(store.summary("srv-1").expect("readable"), None);
        assert!(store.summaries().expect("readable").is_empty());
        // Idempotent, so unregistering a server never fails on cleanup.
        store.delete("srv-1").expect("a delete of nothing succeeds");
    }

    #[test]
    fn a_blank_or_oversized_client_is_refused_by_the_store_itself() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = oauth_clients(directory.path());
        for (client_id, secret) in [
            ("   ", None),
            ("x".repeat(MAX_CLIENT_ID_BYTES + 1).as_str(), None),
            ("issued-client", Some("  ".to_owned())),
            (
                "issued-client",
                Some("x".repeat(MAX_CLIENT_SECRET_BYTES + 1)),
            ),
        ] {
            let error = store
                .put("srv-1", client_id, secret.map(Zeroizing::new))
                .expect_err("the store guards its own bounds");
            assert!(matches!(error, GrantStoreError::InvalidClient), "{error:?}");
        }
        assert!(
            store.get("srv-1").expect("readable").is_none(),
            "a refused write stores nothing"
        );
    }

    #[test]
    fn a_client_sealed_for_one_server_does_not_open_as_another() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = oauth_clients(directory.path());
        store
            .put(
                "srv-1",
                "issued-client",
                Some(Zeroizing::new("issued-secret".to_owned())),
            )
            .expect("the client is stored");

        // The server id is part of the AEAD's additional authenticated data, so
        // a row moved between ids fails to open rather than decrypting as the
        // other server's client.
        let connection = open_store(
            &directory
                .path()
                .join("mcp-data")
                .join("server-credentials.sqlite3"),
        )
        .expect("the store opens");
        connection
            .execute("UPDATE server_oauth_clients SET server_id = 'srv-2'", [])
            .expect("move the row");
        drop(connection);

        let error = store
            .get("srv-2")
            .expect_err("a moved row must not open as another server's client");
        assert!(matches!(error, GrantStoreError::Vault(_)), "{error:?}");
    }
}
