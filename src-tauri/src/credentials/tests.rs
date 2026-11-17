use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use indexmap::IndexMap;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::credentials::{CredentialStatus, ProviderId};
use zeroize::Zeroizing;

use super::{
    envelope::{decrypt, encrypt, CredentialContext, EnvelopeError, ENVELOPE_PREFIX},
    service::{
        CredentialActor, CredentialCreateRequest, CredentialDeleteRequest,
        CredentialMigrationReport, CredentialRecord, CredentialReplaceRequest,
        CredentialRepository, CredentialRepositoryError, CredentialService, CredentialServiceError,
        NewCredential, ReplacementCredential, ResolvedCredential,
    },
    vault::{CredentialVault, VaultError},
    verifier::{CredentialVerifier, VerificationError, VerifiedCredential},
};
use crate::persistence::{InMemorySecretStore, SecretStore};

const KEY: [u8; 32] = [0x42; 32];
const SECRET_CANARY: &str = "sk-secret-credential-canary";

fn context(provider: ProviderId) -> CredentialContext {
    CredentialContext::new("tenant-1", "user-1", provider, "device-key-1").expect("valid context")
}

fn envelope_payload(serialized: &str) -> Value {
    let encoded = serialized
        .strip_prefix(ENVELOPE_PREFIX)
        .expect("desktop envelope prefix");
    let bytes = URL_SAFE_NO_PAD.decode(encoded).expect("base64url payload");
    serde_json::from_slice(&bytes).expect("JSON payload")
}

#[test]
fn xfdc_encryption_uses_unique_nonce_and_envelope_id() {
    let first = encrypt(
        Zeroizing::new(SECRET_CANARY.to_owned()),
        &KEY,
        1,
        &context(ProviderId::OpenAi),
    )
    .expect("first encryption");
    let second = encrypt(
        Zeroizing::new(SECRET_CANARY.to_owned()),
        &KEY,
        1,
        &context(ProviderId::OpenAi),
    )
    .expect("second encryption");

    let first_payload = envelope_payload(first.as_str());
    let second_payload = envelope_payload(second.as_str());
    assert_ne!(first_payload["nonce"], second_payload["nonce"]);
    assert_ne!(first_payload["envelope_id"], second_payload["envelope_id"]);
    assert_ne!(first.as_str(), second.as_str());
}

#[test]
fn xfdc_round_trip_binds_every_authenticated_context_field() {
    let encrypted = encrypt(
        Zeroizing::new(SECRET_CANARY.to_owned()),
        &KEY,
        7,
        &context(ProviderId::Anthropic),
    )
    .expect("encrypt");
    let opened = decrypt(encrypted.as_str(), &KEY, &context(ProviderId::Anthropic))
        .expect("matching context decrypts");
    assert_eq!(opened.as_str(), SECRET_CANARY);

    for wrong in [
        CredentialContext::new(
            "tenant-other",
            "user-1",
            ProviderId::Anthropic,
            "device-key-1",
        )
        .unwrap(),
        CredentialContext::new(
            "tenant-1",
            "user-other",
            ProviderId::Anthropic,
            "device-key-1",
        )
        .unwrap(),
        context(ProviderId::Google),
        CredentialContext::new(
            "tenant-1",
            "user-1",
            ProviderId::Anthropic,
            "device-key-other",
        )
        .unwrap(),
    ] {
        assert_eq!(
            decrypt(encrypted.as_str(), &KEY, &wrong),
            Err(EnvelopeError::AuthenticationFailed)
        );
    }
}

