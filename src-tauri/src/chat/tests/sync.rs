use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use rusqlite::{params, Connection};
use tempfile::tempdir;
use tokio::sync::{oneshot, Barrier};
use uuid::Uuid;
use xfalcon_contracts::{ConversationId, RunId, RunState};

use crate::agent::recovery::is_in_flight_run_state;
use crate::chat::domain::{
    AssistantDraft, ChatBootstrap, ChatError, ChatProjectionEvent, ChatSnapshot, Conversation,
    Message, MessageContent, MessageId, MessageRole, NewConversation, NewMessage,
    PersistedRunEvent, RunRecord, SyncCursor, WorkspaceChatBootstrap,
};
use crate::chat::local_projection::{
    EncryptedChatProjection, CHAT_PROJECTION_DB_FILE, LOCAL_PROJECTIONS_DIR,
};
use crate::chat::repository::{CloudChatRepository, LocalChatProjection};
use crate::chat::sync::{ChatSyncService, SyncAvailability};
use crate::persistence::InMemorySecretStore;
use crate::security::account_cleanup::ValidatedAccountId;

const ACCOUNT: &str = "account-1";
const OTHER_ACCOUNT: &str = "account-2";

fn account(id: &str) -> ValidatedAccountId {
    ValidatedAccountId::parse(id).expect("valid account id")
}

fn conversation_id() -> ConversationId {
    ConversationId::from(Uuid::new_v4().to_string())
}

fn secrets_for(account_id: &str) -> Arc<InMemorySecretStore> {
    let store = Arc::new(InMemorySecretStore::default());
    store
        .put_key(account_id, [0xA5; 32])
        .expect("store account key");
    store
}

fn projection_db_path(root: &std::path::Path) -> PathBuf {
    root.join(LOCAL_PROJECTIONS_DIR)
        .join(CHAT_PROJECTION_DB_FILE)
}

fn open_projection(root: &std::path::Path, account_id: &str) -> EncryptedChatProjection {
    let path = projection_db_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create projections dir");
    }
    EncryptedChatProjection::open(&path, secrets_for(account_id), account(account_id))
        .expect("open projection")
}

fn completed_message(conversation: &ConversationId, sequence: u64, text: &str) -> Message {
    Message::try_new(
        MessageId::new(),
        conversation.clone(),
        sequence,
        MessageRole::User,
        vec![MessageContent::text(text)],
        vec![],
    )
    .expect("message")
    .into_completed()
}

fn snapshot_with(
    conversation: Conversation,
    messages: Vec<Message>,
    draft: Option<AssistantDraft>,
    head: u64,
) -> ChatSnapshot {
    snapshot_with_runs(conversation, messages, draft, vec![], head)
}

fn snapshot_with_runs(
    conversation: Conversation,
    messages: Vec<Message>,
    draft: Option<AssistantDraft>,
    runs: Vec<RunRecord>,
    head: u64,
) -> ChatSnapshot {
    let cursor = SyncCursor::new(head);
    ChatSnapshot::new(conversation, messages, draft, runs, cursor)
}

struct FakeCloud {
    bootstraps: Mutex<HashMap<String, ChatBootstrap>>,
    workspace: Mutex<Result<WorkspaceChatBootstrap, ChatError>>,
    fail_bootstrap: Mutex<Option<ChatError>>,
    bootstrap_calls: AtomicU64,
    hold_bootstrap: Mutex<Option<oneshot::Receiver<()>>>,
    concurrent_barrier: Mutex<Option<Arc<Barrier>>>,
    released: AtomicBool,
}

impl FakeCloud {
    fn with_bootstrap(bootstrap: ChatBootstrap) -> Self {
        let mut map = HashMap::new();
        map.insert(bootstrap.conversation().id().as_str().to_owned(), bootstrap);
        Self {
            bootstraps: Mutex::new(map),
            workspace: Mutex::new(Ok(WorkspaceChatBootstrap::new(vec![], SyncCursor::new(0)))),
            fail_bootstrap: Mutex::new(None),
            bootstrap_calls: AtomicU64::new(0),
            hold_bootstrap: Mutex::new(None),
            concurrent_barrier: Mutex::new(None),
            released: AtomicBool::new(false),
        }
    }

    fn set_hold(&self, rx: oneshot::Receiver<()>) {
        *self.hold_bootstrap.lock().expect("lock") = Some(rx);
    }

    fn set_fail(&self, error: ChatError) {
        *self.fail_bootstrap.lock().expect("lock") = Some(error);
    }

    fn insert_bootstrap(&self, bootstrap: ChatBootstrap) {
        self.bootstraps
            .lock()
            .expect("lock")
            .insert(bootstrap.conversation().id().as_str().to_owned(), bootstrap);
    }

