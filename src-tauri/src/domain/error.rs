use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("the persistence worker is unavailable")]
    PersistenceUnavailable,
    #[error("the persistence worker is busy")]
    PersistenceBusy,
    #[error("the persistence request is invalid: {0}")]
    PersistenceInvalid(&'static str),
    #[error("the account encryption key is unavailable")]
    PersistenceKeyUnavailable,
    #[error("the account encryption key version is not a new immutable version")]
    PersistenceKeyVersionConflict,
    #[error("the encrypted queue has reached its configured capacity")]
    PersistenceCapacityReached,
    #[error("failed to encrypt persisted data")]
    PersistenceEncryptionFailed,
    #[error("failed to decrypt persisted data")]
    PersistenceDecryptionFailed,
    #[error("the persistence database operation failed")]
    PersistenceDatabase(#[source] rusqlite::Error),
    #[error("the persistence database schema is newer than this application supports")]
    PersistenceSchemaUnsupported,
    #[error("the persistence worker could not be started")]
    PersistenceStartup(#[source] std::io::Error),
    #[error("the audit transport request failed")]
    AuditTransportFailed,
    #[error("the generated DA audit response could not be decoded")]
    AuditContractFailed,
    #[error("the audit identity does not match the authenticated DA session")]
    AuditIdentityMismatch,
    #[error("the local audit log could not be read or written")]
    AuditStorageFailed,
}

impl AppError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PersistenceUnavailable => "persistence_unavailable",
            Self::PersistenceBusy => "persistence_busy",
            Self::PersistenceInvalid(_) => "persistence_invalid",
            Self::PersistenceKeyUnavailable => "persistence_key_unavailable",
            Self::PersistenceKeyVersionConflict => "persistence_key_version_conflict",
            Self::PersistenceCapacityReached => "persistence_capacity_reached",
            Self::PersistenceEncryptionFailed => "persistence_encryption_failed",
            Self::PersistenceDecryptionFailed => "persistence_decryption_failed",
            Self::PersistenceDatabase(_) => "persistence_database_failed",
            Self::PersistenceSchemaUnsupported => "persistence_schema_unsupported",
            Self::PersistenceStartup(_) => "persistence_startup_failed",
            Self::AuditTransportFailed => "audit_transport_failed",
            Self::AuditContractFailed => "audit_contract_failed",
            Self::AuditIdentityMismatch => "audit_identity_mismatch",
            Self::AuditStorageFailed => "audit_storage_failed",
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        Self::PersistenceDatabase(error)
    }
}
