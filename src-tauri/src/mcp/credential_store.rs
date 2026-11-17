//! Device-local storage for a registered MCP server's credential.
//!
//! DA's `mcp_servers.auth_config` is encrypted under a key that never leaves
//! xfalcon-web, so desktop can neither read what web wrote there nor write
//! something web could read. Rather than pretend otherwise, a server registered
//! from desktop keeps its credential *here*: sealed by
//! `crate::credentials::CredentialVault` into the device-bound AES-256-GCM
//! envelope model retained for legacy provider-key migration, then written to
//! an account-scoped SQLite file under the app's local data directory.
//!
//! Consequences, by design:
//!
//! - A server added on this device authenticates on this device only. Another
//!   device sees the definition (it is in DA) and reports
//!   [`MissingDeviceCredential`](xfalcon_contracts::mcp::McpServerUnavailableReason::MissingDeviceCredential).
//! - The account clear (run by Reset xFalcon) removes this whole store and leaves every definition
//!   in DA, so each server reappears as "needs a credential on this device"
//!   rather than silently failing to connect. The removal belongs to
//!   `crate::security::account_cleanup`'s session-vault handler, which owns
//!   every store of material sealed under the device keys it deletes.
//! - The stored bytes are ciphertext bound to this account, this device key,
//!   and this server id (the scope is part of the AEAD's additional
//!   authenticated data), so a row copied to another server's id or another
//!   account fails authentication instead of decrypting.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::credentials::vault::{CredentialVault, VaultError};
use crate::security::account_cleanup::account_directory_name;

/// Directory holding this store inside the app's per-account data directory.
pub(crate) const MCP_DATA_DIR: &str = "mcp-data";
/// File name inside [`MCP_DATA_DIR`].
pub(crate) const CREDENTIAL_DB_FILE: &str = "server-credentials.sqlite3";

/// Where one account's MCP credential store lives.
///
/// Inside the per-account directory, so the account clear (run by Reset xFalcon)
/// removes it with the rest of that account's state (`crate::security::account_cleanup`'s
/// session-vault handler owns the removal, because these bytes are sealed under
/// the same device keys it deletes).
#[must_use]
pub(crate) fn credential_db_path(app_local_data: &Path, account_id: &str) -> PathBuf {
    app_local_data
        .join("accounts")
        .join(account_directory_name(account_id))
        .join(MCP_DATA_DIR)
        .join(CREDENTIAL_DB_FILE)
}

/// One account's credential store, opened per operation.
///
/// Deliberately not a held-open connection: the account clear removes this
/// file, and on Windows an open SQLite handle would make that removal fail
/// while the app is running. Each operation opens a database with at most one
/// row per registered server, so the cost is a file open rather than a query
/// plan.
#[derive(Clone)]
pub(crate) struct DeviceCredentials {
    path: PathBuf,
    vault: Arc<CredentialVault>,
    owner: CredentialOwner,
}

impl DeviceCredentials {
    #[must_use]
    pub(crate) fn new(path: PathBuf, vault: Arc<CredentialVault>, owner: CredentialOwner) -> Self {
        Self { path, vault, owner }
    }

    pub(crate) fn get(
        &self,
        server_id: &str,
    ) -> Result<Option<Zeroizing<String>>, CredentialStoreError> {
        self.open()?.get(&self.owner, server_id)
    }

    pub(crate) fn put(
        &self,
        server_id: &str,
        credential: Zeroizing<String>,
    ) -> Result<(), CredentialStoreError> {
        self.open()?.put(&self.owner, server_id, credential)
    }

    /// Forgets `server_id`'s credential. Idempotent, so unregistering a server
    /// never fails on cleanup.
    pub(crate) fn delete(&self, server_id: &str) -> Result<(), CredentialStoreError> {
        self.open()?.delete(&self.owner.account_id, server_id)
    }

    /// Every server id this account holds a credential for.
    ///
    /// An absent store is an empty set, not an error: an account that has never
    /// registered a server has no file yet, and listing servers must not fail
    /// because of it.
    pub(crate) fn stored_ids(&self) -> Result<Vec<String>, CredentialStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        self.open()?.stored_ids(&self.owner.account_id)
    }

    fn open(&self) -> Result<McpCredentialStore, CredentialStoreError> {
        McpCredentialStore::open(&self.path, self.vault.clone())
    }
}

impl std::fmt::Debug for DeviceCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceCredentials")
            .finish_non_exhaustive()
    }
}

