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

const ACCOUNT: &str = "local-search-account";

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
        .put_key(ACCOUNT, [0xB4; 32])
        .expect("store account key");
    let account = ValidatedAccountId::parse(ACCOUNT).expect("valid account id");
    EncryptedChatProjection::open(&path, store, account).expect("open projection")
}

fn snapshot_with_body(title: &str, body: &str) -> ChatSnapshot {
    let conversation_id = ConversationId::from(Uuid::new_v4().to_string());
    let conversation = Conversation::new(conversation_id.clone(), title);
    let message = Message::try_new(
        MessageId::new(),
        conversation_id,
        1,
        MessageRole::User,
        vec![MessageContent::text(body)],
        vec![],
    )
    .expect("message")
    .into_completed();
    ChatSnapshot::new(
        conversation,
        vec![message],
        None,
        vec![],
        SyncCursor::new(1),
    )
}

#[tokio::test]
async fn local_message_search_finds_sealed_bodies_without_writing_plaintext() {
    let directory = tempdir().expect("tempdir");
    let hit = snapshot_with_body("Untitled", "Gross margin for the Berlin office");
    let miss = snapshot_with_body("Other", "Headcount plan");
    let hit_id = hit.conversation().id().clone();

    let local = open_projection(directory.path());
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(hit))
        .await
        .expect("persist hit");
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(miss))
        .await
        .expect("persist miss");

    let matches = local
        .search_message_bodies("berlin")
        .expect("search sealed bodies");
    assert_eq!(matches, vec![hit_id]);

    let bytes = std::fs::read(projection_db_path(directory.path())).expect("read db");
    let needle = b"berlin";
    assert!(
        !bytes.windows(needle.len()).any(|window| window == needle),
        "search must not persist a plaintext index of the query hit"
    );
}
