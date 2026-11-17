use super::*;
use crate::chat::domain::Conversation;
use crate::chat::repository::LocalChatProjection;
use crate::persistence::InMemorySecretStore;
use std::str::FromStr;
use tempfile::TempDir;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    chat: ChatService,
    account: ValidatedAccountId,
    project: crate::projects::ProjectId,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let sessions = Arc::new(AuthSessionStore::new());
        let accounts = Arc::new(CurrentAccountScope::default());
        accounts.bind_native("account-1").unwrap();
        let account = accounts.current().unwrap().unwrap();
        let secrets = Arc::new(InMemorySecretStore::default());
        let projects = Arc::new(ProjectsService::new(
            directory.path().to_owned(),
            accounts.clone(),
            secrets.clone(),
        ));
        let view = projects.create(root.to_str().unwrap(), None).await.unwrap();
        let project = crate::projects::ProjectId::from_str(view.id.as_str()).unwrap();
        let chat = ChatService::new(
            sessions.clone(),
            accounts.clone(),
            directory.path().to_owned(),
            secrets.clone(),
            Arc::new(CredentialRuntime::new(sessions, secrets.clone())),
            Arc::new(crate::clients::da::model_catalog::ModelCatalogCache::default()),
            projects,
            Arc::new(ApprovalsService::new(
                directory.path().to_owned(),
                accounts,
                secrets,
            )),
        );
        Self {
            _directory: directory,
            root,
            chat,
            account,
            project,
        }
    }

    fn local(&self) -> EncryptedChatProjection {
        EncryptedChatProjection::open(
            projection_db_path(&self.chat.app_local_data, &self.account),
            self.chat.secrets.clone(),
            self.account.clone(),
        )
        .unwrap()
    }

    async fn bind(&self, id: &ConversationId) {
        self.chat
            .projects
            .bind_conversation(&self.account, id, self.project)
            .unwrap();
        self.local()
            .apply(ChatProjectionEvent::UpsertConversation(Conversation::new(
                id.clone(),
                "Project chat",
            )))
            .await
            .unwrap();
    }

    async fn cleanup(&self, server: &MockServer) -> Result<(), ChatServiceError> {
        let da = DaChatSession::for_test(&server.uri(), "pinned-user-token").unwrap();
        self.chat
            .cleanup_deleted_projects_for_account(&self.account, &da, vec![self.project], None)
            .await
    }
}

