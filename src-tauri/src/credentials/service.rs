use std::{fmt, sync::Arc};

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::Value;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::credentials::{CredentialStatus, ProviderCredentialView, ProviderId};
use zeroize::Zeroizing;

use super::{
    envelope::ENVELOPE_PREFIX,
    vault::{CredentialVault, VaultError},
    verifier::{CredentialVerifier, VerificationError},
};

const MAX_ID_BYTES: usize = 160;
const MAX_LABEL_BYTES: usize = 80;
pub(crate) const MAX_PROVIDER_KEY_BYTES: usize = 3_072;
const DA_SECRET_OWNER: &str = "data-analyst";
const DA_ENVELOPE_PREFIX: &str = "xfda:v1:";

pub(crate) struct CredentialActor {
    account_id: String,
    tenant_id: String,
    user_id: String,
}

impl CredentialActor {
    pub(crate) fn new(
        account_id: impl Into<String>,
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Result<Self, CredentialServiceError> {
        let actor = Self {
            account_id: account_id.into(),
            tenant_id: tenant_id.into(),
            user_id: user_id.into(),
        };
        for value in [
            actor.account_id.as_str(),
            actor.tenant_id.as_str(),
            actor.user_id.as_str(),
        ] {
            if !is_valid_id(value) {
                return Err(CredentialServiceError::InvalidRequest);
            }
        }
        Ok(actor)
    }
}

pub(crate) struct CredentialCreateRequest {
    provider: ProviderId,
    label: String,
    secret: Zeroizing<String>,
    is_default: bool,
}

impl CredentialCreateRequest {
    pub(crate) fn new(
        provider: ProviderId,
        label: impl Into<String>,
        secret: Zeroizing<String>,
        is_default: bool,
    ) -> Result<Self, CredentialServiceError> {
        let label = label.into();
        let trimmed = label.trim();
        if trimmed.is_empty()
            || trimmed.len() > MAX_LABEL_BYTES
            || secret.len() < 8
            || secret.len() > MAX_PROVIDER_KEY_BYTES
        {
            return Err(CredentialServiceError::InvalidRequest);
        }
        Ok(Self {
            provider,
            label: trimmed.to_owned(),
            secret,
            is_default,
        })
    }
}

/// A request to replace an existing row's DA-owned secret.
///
/// Carries no provider or label: both are read from the stored row, so a caller
/// cannot pair a secret with the wrong provider's verification origin.
pub(crate) struct CredentialReplaceRequest {
    id: Uuid,
    secret: Zeroizing<String>,
}

impl CredentialReplaceRequest {
    pub(crate) fn new(id: &str, secret: Zeroizing<String>) -> Result<Self, CredentialServiceError> {
        if id.len() > MAX_ID_BYTES || secret.len() < 8 || secret.len() > MAX_PROVIDER_KEY_BYTES {
            return Err(CredentialServiceError::InvalidRequest);
        }
        let id = Uuid::parse_str(id).map_err(|_| CredentialServiceError::InvalidRequest)?;
        Ok(Self { id, secret })
    }
}

/// A request to delete one stored provider-key row.
pub(crate) struct CredentialDeleteRequest {
    id: Uuid,
}

impl CredentialDeleteRequest {
    pub(crate) fn new(id: &str) -> Result<Self, CredentialServiceError> {
        if id.len() > MAX_ID_BYTES {
            return Err(CredentialServiceError::InvalidRequest);
        }
        let id = Uuid::parse_str(id).map_err(|_| CredentialServiceError::InvalidRequest)?;
        Ok(Self { id })
    }
}

pub(crate) struct NewCredential {
    provider: ProviderId,
    label: String,
    secret: Zeroizing<String>,
    is_default: bool,
}

impl NewCredential {
    pub(crate) fn into_parts(self) -> (ProviderId, String, Zeroizing<String>, bool) {
        (self.provider, self.label, self.secret, self.is_default)
    }
}

impl fmt::Debug for NewCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewCredential")
            .field("provider", &self.provider)
            .field("label", &self.label)
            .field("secret", &"[REDACTED]")
            .field("is_default", &self.is_default)
            .finish()
    }
}

pub(crate) struct CredentialRecord {
    id: Uuid,
    provider: ProviderId,
    label: String,
    ciphertext: Zeroizing<String>,
    metadata: IndexMap<String, Value>,
    is_default: bool,
    is_enabled: bool,
}

