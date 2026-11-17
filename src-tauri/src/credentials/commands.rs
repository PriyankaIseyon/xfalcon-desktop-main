use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::{
    CredentialErrorCode, CredentialId, CredentialOperationError, ProviderCredentialView, ProviderId,
};
use zeroize::Zeroizing;

use super::{
    service::{
        CredentialActor, CredentialCreateRequest, CredentialDeleteRequest,
        CredentialMigrationReport, CredentialReplaceRequest, CredentialRepository,
        CredentialService, CredentialServiceError, ResolvedCredential,
    },
    vault::CredentialVault,
    verifier::{CredentialVerifier, UnavailableCredentialVerifier},
};
use crate::{
    app::commands::AppRuntimeState,
    audit::{emit, AuditAction, AuditOutcome, AuditResourceType},
    auth::{
        canonical_account_id, ensure_fresh_session, AuthConfig, AuthConfigError, AuthSessionStore,
    },
    clients::da::{mapping::build_hardened_client, provider_keys::DaCredentialRepository},
    persistence::SecretStore,
    providers::FixedOriginCredentialVerifier,
    security::account_cleanup::ValidatedAccountId,
};

pub struct CredentialRuntime {
    sessions: Arc<AuthSessionStore>,
    vault: Arc<CredentialVault>,
    verifier: Arc<dyn CredentialVerifier>,
    mutation_lock: Arc<AsyncMutex<()>>,
    migration_cancellation: Mutex<Option<CancellationToken>>,
    da_client: reqwest::Client,
}

impl CredentialRuntime {
    pub fn new(sessions: Arc<AuthSessionStore>, credential_secrets: Arc<dyn SecretStore>) -> Self {
        let verifier: Arc<dyn CredentialVerifier> = FixedOriginCredentialVerifier::new()
            .map(|verifier| Arc::new(verifier) as Arc<dyn CredentialVerifier>)
            .unwrap_or_else(|_| Arc::new(UnavailableCredentialVerifier));
        Self {
            sessions,
            vault: Arc::new(CredentialVault::new(credential_secrets)),
            verifier,
            mutation_lock: Arc::new(AsyncMutex::new(())),
            migration_cancellation: Mutex::new(None),
            da_client: build_hardened_client()
                .expect("hardened DA credential client must be constructible"),
        }
    }

    /// The device envelope used by local MCP credentials and legacy migration.
    ///
    /// Shared rather than rebuilt so every sealed-secret surface in the app
    /// Provider-key creation no longer uses this vault. It remains shared with
    /// registered-server credentials so legacy `xfdc:v1` provider rows can be
    /// opened and migrated with the same device key that originally sealed them.
    pub(crate) fn vault(&self) -> Arc<CredentialVault> {
        self.vault.clone()
    }

    fn authenticated_snapshot(
        &self,
    ) -> Result<
        (
            xfalcon_contracts::auth::IdentityProjection,
            Zeroizing<String>,
        ),
        CredentialOperationError,
    > {
        let snapshot = self
            .sessions
            .authenticated_snapshot()
            .ok_or_else(|| credential_error(CredentialErrorCode::Unauthorized))?;
        Ok((snapshot.identity, snapshot.access_token))
    }

    fn service_and_actor(
        &self,
    ) -> Result<(CredentialService, CredentialActor), CredentialOperationError> {
        self.service_and_actor_with_config(AuthConfig::from_env)
    }

    async fn fresh_service_and_actor(
        &self,
    ) -> Result<(CredentialService, CredentialActor), CredentialOperationError> {
        // Provider-key operations spend the bearer just like chat, uploads,
        // workspace sync, and MCP. Renew first so an otherwise healthy long-
        // running desktop session is not misreported as unauthorized.
        let _ = ensure_fresh_session(&self.sessions).await;
        self.service_and_actor()
    }

    async fn fresh_service_and_actor_for(
        &self,
        expected_account: &ValidatedAccountId,
    ) -> Result<(CredentialService, CredentialActor), CredentialOperationError> {
        // Authenticate and compare from the same post-renewal snapshot. This
        // prevents a request admitted for account A from spending account B's
        // bearer if the user switches accounts while the request is in flight.
        let _ = ensure_fresh_session(&self.sessions).await;
        self.service_and_actor_for_account_with_config(Some(expected_account), AuthConfig::from_env)
    }