#[test]
fn xfdc_rejects_tamper_foreign_formats_and_malformed_payloads() {
    let encrypted = encrypt(
        Zeroizing::new(SECRET_CANARY.to_owned()),
        &KEY,
        1,
        &context(ProviderId::Google),
    )
    .expect("encrypt");
    let mut payload = envelope_payload(encrypted.as_str());
    payload["ciphertext"] = Value::String(URL_SAFE_NO_PAD.encode(b"tampered"));
    let tampered = format!(
        "{ENVELOPE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    );

    assert_eq!(
        decrypt(&tampered, &KEY, &context(ProviderId::Google)),
        Err(EnvelopeError::AuthenticationFailed)
    );
    for foreign in [
        SECRET_CANARY,
        "enc:v1:nonce:tag:ciphertext",
        "xfdc:v2:future",
        "xfdc:v1:not-base64url!",
    ] {
        assert_eq!(
            decrypt(foreign, &KEY, &context(ProviderId::Google)),
            Err(EnvelopeError::UnsupportedEnvelope)
        );
    }
}

#[test]
fn encrypted_credential_debug_never_exposes_secret_class_material() {
    let encrypted = encrypt(
        Zeroizing::new(SECRET_CANARY.to_owned()),
        &KEY,
        1,
        &context(ProviderId::OpenAi),
    )
    .expect("encrypt");
    let debug = format!("{encrypted:?}");

    assert_eq!(debug, "EncryptedCredential([REDACTED])");
    assert!(!debug.contains(SECRET_CANARY));
    assert!(!debug.contains(encrypted.as_str()));
}

#[test]
fn credential_vault_creates_once_then_fails_closed_without_the_device_key() {
    let store = Arc::new(InMemorySecretStore::default());
    let vault = CredentialVault::new(store.clone());
    let sealed = vault
        .seal(
            "account-1",
            "tenant-1",
            "user-1",
            ProviderId::OpenAi,
            Zeroizing::new(SECRET_CANARY.to_owned()),
        )
        .expect("vault seals");

    assert!(store.contains_key_for_test("account-1", 1));
    assert_eq!(
        vault
            .open(
                "account-1",
                "tenant-1",
                "user-1",
                ProviderId::OpenAi,
                sealed.as_str(),
            )
            .expect("vault opens")
            .as_str(),
        SECRET_CANARY
    );

    store
        .delete_account_keys("account-1")
        .expect("simulate missing or retired device key");
    assert_eq!(
        vault.open(
            "account-1",
            "tenant-1",
            "user-1",
            ProviderId::OpenAi,
            sealed.as_str(),
        ),
        Err(VaultError::DeviceKeyUnavailable)
    );
}

#[derive(Default)]
struct CapturingRepository {
    created: Mutex<Vec<CapturedCreate>>,
    replaced: Mutex<Vec<CapturedReplace>>,
    replace_failures: Mutex<Vec<Uuid>>,
    resolved: Mutex<Vec<(Uuid, ProviderId)>>,
    deleted: Mutex<Vec<Uuid>>,
    listed: Mutex<Vec<CredentialRecord>>,
}

struct CapturedCreate {
    secret: String,
}

struct CapturedReplace {
    id: Uuid,
    provider: ProviderId,
    secret: String,
}

#[derive(Default)]
struct AmbiguousCreateRepository {
    list_calls: Mutex<usize>,
}

#[async_trait]
impl CredentialRepository for AmbiguousCreateRepository {
    async fn list(&self) -> Result<Vec<CredentialRecord>, CredentialRepositoryError> {
        *self.list_calls.lock().expect("list calls lock") += 1;
        Ok(Vec::new())
    }

    async fn create(
        &self,
        _credential: NewCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError> {
        Err(CredentialRepositoryError::ReconciliationRequired)
    }

    async fn replace_secret(
        &self,
        _credential: ReplacementCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError> {
        Err(CredentialRepositoryError::ReconciliationRequired)
    }

    async fn resolve_secret(
        &self,
        _id: Uuid,
        _provider: ProviderId,
    ) -> Result<ResolvedCredential, CredentialRepositoryError> {
        Err(CredentialRepositoryError::Unavailable)
    }

    async fn delete(&self, _id: Uuid) -> Result<(), CredentialRepositoryError> {
        Err(CredentialRepositoryError::Unavailable)
    }
}

#[async_trait]
impl CredentialRepository for CapturingRepository {
    async fn list(&self) -> Result<Vec<CredentialRecord>, CredentialRepositoryError> {
        Ok(self
            .listed
            .lock()
            .expect("listed lock")
            .iter()
            .map(CredentialRecord::duplicate_for_test)
            .collect())
    }

    async fn create(
        &self,
        credential: NewCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError> {
        let (provider, label, secret, is_default) = credential.into_parts();
        self.created
            .lock()
            .expect("created lock")
            .push(CapturedCreate {
                secret: secret.as_str().to_owned(),
            });
        Ok(CredentialRecord::new(
            Uuid::from_u128(1),
            provider,
            label,
            "xfda:v1:server-envelope",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            is_default,
            true,
        ))
    }

    async fn replace_secret(
        &self,
        credential: ReplacementCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError> {
        let ReplacementCredential {
            id,
            provider,
            secret,
        } = credential;
        self.replaced
            .lock()
            .expect("replaced lock")
            .push(CapturedReplace {
                id,
                provider,
                secret: secret.as_str().to_owned(),
            });
        if self
            .replace_failures
            .lock()
            .expect("replace failures lock")
            .contains(&id)
        {
            return Err(CredentialRepositoryError::Unavailable);
        }
        Ok(CredentialRecord::new(
            id,
            provider,
            "Existing label",
            "xfda:v1:server-envelope",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            true,
            true,
        ))
    }

    async fn resolve_secret(
        &self,
        id: Uuid,
        provider: ProviderId,
    ) -> Result<ResolvedCredential, CredentialRepositoryError> {
        self.resolved
            .lock()
            .expect("resolved lock")
            .push((id, provider));
        Ok(ResolvedCredential {
            credential_id: id.to_string(),
            secret: Zeroizing::new(SECRET_CANARY.to_owned()),
            base_url: None,
        })
    }

    async fn delete(&self, id: Uuid) -> Result<(), CredentialRepositoryError> {
        self.deleted.lock().expect("deleted lock").push(id);
        self.listed
            .lock()
            .expect("listed lock")
            .retain(|record| record.to_view(None).id != id.to_string().into());
        Ok(())
    }
}

#[derive(Default)]
struct AcceptingVerifier {
    calls: Mutex<Vec<ProviderId>>,
}

#[async_trait]
impl CredentialVerifier for AcceptingVerifier {
    async fn verify(
        &self,
        provider: ProviderId,
        _secret: &str,
    ) -> Result<VerifiedCredential, VerificationError> {
        self.calls.lock().expect("verifier lock").push(provider);
        Ok(VerifiedCredential)
    }
}

fn authenticated_actor() -> CredentialActor {
    CredentialActor::new("account-1", "tenant-1", "user-1").expect("authenticated actor")
}

#[tokio::test]
async fn credential_create_verifies_then_sends_cleartext_only_to_da() {
    let store = Arc::new(InMemorySecretStore::default());
    let repository = Arc::new(CapturingRepository::default());
    let verifier = Arc::new(AcceptingVerifier::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(store)),
        repository.clone(),
        verifier.clone(),
    );

    let view = service
        .create(
            &authenticated_actor(),
            CredentialCreateRequest::new(
                ProviderId::Anthropic,
                "Work key",
                Zeroizing::new(SECRET_CANARY.to_owned()),
                true,
            )
            .expect("valid request"),
        )
        .await
        .expect("create succeeds");

    assert_eq!(view.provider, ProviderId::Anthropic);
    assert_eq!(view.status, CredentialStatus::Ready);
    assert_eq!(
        verifier.calls.lock().expect("verifier lock").as_slice(),
        [ProviderId::Anthropic]
    );
    let created = repository.created.lock().expect("created lock");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].secret, SECRET_CANARY);
}

#[tokio::test]
async fn authenticated_actor_can_create_without_a_desktop_step_up() {
    let repository = Arc::new(CapturingRepository::default());
    let verifier = Arc::new(AcceptingVerifier::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        verifier.clone(),
    );
    let actor =
        CredentialActor::new("account-1", "tenant-1", "user-1").expect("authenticated actor");

    let view = service
        .create(
            &actor,
            CredentialCreateRequest::new(
                ProviderId::Google,
                "Current session",
                Zeroizing::new(SECRET_CANARY.to_owned()),
                false,
            )
            .unwrap(),
        )
        .await
        .expect("current authenticated session can create");

    assert_eq!(view.provider, ProviderId::Google);
    assert_eq!(
        verifier.calls.lock().expect("verifier lock").as_slice(),
        [ProviderId::Google]
    );
    assert_eq!(repository.created.lock().expect("created lock").len(), 1);
}

#[test]
fn credential_actor_rejects_invalid_authenticated_identity() {
    assert!(matches!(
        CredentialActor::new("tenant-1:user-1", "", "user-1"),
        Err(CredentialServiceError::InvalidRequest)
    ));
}

#[tokio::test]
async fn ambiguous_create_refreshes_the_list_once_and_never_blindly_retries() {
    let repository = Arc::new(AmbiguousCreateRepository::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );

    let result = service
        .create(
            &authenticated_actor(),
            CredentialCreateRequest::new(
                ProviderId::OpenAi,
                "Ambiguous",
                Zeroizing::new(SECRET_CANARY.to_owned()),
                false,
            )
            .unwrap(),
        )
        .await;

    assert_eq!(
        result,
        Err(CredentialServiceError::ReconciliationRequired {
            credentials: Some(Vec::new()),
        })
    );
    assert_eq!(*repository.list_calls.lock().expect("list calls lock"), 1);
}

#[tokio::test]
async fn list_projects_foreign_and_legacy_rows_as_needing_reentry_without_ciphertext() {
    let repository = Arc::new(CapturingRepository::default());
    repository.listed.lock().expect("listed lock").extend([
        CredentialRecord::new(
            Uuid::new_v4(),
            ProviderId::OpenAi,
            "Legacy",
            SECRET_CANARY,
            IndexMap::new(),
            false,
            true,
        ),
        CredentialRecord::new(
            Uuid::new_v4(),
            ProviderId::Google,
            "Web",
            "enc:v1:nonce:tag:ciphertext",
            IndexMap::new(),
            false,
            true,
        ),
        CredentialRecord::new(
            Uuid::new_v4(),
            ProviderId::Anthropic,
            "Untagged desktop envelope",
            "xfdc:v1:untagged",
            IndexMap::new(),
            false,
            true,
        ),
        CredentialRecord::new(
            Uuid::new_v4(),
            ProviderId::OpenRouter,
            "Forged server marker",
            "enc:v1:client-ciphertext",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            false,
            true,
        ),
    ]);
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository,
        Arc::new(AcceptingVerifier::default()),
    );

    let views = service
        .list(&authenticated_actor())
        .await
        .expect("list projects");
    assert_eq!(views.len(), 4);
    assert!(views
        .iter()
        .all(|view| view.status == CredentialStatus::NeedsReentry));
    let wire = serde_json::to_string(&views).expect("views serialize");
    assert!(!wire.contains(SECRET_CANARY));
    assert!(!wire.contains("enc:v1"));
}

#[tokio::test]
async fn list_projects_da_owned_rows_as_ready_without_a_device_key() {
    let repository = Arc::new(CapturingRepository::default());
    repository
        .listed
        .lock()
        .expect("listed lock")
        .push(CredentialRecord::new(
            Uuid::from_u128(6),
            ProviderId::OpenAi,
            "Shared key",
            "xfda:v1:server-envelope",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            true,
            true,
        ));
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository,
        Arc::new(AcceptingVerifier::default()),
    );

    let views = service
        .list(&authenticated_actor())
        .await
        .expect("list succeeds");

    assert_eq!(views.len(), 1);
    assert_eq!(views[0].status, CredentialStatus::Ready);
}