    fn set_concurrent_barrier(&self, barrier: Arc<Barrier>) {
        *self.concurrent_barrier.lock().expect("lock") = Some(barrier);
    }
}

#[async_trait]
impl CloudChatRepository for FakeCloud {
    async fn workspace_bootstrap(&self) -> Result<WorkspaceChatBootstrap, ChatError> {
        self.workspace.lock().expect("lock").clone()
    }

    async fn conversation_bootstrap(&self, id: ConversationId) -> Result<ChatBootstrap, ChatError> {
        self.bootstrap_calls.fetch_add(1, Ordering::SeqCst);
        let held = self.hold_bootstrap.lock().expect("lock").take();
        if let Some(rx) = held {
            let _ = rx.await;
            self.released.store(true, Ordering::SeqCst);
        }
        let barrier = self.concurrent_barrier.lock().expect("lock").clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        if let Some(error) = self.fail_bootstrap.lock().expect("lock").clone() {
            return Err(error);
        }
        self.bootstraps
            .lock()
            .expect("lock")
            .get(id.as_str())
            .cloned()
            .ok_or(ChatError::NotFound)
    }

    async fn create_conversation(
        &self,
        _input: NewConversation,
    ) -> Result<Conversation, ChatError> {
        Err(ChatError::Unavailable)
    }

    async fn append_message(&self, _input: NewMessage) -> Result<Message, ChatError> {
        Err(ChatError::Unavailable)
    }

    async fn record_run_event(&self, _event: PersistedRunEvent) -> Result<(), ChatError> {
        Err(ChatError::Unavailable)
    }
}

#[tokio::test]
async fn local_snapshot_renders_before_cloud_response() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Local first");
    let message = completed_message(conversation.id(), 1, "cached");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![message.clone()],
            None,
            1,
        )))
        .await
        .expect("seed local");

    let cloud = FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![message.clone()],
        vec![],
        SyncCursor::new(1),
    ));
    let (release_tx, release_rx) = oneshot::channel();
    cloud.set_hold(release_rx);
    let cloud = Arc::new(cloud);
    let sync = ChatSyncService::new(cloud.clone(), local, account(ACCOUNT));

    let peeked = sync
        .peek_local(conversation.id().clone())
        .await
        .expect("peek")
        .expect("local snapshot");
    assert_eq!(peeked.messages().len(), 1);
    assert_eq!(
        peeked.messages()[0].content(),
        &[MessageContent::text("cached")]
    );
    assert_eq!(cloud.bootstrap_calls.load(Ordering::SeqCst), 0);
    assert!(!cloud.released.load(Ordering::SeqCst));

    let reconcile = tokio::spawn({
        let sync = ChatSyncService::new(
            cloud.clone(),
            open_projection(directory.path(), ACCOUNT),
            account(ACCOUNT),
        );
        let id = conversation.id().clone();
        async move { sync.reconcile_conversation(id).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(cloud.bootstrap_calls.load(Ordering::SeqCst), 1);
    assert!(!cloud.released.load(Ordering::SeqCst));
    let _ = release_tx.send(());
    let result = reconcile.await.expect("join").expect("reconcile");
    assert_eq!(result.availability, SyncAvailability::Live);
    assert_eq!(result.snapshot.messages().len(), 1);
}

#[tokio::test]
async fn local_preview_cache_miss_does_not_call_cloud() {
    let directory = tempdir().expect("tempdir");
    let cloud_conversation = Conversation::new(conversation_id(), "Cloud only");
    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        cloud_conversation,
        vec![],
        vec![],
        SyncCursor::new(0),
    )));
    let sync = ChatSyncService::new(
        Arc::clone(&cloud),
        open_projection(directory.path(), ACCOUNT),
        account(ACCOUNT),
    );

    let preview = sync
        .peek_local(conversation_id())
        .await
        .expect("local cache miss");

    assert!(preview.is_none());
    assert_eq!(cloud.bootstrap_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bounded_recovery_reconciles_independent_cloud_reads_concurrently() {
    let directory = tempdir().expect("tempdir");
    let first = Conversation::new(conversation_id(), "First");
    let second = Conversation::new(conversation_id(), "Second");
    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        first.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    )));
    cloud.insert_bootstrap(ChatBootstrap::new(
        second.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    ));
    let barrier = Arc::new(Barrier::new(3));
    cloud.set_concurrent_barrier(Arc::clone(&barrier));
    let sync = ChatSyncService::new(
        Arc::clone(&cloud),
        open_projection(directory.path(), ACCOUNT),
        account(ACCOUNT),
    );

    let reconcile = tokio::spawn(async move {
        sync.reconcile_conversations(vec![first.id().clone(), second.id().clone()], 2)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), barrier.wait())
        .await
        .expect("both cloud reads reached the barrier without serial waiting");
    let results = reconcile.await.expect("join");

    assert_eq!(cloud.bootstrap_calls.load(Ordering::SeqCst), 2);
    assert_eq!(results.len(), 2);
    assert!(results.into_iter().all(|(_, result)| result.is_ok()));
}