impl CredentialRecord {
    pub(crate) fn new(
        id: Uuid,
        provider: ProviderId,
        label: impl Into<String>,
        ciphertext: impl Into<String>,
        metadata: IndexMap<String, Value>,
        is_default: bool,
        is_enabled: bool,
    ) -> Self {
        Self {
            id,
            provider,
            label: label.into(),
            ciphertext: Zeroizing::new(ciphertext.into()),
            metadata,
            is_default,
            is_enabled,
        }
    }

    pub(crate) fn to_view(&self, device_key_id: Option<&str>) -> ProviderCredentialView {
        let status = if !self.is_enabled {
            CredentialStatus::Disabled
        } else if self.is_da_owned() || self.is_compatible_with(device_key_id) {
            CredentialStatus::Ready
        } else {
            CredentialStatus::NeedsReentry
        };
        ProviderCredentialView::new(
            self.id.to_string(),
            self.provider,
            self.label.trim(),
            status,
            self.is_default,
        )
    }

    fn is_compatible_with(&self, device_key_id: Option<&str>) -> bool {
        self.ciphertext.starts_with(ENVELOPE_PREFIX)
            && device_key_id.is_some_and(|active_key_id| {
                self.metadata.get("device_key_id").and_then(Value::as_str) == Some(active_key_id)
            })
    }

    fn is_da_owned(&self) -> bool {
        self.ciphertext.starts_with(DA_ENVELOPE_PREFIX)
            && self.metadata.get("secret_owner").and_then(Value::as_str) == Some(DA_SECRET_OWNER)
    }

    #[cfg(test)]
    pub(super) fn duplicate_for_test(&self) -> Self {
        Self::new(
            self.id,
            self.provider,
            self.label.clone(),
            self.ciphertext.as_str(),
            self.metadata.clone(),
            self.is_default,
            self.is_enabled,
        )
    }
}

impl fmt::Debug for CredentialRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialRecord")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("label", &self.label)
            .field("ciphertext", &"[REDACTED]")
            .field("metadata", &"[REDACTED]")
            .field("is_default", &self.is_default)
            .field("is_enabled", &self.is_enabled)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CredentialRepositoryError {
    #[error("credential repository is unauthorized")]
    Unauthorized,
    #[error("credential already exists")]
    Conflict,
    #[error("credential repository is unavailable")]
    Unavailable,
    #[error("credential repository rate limit exceeded")]
    RateLimited,
    #[error("credential repository rejected the request")]
    InvalidRequest,
    #[error("credential mutation completion is unknown")]
    ReconciliationRequired,
    #[error("credential row uses an unsupported provider")]
    UnsupportedProviderRow,
    #[error("no credential row with that id exists")]
    NotFound,
}

