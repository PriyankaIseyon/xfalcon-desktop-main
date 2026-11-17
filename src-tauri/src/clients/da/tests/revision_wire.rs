use crate::clients::da::chat_api::{DaChatApiError, DaChatSession};
use serde_json::{json, Value};
use uuid::Uuid;
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn message(conversation: Uuid, id: Uuid, sequence: i64, role: &str) -> Value {
    json!({"id":id,"tenant_id":"tenant","conversation_id":conversation,"sequence_no":sequence,
        "role":role,"content":"original prompt","created_at":"2026-09-07T00:00:00Z"})
}

#[tokio::test]
async fn revision_message_reads_an_old_user_directly_without_the_bootstrap_window() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
    let conversation = Uuid::new_v4();
    let id = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/chat/messages/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(message(
            conversation,
            id,
            1,
            "user",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let target = session
        .revision_message(conversation, id, "tenant")
        .await
        .unwrap();
    assert_eq!(target.id, id);
    assert_eq!(target.role, "user");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn revision_message_rejects_wrong_identity_scope_and_superseded_rows() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
    let conversation = Uuid::new_v4();
    let id = Uuid::new_v4();
    for (field, value) in [
        ("id", json!(Uuid::new_v4())),
        ("conversation_id", json!(Uuid::new_v4())),
        ("tenant_id", json!("other")),
        ("superseded_by_checkpoint_id", json!(Uuid::new_v4())),
    ] {
        let mut row = message(conversation, id, 1, "user");
        row[field] = value;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/chat/messages/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(row))
            .mount(&server)
            .await;
        assert!(matches!(
            session.revision_message(conversation, id, "tenant").await,
            Err(DaChatApiError::NotFound)
        ));
        server.reset().await;
    }
}

#[tokio::test]
async fn revision_history_keeps_the_user_before_200_tool_rows_and_stops_at_target() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
    let conversation = Uuid::new_v4();
    let user = Uuid::new_v4();
    let cutoff = Uuid::new_v4();
    let target = Uuid::new_v4();
    let mut rows = vec![
        message(conversation, user, 1, "user"),
        message(conversation, cutoff, 2, "assistant"),
    ];
    rows.extend((3..=200).map(|n| message(conversation, Uuid::new_v4(), n, "tool")));
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation}/messages"
        )))
        .and(query_param("active_only", "true"))
        .and(query_param("sort", "sequence_no"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items":rows,"next_cursor":"next +/=","has_more":true})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(query_param("cursor", "next +/="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[message(conversation,target,201,"assistant")],"next_cursor":"unused","has_more":true})))
        .with_priority(1).expect(1).mount(&server).await;
    let history = session
        .revision_history(conversation, target, "tenant")
        .await
        .unwrap();
    assert_eq!(history.len(), 201);
    assert_eq!(history[0].id, user);
    assert_eq!(history[1].id, cutoff);
    assert_eq!(history.last().unwrap().id, target);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn revision_history_fails_on_repeated_cursor_or_missing_target() {
    for repeat in [false, true] {
        let server = MockServer::start().await;
        let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(if repeat {
                json!({"items":[],"next_cursor":"repeat","has_more":true})
            } else {
                json!({"items":[],"next_cursor":null,"has_more":false})
            }))
            .mount(&server)
            .await;
        let result = session
            .revision_history(Uuid::new_v4(), Uuid::new_v4(), "tenant")
            .await;
        assert!(
            matches!(result, Err(error) if error == if repeat { DaChatApiError::InvalidResponse } else { DaChatApiError::NotFound })
        );
    }
}

#[tokio::test]
async fn revision_history_rejects_cross_scope_inactive_and_out_of_order_rows() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
    let conversation = Uuid::new_v4();
    let target = Uuid::new_v4();
    for (field, value) in [
        ("conversation_id", json!(Uuid::new_v4())),
        ("tenant_id", json!("other")),
        ("superseded_by_checkpoint_id", json!(Uuid::new_v4())),
        ("sequence_no", json!(1)),
    ] {
        let first = message(conversation, Uuid::new_v4(), 1, "user");
        let mut last = message(conversation, target, 2, "assistant");
        last[field] = value;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[first,last]})))
            .mount(&server)
            .await;
        assert!(matches!(
            session
                .revision_history(conversation, target, "tenant")
                .await,
            Err(DaChatApiError::InvalidResponse)
        ));
        server.reset().await;
    }
}