#[tokio::test]
async fn cloud_bootstrap_replaces_stale_committed_projections() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Stale title");
    let stale = completed_message(conversation.id(), 1, "stale body");
    let fresh = completed_message(conversation.id(), 1, "fresh body");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![stale],
            None,
            1,
        )))
        .await
        .expect("seed stale");

    let updated = Conversation::new(conversation.id().clone(), "Fresh title");
    let cloud = FakeCloud::with_bootstrap(ChatBootstrap::new(
        updated.clone(),
        vec![fresh.clone()],
        vec![],
        SyncCursor::new(1),
    ));
    let sync = ChatSyncService::new(Arc::new(cloud), local, account(ACCOUNT));
    let result = sync
        .reconcile_conversation(conversation.id().clone())
        .await
        .expect("reconcile");

    assert_eq!(result.snapshot.conversation().title(), "Fresh title");
    assert_eq!(
        result.snapshot.messages()[0].content(),
        &[MessageContent::text("fresh body")]
    );
    assert_eq!(result.availability, SyncAvailability::Live);
}

#[tokio::test]
async fn unsent_local_draft_survives_cloud_bootstrap() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Draft convo");
    let message = completed_message(conversation.id(), 1, "committed");
    let mut draft = AssistantDraft::new(conversation.id().clone(), 2).expect("draft");
    draft.append_text("unsent partial").expect("append");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![message.clone()],
            Some(draft),
            1,
        )))
        .await
        .expect("seed");

    let cloud_message = completed_message(conversation.id(), 1, "committed-from-cloud");
    let cloud = FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![cloud_message],
        vec![],
        SyncCursor::new(1),
    ));
    let sync = ChatSyncService::new(Arc::new(cloud), local, account(ACCOUNT));
    let result = sync
        .reconcile_conversation(conversation.id().clone())
        .await
        .expect("reconcile");

    let draft = result.snapshot.draft().expect("draft survived");
    assert_eq!(draft.text(), "unsent partial");
    assert!(!draft.is_committed());
}

#[tokio::test]
async fn duplicate_da_event_does_not_duplicate_message() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Dupes");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::UpsertConversation(
            conversation.clone(),
        ))
        .await
        .expect("conversation");
    local
        .apply(ChatProjectionEvent::AdvanceCursor(SyncCursor::new(0)))
        .await
        .expect("cursor");

    let message = completed_message(conversation.id(), 1, "once");
    let sync = ChatSyncService::new(
        Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
            conversation.clone(),
            vec![message.clone()],
            vec![],
            SyncCursor::new(1),
        ))),
        local,
        account(ACCOUNT),
    );

    sync.apply_projection_event(ChatProjectionEvent::AppendMessage(message.clone()))
        .await
        .expect("first append");
    sync.apply_projection_event(ChatProjectionEvent::AppendMessage(message.clone()))
        .await
        .expect("duplicate append");

    let snapshot = sync
        .peek_local(conversation.id().clone())
        .await
        .expect("peek")
        .expect("snapshot");
    assert_eq!(snapshot.messages().len(), 1);
    assert_eq!(snapshot.cursor().head_sequence(), 1);
}

#[tokio::test]
async fn out_of_order_sequence_triggers_full_bootstrap() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Ordering");
    let first = completed_message(conversation.id(), 1, "one");
    let third = completed_message(conversation.id(), 3, "three");
    let second = completed_message(conversation.id(), 2, "two");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![first.clone()],
            None,
            1,
        )))
        .await
        .expect("seed");

    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![first.clone(), second.clone(), third.clone()],
        vec![],
        SyncCursor::new(3),
    )));
    let sync = ChatSyncService::new(cloud.clone(), local, account(ACCOUNT));

    sync.apply_projection_event(ChatProjectionEvent::AppendMessage(third))
        .await
        .expect("apply out-of-order recovers via bootstrap");

    assert!(cloud.bootstrap_calls.load(Ordering::SeqCst) >= 1);
    let snapshot = sync
        .peek_local(conversation.id().clone())
        .await
        .expect("peek")
        .expect("snapshot");
    assert_eq!(snapshot.messages().len(), 3);
    assert_eq!(snapshot.cursor().head_sequence(), 3);
}

