use super::*;
use crate::app::events::CHAT_INFERENCE_EVENT_CHANNEL;
use crate::persistence::InMemorySecretStore;
use tauri::Listener;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn stop_during_context_preparation_emits_cancelled_and_releases_the_run() {
    exercise_preparation_cancel(204, false).await;
}

#[tokio::test]
async fn failed_preparation_cancel_preserves_recovery_and_reports_sync_failure() {
    exercise_preparation_cancel(503, false).await;
}

#[tokio::test]
async fn stop_while_mcp_discovery_and_context_are_both_pending_emits_cancelled() {
    exercise_preparation_cancel(204, true).await;
}

#[tokio::test]
async fn failed_stop_while_both_prerequisite_reads_are_pending_preserves_recovery() {
    exercise_preparation_cancel(503, true).await;
}

#[tokio::test]
async fn prerequisite_reads_overlap_and_return_both_values() {
    let server = MockServer::start().await;
    let conversation_uuid = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation_uuid}/context"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(1000))
                .set_body_json(serde_json::json!({
                    "active_compaction": null,
                    "boundary_sequence_no": 0,
                    "compactable_source_message_count": 0,
                    "compactable_through_sequence_no": 0,
                    "has_more": false,
                    "history_revision": 7,
                    "messages": [],
                    "next_cursor": null,
                    "snapshot_head_sequence_no": 0,
                    "source_message_ids_sha256": "0".repeat(64)
                })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let session = DaChatSession::for_test(&server.uri(), "test-bearer").unwrap();

    let started = std::time::Instant::now();
    let (mcp_value, context) = read_run_prerequisites(
        async {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            42_u8
        },
        &session,
        conversation_uuid,
    )
    .await;
    let elapsed = started.elapsed();

    assert_eq!(mcp_value, 42);
    assert_eq!(context.expect("context snapshot").history_revision, 7);
    // Serial reads would take at least 2000 ms; overlapped reads take about
    // 1000 ms.
    assert!(
        elapsed < Duration::from_millis(1600),
        "prerequisite reads ran serially: {elapsed:?}"
    );
}

#[tokio::test]
async fn context_failure_keeps_the_mcp_result_and_surfaces_the_error() {
    let server = MockServer::start().await;
    let conversation_uuid = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation_uuid}/context"
        )))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let session = DaChatSession::for_test(&server.uri(), "test-bearer").unwrap();

    let (mcp_value, context) =
        read_run_prerequisites(async { 7_u8 }, &session, conversation_uuid).await;

    assert_eq!(mcp_value, 7);
    assert!(context.is_err(), "a failed context read must stay an Err");
}

async fn exercise_preparation_cancel(cancel_status: u16, joined: bool) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir().unwrap();
    let account = ValidatedAccountId::parse("tenant:user").unwrap();
    let run_id = RunId::from(Uuid::new_v4().to_string());
    Mock::given(method("PATCH"))
        .and(path(format!(
            "/api/v1/chat/runs/{}/status",
            run_id.as_str()
        )))
        .and(body_json(serde_json::json!({ "status": "cancelled" })))
        .respond_with(ResponseTemplate::new(cancel_status))
        .expect(1)
        .mount(&server)
        .await;
    let conversation_uuid = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let run_cancels = Arc::new(AsyncMutex::new(HashMap::from([(
        run_id.as_str().to_owned(),
        ActiveLocalRun {
            account: account.clone(),
            conversation_id: ConversationId::from(conversation_uuid.to_string()),
            cancel: cancel.clone(),
            teardown_failed: false,
        },
    )])));
    let journals = RunJournalStore::new(directory.path().to_owned(), account.clone());
    journals
        .append_transition(&run_id, 1, RunState::Draft, RunState::Queued)
        .unwrap();
    let app = tauri::test::mock_app();
    let events = Arc::new(Mutex::new(Vec::new()));
    let emitted = events.clone();
    app.listen(CHAT_INFERENCE_EVENT_CHANNEL, move |event| {
        emitted.lock().unwrap().push(event.payload().to_owned());
    });
    // The production sync client rejects cleartext HTTP. This also proves
    // terminal cleanup succeeds when reconciliation is unavailable.
    let config = AuthConfig {
        da_base_url: server.uri().parse().unwrap(),
        casdoor_issuer: "https://auth.test.invalid".parse().unwrap(),
        client_id: "test-client".to_owned(),
        organization: "test".to_owned(),
        loopback_port: 0,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    let session = DaChatSession::for_test(&server.uri(), "test-bearer").unwrap();
    let da_client = crate::clients::da::mapping::build_hardened_client().expect("client");
    let preparation = prepare_run_step(
        async {
            if joined {
                // MCP discovery never answers here, so Stop must win while
                // both prerequisite reads are pending.
                read_run_prerequisites(std::future::pending::<()>(), &session, conversation_uuid)
                    .await
                    .1
            } else {
                session.read_context_snapshot(conversation_uuid).await
            }
        },
        &cancel,
        &session,
        RunTeardown {
            app: app.handle(),
            config: &config,
            bearer: "test-bearer",
            da_client: &da_client,
            tenant_id: "tenant",
            account,
            app_local_data: directory.path().to_owned(),
            secrets: Arc::new(InMemorySecretStore::default()),
            conversation_id: ConversationId::from(conversation_uuid.to_string()),
            run_id: run_id.clone(),
            run_id_string: run_id.as_str().to_owned(),
            run_state: RunState::Cancelled,
            run_cancels: run_cancels.clone(),
        },
    );
    let stop = async {
        loop {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(preparation, stop)
    })
    .await
    .expect("Stop must not wait for context response headers");
    assert!(result.is_none());
    assert!(run_cancels.lock().await.is_empty());
    let entries = journals.load_entries(&run_id).unwrap();
    let events = events.lock().unwrap();
    if cancel_status == 204 {
        assert!(entries.is_empty());
        assert_eq!(events.len(), 1);
    } else {
        assert_eq!(
            entries.last().expect("recovery marker").to,
            RunState::Interrupted
        );
        assert_eq!(events.len(), 2);
        let failure: serde_json::Value = serde_json::from_str(&events[1]).unwrap();
        assert_eq!(failure["type"], "failed");
        assert!(failure["message"]
            .as_str()
            .unwrap()
            .contains("could not be synchronized"));
    }
    let event: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
    assert_eq!(event["type"], "completed");
    assert_eq!(event["run_id"], run_id.as_str());
    assert_eq!(event["finish_reason"], "cancelled");
}
