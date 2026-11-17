use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use zeroize::Zeroizing;

use crate::domain::error::{AppError, AppResult};

pub(crate) const AAD_SCHEMA_VERSION: u32 = 1;

pub(crate) struct EncryptedPayload {
    pub aad_schema_version: u32,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub(crate) struct AuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub event_id: &'a str,
    pub run_id: &'a str,
    pub sequence: u64,
    pub event_type: &'a str,
    pub state: &'a str,
    pub created_at_ms: i64,
}

pub(crate) struct ProjectionAuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub conversation_id: &'a str,
    pub entity_kind: &'a str,
    pub entity_id: &'a str,
    pub sequence: u64,
}

/// AAD context for one sealed `trusted_projects` row (see
/// `crate::projects::TrustStore`). Distinct domain separator
/// (`xfalcon-project-trust-v1`) from [`ProjectionAuthenticatedMetadata`]'s
/// `xfalcon-chat-projection-v1`: reusing another context's authenticated-data
/// domain tag would let a sealed row from one table be silently accepted as
/// if it belonged to the other, which is exactly the domain-confusion class
/// of bug versioned/tagged AAD exists to rule out.
pub(crate) struct ProjectTrustAuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub project_id: &'a str,
}

/// AAD context for one sealed `project_index_files` row (see
/// `crate::projects::index::IndexStore`). Distinct domain separator
/// (`xfalcon-project-index-v1`) from both [`ProjectTrustAuthenticatedMetadata`]'s
/// `xfalcon-project-trust-v1` and [`ProjectionAuthenticatedMetadata`]'s
/// `xfalcon-chat-projection-v1` -- same domain-confusion rationale as those
/// two: a sealed index row must never be silently accepted as a trust row or
/// a chat-projection row, or vice versa. `path_key` (the row's own plaintext
/// lookup key -- see the migration's `path_key BLOB` column) is additionally
/// folded into this AAD so a ciphertext cannot be swapped between two rows
/// of the *same* project that happen to share an account/project id (the
/// `trusted_projects` table has no per-row secondary identifier to bind to
/// this way, since it only ever holds one row per project; this table holds
/// many rows per project, so the extra binding matters here).
pub(crate) struct ProjectIndexAuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub project_id: &'a str,
    pub path_key: &'a [u8],
}

pub(crate) struct BackupAuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub project_id: &'a str,
    pub path_key: &'a [u8],
    pub backup_id: &'a str,
}

/// AAD context for one sealed `project_mutation_approvals` row (see
/// `crate::approvals::service::ApprovalStore`). Distinct domain separator
/// (`xfalcon-project-approval-v1`) from every other sealed-row context above
/// -- same domain-confusion rationale (a sealed approval-request row must
/// never be silently accepted as a trust row, an index row, or a
/// chat-projection row, or vice versa). Task 5 used
/// `xfalcon-project-index-v1` for its own table; this is a new, similarly
/// distinct tag, not a reuse. `plan_id` (the row's own plaintext lookup key
/// -- the `MutationPlan`'s content-addressed BLAKE3 id, hex-encoded) is
/// folded into this AAD for the same reason `ProjectIndexAuthenticatedMetadata`
/// folds in `path_key`: this table holds many rows per project, so the extra
/// binding stops a ciphertext being swapped between two approval rows that
/// happen to share an account/project id.
pub(crate) struct ApprovalAuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub project_id: &'a str,
    pub plan_id: &'a str,
}

/// AAD context for one sealed ChatGPT plan session file (see
/// `crate::providers::chatgpt_plan::store`). Distinct domain separator
/// (`xfalcon-chatgpt-plan-session-v1`) from every other sealed-row context
/// above, so a session file can never be accepted as an approval, trust, or
/// chat row, or the reverse. `client_id` is the issued OAuth client id that
/// names the file, folded in so one account's session cannot be moved onto
/// another client's file within the same account.
pub(crate) struct PlanSessionAuthenticatedMetadata<'a> {
    pub account_id: &'a str,
    pub client_id: &'a str,
}