#[tokio::test]
async fn regressing_sequence_triggers_full_bootstrap() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Regress");
    let first = completed_message(conversation.id(), 1, "one");
    let second = completed_message(conversation.id(), 2, "two");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![first.clone(), second.clone()],
            None,
            2,
        )))
        .await
        .expect("seed");

    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![first.clone(), second.clone()],
        vec![],
        SyncCursor::new(2),
    )));
    let sync = ChatSyncService::new(cloud.clone(), local, account(ACCOUNT));

    // Same sequence as head (not next) must reject and recover via bootstrap.
    let regressing = completed_message(conversation.id(), 2, "stale-retry");
    sync.apply_projection_event(ChatProjectionEvent::AppendMessage(regressing))
        .await
        .expect("regressing sequence recovers via bootstrap");

    assert!(cloud.bootstrap_calls.load(Ordering::SeqCst) >= 1);
    let snapshot = sync
        .peek_local(conversation.id().clone())
        .await
        .expect("peek")
        .expect("snapshot");
    assert_eq!(snapshot.messages().len(), 2);
    assert_eq!(snapshot.cursor().head_sequence(), 2);
}

#[tokio::test]
async fn account_mismatch_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Owned");
    let message = completed_message(conversation.id(), 1, "secret");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![message],
            None,
            1,
        )))
        .await
        .expect("seed");

    let err = local
        .purge_account(account(OTHER_ACCOUNT))
        .await
        .expect_err("foreign account rejected");
    assert_eq!(err, ChatError::Validation("account mismatch"));

    let snapshot = local
        .load_snapshot(conversation.id().clone())
        .await
        .expect("load")
        .expect("still present");
    assert_eq!(snapshot.messages().len(), 1);

    let sync = ChatSyncService::new(
        Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
            conversation.clone(),
            vec![],
            vec![],
            SyncCursor::new(0),
        ))),
        open_projection(directory.path(), ACCOUNT),
        account(OTHER_ACCOUNT),
    );
    let mismatch = sync
        .reconcile_conversation(conversation.id().clone())
        .await
        .expect_err("sync account mismatch");
    assert_eq!(mismatch, ChatError::Validation("account mismatch"));
}

#[tokio::test]
async fn offline_mode_returns_local_snapshot_with_stale_status() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Offline");
    let message = completed_message(conversation.id(), 1, "cached offline");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![message],
            None,
            1,
        )))
        .await
        .expect("seed");

    let cloud = FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    ));
    cloud.set_fail(ChatError::Unavailable);
    let sync = ChatSyncService::new(Arc::new(cloud), local, account(ACCOUNT));
    let result = sync
        .reconcile_conversation(conversation.id().clone())
        .await
        .expect("offline reconcile");

    assert_eq!(result.availability, SyncAvailability::Stale);
    assert_eq!(result.snapshot.messages().len(), 1);
    assert_eq!(
        result.snapshot.messages()[0].content(),
        &[MessageContent::text("cached offline")]
    );
}

/// Spec `chat-conversation-open`: with no local copy, the fallback must
/// surface the ORIGINAL cloud error class. NotFound drives the caller's
/// self-heal (purge + list refresh); collapsing it into Unavailable made a
/// deleted conversation indistinguishable from a network blip.
#[tokio::test]
async fn reconcile_without_local_copy_preserves_not_found() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Gone");
    let local = open_projection(directory.path(), ACCOUNT);

    let cloud = FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    ));
    cloud.set_fail(ChatError::NotFound);
    let sync = ChatSyncService::new(Arc::new(cloud), local, account(ACCOUNT));
    let result = sync.reconcile_conversation(conversation.id().clone()).await;

    assert!(matches!(result, Err(ChatError::NotFound)));
}

#[tokio::test]
async fn reconcile_without_local_copy_preserves_unavailable() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Offline");
    let local = open_projection(directory.path(), ACCOUNT);

    let cloud = FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    ));
    cloud.set_fail(ChatError::Unavailable);
    let sync = ChatSyncService::new(Arc::new(cloud), local, account(ACCOUNT));
    let result = sync.reconcile_conversation(conversation.id().clone()).await;

    assert!(matches!(result, Err(ChatError::Unavailable)));
}