    fn service_and_actor_with_config(
        &self,
        load_config: impl FnOnce() -> Result<AuthConfig, AuthConfigError>,
    ) -> Result<(CredentialService, CredentialActor), CredentialOperationError> {
        self.service_and_actor_for_account_with_config(None, load_config)
    }

    fn service_and_actor_for_account_with_config(
        &self,
        expected_account: Option<&ValidatedAccountId>,
        load_config: impl FnOnce() -> Result<AuthConfig, AuthConfigError>,
    ) -> Result<(CredentialService, CredentialActor), CredentialOperationError> {
        let (identity, access_token) = self.authenticated_snapshot()?;
        let account_id = canonical_account_id(&identity)
            .map_err(|_| credential_error(CredentialErrorCode::Unauthorized))?;
        if expected_account.is_some_and(|expected| expected.as_str() != account_id) {
            return Err(credential_error(CredentialErrorCode::Unauthorized));
        }
        let config =
            load_config().map_err(|_| credential_error(CredentialErrorCode::ServiceUnavailable))?;
        let repository: Arc<dyn CredentialRepository> = Arc::new(
            DaCredentialRepository::new(
                config.da_base_url.as_str(),
                access_token.as_str(),
                &identity,
                self.da_client.clone(),
            )
            .map_err(|_| credential_error(CredentialErrorCode::ServiceUnavailable))?,
        );
        let actor = CredentialActor::new(account_id, &identity.tenant_id, &identity.user_id)
            .map_err(map_service_error)?;
        Ok((
            CredentialService::with_mutation_lock(
                self.vault.clone(),
                repository,
                self.verifier.clone(),
                self.mutation_lock.clone(),
            ),
            actor,
        ))
    }

    async fn create(
        &self,
        provider: ProviderId,
        label: String,
        secret: Zeroizing<String>,
        is_default: bool,
    ) -> Result<ProviderCredentialView, CredentialOperationError> {
        let (service, actor) = self.fresh_service_and_actor().await?;
        let request = CredentialCreateRequest::new(provider, label, secret, is_default)
            .map_err(map_service_error)?;
        service
            .create(&actor, request)
            .await
            .map_err(map_service_error)
    }

    async fn replace_secret(
        &self,
        id: &str,
        secret: Zeroizing<String>,
    ) -> Result<ProviderCredentialView, CredentialOperationError> {
        let (service, actor) = self.fresh_service_and_actor().await?;
        let request = CredentialReplaceRequest::new(id, secret).map_err(map_service_error)?;
        service
            .replace_secret(&actor, request)
            .await
            .map_err(map_service_error)
    }

    async fn delete(&self, id: &str) -> Result<(), CredentialOperationError> {
        let (service, actor) = self.fresh_service_and_actor().await?;
        let request = CredentialDeleteRequest::new(id).map_err(map_service_error)?;
        service
            .delete(&actor, request)
            .await
            .map_err(map_service_error)
    }

    pub(crate) fn spawn_legacy_provider_key_migration(&self) {
        let Ok((service, actor)) = self.service_and_actor() else {
            tracing::warn!("provider-key migration could not start");
            return;
        };
        let cancellation = CancellationToken::new();
        if let Some(previous) = self
            .migration_cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(cancellation.clone())
        {
            previous.cancel();
        }
        tauri::async_runtime::spawn(async move {
            match service
                .migrate_legacy_provider_keys_until(&actor, &cancellation)
                .await
            {
                Ok(CredentialMigrationReport {
                    migrated,
                    requires_reentry,
                    failed,
                }) => tracing::info!(
                    migrated,
                    requires_reentry,
                    failed,
                    "provider-key migration finished"
                ),
                Err(error) => tracing::warn!(%error, "provider-key migration failed"),
            }
        });
    }

