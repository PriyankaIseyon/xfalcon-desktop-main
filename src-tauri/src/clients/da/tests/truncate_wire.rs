use serde_json::json;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

use crate::clients::da::chat_api::{DaChatApiError, DaChatSession};

#[derive(Clone)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("captured log lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLog {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn truncate_conversation_maps_success_and_typed_errors() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let conv_id = Uuid::new_v4();
    let msg_id = Uuid::new_v4();
    let expected_path = format!("/api/v1/chat/conversations/{conv_id}:truncate");

    // 1. Success 200 -> TruncateResult
    let expected_body = json!({
        "cutoff_message_id": msg_id,
        "deleted_messages": 2,
        "deleted_tool_calls": 0,
        "deleted_artifacts": 0,
        "deleted_runs": 1,
        "updated_at": "2026-08-19T00:00:00Z",
        "freed_attachment_storage_keys": null
    });
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&expected_body))
        .mount(&server)
        .await;

    let res = session
        .truncate_conversation(conv_id, msg_id)
        .await
        .expect("should succeed");
    assert_eq!(res.deleted_messages, 2);
    assert_eq!(res.deleted_runs, 1);
    assert_eq!(res.cutoff_message_id, msg_id);
    server.reset().await;

    // 2. 401 Unauthorized -> DaChatApiError::Unauthorized
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let err = match session.truncate_conversation(conv_id, msg_id).await {
        Err(e) => e,
        Ok(_) => panic!("should fail with 401"),
    };
    assert!(matches!(err, DaChatApiError::Unauthorized));
    server.reset().await;

    // 3. 403 Forbidden -> DaChatApiError::Unauthorized
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .respond_with(ResponseTemplate::new(403).set_body_string("Forbidden"))
        .mount(&server)
        .await;

    let err = match session.truncate_conversation(conv_id, msg_id).await {
        Err(e) => e,
        Ok(_) => panic!("should fail with 403"),
    };
    assert!(matches!(err, DaChatApiError::Unauthorized));
    server.reset().await;

    // 4. 404 Not Found -> DaChatApiError::NotFound
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
        .mount(&server)
        .await;

    let err = match session.truncate_conversation(conv_id, msg_id).await {
        Err(e) => e,
        Ok(_) => panic!("should fail with 404"),
    };
    assert!(matches!(err, DaChatApiError::NotFound));
    server.reset().await;

    // 5. 500 Server Error -> DaChatApiError::RequestFailed
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .respond_with(ResponseTemplate::new(500).set_body_string("Server Error"))
        .mount(&server)
        .await;

    let err = match session.truncate_conversation(conv_id, msg_id).await {
        Err(e) => e,
        Ok(_) => panic!("should fail with 500"),
    };
    assert!(matches!(err, DaChatApiError::RequestFailed));
    server.reset().await;

    // 6. 200 with invalid schema body -> DaChatApiError::InvalidResponse
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let err = match session.truncate_conversation(conv_id, msg_id).await {
        Err(e) => e,
        Ok(_) => panic!("should fail with invalid json"),
    };
    assert!(matches!(err, DaChatApiError::InvalidResponse));
}

#[tokio::test]
async fn malformed_da_response_does_not_log_body_values() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let conv_id = Uuid::new_v4();
    let msg_id = Uuid::new_v4();
    let expected_path = format!("/api/v1/chat/conversations/{conv_id}:truncate");
    let canary = "decode-secret-canary@example.com";
    Mock::given(method("POST"))
        .and(path(&expected_path))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"deleted_messages":"{canary}"}}"#)),
        )
        .mount(&server)
        .await;

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CapturedLog(bytes.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let result = session.truncate_conversation(conv_id, msg_id).await;

    assert!(matches!(result, Err(DaChatApiError::InvalidResponse)));
    let log = String::from_utf8(bytes.lock().expect("captured log lock").clone())
        .expect("captured logs are UTF-8");
    assert!(!log.contains(canary));
    assert!(log.contains("status=200"));
    assert!(log.contains(&expected_path));
}