#[tokio::test]
async fn message_bodies_are_ciphertext_at_rest() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Cipher");
    let plaintext = "super-secret-message-body";
    let filename = "secret-report.pdf";
    let artifact_title = "secret-artifact-payload";
    let message = Message::try_new(
        MessageId::new(),
        conversation.id().clone(),
        1,
        MessageRole::Assistant,
        vec![
            MessageContent::text(plaintext),
            MessageContent::AttachmentRef {
                attachment_id: crate::chat::domain::AttachmentId::new(),
                file_name: filename.to_owned(),
                mime_type: "application/pdf".to_owned(),
            },
            MessageContent::ArtifactRef {
                artifact_id: crate::chat::domain::ArtifactId::new(),
                title: artifact_title.to_owned(),
                kind: "doc".to_owned(),
            },
        ],
        vec![],
    )
    .expect("message")
    .into_completed();
    let run = RunRecord::try_new(
        xfalcon_contracts::RunId::from("run-cipher-1"),
        conversation.id().clone(),
        xfalcon_contracts::RunState::AwaitingApproval,
        1,
    )
    .expect("run");

    let db_path = projection_db_path(directory.path());
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with_runs(
            conversation.clone(),
            vec![message],
            None,
            vec![run],
            1,
        )))
        .await
        .expect("persist");

    let connection = Connection::open(&db_path).expect("open raw db");
    let blobs: Vec<Vec<u8>> = connection
        .prepare(
            "SELECT ciphertext FROM chat_messages
             UNION ALL
             SELECT ciphertext FROM chat_conversations
             UNION ALL
             SELECT ciphertext FROM chat_drafts
             UNION ALL
             SELECT ciphertext FROM chat_runs",
        )
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert!(!blobs.is_empty());
    for blob in blobs {
        let haystack = String::from_utf8_lossy(&blob);
        assert!(!haystack.contains(plaintext));
        assert!(!haystack.contains(filename));
        assert!(!haystack.contains(artifact_title));
        assert!(!blob
            .windows(plaintext.len())
            .any(|w| w == plaintext.as_bytes()));
    }

    // Legacy state column must not store cleartext run state (only sealed payload may).
    let clear_states: Vec<String> = connection
        .prepare("SELECT state FROM chat_runs")
        .expect("prepare state")
        .query_map([], |row| row.get(0))
        .expect("query state")
        .collect::<Result<_, _>>()
        .expect("states");
    assert_eq!(clear_states.len(), 1);
    assert!(
        clear_states[0].is_empty(),
        "run state column must remain empty placeholder, got {:?}",
        clear_states[0]
    );
    let forbidden = [
        "AwaitingApproval",
        "awaiting_approval",
        "Completed",
        "Running",
        "Failed",
    ];
    for needle in forbidden {
        assert!(
            !clear_states[0].contains(needle),
            "plaintext run state leaked into column: {needle}"
        );
    }
}

#[tokio::test]
async fn account_cleanup_deletes_chat_projections() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Cleanup");
    let mut draft = AssistantDraft::new(conversation.id().clone(), 2).expect("draft");
    draft.append_text("draft to wipe").expect("append");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with(
            conversation.clone(),
            vec![completed_message(conversation.id(), 1, "wipe-me")],
            Some(draft),
            1,
        )))
        .await
        .expect("seed");

    let db_path = projection_db_path(directory.path());
    assert!(db_path.exists());

    local.purge_account(account(ACCOUNT)).await.expect("purge");

    let emptied = EncryptedChatProjection::open(&db_path, secrets_for(ACCOUNT), account(ACCOUNT))
        .expect("reopen after purge");
    assert!(emptied
        .load_snapshot(conversation.id().clone())
        .await
        .expect("load")
        .is_none());

    // ActiveWork run-engine cancellation is not wired yet (Task 4). Projection,
    // draft, and reconciliation rows are cleared by purge_account; journal wipe
    // remains owned by account_cleanup's Journal handler.
}

#[tokio::test]
async fn late_cloud_snapshot_cannot_restore_a_deleted_conversation() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Delete during refresh");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::UpsertConversation(
            conversation.clone(),
        ))
        .await
        .expect("seed");
    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![completed_message(conversation.id(), 1, "old response")],
        vec![],
        SyncCursor::new(1),
    )));
    let (release, held) = oneshot::channel();
    cloud.set_hold(held);
    let sync = ChatSyncService::new(
        cloud.clone(),
        open_projection(directory.path(), ACCOUNT),
        account(ACCOUNT),
    );
    let id = conversation.id().clone();
    let reader = tokio::spawn(async move { sync.reconcile_conversation(id).await });
    tokio::task::yield_now().await;
    assert_eq!(cloud.bootstrap_calls.load(Ordering::SeqCst), 1);
    local
        .apply(ChatProjectionEvent::RemoveConversation {
            conversation_id: conversation.id().clone(),
        })
        .await
        .expect("delete");
    release.send(()).expect("release stale response");
    assert!(matches!(
        reader.await.expect("reader"),
        Err(ChatError::NotFound)
    ));
    assert!(local
        .load_snapshot(conversation.id().clone())
        .await
        .expect("snapshot")
        .is_none());
    let reopened = open_projection(directory.path(), ACCOUNT);
    assert!(reopened
        .deleted_conversations()
        .await
        .expect("tombstones")
        .contains(conversation.id()));
    assert!(matches!(
        reopened
            .apply(ChatProjectionEvent::UpsertConversation(conversation))
            .await,
        Err(ChatError::NotFound)
    ));
}