    pub(crate) fn cancel_legacy_provider_key_migration(&self) {
        if let Some(cancellation) = self
            .migration_cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            cancellation.cancel();
        }
    }

    /// Resolves the DA-owned secret to use for one local inference call.
    /// Internal only: never crosses a `#[tauri::command]` boundary, so failures stay
    /// typed as [`CredentialServiceError`] rather than the redacted wire error.
    pub(crate) async fn resolve_for_inference(
        &self,
        expected_account: &ValidatedAccountId,
        provider: ProviderId,
    ) -> Result<ResolvedCredential, CredentialServiceError> {
        let (service, actor) = self
            .fresh_service_and_actor_for(expected_account)
            .await
            .map_err(map_operation_error)?;
        service.resolve_for_inference(&actor, provider).await
    }

    pub(crate) async fn resolve_exact_custom_for_inference(
        &self,
        expected_account: &ValidatedAccountId,
        id: uuid::Uuid,
    ) -> Result<ResolvedCredential, CredentialServiceError> {
        let (service, actor) = self
            .fresh_service_and_actor_for(expected_account)
            .await
            .map_err(map_operation_error)?;
        service.resolve_exact_custom_for_inference(&actor, id).await
    }
}

#[tauri::command]
pub async fn credential_list(
    state: tauri::State<'_, AppRuntimeState>,
) -> Result<Vec<ProviderCredentialView>, CredentialOperationError> {
    let (service, actor) = state.credentials().fresh_service_and_actor().await?;
    service.list(&actor).await.map_err(map_service_error)
}

/// Replaces an existing key through DA's server-owned secret endpoint.
///
/// Separate command from [`credential_create`] because it is a different act on
/// a different row — see `CredentialService::replace_secret`.
#[tauri::command]
pub async fn credential_replace(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppRuntimeState>,
    id: String,
    secret: String,
) -> Result<ProviderCredentialView, CredentialOperationError> {
    let secret = Zeroizing::new(secret);
    let result = state.credentials().replace_secret(&id, secret).await;
    record_credential(
        &app,
        AuditAction::CredentialReplace,
        &id,
        emit::outcome_for(&result),
    );
    result
}

/// Deletes a stored key through DA, for every signed-in client that shares it.
#[tauri::command]
pub async fn credential_delete(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppRuntimeState>,
    id: String,
) -> Result<(), CredentialOperationError> {
    let result = state.credentials().delete(&id).await;
    record_credential(
        &app,
        AuditAction::CredentialDelete,
        &id,
        emit::outcome_for(&result),
    );
    result
}

#[tauri::command]
pub async fn credential_create(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppRuntimeState>,
    provider: ProviderId,
    label: String,
    secret: String,
    is_default: bool,
) -> Result<ProviderCredentialView, CredentialOperationError> {
    let secret = Zeroizing::new(secret);
    let result = state
        .credentials()
        .create(provider, label, secret, is_default)
        .await;
    let created_id = result
        .as_ref()
        .ok()
        .and_then(|view| credential_id_text(&view.id));
    record_credential_create(
        &app,
        provider,
        created_id.as_deref(),
        emit::outcome_for(&result),
    );
    result
}

/// Records a replace or delete of the row `id`. Label and secret never reach
/// the audit log: the helpers take only the id and the outcome.
fn record_credential<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: AuditAction,
    id: &str,
    outcome: AuditOutcome,
) {
    emit::record(app, action, AuditResourceType::Credential, id, outcome);
}

/// Records a create. The id is the new row's id on success and `new` otherwise.
fn record_credential_create<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    provider: ProviderId,
    created_id: Option<&str>,
    outcome: AuditOutcome,
) {
    emit::record_with_detail(
        app,
        AuditAction::CredentialCreate,
        AuditResourceType::Credential,
        created_id.unwrap_or("new"),
        outcome,
        provider_detail(provider),
    );
}

const fn provider_detail(provider: ProviderId) -> &'static str {
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

/// The contract keeps the id's string private, but it serializes as the bare
/// string, so the wire form is the id text.
fn credential_id_text(id: &CredentialId) -> Option<String> {
    serde_json::to_value(id).ok()?.as_str().map(str::to_owned)
}