pub(crate) fn encrypt(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &AuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &journal_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &AuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &journal_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

pub(crate) fn encrypt_projection(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &ProjectionAuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &projection_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt_projection(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &ProjectionAuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &projection_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

pub(crate) fn encrypt_project_trust(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &ProjectTrustAuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &project_trust_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt_project_trust(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &ProjectTrustAuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &project_trust_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

pub(crate) fn encrypt_project_index(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &ProjectIndexAuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &project_index_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt_project_index(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &ProjectIndexAuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &project_index_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

pub(crate) fn encrypt_backup(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &BackupAuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &backup_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt_backup(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &BackupAuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &backup_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

pub(crate) fn encrypt_approval(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &ApprovalAuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &approval_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt_approval(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &ApprovalAuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &approval_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

pub(crate) fn encrypt_plan_session(
    key: &Zeroizing<[u8; 32]>,
    key_version: u32,
    metadata: &PlanSessionAuthenticatedMetadata<'_>,
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    encrypt_with_aad(
        key,
        &plan_session_authenticated_data(AAD_SCHEMA_VERSION, key_version, metadata),
        plaintext,
    )
}

pub(crate) fn decrypt_plan_session(
    key: &Zeroizing<[u8; 32]>,
    aad_schema_version: u32,
    key_version: u32,
    nonce: &[u8],
    metadata: &PlanSessionAuthenticatedMetadata<'_>,
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    if aad_schema_version != AAD_SCHEMA_VERSION || nonce.len() != 12 {
        return Err(AppError::PersistenceDecryptionFailed);
    }
    decrypt_with_aad(
        key,
        nonce,
        &plan_session_authenticated_data(aad_schema_version, key_version, metadata),
        ciphertext,
    )
}

fn encrypt_with_aad(
    key: &Zeroizing<[u8; 32]>,
    aad: &[u8],
    plaintext: &[u8],
) -> AppResult<EncryptedPayload> {
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| AppError::PersistenceEncryptionFailed)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| AppError::PersistenceEncryptionFailed)?;

    Ok(EncryptedPayload {
        aad_schema_version: AAD_SCHEMA_VERSION,
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn decrypt_with_aad(
    key: &Zeroizing<[u8; 32]>,
    nonce: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> AppResult<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| AppError::PersistenceDecryptionFailed)?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| AppError::PersistenceDecryptionFailed)
}

fn journal_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &AuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        32 + metadata.account_id.len()
            + metadata.event_id.len()
            + metadata.run_id.len()
            + metadata.event_type.len(),
    );
    aad.extend_from_slice(b"xfalcon-journal-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.event_id.as_bytes());
    append_field(&mut aad, metadata.run_id.as_bytes());
    aad.extend_from_slice(&metadata.sequence.to_be_bytes());
    append_field(&mut aad, metadata.event_type.as_bytes());
    append_field(&mut aad, metadata.state.as_bytes());
    aad.extend_from_slice(&metadata.created_at_ms.to_be_bytes());
    aad
}

fn projection_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &ProjectionAuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        40 + metadata.account_id.len()
            + metadata.conversation_id.len()
            + metadata.entity_kind.len()
            + metadata.entity_id.len(),
    );
    aad.extend_from_slice(b"xfalcon-chat-projection-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.conversation_id.as_bytes());
    append_field(&mut aad, metadata.entity_kind.as_bytes());
    append_field(&mut aad, metadata.entity_id.as_bytes());
    aad.extend_from_slice(&metadata.sequence.to_be_bytes());
    aad
}

fn project_trust_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &ProjectTrustAuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + metadata.account_id.len() + metadata.project_id.len());
    aad.extend_from_slice(b"xfalcon-project-trust-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.project_id.as_bytes());
    aad
}

fn project_index_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &ProjectIndexAuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        32 + metadata.account_id.len() + metadata.project_id.len() + metadata.path_key.len(),
    );
    aad.extend_from_slice(b"xfalcon-project-index-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.project_id.as_bytes());
    append_field(&mut aad, metadata.path_key);
    aad
}

fn backup_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &BackupAuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        40 + metadata.account_id.len()
            + metadata.project_id.len()
            + metadata.path_key.len()
            + metadata.backup_id.len(),
    );
    aad.extend_from_slice(b"xfalcon-project-backup-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.project_id.as_bytes());
    append_field(&mut aad, metadata.path_key);
    append_field(&mut aad, metadata.backup_id.as_bytes());
    aad
}

fn approval_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &ApprovalAuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        32 + metadata.account_id.len() + metadata.project_id.len() + metadata.plan_id.len(),
    );
    aad.extend_from_slice(b"xfalcon-project-approval-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.project_id.as_bytes());
    append_field(&mut aad, metadata.plan_id.as_bytes());
    aad
}

fn plan_session_authenticated_data(
    aad_schema_version: u32,
    key_version: u32,
    metadata: &PlanSessionAuthenticatedMetadata<'_>,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + metadata.account_id.len() + metadata.client_id.len());
    aad.extend_from_slice(b"xfalcon-chatgpt-plan-session-v1");
    aad.extend_from_slice(&aad_schema_version.to_be_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    append_field(&mut aad, metadata.account_id.as_bytes());
    append_field(&mut aad, metadata.client_id.as_bytes());
    aad
}

fn append_field(destination: &mut Vec<u8>, field: &[u8]) {
    destination.extend_from_slice(&(field.len() as u64).to_be_bytes());
    destination.extend_from_slice(field);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_session_opens_only_for_its_own_account_and_client() {
        let key = Zeroizing::new([7_u8; 32]);
        let plaintext: &[u8] = b"plan session record";
        let metadata = PlanSessionAuthenticatedMetadata {
            account_id: "acct-1",
            client_id: "oaiapp_one",
        };
        let sealed = encrypt_plan_session(&key, 3, &metadata, plaintext).expect("sealed");

        let opened = decrypt_plan_session(
            &key,
            sealed.aad_schema_version,
            3,
            &sealed.nonce,
            &metadata,
            &sealed.ciphertext,
        )
        .expect("opened");
        assert_eq!(opened, plaintext);

        let other_client = PlanSessionAuthenticatedMetadata {
            account_id: "acct-1",
            client_id: "oaiapp_two",
        };
        assert!(decrypt_plan_session(
            &key,
            sealed.aad_schema_version,
            3,
            &sealed.nonce,
            &other_client,
            &sealed.ciphertext,
        )
        .is_err());

        let other_account = PlanSessionAuthenticatedMetadata {
            account_id: "acct-2",
            client_id: "oaiapp_one",
        };
        assert!(decrypt_plan_session(
            &key,
            sealed.aad_schema_version,
            3,
            &sealed.nonce,
            &other_account,
            &sealed.ciphertext,
        )
        .is_err());
    }
}