#[tokio::test]
async fn deleted_conversation_is_filtered_from_workspace_but_not_other_accounts() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Deleted");
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::RemoveConversation {
            conversation_id: conversation.id().clone(),
        })
        .await
        .expect("delete");
    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        conversation.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    )));
    *cloud.workspace.lock().expect("workspace") = Ok(WorkspaceChatBootstrap::new(
        vec![crate::chat::domain::ConversationSummary::from_conversation(
            &conversation,
        )],
        SyncCursor::new(1),
    ));
    let sync = ChatSyncService::new(cloud.clone(), local, account(ACCOUNT));
    assert!(sync
        .workspace_bootstrap()
        .await
        .expect("workspace")
        .conversations()
        .is_empty());
    let other = ChatSyncService::new(
        cloud,
        open_projection(directory.path(), OTHER_ACCOUNT),
        account(OTHER_ACCOUNT),
    );
    assert_eq!(
        other
            .workspace_bootstrap()
            .await
            .expect("other workspace")
            .conversations()
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_local_delete_rolls_back_its_tombstone_and_all_rows() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Keep on failure");
    let local = open_projection(directory.path(), ACCOUNT);
    let snapshot = snapshot_with(
        conversation.clone(),
        vec![completed_message(conversation.id(), 1, "keep")],
        None,
        1,
    );
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot.clone()))
        .await
        .expect("seed");
    let connection = Connection::open(projection_db_path(directory.path())).expect("connection");
    connection.execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON chat_conversations BEGIN SELECT RAISE(ABORT, 'blocked'); END;").expect("inject storage failure");
    assert!(local
        .apply(ChatProjectionEvent::RemoveConversation {
            conversation_id: conversation.id().clone()
        })
        .await
        .is_err());
    assert!(local
        .deleted_conversations()
        .await
        .expect("tombstones")
        .is_empty());
    assert_eq!(
        local
            .load_snapshot(conversation.id().clone())
            .await
            .expect("snapshot"),
        Some(snapshot)
    );
}

#[tokio::test]
async fn deletion_fence_rejects_every_conversation_mutation() {
    let directory = tempdir().expect("tempdir");
    let conversation = Conversation::new(conversation_id(), "Deleted");
    let id = conversation.id().clone();
    let local = open_projection(directory.path(), ACCOUNT);
    local
        .apply(ChatProjectionEvent::RemoveConversation {
            conversation_id: id.clone(),
        })
        .await
        .expect("delete");
    let mutations = vec![
        ChatProjectionEvent::UpsertConversation(conversation.clone()),
        ChatProjectionEvent::ReplaceSnapshot(snapshot_with(conversation, vec![], None, 0)),
        ChatProjectionEvent::AppendMessage(completed_message(&id, 1, "late message")),
        ChatProjectionEvent::UpsertDraft(AssistantDraft::new(id.clone(), 1).expect("draft")),
        ChatProjectionEvent::ClearDraft {
            conversation_id: id.clone(),
        },
        ChatProjectionEvent::UpsertRun(
            RunRecord::try_new(
                xfalcon_contracts::RunId::from("run"),
                id.clone(),
                xfalcon_contracts::RunState::Running,
                1,
            )
            .expect("run"),
        ),
    ];
    for event in mutations {
        assert!(matches!(local.apply(event).await, Err(ChatError::NotFound)));
    }
    assert!(local.load_snapshot(id).await.expect("snapshot").is_none());
}

fn run_record(conversation: &ConversationId, state: RunState, sequence: u64) -> RunRecord {
    RunRecord::try_new(
        RunId::from(Uuid::new_v4().to_string()),
        conversation.clone(),
        state,
        sequence,
    )
    .expect("run record")
}

const ALL_RUN_STATES: [RunState; 9] = [
    RunState::Draft,
    RunState::Queued,
    RunState::Running,
    RunState::AwaitingApproval,
    RunState::Completed,
    RunState::Failed,
    RunState::Cancelled,
    RunState::Interrupted,
    RunState::Recovering,
];