fn credential_error(code: CredentialErrorCode) -> CredentialOperationError {
    CredentialOperationError::new(code, None)
}

fn map_service_error(error: CredentialServiceError) -> CredentialOperationError {
    match error {
        CredentialServiceError::InvalidRequest => {
            credential_error(CredentialErrorCode::InvalidRequest)
        }
        CredentialServiceError::InvalidCredential => {
            credential_error(CredentialErrorCode::InvalidCredential)
        }
        CredentialServiceError::InsufficientPermissions => {
            credential_error(CredentialErrorCode::InsufficientPermissions)
        }
        CredentialServiceError::ProviderUnavailable => {
            credential_error(CredentialErrorCode::ProviderUnavailable)
        }
        CredentialServiceError::RateLimited => credential_error(CredentialErrorCode::RateLimited),
        CredentialServiceError::StorageUnavailable => {
            credential_error(CredentialErrorCode::StorageUnavailable)
        }
        CredentialServiceError::ServiceUnavailable => {
            credential_error(CredentialErrorCode::ServiceUnavailable)
        }
        CredentialServiceError::Conflict => credential_error(CredentialErrorCode::Conflict),
        CredentialServiceError::ReconciliationRequired { credentials } => {
            CredentialOperationError::new(CredentialErrorCode::ReconciliationRequired, credentials)
        }
        CredentialServiceError::Unauthorized => credential_error(CredentialErrorCode::Unauthorized),
        CredentialServiceError::NoUsableCredential => {
            credential_error(CredentialErrorCode::ServiceUnavailable)
        }
        // No dedicated wire code: an id that names no row means the list this
        // request was built from is stale, and the reload that follows any
        // failure already resolves it. A new contract arm would describe a race
        // the user never has to act on differently.
        CredentialServiceError::NotFound => credential_error(CredentialErrorCode::InvalidRequest),
    }
}