/// A legacy row this device cannot migrate lists as `NeedsReentry`. Pasting the
/// key again must make *that row* DA-owned and usable — same id, same default
/// flag — rather than leaving it behind.
#[tokio::test]
async fn re_entering_a_key_replaces_the_same_row_through_da() {
    let foreign = CredentialRecord::new(
        Uuid::from_u128(7),
        ProviderId::OpenRouter,
        "Team OpenRouter",
        "sealed-by-another-device",
        IndexMap::new(),
        true,
        true,
    );
    let repository = Arc::new(CapturingRepository::default());
    *repository.listed.lock().expect("listed lock") = vec![foreign];
    let verifier = Arc::new(AcceptingVerifier::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        verifier.clone(),
    );

    let view = service
        .replace_secret(
            &authenticated_actor(),
            CredentialReplaceRequest::new(
                &Uuid::from_u128(7).to_string(),
                Zeroizing::new(SECRET_CANARY.to_owned()),
            )
            .expect("valid request"),
        )
        .await
        .expect("re-entry succeeds");

    assert_eq!(view.status, CredentialStatus::Ready);
    assert!(view.is_default, "the row's default flag is left alone");

    // The provider comes from the stored row, so verification went to
    // OpenRouter's origin rather than one a caller could have named.
    assert_eq!(
        verifier.calls.lock().expect("verifier lock").as_slice(),
        [ProviderId::OpenRouter]
    );

    let replaced = repository.replaced.lock().expect("replaced lock");
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].id, Uuid::from_u128(7));
    assert_eq!(replaced[0].provider, ProviderId::OpenRouter);
    assert_eq!(replaced[0].secret, SECRET_CANARY);
    assert!(
        repository.created.lock().expect("created lock").is_empty(),
        "re-entry must not add a second row for the same key"
    );
}