#[tokio::test]
async fn load_runs_matches_load_snapshot_runs() {
    let directory = tempdir().expect("tempdir");
    let local = open_projection(directory.path(), ACCOUNT);

    let no_runs = Conversation::new(conversation_id(), "No runs");
    let one_run = Conversation::new(conversation_id(), "One run");
    let many_runs = Conversation::new(conversation_id(), "Many runs");
    let fixtures = [
        (no_runs.clone(), vec![]),
        (
            one_run.clone(),
            vec![run_record(one_run.id(), RunState::Running, 1)],
        ),
        (
            many_runs.clone(),
            ALL_RUN_STATES
                .iter()
                .cloned()
                .chain(ALL_RUN_STATES.iter().cloned())
                .enumerate()
                .map(|(index, state)| run_record(many_runs.id(), state, index as u64 + 1))
                .collect(),
        ),
    ];
    let expected_counts = [0, 1, 18];

    for (conversation, runs) in &fixtures {
        let head = runs.len() as u64 + 1;
        local
            .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with_runs(
                conversation.clone(),
                vec![completed_message(conversation.id(), 1, "hello")],
                None,
                runs.clone(),
                head,
            )))
            .await
            .expect("seed snapshot");
    }

    for ((conversation, _), expected_count) in fixtures.iter().zip(expected_counts) {
        let snapshot = local
            .load_snapshot(conversation.id().clone())
            .await
            .expect("load snapshot")
            .expect("snapshot present");
        let runs = local
            .load_runs(conversation.id().clone())
            .await
            .expect("load runs")
            .expect("runs present");
        assert_eq!(runs.len(), expected_count);
        assert_eq!(runs, snapshot.runs());
    }

    let unknown = conversation_id();
    assert!(local
        .load_snapshot(unknown.clone())
        .await
        .expect("snapshot")
        .is_none());
    assert!(local.load_runs(unknown).await.expect("runs").is_none());

    local
        .apply(ChatProjectionEvent::RemoveConversation {
            conversation_id: many_runs.id().clone(),
        })
        .await
        .expect("remove conversation");
    assert!(local
        .load_snapshot(many_runs.id().clone())
        .await
        .expect("snapshot")
        .is_none());
    assert!(local
        .load_runs(many_runs.id().clone())
        .await
        .expect("runs")
        .is_none());
}

#[tokio::test]
async fn peek_runs_flags_only_conversations_with_in_flight_runs() {
    let directory = tempdir().expect("tempdir");
    let local = open_projection(directory.path(), ACCOUNT);
    let busy = Conversation::new(conversation_id(), "Busy");
    let settled = Conversation::new(conversation_id(), "Settled");
    for (conversation, state) in [
        (&busy, RunState::AwaitingApproval),
        (&settled, RunState::Completed),
    ] {
        local
            .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with_runs(
                conversation.clone(),
                vec![],
                None,
                vec![run_record(conversation.id(), state, 1)],
                1,
            )))
            .await
            .expect("seed snapshot");
    }
    let cloud = Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
        busy.clone(),
        vec![],
        vec![],
        SyncCursor::new(0),
    )));
    let sync = ChatSyncService::new(cloud, local, account(ACCOUNT));

    let flagged = |runs: Option<Vec<RunRecord>>| {
        runs.is_some_and(|runs| {
            runs.iter()
                .any(|run| is_in_flight_run_state(run.state().clone()))
        })
    };
    let busy_runs = sync.peek_runs(busy.id().clone()).await.expect("peek busy");
    let settled_runs = sync
        .peek_runs(settled.id().clone())
        .await
        .expect("peek settled");
    assert!(flagged(busy_runs));
    assert!(!flagged(settled_runs));
    assert!(sync
        .peek_runs(conversation_id())
        .await
        .expect("peek unknown")
        .is_none());
}

/// Overwrites one stored ciphertext row with zeroes of the same length. The
/// AEAD check then fails on open, which is how an unreadable cache row looks
/// to the projection.
fn corrupt_ciphertext(root: &std::path::Path, sql: &str, conversation: &ConversationId) {
    let connection = Connection::open(projection_db_path(root)).expect("open raw db");
    let changed = connection
        .execute(sql, params![conversation.as_str()])
        .expect("corrupt ciphertext");
    assert_eq!(changed, 1, "exactly one ciphertext row must be corrupted");
}