/// Ceiling on one stored credential. The envelope itself is already bounded at
/// 32 KiB by `crate::credentials::envelope`; this is the plaintext bound, sized
/// for tokens and API keys rather than for documents.
///
/// Visible to the command surface so an over-long value is reported as a bad
/// request before anything is written, rather than as a storage failure after.
pub(crate) const MAX_CREDENTIAL_BYTES: usize = 4 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CredentialStoreError {
    #[error("the local MCP credential store is unavailable")]
    Storage,
    #[error("the supplied credential is empty or too large")]
    InvalidCredential,
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// Who a stored credential belongs to. Every method takes it, so no call can
/// read or write another account's row by omission.
#[derive(Clone, Debug)]
pub(crate) struct CredentialOwner {
    pub(crate) account_id: String,
    pub(crate) tenant_id: String,
    pub(crate) user_id: String,
}

pub(crate) struct McpCredentialStore {
    connection: Mutex<Connection>,
    vault: Arc<CredentialVault>,
}

impl McpCredentialStore {
    pub(crate) fn open(
        path: impl AsRef<Path>,
        vault: Arc<CredentialVault>,
    ) -> Result<Self, CredentialStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CredentialStoreError::Storage)?;
        }
        let connection = crate::persistence::open_local_sqlite_store(
            path,
            crate::persistence::LocalSqliteSynchronous::Full,
            |connection| {
                connection.execute_batch(
                    "CREATE TABLE IF NOT EXISTS server_credentials (
                         account_id TEXT NOT NULL,
                         server_id  TEXT NOT NULL,
                         envelope   TEXT NOT NULL,
                         PRIMARY KEY (account_id, server_id)
                     );",
                )?;
                Ok(())
            },
        )
        .map_err(|_| CredentialStoreError::Storage)?;
        Ok(Self {
            connection: Mutex::new(connection),
            vault,
        })
    }

    #[cfg(test)]
    pub(crate) fn in_memory(vault: Arc<CredentialVault>) -> Result<Self, CredentialStoreError> {
        let connection = Connection::open_in_memory().map_err(|_| CredentialStoreError::Storage)?;
        connection
            .execute_batch(
                "CREATE TABLE server_credentials (
                     account_id TEXT NOT NULL,
                     server_id  TEXT NOT NULL,
                     envelope   TEXT NOT NULL,
                     PRIMARY KEY (account_id, server_id)
                 );",
            )
            .map_err(|_| CredentialStoreError::Storage)?;
        Ok(Self {
            connection: Mutex::new(connection),
            vault,
        })
    }

    /// Seals `credential` for `server_id` and replaces whatever was stored.
    ///
    /// Replacing rather than erroring is deliberate: re-entering a rotated
    /// token is the common case, and a store that refused would make rotation
    /// a delete-then-add dance with a window where the server is unusable.
    pub(crate) fn put(
        &self,
        owner: &CredentialOwner,
        server_id: &str,
        credential: Zeroizing<String>,
    ) -> Result<(), CredentialStoreError> {
        if credential.trim().is_empty() || credential.len() > MAX_CREDENTIAL_BYTES {
            return Err(CredentialStoreError::InvalidCredential);
        }
        let sealed = self.vault.seal_scoped(
            &owner.account_id,
            &owner.tenant_id,
            &owner.user_id,
            &CredentialVault::mcp_server_scope(server_id),
            credential,
        )?;
        let (envelope, _device_key_id) = sealed.into_parts();
        self.connection
            .lock()
            .map_err(|_| CredentialStoreError::Storage)?
            .execute(
                "INSERT INTO server_credentials (account_id, server_id, envelope)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (account_id, server_id) DO UPDATE SET envelope = excluded.envelope",
                (&owner.account_id, server_id, envelope.as_str()),
            )
            .map_err(|_| CredentialStoreError::Storage)?;
        Ok(())
    }

    /// The decrypted credential for `server_id`, or `None` when this device
    /// holds none.
    ///
    /// A stored row that fails to open (device key rotated, envelope tampered
    /// with, or the row moved between server ids) returns the vault's error
    /// rather than `None`: "there is a credential but it is unusable" and
    /// "there is no credential" lead to different messages, and collapsing them
    /// would silently present a broken server as an unconfigured one.
    pub(crate) fn get(
        &self,
        owner: &CredentialOwner,
        server_id: &str,
    ) -> Result<Option<Zeroizing<String>>, CredentialStoreError> {
        let Some(envelope) = self.stored_envelope(&owner.account_id, server_id)? else {
            return Ok(None);
        };
        let opened = self.vault.open_scoped(
            &owner.account_id,
            &owner.tenant_id,
            &owner.user_id,
            &CredentialVault::mcp_server_scope(server_id),
            &envelope,
        )?;
        Ok(Some(opened))
    }

    /// Every server id this account has a stored credential for.
    ///
    /// This is what `super::registry::classify` asks, and it answers a whole
    /// list in one query without the device key — so an account whose keychain
    /// is locked still gets a correct list instead of an error.
    pub(crate) fn stored_ids(&self, account_id: &str) -> Result<Vec<String>, CredentialStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CredentialStoreError::Storage)?;
        let mut statement = connection
            .prepare("SELECT server_id FROM server_credentials WHERE account_id = ?1")
            .map_err(|_| CredentialStoreError::Storage)?;
        let ids = statement
            .query_map([account_id], |row| row.get::<_, String>(0))
            .map_err(|_| CredentialStoreError::Storage)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|_| CredentialStoreError::Storage)?;
        Ok(ids)
    }

    /// Forgets `server_id`'s credential. Idempotent — deleting one that is not
    /// there is success, so unregistering a server never fails on cleanup.
    pub(crate) fn delete(
        &self,
        account_id: &str,
        server_id: &str,
    ) -> Result<(), CredentialStoreError> {
        self.connection
            .lock()
            .map_err(|_| CredentialStoreError::Storage)?
            .execute(
                "DELETE FROM server_credentials WHERE account_id = ?1 AND server_id = ?2",
                (account_id, server_id),
            )
            .map_err(|_| CredentialStoreError::Storage)?;
        Ok(())
    }

    fn stored_envelope(
        &self,
        account_id: &str,
        server_id: &str,
    ) -> Result<Option<String>, CredentialStoreError> {
        self.connection
            .lock()
            .map_err(|_| CredentialStoreError::Storage)?
            .query_row(
                "SELECT envelope FROM server_credentials WHERE account_id = ?1 AND server_id = ?2",
                (account_id, server_id),
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| CredentialStoreError::Storage)
    }
}

impl std::fmt::Debug for McpCredentialStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpCredentialStore")
            .finish_non_exhaustive()
    }
}
