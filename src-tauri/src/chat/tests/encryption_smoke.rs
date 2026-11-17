//! Smoke checks that chat projection SQLite does not store message bodies in plaintext
//! (chat-da-runtime slice exit gate item 7).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use tempfile::tempdir;
use uuid::Uuid;
use xfalcon_contracts::ConversationId;

use crate::chat::domain::{
    ChatProjectionEvent, ChatSnapshot, Conversation, Message, MessageContent, MessageId,
    MessageRole, SyncCursor,
};
use crate::chat::local_projection::{
    EncryptedChatProjection, CHAT_PROJECTION_DB_FILE, LOCAL_PROJECTIONS_DIR,
};
use crate::chat::repository::LocalChatProjection;
use crate::persistence::InMemorySecretStore;
use crate::security::account_cleanup::ValidatedAccountId;

const ACCOUNT: &str = "encryption-smoke-account";
const PLAINTEXT_MARKER: &str = "gate7-plaintext-marker-7f3c9e2a";

fn projection_db_path(root: &std::path::Path) -> PathBuf {
    root.join(LOCAL_PROJECTIONS_DIR)
        .join(CHAT_PROJECTION_DB_FILE)
}

fn open_projection(root: &std::path::Path) -> EncryptedChatProjection {
    let path = projection_db_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create projections dir");
    }
    let store = Arc::new(InMemorySecretStore::default());
    store
        .put_key(ACCOUNT, [0xB3; 32])
        .expect("store account key");
    let account = ValidatedAccountId::parse(ACCOUNT).expect("valid account id");
    EncryptedChatProjection::open(&path, store, account).expect("open projection")
}

#[tokio::test]
async fn projection_sqlite_bytes_do_not_contain_message_plaintext() {
    let directory = tempdir().expect("tempdir");
    let conversation_id = ConversationId::from(Uuid::new_v4().to_string());
    let conversation = Conversation::new(conversation_id.clone(), "Smoke");
    let message = Message::try_new(
        MessageId::new(),
        conversation_id.clone(),
        1,
        MessageRole::User,
        vec![MessageContent::text(PLAINTEXT_MARKER)],
        vec![],
    )
    .expect("message")
    .into_completed();
    let snapshot = ChatSnapshot::new(
        conversation,
        vec![message],
        None,
        vec![],
        SyncCursor::new(1),
    );

    let local = open_projection(directory.path());
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot))
        .await
        .expect("persist");

    let db_path = projection_db_path(directory.path());
    let bytes = fs::read(&db_path).expect("read projection db");
    let needle = PLAINTEXT_MARKER.as_bytes();
    assert!(
        !bytes.windows(needle.len()).any(|window| window == needle),
        "message plaintext must not appear in projection db bytes"
    );
    let lossy = String::from_utf8_lossy(&bytes);
    assert!(
        !lossy.contains(PLAINTEXT_MARKER),
        "message plaintext must not appear as utf-8 in projection db"
    );
}

/// A snapshot sealed under a key this device no longer holds is a cache miss,
/// not a broken conversation: the cloud copy is the truth and the next
/// reconcile reseals the row. Erroring here bricked every open of an affected
/// conversation with "not found for your account".
#[tokio::test]
async fn a_snapshot_sealed_under_a_lost_key_reads_as_absent_not_as_an_error() {
    let directory = tempdir().expect("tempdir");
    let conversation_id = ConversationId::from(Uuid::new_v4().to_string());
    let conversation = Conversation::new(conversation_id.clone(), "Smoke");
    let snapshot = ChatSnapshot::new(conversation, vec![], None, vec![], SyncCursor::new(0));

    let path = projection_db_path(directory.path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create projections dir");
    }
    let account = ValidatedAccountId::parse(ACCOUNT).expect("valid account id");

    let old_store = Arc::new(InMemorySecretStore::default());
    old_store
        .put_key(ACCOUNT, [0xB3; 32])
        .expect("store account key");
    let local =
        EncryptedChatProjection::open(&path, old_store, account.clone()).expect("open projection");
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot))
        .await
        .expect("persist");
    drop(local);

    let new_store = Arc::new(InMemorySecretStore::default());
    new_store
        .put_key(ACCOUNT, [0x5C; 32])
        .expect("store rotated key");
    let reopened =
        EncryptedChatProjection::open(&path, new_store, account).expect("open projection");
    let loaded = reopened
        .load_snapshot(conversation_id)
        .await
        .expect("an unreadable cache row must not be an error");
    assert!(loaded.is_none(), "an unreadable cache row reads as absent");
}

/// The regression behind "chat is unavailable" on every send: a device that
/// has never sealed anything for this account holds no active key, and the
/// seal path must mint one rather than refuse the write.
#[tokio::test]
async fn a_fresh_device_with_no_account_key_can_still_seal_a_snapshot() {
    let directory = tempdir().expect("tempdir");
    let conversation_id = ConversationId::from(Uuid::new_v4().to_string());
    let conversation = Conversation::new(conversation_id.clone(), "Smoke");
    let snapshot = ChatSnapshot::new(conversation, vec![], None, vec![], SyncCursor::new(0));

    let path = projection_db_path(directory.path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create projections dir");
    }
    let account = ValidatedAccountId::parse(ACCOUNT).expect("valid account id");
    // Deliberately NO put_key: the store is what a first run on a new machine sees.
    let store = Arc::new(InMemorySecretStore::default());
    let local = EncryptedChatProjection::open(&path, store, account).expect("open projection");

    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot))
        .await
        .expect("a fresh device must mint its key on first seal");

    let loaded = local
        .load_snapshot(conversation_id)
        .await
        .expect("readable")
        .expect("the snapshot just sealed");
    assert_eq!(loaded.conversation().title(), "Smoke");
}