/// Removing a key deletes that one row through DA, and the next list no longer
/// shows it.
#[tokio::test]
async fn removing_a_key_deletes_that_row_through_da() {
    let listed = |id: u128| {
        CredentialRecord::new(
            Uuid::from_u128(id),
            ProviderId::OpenAi,
            format!("openai_{id}"),
            "xfda:v1:server-envelope",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            id == 1,
            true,
        )
    };
    let repository = Arc::new(CapturingRepository::default());
    *repository.listed.lock().expect("listed lock") = vec![listed(1), listed(2)];
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );

    service
        .delete(
            &authenticated_actor(),
            CredentialDeleteRequest::new(&Uuid::from_u128(2).to_string()).expect("valid request"),
        )
        .await
        .expect("delete succeeds");

    assert_eq!(
        repository.deleted.lock().expect("deleted lock").as_slice(),
        [Uuid::from_u128(2)]
    );
    let views = service
        .list(&authenticated_actor())
        .await
        .expect("list succeeds");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].id, Uuid::from_u128(1).to_string().into());
}

/// A key the fresh list no longer holds is already removed: the request
/// succeeds, and DA is asked to delete nothing, so a stale settings screen
/// cannot remove some other row.
#[tokio::test]
async fn removing_a_key_that_is_already_gone_succeeds_and_deletes_nothing() {
    let repository = Arc::new(CapturingRepository::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );

    let outcome = service
        .delete(
            &authenticated_actor(),
            CredentialDeleteRequest::new(&Uuid::from_u128(99).to_string()).expect("valid request"),
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert!(repository.deleted.lock().expect("deleted lock").is_empty());
}

#[test]
fn a_delete_request_must_name_a_row_id() {
    assert!(matches!(
        CredentialDeleteRequest::new("not-a-uuid"),
        Err(CredentialServiceError::InvalidRequest)
    ));
}

/// A row the list does not contain cannot be re-sealed, and nothing may be sent
/// to a provider on its behalf.
#[tokio::test]
async fn re_entering_an_unknown_row_fails_without_verifying_anything() {
    let repository = Arc::new(CapturingRepository::default());
    let verifier = Arc::new(AcceptingVerifier::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        verifier.clone(),
    );

    let outcome = service
        .replace_secret(
            &authenticated_actor(),
            CredentialReplaceRequest::new(
                &Uuid::from_u128(99).to_string(),
                Zeroizing::new(SECRET_CANARY.to_owned()),
            )
            .expect("valid request"),
        )
        .await;

    assert_eq!(outcome.err(), Some(CredentialServiceError::NotFound));
    assert!(verifier.calls.lock().expect("verifier lock").is_empty());
    assert!(repository
        .replaced
        .lock()
        .expect("replaced lock")
        .is_empty());
}

/// A key the provider rejects must never displace a stored one: the row keeps
/// whatever it held, so a bad paste cannot make a working key unusable.
#[tokio::test]
async fn a_rejected_key_is_not_written_over_the_stored_one() {
    struct RejectingVerifier;

    #[async_trait]
    impl CredentialVerifier for RejectingVerifier {
        async fn verify(
            &self,
            _provider: ProviderId,
            _secret: &str,
        ) -> Result<VerifiedCredential, VerificationError> {
            Err(VerificationError::InvalidCredential)
        }
    }

    let repository = Arc::new(CapturingRepository::default());
    *repository.listed.lock().expect("listed lock") = vec![CredentialRecord::new(
        Uuid::from_u128(5),
        ProviderId::OpenAi,
        "Work key",
        "sealed-by-another-device",
        IndexMap::new(),
        false,
        true,
    )];
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        Arc::new(RejectingVerifier),
    );

    let outcome = service
        .replace_secret(
            &authenticated_actor(),
            CredentialReplaceRequest::new(
                &Uuid::from_u128(5).to_string(),
                Zeroizing::new(SECRET_CANARY.to_owned()),
            )
            .expect("valid request"),
        )
        .await;

    assert_eq!(
        outcome.err(),
        Some(CredentialServiceError::InvalidCredential)
    );
    assert!(repository
        .replaced
        .lock()
        .expect("replaced lock")
        .is_empty());
}