fn map_operation_error(error: CredentialOperationError) -> CredentialServiceError {
    match error.code {
        CredentialErrorCode::InvalidRequest => CredentialServiceError::InvalidRequest,
        CredentialErrorCode::InvalidCredential => CredentialServiceError::InvalidCredential,
        CredentialErrorCode::InsufficientPermissions => {
            CredentialServiceError::InsufficientPermissions
        }
        CredentialErrorCode::ProviderUnavailable => CredentialServiceError::ProviderUnavailable,
        CredentialErrorCode::RateLimited => CredentialServiceError::RateLimited,
        CredentialErrorCode::StorageUnavailable => CredentialServiceError::StorageUnavailable,
        CredentialErrorCode::ServiceUnavailable => CredentialServiceError::ServiceUnavailable,
        CredentialErrorCode::Conflict => CredentialServiceError::Conflict,
        CredentialErrorCode::ReconciliationRequired => {
            CredentialServiceError::ReconciliationRequired {
                credentials: error.credentials,
            }
        }
        CredentialErrorCode::Unauthorized => CredentialServiceError::Unauthorized,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::persistence::InMemorySecretStore;
    use xfalcon_contracts::auth::{IdentityProjection, SessionProjection};

    #[test]
    fn unauthenticated_runtime_returns_unauthorized_before_configuration() {
        let runtime = CredentialRuntime::new(
            Arc::new(AuthSessionStore::default()),
            Arc::new(InMemorySecretStore::default()),
        );
        let configuration_called = Cell::new(false);

        let result = runtime.service_and_actor_with_config(|| {
            configuration_called.set(true);
            Err(crate::auth::AuthConfigError::NotConfigured)
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("signed-out runtime must fail before configuration"),
        };

        assert_eq!(error.code, CredentialErrorCode::Unauthorized);
        assert!(!configuration_called.get());
    }

    #[tokio::test]
    async fn unauthenticated_create_returns_unauthorized_before_request_validation() {
        let runtime = CredentialRuntime::new(
            Arc::new(AuthSessionStore::default()),
            Arc::new(InMemorySecretStore::default()),
        );

        let result = runtime
            .create(
                ProviderId::OpenAi,
                String::new(),
                Zeroizing::new("x".to_owned()),
                false,
            )
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("signed-out create must fail before request validation"),
        };

        assert_eq!(error.code, CredentialErrorCode::Unauthorized);
    }

    #[test]
    fn inference_resolution_rejects_a_different_current_account_before_configuration() {
        let sessions = Arc::new(AuthSessionStore::default());
        sessions.set_authenticated(
            SessionProjection::authenticated(
                IdentityProjection::new("user-b", "tenant-b", "User B", None),
                i64::MAX,
            ),
            "access-b",
            None,
        );
        let runtime = CredentialRuntime::new(sessions, Arc::new(InMemorySecretStore::default()));
        let expected = ValidatedAccountId::parse("tenant-a:user-a").expect("expected account");
        let configuration_called = Cell::new(false);

        let error = match runtime.service_and_actor_for_account_with_config(Some(&expected), || {
            configuration_called.set(true);
            Err(crate::auth::AuthConfigError::NotConfigured)
        }) {
            Err(error) => error,
            Ok(_) => panic!("account B must not provide a credential for account A's request"),
        };

        assert_eq!(error.code, CredentialErrorCode::Unauthorized);
        assert!(!configuration_called.get());
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use crate::audit::AuditEvent;
    use tempfile::tempdir;

    fn recorded_events(hub: &crate::audit::AuditHub) -> Vec<AuditEvent> {
        let mut events: Vec<AuditEvent> = hub
            .store()
            .recent(10)
            .expect("read audit log")
            .iter()
            .map(|record| record.event().clone())
            .collect();
        events.sort_by_key(AuditEvent::sequence);
        events
    }

    #[test]
    fn credential_writes_record_their_action_id_and_outcome_only() {
        let root = tempdir().expect("temp dir");
        let app = tauri::test::mock_app();
        let hub = emit::test_hub(app.handle(), root.path());

        record_credential_create(
            app.handle(),
            ProviderId::Anthropic,
            Some("cred-new"),
            AuditOutcome::Success,
        );
        record_credential_create(
            app.handle(),
            ProviderId::CustomOpenAi,
            None,
            AuditOutcome::Failure,
        );
        record_credential(
            app.handle(),
            AuditAction::CredentialReplace,
            "cred-1",
            AuditOutcome::Success,
        );
        record_credential(
            app.handle(),
            AuditAction::CredentialDelete,
            "cred-1",
            AuditOutcome::Failure,
        );

        let events = recorded_events(&hub);
        let summary: Vec<_> = events
            .iter()
            .map(|event| {
                (
                    event.action(),
                    event.resource_type(),
                    event.resource_id().to_owned(),
                    event.outcome(),
                    event.detail().map(str::to_owned),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    AuditAction::CredentialCreate,
                    AuditResourceType::Credential,
                    "cred-new".to_owned(),
                    AuditOutcome::Success,
                    Some("anthropic".to_owned()),
                ),
                (
                    AuditAction::CredentialCreate,
                    AuditResourceType::Credential,
                    "new".to_owned(),
                    AuditOutcome::Failure,
                    Some("custom_openai".to_owned()),
                ),
                (
                    AuditAction::CredentialReplace,
                    AuditResourceType::Credential,
                    "cred-1".to_owned(),
                    AuditOutcome::Success,
                    None,
                ),
                (
                    AuditAction::CredentialDelete,
                    AuditResourceType::Credential,
                    "cred-1".to_owned(),
                    AuditOutcome::Failure,
                    None,
                ),
            ]
        );
    }

    #[test]
    fn credential_id_text_reads_the_wire_string() {
        assert_eq!(
            credential_id_text(&CredentialId::from("cred-7")),
            Some("cred-7".to_owned())
        );
    }

    #[test]
    fn provider_detail_matches_the_wire_provider_names() {
        for provider in [
            ProviderId::OpenAi,
            ProviderId::Anthropic,
            ProviderId::Google,
            ProviderId::OpenRouter,
            ProviderId::CustomOpenAi,
        ] {
            let wire = serde_json::to_value(provider).expect("serialize provider");
            assert_eq!(wire.as_str(), Some(provider_detail(provider)));
        }
    }
}
