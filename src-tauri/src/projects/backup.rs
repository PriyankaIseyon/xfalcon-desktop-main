use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::error::{AppError, AppResult};
use crate::persistence::{
    decrypt_backup, encrypt_backup, BackupAuthenticatedMetadata, SecretStore,
    PROJECT_BACKUP_MIGRATION, PROJECT_BACKUP_SCHEMA_VERSION,
};
use crate::security::account_cleanup::ValidatedAccountId;

use super::{ProjectId, RelativeProjectPath, MAX_EDITABLE_FILE_BYTES};

pub const MAX_REVISIONS_PER_FILE: usize = 50;
pub const MAX_BACKUP_ACCOUNT_BYTES: u64 = 512 * 1024 * 1024;

const PAYLOAD_MAGIC: &[u8; 4] = b"XFB1";
const PAYLOAD_FIXED_BYTES: usize = 4 + 4 + 8 + 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupRevision {
    pub backup_id: Uuid,
    pub created_at_ms: i64,
    pub original_bytes: u64,
    pub compressed_bytes: u64,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedBackup {
    pub project_id: ProjectId,
    pub relative_path: RelativeProjectPath,
    pub content: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct BackupCapture {
    pub project_id: ProjectId,
    pub relative_path: RelativeProjectPath,
    pub content: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ShadowBackupStoreError {
    #[error("project revision storage is unavailable")]
    Storage,
    #[error("project revision does not exist")]
    NotFound,
    #[error("project revision is invalid")]
    InvalidRevision,
    #[error("project file is too large to revise")]
    TooLarge,
}

#[derive(Clone, Copy)]
struct RetentionPolicy {
    per_file: usize,
    account_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            per_file: MAX_REVISIONS_PER_FILE,
            account_bytes: MAX_BACKUP_ACCOUNT_BYTES,
        }
    }
}

pub struct ShadowBackupStore {
    connection: Mutex<Connection>,
    secrets: Arc<dyn SecretStore>,
    retention: RetentionPolicy,
}

impl ShadowBackupStore {
    pub fn open(
        path: impl AsRef<Path>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, ShadowBackupStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| ShadowBackupStoreError::Storage)?;
        }
        let connection = open_connection(path).map_err(map_persist)?;
        Ok(Self {
            connection: Mutex::new(connection),
            secrets,
            retention: RetentionPolicy::default(),
        })
    }

    pub fn capture(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        relative_path: &RelativeProjectPath,
        content: &[u8],
        created_at_ms: i64,
    ) -> Result<BackupRevision, ShadowBackupStoreError> {
        let revisions = self.capture_batch(
            account,
            &[BackupCapture {
                project_id,
                relative_path: relative_path.clone(),
                content: content.to_vec(),
            }],
            created_at_ms,
        )?;
        revisions
            .into_iter()
            .next()
            .ok_or(ShadowBackupStoreError::Storage)
    }

    /// Seals every prior file version before inserting any row. The insert
    /// and retention work then commit as one transaction, so a multi-file
    /// mutation can never start with only some of its rollback points stored.
    pub fn capture_batch(
        &self,
        account: &ValidatedAccountId,
        captures: &[BackupCapture],
        created_at_ms: i64,
    ) -> Result<Vec<BackupRevision>, ShadowBackupStoreError> {
        if captures.is_empty() {
            return Ok(Vec::new());
        }
        let mut prepared = Vec::with_capacity(captures.len());
        let mut batch_bytes = 0u64;
        for capture in captures {
            if capture.content.len() as u64 > MAX_EDITABLE_FILE_BYTES {
                return Err(ShadowBackupStoreError::TooLarge);
            }
            let backup_id = Uuid::new_v4();
            let project_id_string = capture.project_id.to_string();
            let backup_id_string = backup_id.to_string();
            let path_key = path_key(&capture.relative_path);
            let content_hash: [u8; 32] = Sha256::digest(&capture.content).into();
            let compressed = compress(&capture.content)?;
            let compressed_bytes = compressed.len() as u64;
            batch_bytes = batch_bytes
                .checked_add(compressed_bytes)
                .ok_or(ShadowBackupStoreError::TooLarge)?;
            let plaintext = encode_payload(
                &capture.relative_path,
                capture.content.len() as u64,
                &content_hash,
                &compressed,
            )?;
            let sealed = seal(
                self.secrets.as_ref(),
                account.as_str(),
                &project_id_string,
                &path_key,
                &backup_id_string,
                &plaintext,
            )?;
            prepared.push(PreparedBackup {
                revision: BackupRevision {
                    backup_id,
                    created_at_ms: created_at_ms.max(0),
                    original_bytes: capture.content.len() as u64,
                    compressed_bytes,
                    content_hash: hex_digest(&content_hash),
                },
                project_id: project_id_string,
                relative_path: capture.relative_path.clone(),
                path_key,
                backup_id: backup_id_string,
                sealed,
            });
        }
        if batch_bytes > self.retention.account_bytes {
            return Err(ShadowBackupStoreError::TooLarge);
        }

        let protected: Vec<Uuid> = prepared
            .iter()
            .map(|backup| backup.revision.backup_id)
            .collect();
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ShadowBackupStoreError::Storage)?;
        let transaction = connection
            .transaction()
            .map_err(|_| ShadowBackupStoreError::Storage)?;
        for backup in &prepared {
            transaction
                .execute(
                    "INSERT INTO project_file_backups (
                    account_id, project_id, path_key, backup_id, created_at_ms,
                    compressed_bytes, aad_schema_version, key_version, nonce, ciphertext
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        account.as_str(),
                        backup.project_id,
                        backup.path_key,
                        backup.backup_id,
                        created_at_ms.max(0),
                        i64::try_from(backup.revision.compressed_bytes)
                            .map_err(|_| ShadowBackupStoreError::TooLarge)?,
                        i64::from(backup.sealed.aad_schema_version),
                        i64::from(backup.sealed.key_version),
                        backup.sealed.nonce,
                        backup.sealed.ciphertext,
                    ],
                )
                .map_err(|_| ShadowBackupStoreError::Storage)?;
        }
        for backup in &prepared {
            prune_file(
                &transaction,
                account.as_str(),
                &backup.project_id,
                &backup.relative_path,
                &protected,
                self.retention.per_file,
            )?;
        }
        prune_account(
            &transaction,
            account.as_str(),
            &protected,
            self.retention.account_bytes,
        )?;
        transaction
            .commit()
            .map_err(|_| ShadowBackupStoreError::Storage)?;

        Ok(prepared.into_iter().map(|backup| backup.revision).collect())
    }

    #[cfg(test)]
    pub(super) fn with_retention_limits(mut self, per_file: usize, account_bytes: u64) -> Self {
        self.retention = RetentionPolicy {
            per_file,
            account_bytes,
        };
        self
    }

    pub fn list(
        &self,
        account: &ValidatedAccountId,
        project_id: ProjectId,
        relative_path: &RelativeProjectPath,
    ) -> Result<Vec<BackupRevision>, ShadowBackupStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ShadowBackupStoreError::Storage)?;
        let mut statement = connection
            .prepare(
                "SELECT backup_id, created_at_ms, compressed_bytes,
                        aad_schema_version, key_version, nonce, ciphertext
                 FROM project_file_backups
                 WHERE account_id = ?1 AND project_id = ?2 AND path_key = ?3
                 ORDER BY created_at_ms DESC, backup_id DESC",
            )
            .map_err(|_| ShadowBackupStoreError::Storage)?;
        let key = path_key(relative_path);
        let rows = statement
            .query_map(
                params![account.as_str(), project_id.to_string(), key],
                |row| {
                    Ok(BackupRow {
                        project_id: project_id.to_string(),
                        path_key: path_key(relative_path),
                        backup_id: row.get(0)?,
                        created_at_ms: row.get(1)?,
                        compressed_bytes: row.get(2)?,
                        aad_schema_version: row.get(3)?,
                        key_version: row.get(4)?,
                        nonce: row.get(5)?,
                        ciphertext: row.get(6)?,
                    })
                },
            )
            .map_err(|_| ShadowBackupStoreError::Storage)?;
        let mut revisions = Vec::new();
        for row in rows {
            let row = row.map_err(|_| ShadowBackupStoreError::Storage)?;
            let decoded = open(self.secrets.as_ref(), account.as_str(), &row)?;
            let payload = decode_payload(&decoded)?;
            if path_key(&payload.relative_path) != row.path_key {
                return Err(ShadowBackupStoreError::InvalidRevision);
            }
            revisions.push(BackupRevision {
                backup_id: Uuid::parse_str(&row.backup_id)
                    .map_err(|_| ShadowBackupStoreError::InvalidRevision)?,
                created_at_ms: row.created_at_ms,
                original_bytes: payload.original_bytes,
                compressed_bytes: u64::try_from(row.compressed_bytes)
                    .map_err(|_| ShadowBackupStoreError::InvalidRevision)?,
                content_hash: hex_digest(&payload.content_hash),
            });
        }
        Ok(revisions)
    }

    pub fn load(
        &self,
        account: &ValidatedAccountId,
        backup_id: Uuid,
    ) -> Result<LoadedBackup, ShadowBackupStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ShadowBackupStoreError::Storage)?;
        let row = connection
            .query_row(
                "SELECT project_id, path_key, backup_id, created_at_ms, compressed_bytes,
                        aad_schema_version, key_version, nonce, ciphertext
                 FROM project_file_backups
                 WHERE account_id = ?1 AND backup_id = ?2",
                params![account.as_str(), backup_id.to_string()],
                |row| {
                    Ok(BackupRow {
                        project_id: row.get(0)?,
                        path_key: row.get(1)?,
                        backup_id: row.get(2)?,
                        created_at_ms: row.get(3)?,
                        compressed_bytes: row.get(4)?,
                        aad_schema_version: row.get(5)?,
                        key_version: row.get(6)?,
                        nonce: row.get(7)?,
                        ciphertext: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|_| ShadowBackupStoreError::Storage)?
            .ok_or(ShadowBackupStoreError::NotFound)?;
        let decoded = open(self.secrets.as_ref(), account.as_str(), &row)?;
        let payload = decode_payload(&decoded)?;
        if path_key(&payload.relative_path) != row.path_key {
            return Err(ShadowBackupStoreError::InvalidRevision);
        }
        let content = decompress(&payload.compressed, payload.original_bytes)?;
        let actual_hash: [u8; 32] = Sha256::digest(&content).into();
        if actual_hash != payload.content_hash {
            return Err(ShadowBackupStoreError::InvalidRevision);
        }
        Ok(LoadedBackup {
            project_id: ProjectId::from_str(&row.project_id)
                .map_err(|_| ShadowBackupStoreError::InvalidRevision)?,
            relative_path: payload.relative_path,
            content,
        })
    }
}

struct PreparedBackup {
    revision: BackupRevision,
    project_id: String,
    relative_path: RelativeProjectPath,
    path_key: Vec<u8>,
    backup_id: String,
    sealed: Sealed,
}

struct BackupRow {
    project_id: String,
    path_key: Vec<u8>,
    backup_id: String,
    created_at_ms: i64,
    compressed_bytes: i64,
    aad_schema_version: i64,
    key_version: i64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

struct DecodedPayload {
    relative_path: RelativeProjectPath,
    original_bytes: u64,
    content_hash: [u8; 32],
    compressed: Vec<u8>,
}

struct Sealed {
    aad_schema_version: u32,
    key_version: u32,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn seal(
    secrets: &dyn SecretStore,
    account_id: &str,
    project_id: &str,
    path_key: &[u8],
    backup_id: &str,
    plaintext: &[u8],
) -> Result<Sealed, ShadowBackupStoreError> {
    let (key_version, key) = secrets
        .load_or_create_active_key(account_id)
        .map_err(map_persist)?;
    let metadata = BackupAuthenticatedMetadata {
        account_id,
        project_id,
        path_key,
        backup_id,
    };
    let encrypted = encrypt_backup(&key, key_version, &metadata, plaintext).map_err(map_persist)?;
    Ok(Sealed {
        aad_schema_version: encrypted.aad_schema_version,
        key_version,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
}

fn open(
    secrets: &dyn SecretStore,
    account_id: &str,
    row: &BackupRow,
) -> Result<Vec<u8>, ShadowBackupStoreError> {
    let aad_schema_version = u32::try_from(row.aad_schema_version)
        .map_err(|_| ShadowBackupStoreError::InvalidRevision)?;
    let key_version =
        u32::try_from(row.key_version).map_err(|_| ShadowBackupStoreError::InvalidRevision)?;
    let key = secrets
        .account_key(account_id, key_version)
        .map_err(map_persist)?;
    decrypt_backup(
        &key,
        aad_schema_version,
        key_version,
        &row.nonce,
        &BackupAuthenticatedMetadata {
            account_id,
            project_id: &row.project_id,
            path_key: &row.path_key,
            backup_id: &row.backup_id,
        },
        &row.ciphertext,
    )
    .map_err(map_persist)
}

fn encode_payload(
    relative_path: &RelativeProjectPath,
    original_bytes: u64,
    content_hash: &[u8],
    compressed: &[u8],
) -> Result<Vec<u8>, ShadowBackupStoreError> {
    let path = relative_path.as_str().as_bytes();
    let path_len = u32::try_from(path.len()).map_err(|_| ShadowBackupStoreError::TooLarge)?;
    let mut payload = Vec::with_capacity(PAYLOAD_FIXED_BYTES + path.len() + compressed.len());
    payload.extend_from_slice(PAYLOAD_MAGIC);
    payload.extend_from_slice(&path_len.to_be_bytes());
    payload.extend_from_slice(&original_bytes.to_be_bytes());
    payload.extend_from_slice(content_hash);
    payload.extend_from_slice(path);
    payload.extend_from_slice(compressed);
    Ok(payload)
}

fn decode_payload(payload: &[u8]) -> Result<DecodedPayload, ShadowBackupStoreError> {
    if payload.len() < PAYLOAD_FIXED_BYTES || &payload[..4] != PAYLOAD_MAGIC {
        return Err(ShadowBackupStoreError::InvalidRevision);
    }
    let path_len = u32::from_be_bytes(
        payload[4..8]
            .try_into()
            .map_err(|_| ShadowBackupStoreError::InvalidRevision)?,
    ) as usize;
    let original_bytes = u64::from_be_bytes(
        payload[8..16]
            .try_into()
            .map_err(|_| ShadowBackupStoreError::InvalidRevision)?,
    );
    if original_bytes > MAX_EDITABLE_FILE_BYTES || payload.len() < PAYLOAD_FIXED_BYTES + path_len {
        return Err(ShadowBackupStoreError::InvalidRevision);
    }
    let content_hash: [u8; 32] = payload[16..48]
        .try_into()
        .map_err(|_| ShadowBackupStoreError::InvalidRevision)?;
    let path_end = PAYLOAD_FIXED_BYTES + path_len;
    let path = std::str::from_utf8(&payload[PAYLOAD_FIXED_BYTES..path_end])
        .map_err(|_| ShadowBackupStoreError::InvalidRevision)?;
    Ok(DecodedPayload {
        relative_path: RelativeProjectPath::new(path)
            .map_err(|_| ShadowBackupStoreError::InvalidRevision)?,
        original_bytes,
        content_hash,
        compressed: payload[path_end..].to_vec(),
    })
}

fn compress(content: &[u8]) -> Result<Vec<u8>, ShadowBackupStoreError> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(content)
        .map_err(|_| ShadowBackupStoreError::Storage)?;
    encoder
        .finish()
        .map_err(|_| ShadowBackupStoreError::Storage)
}

fn decompress(compressed: &[u8], original_bytes: u64) -> Result<Vec<u8>, ShadowBackupStoreError> {
    let mut content = Vec::with_capacity(original_bytes as usize);
    DeflateDecoder::new(compressed)
        .take(original_bytes.saturating_add(1))
        .read_to_end(&mut content)
        .map_err(|_| ShadowBackupStoreError::InvalidRevision)?;
    if content.len() as u64 != original_bytes {
        return Err(ShadowBackupStoreError::InvalidRevision);
    }
    Ok(content)
}

fn prune_file(
    transaction: &Transaction<'_>,
    account_id: &str,
    project_id: &str,
    relative_path: &RelativeProjectPath,
    protected: &[Uuid],
    per_file: usize,
) -> Result<(), ShadowBackupStoreError> {
    let key = path_key(relative_path);
    let mut file_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM project_file_backups
             WHERE account_id = ?1 AND project_id = ?2 AND path_key = ?3",
            params![account_id, project_id, key],
            |row| row.get(0),
        )
        .map_err(|_| ShadowBackupStoreError::Storage)?;
    while file_count > per_file as i64 {
        if delete_oldest(transaction, account_id, Some((project_id, &key)), protected)?.is_none() {
            return Err(ShadowBackupStoreError::TooLarge);
        }
        file_count -= 1;
    }

    Ok(())
}

fn prune_account(
    transaction: &Transaction<'_>,
    account_id: &str,
    protected: &[Uuid],
    account_limit: u64,
) -> Result<(), ShadowBackupStoreError> {
    let mut account_bytes: i64 = transaction
        .query_row(
            "SELECT COALESCE(SUM(compressed_bytes), 0) FROM project_file_backups
             WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(|_| ShadowBackupStoreError::Storage)?;
    while account_bytes > account_limit as i64 {
        let removed = delete_oldest(transaction, account_id, None, protected)?;
        let Some(removed) = removed else {
            return Err(ShadowBackupStoreError::TooLarge);
        };
        account_bytes = account_bytes.saturating_sub(removed);
    }
    Ok(())
}

fn delete_oldest(
    transaction: &Transaction<'_>,
    account_id: &str,
    path: Option<(&str, &[u8])>,
    protected: &[Uuid],
) -> Result<Option<i64>, ShadowBackupStoreError> {
    let protected_strings: Vec<String> = protected.iter().map(Uuid::to_string).collect();
    let placeholders = std::iter::repeat_n("?", protected_strings.len())
        .collect::<Vec<_>>()
        .join(", ");
    let query = if path.is_some() {
        format!(
            "SELECT backup_id, compressed_bytes FROM project_file_backups
             WHERE account_id = ? AND project_id = ? AND path_key = ?
               AND backup_id NOT IN ({placeholders})
             ORDER BY created_at_ms ASC, backup_id ASC LIMIT 1"
        )
    } else {
        format!(
            "SELECT backup_id, compressed_bytes FROM project_file_backups
             WHERE account_id = ? AND backup_id NOT IN ({placeholders})
             ORDER BY created_at_ms ASC, backup_id ASC LIMIT 1"
        )
    };
    let row: Option<(String, i64)> = match path {
        Some((project_id, key)) => {
            let mut values: Vec<&dyn rusqlite::ToSql> = vec![&account_id, &project_id, &key];
            values.extend(
                protected_strings
                    .iter()
                    .map(|value| value as &dyn rusqlite::ToSql),
            );
            transaction
                .query_row(&query, values.as_slice(), |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()
        }
        None => {
            let mut values: Vec<&dyn rusqlite::ToSql> = vec![&account_id];
            values.extend(
                protected_strings
                    .iter()
                    .map(|value| value as &dyn rusqlite::ToSql),
            );
            transaction
                .query_row(&query, values.as_slice(), |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()
        }
    }
    .map_err(|_| ShadowBackupStoreError::Storage)?;
    let Some((backup_id, compressed_bytes)) = row else {
        return Ok(None);
    };
    transaction
        .execute(
            "DELETE FROM project_file_backups WHERE account_id = ?1 AND backup_id = ?2",
            params![account_id, backup_id],
        )
        .map_err(|_| ShadowBackupStoreError::Storage)?;
    Ok(Some(compressed_bytes.max(0)))
}

fn path_key(relative_path: &RelativeProjectPath) -> Vec<u8> {
    Sha256::digest(relative_path.as_str().as_bytes()).to_vec()
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn open_connection(path: &Path) -> AppResult<Connection> {
    crate::persistence::open_local_sqlite_store(
        path,
        crate::persistence::LocalSqliteSynchronous::Normal,
        migrate,
    )
}

fn migrate(connection: &mut Connection) -> AppResult<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > PROJECT_BACKUP_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_BACKUP_SCHEMA_VERSION {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > PROJECT_BACKUP_SCHEMA_VERSION {
        return Err(AppError::PersistenceSchemaUnsupported);
    }
    if version == PROJECT_BACKUP_SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    transaction.execute_batch(PROJECT_BACKUP_MIGRATION)?;
    transaction.pragma_update(None, "user_version", PROJECT_BACKUP_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn map_persist(_: AppError) -> ShadowBackupStoreError {
    ShadowBackupStoreError::Storage
}