#[tokio::test]
async fn inference_prefers_the_default_da_owned_row_and_resolves_only_that_row() {
    let repository = Arc::new(CapturingRepository::default());
    *repository.listed.lock().expect("listed lock") = vec![
        CredentialRecord::new(
            Uuid::from_u128(10),
            ProviderId::OpenAi,
            "Secondary",
            "xfda:v1:secondary",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            false,
            true,
        ),
        CredentialRecord::new(
            Uuid::from_u128(11),
            ProviderId::OpenAi,
            "Default",
            "xfda:v1:default",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            true,
            true,
        ),
        CredentialRecord::new(
            Uuid::from_u128(12),
            ProviderId::OpenAi,
            "Disabled",
            "xfda:v1:disabled",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            true,
            false,
        ),
        CredentialRecord::new(
            Uuid::from_u128(13),
            ProviderId::OpenAi,
            "Foreign default",
            "enc:v1:client-ciphertext",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            true,
            true,
        ),
        CredentialRecord::new(
            Uuid::from_u128(14),
            ProviderId::OpenAi,
            "Older default",
            "xfda:v1:older-default",
            IndexMap::from([(
                "secret_owner".to_owned(),
                Value::String("data-analyst".to_owned()),
            )]),
            true,
            true,
        ),
    ];
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );

    let resolved = service
        .resolve_for_inference(&authenticated_actor(), ProviderId::OpenAi)
        .await
        .expect("default resolves");

    assert_eq!(resolved.credential_id, Uuid::from_u128(11).to_string());
    assert_eq!(
        repository
            .resolved
            .lock()
            .expect("resolved lock")
            .as_slice(),
        [(Uuid::from_u128(11), ProviderId::OpenAi)]
    );
    assert!(repository
        .replaced
        .lock()
        .expect("replaced lock")
        .is_empty());
}

