use serde_json::json;
use uuid::Uuid;
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

use crate::clients::da::chat_api::DaChatSession;

fn conversation(id: Uuid, model: &str) -> serde_json::Value {
    json!({
        "id": id,
        "tenant_id": "tenant",
        "user_id": "user",
        "title": "Conversation",
        "model": model,
        "created_at": "2026-09-04T00:00:00Z",
        "updated_at": "2026-09-04T00:00:00Z"
    })
}

#[tokio::test]
async fn conversation_creation_persists_the_selected_model_in_the_first_write() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let conversation_id = Uuid::new_v4();

    Mock::given(method("POST"))
        .and(path("/api/v1/chat/conversations"))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(json!({
            "title": "Opening question",
            "model": "openai/gpt-5.6-luna",
            "auto_attach": true
        })))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(conversation(conversation_id, "openai/gpt-5.6-luna")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let created = session
        .create_conversation(
            Some("Opening question"),
            Some("openai/gpt-5.6-luna"),
            None,
            true,
        )
        .await
        .expect("conversation create");

    assert_eq!(created.id, conversation_id);
    assert_eq!(created.model.as_deref(), Some("openai/gpt-5.6-luna"));
}

#[tokio::test]
async fn scoped_conversation_creation_sends_metadata_without_auto_attaching_all_servers() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let metadata = json!({
        "xfalcon_desktop_mcp": {
            "version": 1,
            "server_id": "tenant",
            "endpoint": "https://tenant.example.com/mcp"
        },
        "unrelated": {"kept": true}
    });

    Mock::given(method("POST"))
        .and(path("/api/v1/chat/conversations"))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(
            json!({"metadata": metadata.clone(), "auto_attach": false}),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(conversation(conversation_id, "openai/gpt-5.6-luna")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let created = session
        .create_conversation(
            None,
            None,
            Some(metadata.as_object().unwrap().clone()),
            false,
        )
        .await
        .expect("conversation create");

    assert_eq!(created.id, conversation_id);
    server.verify().await;
}

#[tokio::test]
async fn partial_conversation_updates_never_clear_the_locked_model_as_collateral() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let endpoint = format!("/api/v1/chat/conversations/{conversation_id}");

    for (body, operation) in [
        (json!({"model":"~x-ai/grok-latest"}), "model"),
        (json!({"title":"Renamed"}), "title"),
        (json!({"is_archived":true}), "archive"),
    ] {
        Mock::given(method("PATCH"))
            .and(path(&endpoint))
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(body.clone()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(conversation(conversation_id, "~x-ai/grok-latest")),
            )
            .expect(1)
            .mount(&server)
            .await;

        match operation {
            "model" => {
                session
                    .set_conversation_model(conversation_id, "~x-ai/grok-latest")
                    .await
                    .expect("model lock");
            }
            "title" => {
                session
                    .set_conversation_title(conversation_id, "Renamed")
                    .await
                    .expect("title update");
            }
            "archive" => {
                session
                    .set_conversation_archived(conversation_id, true)
                    .await
                    .expect("archive update");
            }
            _ => unreachable!(),
        }
        server.verify().await;
        server.reset().await;
    }
}