async fn deletion(server: &MockServer, id: &ConversationId, status: u16) {
    Mock::given(method("DELETE"))
        .and(path(format!("/api/v1/chat/conversations/{}", id.as_str())))
        .and(header("authorization", "Bearer pinned-user-token"))
        .respond_with(ResponseTemplate::new(status))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn deletes_bound_chats_and_cancels_active_run_before_revoking() {
    let f = Fixture::new().await;
    let server = MockServer::start().await;
    let id = ConversationId::from(Uuid::new_v4().to_string());
    f.bind(&id).await;
    f.chat.set_active_conversation(&f.account, id.clone());
    let cancel = CancellationToken::new();
    f.chat.run_cancels.lock().await.insert(
        "run".to_owned(),
        ActiveLocalRun {
            account: f.account.clone(),
            conversation_id: id.clone(),
            cancel: cancel.clone(),
            teardown_failed: false,
        },
    );
    std::fs::remove_dir(&f.root).unwrap();
    let runs = f.chat.run_cancels.clone();
    let driver_cancel = cancel.clone();
    let projection = f.local();
    let driver_id = id.clone();
    let driver = tokio::spawn(async move {
        driver_cancel.cancelled().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        projection
            .apply(ChatProjectionEvent::UpsertConversation(Conversation::new(
                driver_id,
                "Final driver reconciliation",
            )))
            .await
            .unwrap();
        runs.lock().await.remove("run");
    });
    deletion(&server, &id, 204).await;
    f.cleanup(&server).await.unwrap();
    driver.await.unwrap();
    assert!(cancel.is_cancelled());
    assert!(f.chat.projects.list().unwrap().projects.is_empty());
    assert!(f
        .chat
        .projects
        .conversation_bindings(&f.account)
        .unwrap()
        .is_empty());
    assert!(f.local().load_snapshot(id).await.unwrap().is_none());
    assert!(!f
        .chat
        .active_conversations
        .lock()
        .unwrap()
        .contains_key(f.account.as_str()));
}

#[tokio::test]
async fn stopping_timeout_preserves_conversation_and_project() {
    let f = Fixture::new().await;
    let id = ConversationId::from(Uuid::new_v4().to_string());
    f.bind(&id).await;
    std::fs::remove_dir(&f.root).unwrap();
    let cancel = CancellationToken::new();
    f.chat.run_cancels.lock().await.insert(
        "blocked-driver".to_owned(),
        ActiveLocalRun {
            account: f.account.clone(),
            conversation_id: id.clone(),
            cancel: cancel.clone(),
            teardown_failed: false,
        },
    );
    let error = f
        .chat
        .stop_conversation_runs(&f.account, &id, Duration::from_millis(10))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("still stopping"));
    assert!(cancel.is_cancelled());
    assert!(f
        .chat
        .run_cancels
        .lock()
        .await
        .contains_key("blocked-driver"));
    assert!(f
        .chat
        .projects
        .conversation_bindings(&f.account)
        .unwrap()
        .contains_key(id.as_str()));
    assert!(f.local().load_snapshot(id).await.unwrap().is_some());
    assert_eq!(f.chat.projects.list().unwrap().projects.len(), 1);
}

#[tokio::test]
async fn empty_deleted_project_is_revoked_without_authentication() {
    let f = Fixture::new().await;
    std::fs::remove_dir(&f.root).unwrap();
    f.chat.cleanup_deleted_projects(None).await.unwrap();
    assert!(f.chat.projects.list().unwrap().projects.is_empty());
}

#[tokio::test]
async fn failed_delete_retains_trust_and_remaining_chat_for_retry() {
    let f = Fixture::new().await;
    let server = MockServer::start().await;
    let first = ConversationId::from("00000000-0000-0000-0000-000000000001");
    let second = ConversationId::from("00000000-0000-0000-0000-000000000002");
    f.bind(&first).await;
    f.bind(&second).await;
    std::fs::remove_dir(&f.root).unwrap();
    deletion(&server, &first, 204).await;
    deletion(&server, &second, 503).await;
    assert!(f.cleanup(&server).await.is_err());
    assert_eq!(f.chat.projects.list().unwrap().projects.len(), 1);
    let bindings = f.chat.projects.conversation_bindings(&f.account).unwrap();
    assert!(!bindings.contains_key(first.as_str()));
    assert!(bindings.contains_key(second.as_str()));
    assert!(f
        .local()
        .load_snapshot(second.clone())
        .await
        .unwrap()
        .is_some());
    server.verify().await;
    server.reset().await;
    deletion(&server, &second, 404).await;
    f.cleanup(&server).await.unwrap();
    assert!(f.chat.projects.list().unwrap().projects.is_empty());
}