#[tokio::test]
async fn inference_with_no_compatible_row_reports_no_usable_credential() {
    let repository = Arc::new(CapturingRepository::default());
    let service = CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        repository,
        Arc::new(AcceptingVerifier::default()),
    );

    assert_eq!(
        service
            .resolve_for_inference(&authenticated_actor(), ProviderId::OpenRouter)
            .await
            .err(),
        Some(CredentialServiceError::NoUsableCredential)
    );
}

#[tokio::test]
async fn migration_reseals_readable_legacy_rows_and_isolates_row_failures() {
    let store = Arc::new(InMemorySecretStore::default());
    let vault = Arc::new(CredentialVault::new(store));
    let actor = authenticated_actor();
    let mut legacy_rows = Vec::new();
    for (id, provider) in [
        (Uuid::from_u128(20), ProviderId::Anthropic),
        (Uuid::from_u128(21), ProviderId::Google),
    ] {
        let sealed = vault
            .seal(
                "account-1",
                "tenant-1",
                "user-1",
                provider,
                Zeroizing::new(SECRET_CANARY.to_owned()),
            )
            .expect("legacy seal");
        let (ciphertext, device_key_id) = sealed.into_parts();
        legacy_rows.push(CredentialRecord::new(
            id,
            provider,
            "Legacy",
            ciphertext.as_str(),
            IndexMap::from([("device_key_id".to_owned(), Value::String(device_key_id))]),
            false,
            true,
        ));
    }
    legacy_rows.push(CredentialRecord::new(
        Uuid::from_u128(22),
        ProviderId::OpenRouter,
        "Other device",
        "xfdc:v1:foreign",
        IndexMap::from([(
            "device_key_id".to_owned(),
            Value::String("dk-other".to_owned()),
        )]),
        false,
        true,
    ));
    legacy_rows.push(CredentialRecord::new(
        Uuid::from_u128(23),
        ProviderId::OpenAi,
        "Already migrated",
        "xfda:v1:server-envelope",
        IndexMap::from([(
            "secret_owner".to_owned(),
            Value::String("data-analyst".to_owned()),
        )]),
        true,
        true,
    ));

    let repository = Arc::new(CapturingRepository::default());
    *repository.listed.lock().expect("listed lock") = legacy_rows;
    repository
        .replace_failures
        .lock()
        .expect("replace failures lock")
        .push(Uuid::from_u128(21));
    let service = CredentialService::new(
        vault,
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );

    assert_eq!(
        service
            .migrate_legacy_provider_keys(&actor)
            .await
            .expect("migration completes"),
        CredentialMigrationReport {
            migrated: 1,
            requires_reentry: 1,
            failed: 1,
        }
    );
    let replaced = repository.replaced.lock().expect("replaced lock");
    assert_eq!(replaced.len(), 2);
    assert!(replaced.iter().all(|row| row.secret == SECRET_CANARY));
    assert!(replaced.iter().all(|row| row.id != Uuid::from_u128(23)));
}

