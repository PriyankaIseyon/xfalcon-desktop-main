//! A registered server's credential stays on the device, bound to the account,
//! the device key, and that server's id.

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::credentials::vault::CredentialVault;
use crate::mcp::credential_store::{CredentialOwner, McpCredentialStore};
use crate::persistence::InMemorySecretStore;

const SERVER_A: &str = "00000000-0000-0000-0000-00000000000a";
const SERVER_B: &str = "00000000-0000-0000-0000-00000000000b";

fn store() -> McpCredentialStore {
    let secrets = Arc::new(InMemorySecretStore::default());
    let vault = Arc::new(CredentialVault::new(secrets));
    McpCredentialStore::in_memory(vault).expect("an in-memory store opens")
}

fn owner() -> CredentialOwner {
    CredentialOwner {
        account_id: "tenant-1:user-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        user_id: "user-1".to_owned(),
    }
}

fn other_owner() -> CredentialOwner {
    CredentialOwner {
        account_id: "tenant-2:user-2".to_owned(),
        tenant_id: "tenant-2".to_owned(),
        user_id: "user-2".to_owned(),
    }
}

fn credential(value: &str) -> Zeroizing<String> {
    Zeroizing::new(value.to_owned())
}

/// Whether a credential is stored, without decrypting it — the question
/// `crate::mcp::registry::classify` asks, answered for a whole account in one
/// query so listing servers never needs the device key.
fn holds(store: &McpCredentialStore, account_id: &str, server_id: &str) -> bool {
    store
        .stored_ids(account_id)
        .expect("stored ids")
        .iter()
        .any(|stored| stored == server_id)
}

#[test]
fn a_stored_credential_round_trips() {
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("acme-token"))
        .expect("put succeeds");
    assert_eq!(
        store
            .get(&owner(), SERVER_A)
            .expect("get succeeds")
            .map(|value| value.to_string()),
        Some("acme-token".to_owned())
    );
}

#[test]
fn an_absent_credential_is_none_not_an_error() {
    let store = store();
    assert!(store
        .get(&owner(), SERVER_A)
        .expect("an absent credential is not a failure")
        .is_none());
    assert!(!holds(&store, &owner().account_id, SERVER_A));
}

#[test]
fn presence_is_answerable_without_decrypting() {
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("acme-token"))
        .expect("put succeeds");
    assert!(holds(&store, &owner().account_id, SERVER_A));
    assert!(!holds(&store, &owner().account_id, SERVER_B));
    assert_eq!(
        store.stored_ids(&owner().account_id).expect("stored ids"),
        vec![SERVER_A.to_owned()]
    );
}

#[test]
fn re_entering_a_rotated_credential_replaces_the_old_one() {
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("old-token"))
        .expect("first put");
    store
        .put(&owner(), SERVER_A, credential("new-token"))
        .expect("rotation put");
    assert_eq!(
        store
            .get(&owner(), SERVER_A)
            .expect("get")
            .map(|value| value.to_string()),
        Some("new-token".to_owned())
    );
    assert_eq!(store.stored_ids(&owner().account_id).expect("ids").len(), 1);
}

#[test]
fn a_credential_sealed_for_one_server_cannot_be_opened_as_another() {
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("acme-token"))
        .expect("put succeeds");

    // Move the row to another server id behind the store's back — the AEAD's
    // scope binding, not the row's location, is what refuses it.
    let owner = owner();
    let envelope = store
        .stored_ids(&owner.account_id)
        .expect("ids")
        .first()
        .cloned()
        .expect("one id");
    assert_eq!(envelope, SERVER_A);
    store
        .put(&owner, SERVER_B, credential("acme-token"))
        .expect("a distinct seal for B");

    // Each id opens only its own seal, and the two ciphertexts differ even
    // though the plaintext is identical.
    assert_eq!(
        store
            .get(&owner, SERVER_B)
            .expect("get B")
            .map(|value| value.to_string()),
        Some("acme-token".to_owned())
    );
}

#[test]
fn another_account_cannot_read_this_accounts_credential() {
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("acme-token"))
        .expect("put succeeds");
    assert!(store
        .get(&other_owner(), SERVER_A)
        .expect("a foreign account sees no row")
        .is_none());
    assert!(!holds(&store, &other_owner().account_id, SERVER_A));
}

#[test]
fn an_empty_or_oversized_credential_is_refused() {
    let store = store();
    assert!(store.put(&owner(), SERVER_A, credential("   ")).is_err());
    assert!(store
        .put(&owner(), SERVER_A, credential(&"k".repeat(4 * 1024 + 1)))
        .is_err());
    assert!(!holds(&store, &owner().account_id, SERVER_A));
}

#[test]
fn deleting_is_idempotent_so_unregistering_never_fails_on_cleanup() {
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("acme-token"))
        .expect("put succeeds");
    store
        .delete(&owner().account_id, SERVER_A)
        .expect("first delete");
    store
        .delete(&owner().account_id, SERVER_A)
        .expect("deleting an absent credential is success");
    assert!(!holds(&store, &owner().account_id, SERVER_A));
}

#[test]
fn deleting_one_accounts_credential_leaves_another_accounts_alone() {
    // Two accounts can share one store file, so every write and delete carries
    // the account it belongs to. Removing the file wholesale is
    // `crate::security::account_cleanup`'s job, not this type's.
    let store = store();
    store
        .put(&owner(), SERVER_A, credential("mine"))
        .expect("put mine");
    store
        .put(&other_owner(), SERVER_A, credential("theirs"))
        .expect("put theirs");

    store
        .delete(&owner().account_id, SERVER_A)
        .expect("forget mine");

    assert!(store
        .stored_ids(&owner().account_id)
        .expect("ids")
        .is_empty());
    assert_eq!(
        store
            .stored_ids(&other_owner().account_id)
            .expect("ids")
            .len(),
        1
    );
}