#[tokio::test]
async fn a_conversation_with_unreadable_messages_still_reports_its_in_flight_runs() {
    let directory = tempdir().expect("tempdir");
    let local = open_projection(directory.path(), ACCOUNT);
    let conversation = Conversation::new(conversation_id(), "Unreadable messages");
    let run = run_record(conversation.id(), RunState::AwaitingApproval, 1);
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with_runs(
            conversation.clone(),
            vec![completed_message(conversation.id(), 1, "hello")],
            None,
            vec![run.clone()],
            2,
        )))
        .await
        .expect("seed snapshot");
    assert!(local
        .load_snapshot(conversation.id().clone())
        .await
        .expect("snapshot before corruption")
        .is_some());

    corrupt_ciphertext(
        directory.path(),
        "UPDATE chat_messages SET ciphertext = zeroblob(length(ciphertext))
         WHERE conversation_id = ?1",
        conversation.id(),
    );

    // The full snapshot cannot open the message, so it reports a cache miss
    // as the old peek did.
    assert!(local
        .load_snapshot(conversation.id().clone())
        .await
        .expect("snapshot after corruption")
        .is_none());
    // The runs-only read never opens messages, so the in-flight run is still
    // reported. This is the accepted difference: the bootstrap reconciles.
    let runs = local
        .load_runs(conversation.id().clone())
        .await
        .expect("runs after corruption")
        .expect("runs present");
    assert_eq!(runs, vec![run.clone()]);
    assert!(is_in_flight_run_state(runs[0].state().clone()));

    // The production consumer is `ChatSyncService::peek_runs`, which the
    // workspace bootstrap loop calls. It must report the same runs.
    let sync = ChatSyncService::new(
        Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
            conversation.clone(),
            vec![],
            vec![],
            SyncCursor::new(0),
        ))),
        local,
        account(ACCOUNT),
    );
    let peeked = sync
        .peek_runs(conversation.id().clone())
        .await
        .expect("peek runs after corruption")
        .expect("peeked runs present");
    assert_eq!(peeked, vec![run]);
}

#[tokio::test]
async fn a_conversation_with_an_unreadable_run_row_is_skipped_by_both_reads() {
    let directory = tempdir().expect("tempdir");
    let local = open_projection(directory.path(), ACCOUNT);
    let conversation = Conversation::new(conversation_id(), "Unreadable run");
    local
        .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with_runs(
            conversation.clone(),
            vec![completed_message(conversation.id(), 1, "hello")],
            None,
            vec![run_record(conversation.id(), RunState::Running, 1)],
            2,
        )))
        .await
        .expect("seed snapshot");

    corrupt_ciphertext(
        directory.path(),
        "UPDATE chat_runs SET ciphertext = zeroblob(length(ciphertext))
         WHERE conversation_id = ?1",
        conversation.id(),
    );

    assert!(local
        .load_snapshot(conversation.id().clone())
        .await
        .expect("snapshot")
        .is_none());
    assert!(local
        .load_runs(conversation.id().clone())
        .await
        .expect("runs")
        .is_none());

    // The production consumer, `ChatSyncService::peek_runs`, skips it too.
    let sync = ChatSyncService::new(
        Arc::new(FakeCloud::with_bootstrap(ChatBootstrap::new(
            conversation.clone(),
            vec![],
            vec![],
            SyncCursor::new(0),
        ))),
        local,
        account(ACCOUNT),
    );
    assert!(sync
        .peek_runs(conversation.id().clone())
        .await
        .expect("peek runs")
        .is_none());
}

/// Diagnostic only, so it stays out of the normal run. Compares the cost of
/// the full snapshot read against the runs-only read on a large cache.
/// Run with: cargo test --lib load_runs_timing -- --ignored --nocapture
#[ignore]
#[tokio::test]
async fn load_runs_timing_versus_load_snapshot() {
    const CONVERSATIONS: usize = 100;
    const MESSAGES_PER_CONVERSATION: u64 = 200;
    let directory = tempdir().expect("tempdir");
    let local = open_projection(directory.path(), ACCOUNT);
    let body = "x".repeat(600);

    let mut ids = Vec::with_capacity(CONVERSATIONS);
    for _ in 0..CONVERSATIONS {
        let conversation = Conversation::new(conversation_id(), "Timing");
        let messages = (1..=MESSAGES_PER_CONVERSATION)
            .map(|sequence| completed_message(conversation.id(), sequence, &body))
            .collect();
        let runs = vec![run_record(conversation.id(), RunState::Completed, 1)];
        local
            .apply(ChatProjectionEvent::ReplaceSnapshot(snapshot_with_runs(
                conversation.clone(),
                messages,
                None,
                runs,
                MESSAGES_PER_CONVERSATION,
            )))
            .await
            .expect("seed snapshot");
        ids.push(conversation.id().clone());
    }

    let started = Instant::now();
    for id in &ids {
        let snapshot = local
            .load_snapshot(id.clone())
            .await
            .expect("load snapshot")
            .expect("snapshot present");
        assert_eq!(snapshot.runs().len(), 1);
    }
    let snapshot_elapsed = started.elapsed();

    let started = Instant::now();
    for id in &ids {
        let runs = local
            .load_runs(id.clone())
            .await
            .expect("load runs")
            .expect("runs present");
        assert_eq!(runs.len(), 1);
    }
    let runs_elapsed = started.elapsed();

    eprintln!(
        "load_runs timing: {CONVERSATIONS} conversations x {MESSAGES_PER_CONVERSATION} messages: \
         100 load_snapshot calls = {snapshot_elapsed:?}, 100 load_runs calls = {runs_elapsed:?}"
    );
}