#[async_trait]
pub(crate) trait CredentialRepository: Send + Sync + 'static {
    async fn list(&self) -> Result<Vec<CredentialRecord>, CredentialRepositoryError>;
    async fn reconcile_after_create(
        &self,
    ) -> Result<Vec<CredentialRecord>, CredentialRepositoryError> {
        self.list().await
    }
    async fn create(
        &self,
        credential: NewCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError>;
    /// Replaces one existing row's secret through DA, leaving its identity, label,
    /// and default flag alone.
    ///
    /// Separate from [`Self::create`] because re-entering a key is not the same
    /// act as adding one: the row's uniqueness key is
    /// `(tenant, user, provider, label)`, so creating "the same key again" under
    /// the same label conflicts, and creating it under a *different* label
    /// leaves the unusable row behind — still holding the default flag.
    async fn replace_secret(
        &self,
        credential: ReplacementCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError>;
    async fn resolve_secret(
        &self,
        id: Uuid,
        provider: ProviderId,
    ) -> Result<ResolvedCredential, CredentialRepositoryError>;
    /// Deletes one row through DA. Every signed-in xFalcon client shares the
    /// row, so this removes the key for all of them, not only this desktop.
    async fn delete(&self, id: Uuid) -> Result<(), CredentialRepositoryError>;
}

pub(crate) struct ReplacementCredential {
    pub(crate) id: Uuid,
    pub(crate) provider: ProviderId,
    pub(crate) secret: Zeroizing<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CredentialMigrationReport {
    pub(crate) migrated: usize,
    pub(crate) requires_reentry: usize,
    pub(crate) failed: usize,
}

pub(crate) struct CredentialService {
    vault: Arc<CredentialVault>,
    repository: Arc<dyn CredentialRepository>,
    verifier: Arc<dyn CredentialVerifier>,
    mutation_lock: Arc<AsyncMutex<()>>,
}

impl CredentialService {
    pub(crate) fn new(
        vault: Arc<CredentialVault>,
        repository: Arc<dyn CredentialRepository>,
        verifier: Arc<dyn CredentialVerifier>,
    ) -> Self {
        Self::with_mutation_lock(vault, repository, verifier, Arc::new(AsyncMutex::new(())))
    }

    pub(crate) const fn with_mutation_lock(
        vault: Arc<CredentialVault>,
        repository: Arc<dyn CredentialRepository>,
        verifier: Arc<dyn CredentialVerifier>,
        mutation_lock: Arc<AsyncMutex<()>>,
    ) -> Self {
        Self {
            vault,
            repository,
            verifier,
            mutation_lock,
        }
    }

    pub(crate) async fn list(
        &self,
        actor: &CredentialActor,
    ) -> Result<Vec<ProviderCredentialView>, CredentialServiceError> {
        let records = self.repository.list().await?;
        let device_key_id = records
            .iter()
            .any(|record| !record.is_da_owned() && record.ciphertext.starts_with(ENVELOPE_PREFIX))
            .then(|| self.vault.active_device_key_id(&actor.account_id).ok())
            .flatten();
        Ok(records
            .iter()
            .map(|record| record.to_view(device_key_id.as_deref()))
            .collect())
    }

    pub(crate) async fn create(
        &self,
        actor: &CredentialActor,
        request: CredentialCreateRequest,
    ) -> Result<ProviderCredentialView, CredentialServiceError> {
        self.verifier
            .verify(request.provider, request.secret.as_str())
            .await?;
        let created = match self
            .repository
            .create(NewCredential {
                provider: request.provider,
                label: request.label.clone(),
                secret: request.secret.clone(),
                is_default: request.is_default,
            })
            .await
        {
            Ok(created) => created,
            Err(CredentialRepositoryError::ReconciliationRequired) => {
                // A lost response does not establish whether DA committed. Confirm
                // the stored secret before reporting success; never repeat POST.
                let records = match self.repository.reconcile_after_create().await {
                    Ok(records) => records,
                    Err(_) => {
                        return Err(CredentialServiceError::ReconciliationRequired {
                            credentials: None,
                        })
                    }
                };
                if let Some(record) = records.iter().find(|record| {
                    record.provider == request.provider
                        && record.label == request.label
                        && record.is_default == request.is_default
                        && record.is_enabled
                        && record.is_da_owned()
                }) {
                    match self
                        .repository
                        .resolve_secret(record.id, record.provider)
                        .await
                    {
                        Ok(resolved)
                            if bool::from(
                                resolved.secret.as_bytes().ct_eq(request.secret.as_bytes()),
                            ) =>
                        {
                            return Ok(record.to_view(None));
                        }
                        Ok(_) | Err(_) => {}
                    }
                }
                let device_key_id = self.vault.active_device_key_id(&actor.account_id).ok();
                let credentials = Some(
                    records
                        .iter()
                        .map(|record| record.to_view(device_key_id.as_deref()))
                        .collect(),
                );
                return Err(CredentialServiceError::ReconciliationRequired { credentials });
            }
            Err(error) => return Err(error.into()),
        };
        Ok(created.to_view(None))
    }

    /// Replaces an existing row's secret through DA, in place.
    ///
    /// This is what the `needs_reentry` status asks the user to do. It exists
    /// because "add it again" cannot express it: the row's uniqueness key
    /// includes the label, so re-adding under the same label conflicts, and
    /// under a new label the unusable row survives — keeping the default flag
    /// and leaving a permanently broken entry in the list.
    ///
    /// The secret is verified with the provider before anything is written, so a
    /// bad paste cannot replace a good key with one that does not work.
    pub(crate) async fn replace_secret(
        &self,
        _actor: &CredentialActor,
        request: CredentialReplaceRequest,
    ) -> Result<ProviderCredentialView, CredentialServiceError> {
        let _mutation_guard = self.mutation_lock.lock().await;
        let records = self.repository.list().await?;
        let existing = records
            .iter()
            .find(|record| record.id == request.id)
            .ok_or(CredentialServiceError::NotFound)?;

        // The provider comes from the stored row, never from the caller: a row's
        // provider decides which origin the secret is sent to for verification,
        // so accepting a caller-supplied one would let a mismatched pair be written.
        let provider = existing.provider;
        self.verifier
            .verify(provider, request.secret.as_str())
            .await?;
        let replaced = self
            .repository
            .replace_secret(ReplacementCredential {
                id: request.id,
                provider,
                secret: request.secret,
            })
            .await?;
        Ok(replaced.to_view(None))
    }

    /// Deletes a stored key.
    ///
    /// Held under the mutation lock and checked against a fresh list, like
    /// [`Self::replace_secret`]: only the row the id names now is ever sent to
    /// DA, and a background migration must not re-write the row between the
    /// check and the delete.
    ///
    /// Idempotent: a key already gone -- removed by another client, or by a
    /// delete whose answer was lost -- is what the person asked for, so it
    /// succeeds without deleting anything, whether the fresh list or DA itself
    /// is the one to say so.
    pub(crate) async fn delete(
        &self,
        _actor: &CredentialActor,
        request: CredentialDeleteRequest,
    ) -> Result<(), CredentialServiceError> {
        let _mutation_guard = self.mutation_lock.lock().await;
        let records = self.repository.list().await?;
        if !records.iter().any(|record| record.id == request.id) {
            return Ok(());
        }
        match self.repository.delete(request.id).await {
            Ok(()) | Err(CredentialRepositoryError::NotFound) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) async fn migrate_legacy_provider_keys(
        &self,
        actor: &CredentialActor,
    ) -> Result<CredentialMigrationReport, CredentialServiceError> {
        self.migrate_legacy_provider_keys_until(actor, &CancellationToken::new())
            .await
    }

    pub(crate) async fn migrate_legacy_provider_keys_until(
        &self,
        actor: &CredentialActor,
        cancellation: &CancellationToken,
    ) -> Result<CredentialMigrationReport, CredentialServiceError> {
        let records = self.repository.list().await?;
        let device_key_id = records
            .iter()
            .any(|record| !record.is_da_owned() && record.ciphertext.starts_with(ENVELOPE_PREFIX))
            .then(|| self.vault.active_device_key_id(&actor.account_id).ok())
            .flatten();
        let mut report = CredentialMigrationReport::default();

        for listed_record in records {
            if cancellation.is_cancelled() {
                break;
            }
            if listed_record.is_da_owned() {
                continue;
            }
            if !listed_record.is_compatible_with(device_key_id.as_deref()) {
                report.requires_reentry += 1;
                continue;
            }

            // Serialize migrations with local re-entry, then re-read the row
            // under that lock. A stale launch snapshot must never overwrite a
            // newer secret the user just supplied on this desktop.
            let _mutation_guard = self.mutation_lock.lock().await;
            if cancellation.is_cancelled() {
                break;
            }
            let current_records = match self.repository.list().await {
                Ok(records) => records,
                Err(_) => {
                    report.failed += 1;
                    continue;
                }
            };
            let Some(record) = current_records
                .iter()
                .find(|record| record.id == listed_record.id)
            else {
                continue;
            };
            if record.is_da_owned() {
                continue;
            }
            if !record.is_compatible_with(device_key_id.as_deref()) {
                report.requires_reentry += 1;
                continue;
            }
            let secret = match self.vault.open(
                &actor.account_id,
                &actor.tenant_id,
                &actor.user_id,
                record.provider,
                record.ciphertext.as_str(),
            ) {
                Ok(secret) => secret,
                Err(_) => {
                    report.requires_reentry += 1;
                    continue;
                }
            };
            match self
                .repository
                .replace_secret(ReplacementCredential {
                    id: record.id,
                    provider: record.provider,
                    secret,
                })
                .await
            {
                Ok(_) => report.migrated += 1,
                Err(_) => report.failed += 1,
            }
        }

        Ok(report)
    }

    /// Resolves the plaintext secret to use for one local inference call: the
    /// tenant/user-bound enabled credential for `provider`, preferring the account's
    /// default. The returned secret is bounded to this call — callers must
    /// never persist, log, or clone it into longer-lived state.
    pub(crate) async fn resolve_for_inference(
        &self,
        actor: &CredentialActor,
        provider: ProviderId,
    ) -> Result<ResolvedCredential, CredentialServiceError> {
        let records = self.repository.list().await?;
        let device_key_id = records
            .iter()
            .any(|record| !record.is_da_owned() && record.ciphertext.starts_with(ENVELOPE_PREFIX))
            .then(|| self.vault.active_device_key_id(&actor.account_id).ok())
            .flatten();
        // DA returns provider keys newest-first. Preserve that order within a
        // priority class: choose the newest enabled default, otherwise the
        // newest enabled compatible row. `max_by_key(is_default)` picks the
        // *last* tied row and silently selects the oldest credential.
        let is_candidate = |record: &&CredentialRecord| {
            record.provider == provider
                && record.is_enabled
                && (record.is_da_owned() || record.is_compatible_with(device_key_id.as_deref()))
        };
        let candidate = records
            .iter()
            .filter(is_candidate)
            .find(|record| record.is_default)
            .or_else(|| records.iter().find(is_candidate))
            .ok_or(CredentialServiceError::NoUsableCredential)?;

        if !candidate.is_da_owned() {
            let _mutation_guard = self.mutation_lock.lock().await;
            let refreshed_records = self.repository.list().await?;
            let candidate = refreshed_records
                .iter()
                .find(|record| {
                    record.id == candidate.id
                        && record.provider == provider
                        && record.is_enabled
                        && (record.is_da_owned()
                            || record.is_compatible_with(device_key_id.as_deref()))
                })
                .ok_or(CredentialServiceError::NoUsableCredential)?;
            if candidate.is_da_owned() {
                return self
                    .repository
                    .resolve_secret(candidate.id, provider)
                    .await
                    .map_err(map_inference_repository_error);
            }
            let secret = self.vault.open(
                &actor.account_id,
                &actor.tenant_id,
                &actor.user_id,
                provider,
                candidate.ciphertext.as_str(),
            )?;
            self.repository
                .replace_secret(ReplacementCredential {
                    id: candidate.id,
                    provider,
                    secret,
                })
                .await
                .map_err(map_inference_repository_error)?;
        }

        self.repository
            .resolve_secret(candidate.id, provider)
            .await
            .map_err(map_inference_repository_error)
    }

    /// Private models bind one exact DA credential. Never select another key
    /// by provider or default status when that binding is missing.
    pub(crate) async fn resolve_exact_custom_for_inference(
        &self,
        _actor: &CredentialActor,
        id: Uuid,
    ) -> Result<ResolvedCredential, CredentialServiceError> {
        self.repository
            .resolve_secret(id, ProviderId::CustomOpenAi)
            .await
            .map_err(map_inference_repository_error)
    }
}

/// A decrypted provider secret bounded to a single inference call.
#[derive(Clone)]
pub(crate) struct ResolvedCredential {
    pub(crate) credential_id: String,
    pub(crate) secret: Zeroizing<String>,
    pub(crate) base_url: Option<String>,
}

impl fmt::Debug for ResolvedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedCredential")
            .field("credential_id", &self.credential_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CredentialServiceError {
    #[error("credential request is invalid")]
    InvalidRequest,
    #[error("the provider rejected this credential")]
    InvalidCredential,
    #[error("the credential lacks required provider permissions")]
    InsufficientPermissions,
    #[error("the provider is temporarily unavailable")]
    ProviderUnavailable,
    #[error("the provider rate limited credential verification")]
    RateLimited,
    #[error("credential storage is unavailable")]
    StorageUnavailable,
    #[error("credential service is unavailable")]
    ServiceUnavailable,
    #[error("credential already exists")]
    Conflict,
    #[error("credential reconciliation is required")]
    ReconciliationRequired {
        credentials: Option<Vec<ProviderCredentialView>>,
    },
    #[error("credential session is unauthorized")]
    Unauthorized,
    #[error("no enabled credential is available for this provider")]
    NoUsableCredential,
    #[error("no credential with that id is in this workspace")]
    NotFound,
}

impl From<VaultError> for CredentialServiceError {
    fn from(_: VaultError) -> Self {
        Self::StorageUnavailable
    }
}

impl From<VerificationError> for CredentialServiceError {
    fn from(error: VerificationError) -> Self {
        match error {
            VerificationError::InvalidCredential => Self::InvalidCredential,
            VerificationError::InsufficientPermissions => Self::InsufficientPermissions,
            VerificationError::ProviderUnavailable => Self::ProviderUnavailable,
            VerificationError::RateLimited => Self::RateLimited,
        }
    }
}

impl From<CredentialRepositoryError> for CredentialServiceError {
    fn from(error: CredentialRepositoryError) -> Self {
        match error {
            CredentialRepositoryError::Unauthorized => Self::Unauthorized,
            CredentialRepositoryError::Conflict => Self::Conflict,
            CredentialRepositoryError::Unavailable => Self::ServiceUnavailable,
            CredentialRepositoryError::RateLimited => Self::RateLimited,
            CredentialRepositoryError::InvalidRequest => Self::InvalidRequest,
            CredentialRepositoryError::ReconciliationRequired => {
                Self::ReconciliationRequired { credentials: None }
            }
            CredentialRepositoryError::UnsupportedProviderRow => Self::ServiceUnavailable,
            CredentialRepositoryError::NotFound => Self::NotFound,
        }
    }
}

fn map_inference_repository_error(error: CredentialRepositoryError) -> CredentialServiceError {
    match error {
        CredentialRepositoryError::Conflict | CredentialRepositoryError::NotFound => {
            CredentialServiceError::NoUsableCredential
        }
        other => other.into(),
    }
}

fn is_valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
