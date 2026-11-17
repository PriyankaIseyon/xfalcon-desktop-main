use crate::clients::da::chat_api::{DaChatApiError, DaChatSession};
use serde_json::json;
use uuid::Uuid;
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn stop_accepts_only_verified_terminal_conflicts_without_rewriting_them() {
    // "failed" is DA's other terminal-failure spelling (`da_repository` maps
    // "failed" | "error" onto RunState::Failed). It was absent here, and absent
    // from the check this exercises, so Stop on an already-dead run reported a
    // failure to the reader.
    for status in [
        "completed",
        "error",
        "failed",
        "cancelled",
        "CANCELLED",
        " completed ",
        "running",
    ] {
        let server = MockServer::start().await;
        let run_id = Uuid::new_v4();
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/chat/runs/{run_id}/status")))
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(json!({"status": "cancelled"})))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path(format!("/api/v1/chat/runs/{run_id}")))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": run_id, "tenant_id": "tenant", "user_id": "user", "conversation_id": Uuid::new_v4(),
                "model": "model", "provider": "openai", "status": status, "input_message": "Question"
            }))).expect(1).mount(&server).await;
        let session = DaChatSession::for_test(&server.uri(), "test-token").unwrap();
        let outcome = session.cancel_run(run_id).await;
        if status == "running" {
            assert!(matches!(outcome, Err(DaChatApiError::Conflict)));
        } else {
            assert!(outcome.is_ok(), "{status}: {outcome:?}");
        }
    }
}

#[tokio::test]
async fn stop_preserves_transport_and_authorization_failures() {
    for status in [401, 403, 404, 500] {
        let server = MockServer::start().await;
        let run_id = Uuid::new_v4();
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/chat/runs/{run_id}/status")))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let session = DaChatSession::for_test(&server.uri(), "test-token").unwrap();
        assert!(session.cancel_run(run_id).await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