#[tokio::test]
async fn cancelled_migration_does_not_open_or_replace_any_legacy_row() {
    let store = Arc::new(InMemorySecretStore::default());
    let vault = Arc::new(CredentialVault::new(store));
    let actor = authenticated_actor();
    let sealed = vault
        .seal(
            "account-1",
            "tenant-1",
            "user-1",
            ProviderId::OpenAi,
            Zeroizing::new(SECRET_CANARY.to_owned()),
        )
        .expect("legacy seal");
    let (ciphertext, device_key_id) = sealed.into_parts();
    let repository = Arc::new(CapturingRepository::default());
    repository
        .listed
        .lock()
        .expect("listed lock")
        .push(CredentialRecord::new(
            Uuid::from_u128(24),
            ProviderId::OpenAi,
            "Legacy",
            ciphertext.as_str(),
            IndexMap::from([("device_key_id".to_owned(), Value::String(device_key_id))]),
            true,
            true,
        ));
    let service = CredentialService::new(
        vault,
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    assert_eq!(
        service
            .migrate_legacy_provider_keys_until(&actor, &cancellation)
            .await
            .expect("cancellation is a clean stop"),
        CredentialMigrationReport::default()
    );
    assert!(repository
        .replaced
        .lock()
        .expect("replaced lock")
        .is_empty());
}

#[tokio::test]
async fn inference_migrates_a_readable_legacy_row_before_resolving_it() {
    let store = Arc::new(InMemorySecretStore::default());
    let vault = Arc::new(CredentialVault::new(store));
    let actor = authenticated_actor();
    let sealed = vault
        .seal(
            "account-1",
            "tenant-1",
            "user-1",
            ProviderId::OpenRouter,
            Zeroizing::new(SECRET_CANARY.to_owned()),
        )
        .expect("legacy seal");
    let (ciphertext, device_key_id) = sealed.into_parts();
    let repository = Arc::new(CapturingRepository::default());
    repository
        .listed
        .lock()
        .expect("listed lock")
        .push(CredentialRecord::new(
            Uuid::from_u128(30),
            ProviderId::OpenRouter,
            "Legacy default",
            ciphertext.as_str(),
            IndexMap::from([("device_key_id".to_owned(), Value::String(device_key_id))]),
            true,
            true,
        ));
    let service = CredentialService::new(
        vault,
        repository.clone(),
        Arc::new(AcceptingVerifier::default()),
    );

    let resolved = service
        .resolve_for_inference(&actor, ProviderId::OpenRouter)
        .await
        .expect("legacy row migrates and resolves");

    assert_eq!(resolved.credential_id, Uuid::from_u128(30).to_string());
    assert_eq!(repository.replaced.lock().expect("replaced lock").len(), 1);
    assert_eq!(repository.resolved.lock().expect("resolved lock").len(), 1);
}

#[test]
fn a_replace_request_rejects_an_id_that_is_not_a_row_id() {
    for id in ["", "not-a-uuid", "../../etc/passwd", &"9".repeat(200)] {
        assert_eq!(
            CredentialReplaceRequest::new(id, Zeroizing::new(SECRET_CANARY.to_owned())).err(),
            Some(CredentialServiceError::InvalidRequest),
            "{id:?} must not reach the repository"
        );
    }
}