#[tokio::test]
async fn keeps_unrelated_active_chat_and_refuses_unconfirmed_or_switched_account() {
    let f = Fixture::new().await;
    let server = MockServer::start().await;
    let id = ConversationId::from(Uuid::new_v4().to_string());
    f.bind(&id).await;
    assert!(f.cleanup(&server).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    std::fs::remove_dir(&f.root).unwrap();
    f.chat.accounts.bind_native("account-2").unwrap();
    assert!(f.cleanup(&server).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    f.chat.accounts.bind_native("account-1").unwrap();
    let unrelated = ConversationId::from(Uuid::new_v4().to_string());
    f.chat
        .set_active_conversation(&f.account, unrelated.clone());
    deletion(&server, &id, 204).await;
    f.cleanup(&server).await.unwrap();
    assert_eq!(
        f.chat
            .active_conversations
            .lock()
            .unwrap()
            .get(f.account.as_str()),
        Some(&unrelated)
    );
}

#[tokio::test]
async fn account_switch_during_delete_stops_before_next_cloud_mutation() {
    let f = Fixture::new().await;
    let server = MockServer::start().await;
    let first = ConversationId::from("00000000-0000-0000-0000-000000000001");
    let second = ConversationId::from("00000000-0000-0000-0000-000000000002");
    f.bind(&first).await;
    f.bind(&second).await;
    std::fs::remove_dir(&f.root).unwrap();
    let accounts = f.chat.accounts.clone();
    Mock::given(method("DELETE"))
        .and(path(format!(
            "/api/v1/chat/conversations/{}",
            first.as_str()
        )))
        .and(header("authorization", "Bearer pinned-user-token"))
        .respond_with(move |_: &wiremock::Request| {
            accounts.bind_native("account-2").unwrap();
            ResponseTemplate::new(204)
        })
        .expect(1)
        .mount(&server)
        .await;
    assert!(f.cleanup(&server).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    let bindings = f.chat.projects.conversation_bindings(&f.account).unwrap();
    assert!(!bindings.contains_key(first.as_str()));
    assert!(bindings.contains_key(second.as_str()));
    assert!(f
        .chat
        .projects
        .conversation_bindings(&ValidatedAccountId::parse("account-2").unwrap())
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn manual_delete_keeps_project_and_other_active_chat_and_retries_not_found() {
    let f = Fixture::new().await;
    let server = MockServer::start().await;
    let id = ConversationId::from(Uuid::new_v4().to_string());
    let other = ConversationId::from(Uuid::new_v4().to_string());
    f.bind(&id).await;
    f.chat.set_active_conversation(&f.account, other.clone());
    deletion(&server, &id, 404).await;
    let da = DaChatSession::for_test(&server.uri(), "pinned-user-token").unwrap();
    f.chat
        .delete_conversation_for_account(&f.account, &da, &f.local(), id.clone(), None, None)
        .await
        .unwrap();
    assert_eq!(f.chat.projects.list().unwrap().projects.len(), 1);
    assert_eq!(
        f.chat
            .active_conversations
            .lock()
            .unwrap()
            .get(f.account.as_str()),
        Some(&other)
    );
    assert!(!f
        .chat
        .projects
        .conversation_bindings(&f.account)
        .unwrap()
        .contains_key(id.as_str()));
    assert!(f
        .local()
        .deleted_conversations()
        .await
        .unwrap()
        .contains(&id));
}

#[tokio::test]
async fn resumed_run_owner_is_independent_of_sidebar_selection_and_account_scoped() {
    let f = Fixture::new().await;
    let owner = ConversationId::from(Uuid::new_v4().to_string());
    let other = ConversationId::from(Uuid::new_v4().to_string());
    f.chat.run_cancels.lock().await.insert(
        "run".to_owned(),
        ActiveLocalRun {
            account: f.account.clone(),
            conversation_id: owner.clone(),
            cancel: CancellationToken::new(),
            teardown_failed: false,
        },
    );
    assert_eq!(
        f.chat
            .run_owner_conversation(&f.account, "run")
            .await
            .unwrap(),
        owner
    );
    f.chat.set_active_conversation(&f.account, other);
    assert_eq!(
        f.chat
            .run_owner_conversation(&f.account, "run")
            .await
            .unwrap(),
        owner
    );
    assert!(f
        .chat
        .run_owner_conversation(&ValidatedAccountId::parse("other").unwrap(), "run")
        .await
        .is_err());
}

#[tokio::test]
async fn teardown_failure_is_explicit_and_preserves_chat_for_restart_retry() {
    let f = Fixture::new().await;
    let id = ConversationId::from(Uuid::new_v4().to_string());
    f.bind(&id).await;
    f.chat.run_cancels.lock().await.insert(
        "failed".to_owned(),
        ActiveLocalRun {
            account: f.account.clone(),
            conversation_id: id.clone(),
            cancel: CancellationToken::new(),
            teardown_failed: true,
        },
    );
    let result = f
        .chat
        .stop_conversation_runs(&f.account, &id, Duration::from_millis(10))
        .await;
    assert!(result.unwrap_err().to_string().contains("Restart the app"));
    assert!(f.local().load_snapshot(id).await.unwrap().is_some());
}

#[tokio::test]
async fn stale_run_cancel_cannot_cancel_another_accounts_run() {
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    f.chat.run_cancels.lock().await.insert(
        "old-account-run".to_owned(),
        ActiveLocalRun {
            account: ValidatedAccountId::parse("old-account").unwrap(),
            conversation_id: ConversationId::from(Uuid::new_v4().to_string()),
            cancel: cancel.clone(),
            teardown_failed: false,
        },
    );
    assert!(f
        .chat
        .run_cancel("old-account-run".to_owned(), None)
        .await
        .is_err());
    assert!(!cancel.is_cancelled());
    assert!(f
        .chat
        .run_cancels
        .lock()
        .await
        .contains_key("old-account-run"));
}

#[tokio::test]
async fn missing_cloud_conversation_waits_for_local_driver_before_delinking() {
    let f = Fixture::new().await;
    let id = ConversationId::from(Uuid::new_v4().to_string());
    f.bind(&id).await;
    let cancel = CancellationToken::new();
    f.chat.run_cancels.lock().await.insert(
        "running".to_owned(),
        ActiveLocalRun {
            account: f.account.clone(),
            conversation_id: id.clone(),
            cancel: cancel.clone(),
            teardown_failed: false,
        },
    );
    let driver = async {
        cancel.cancelled().await;
        assert!(f.chat.deleted_project_cleanup.try_lock().is_err());
        assert!(!f
            .local()
            .deleted_conversations()
            .await
            .unwrap()
            .contains(&id));
        assert!(f
            .chat
            .projects
            .conversation_bindings(&f.account)
            .unwrap()
            .contains_key(id.as_str()));
        f.local()
            .apply(ChatProjectionEvent::UpsertConversation(Conversation::new(
                id.clone(),
                "Final driver reconciliation",
            )))
            .await
            .unwrap();
        f.chat.run_cancels.lock().await.remove("running");
    };
    let (removed, ()) = tokio::join!(
        f.chat
            .remove_missing_conversation(&f.account, id.clone(), None),
        driver,
    );
    removed.unwrap();
    assert!(cancel.is_cancelled());
    assert!(f.local().load_snapshot(id.clone()).await.unwrap().is_none());
    assert!(f
        .local()
        .deleted_conversations()
        .await
        .unwrap()
        .contains(&id));
    assert!(!f
        .chat
        .projects
        .conversation_bindings(&f.account)
        .unwrap()
        .contains_key(id.as_str()));
}

#[tokio::test]
async fn stop_signals_the_owned_run_before_waiting_for_the_cleanup_gate() {
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    f.chat.run_cancels.lock().await.insert(
        "running".to_owned(),
        ActiveLocalRun {
            account: f.account.clone(),
            conversation_id: ConversationId::from(Uuid::new_v4().to_string()),
            cancel: cancel.clone(),
            teardown_failed: false,
        },
    );
    let gate = f.chat.deleted_project_cleanup.lock().await;
    let stop = f.chat.run_cancel("running".to_owned(), None);
    tokio::pin!(stop);
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            result = &mut stop => panic!("Stop bypassed parked ownership gate: {result:?}"),
            () = cancel.cancelled() => {}
        }
    })
    .await
    .expect("cancellation must reach the tool while the gate remains held");
    assert!(cancel.is_cancelled());
    assert!(f.chat.deleted_project_cleanup.try_lock().is_err());
    drop(gate);
    stop.await.expect("finish stop after gate release");
}
