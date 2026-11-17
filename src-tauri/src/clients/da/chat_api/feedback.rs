use super::{map_empty_response, map_response, DaChatApiError, DaChatSession};
use crate::clients::da::generated::{
    CursorPageMessageFeedback, MessageFeedback, MessageFeedbackUpsert, MessageFeedbackUpsertRating,
};
use std::collections::HashSet;
use uuid::Uuid;
use xfalcon_contracts::chat::ChatMessageFeedback;

impl DaChatSession {
    async fn feedback_rows(
        &self,
        filter: &str,
        id: Uuid,
    ) -> Result<Vec<MessageFeedback>, DaChatApiError> {
        let mut rows = Vec::new();
        let mut cursor = None::<String>;
        let mut seen = HashSet::new();
        loop {
            let mut url = self.url("/api/v1/chat/message-feedback");
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair(filter, &id.to_string())
                    .append_pair("limit", "200");
                if let Some(cursor) = &cursor {
                    query.append_pair("cursor", cursor);
                }
            }
            let response = self
                .client
                .get(url)
                .bearer_auth(self.bearer.as_str())
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            let page: CursorPageMessageFeedback = map_response(response).await?;
            for row in page.items.ok_or(DaChatApiError::InvalidResponse)? {
                if !matches!(row.rating, -1 | 1)
                    || (filter == "conversation_id" && row.conversation_id != Some(id))
                    || (filter == "message_id" && row.message_id != id)
                {
                    return Err(DaChatApiError::InvalidResponse);
                }
                rows.push(row);
            }
            if !page.has_more.unwrap_or(false) {
                return Ok(rows);
            }
            let next = page
                .next_cursor
                .filter(|value| !value.is_empty())
                .ok_or(DaChatApiError::InvalidResponse)?;
            if !seen.insert(next.clone()) {
                return Err(DaChatApiError::InvalidResponse);
            }
            cursor = Some(next);
        }
    }

    pub(crate) async fn message_feedback_list(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ChatMessageFeedback>, DaChatApiError> {
        Ok(self
            .feedback_rows("conversation_id", conversation_id)
            .await?
            .into_iter()
            .map(|row| ChatMessageFeedback {
                message_id: row.message_id.to_string(),
                rating: row.rating as i32,
            })
            .collect())
    }

    pub(crate) async fn message_feedback_set(
        &self,
        conversation_id: Uuid,
        message_id: Uuid,
        rating: Option<i32>,
        tenant_id: &str,
    ) -> Result<Option<ChatMessageFeedback>, DaChatApiError> {
        let rating = match rating {
            Some(1) => Some(MessageFeedbackUpsertRating::Value1),
            Some(-1) => Some(MessageFeedbackUpsertRating::Value_1),
            None => None,
            _ => return Err(DaChatApiError::Validation),
        };
        let message = self
            .revision_message(conversation_id, message_id, tenant_id)
            .await?;
        if message.role != "assistant" {
            return Err(DaChatApiError::Validation);
        }
        if let Some(rating) = rating {
            let response = self
                .client
                .post(self.url("/api/v1/chat/message-feedback"))
                .bearer_auth(self.bearer.as_str())
                .json(&MessageFeedbackUpsert {
                    message_id,
                    conversation_id: Some(message.conversation_id),
                    rating,
                    comment: None,
                    metadata: None,
                })
                .send()
                .await
                .map_err(|_| DaChatApiError::RequestFailed)?;
            let row: MessageFeedback = map_response(response).await?;
            if row.message_id != message_id
                || row.conversation_id != Some(conversation_id)
                || !matches!(row.rating, -1 | 1)
            {
                return Err(DaChatApiError::InvalidResponse);
            }
            Ok(Some(ChatMessageFeedback {
                message_id: row.message_id.to_string(),
                rating: row.rating as i32,
            }))
        } else {
            for row in self.feedback_rows("message_id", message_id).await? {
                let response = self
                    .client
                    .delete(self.url(&format!("/api/v1/chat/message-feedback/{}", row.id)))
                    .bearer_auth(self.bearer.as_str())
                    .send()
                    .await
                    .map_err(|_| DaChatApiError::RequestFailed)?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    continue;
                }
                map_empty_response(response).await?;
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{body_partial_json, header, method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };

    fn row(conversation: Uuid, message: Uuid, id: Uuid, rating: i32) -> serde_json::Value {
        json!({"id":id,"tenant_id":"tenant","user_id":"user","message_id":message,
            "conversation_id":conversation,"rating":rating,"metadata":{},
            "created_at":"2026-09-08T00:00:00Z","updated_at":"2026-09-08T00:00:00Z"})
    }

    #[tokio::test]
    async fn feedback_hydration_follows_all_pages_with_authenticated_conversation_filter() {
        let server = MockServer::start().await;
        let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
        let conversation = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        Mock::given(method("GET")).and(path("/api/v1/chat/message-feedback"))
            .and(header("authorization", "Bearer token")).and(query_param("conversation_id", conversation.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[row(conversation,first,Uuid::new_v4(),1)],"has_more":true,"next_cursor":"next +/="})))
            .up_to_n_times(1).expect(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/message-feedback"))
            .and(query_param("cursor", "next +/="))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"items":[row(conversation,second,Uuid::new_v4(),-1)],"has_more":false}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let ratings = session.message_feedback_list(conversation).await.unwrap();
        assert_eq!(ratings.len(), 2);
        assert_eq!(ratings[0].message_id, first.to_string());
        assert_eq!(ratings[1].rating, -1);
    }

    #[tokio::test]
    async fn feedback_upsert_and_clear_use_web_endpoints_and_preserve_delete_failures() {
        let server = MockServer::start().await;
        let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
        let conversation = Uuid::new_v4();
        let message = Uuid::new_v4();
        let id = Uuid::new_v4();
        Mock::given(method("GET")).and(path(format!("/api/v1/chat/messages/{message}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":message,"tenant_id":"tenant","conversation_id":conversation,"role":"assistant","sequence_no":1,"content":"answer","created_at":"2026-09-08T00:00:00Z"})))
            .mount(&server).await;
        for rating in [1, -1] {
            Mock::given(method("POST"))
                .and(path("/api/v1/chat/message-feedback"))
                .and(header("authorization", "Bearer token"))
                .and(body_partial_json(
                    json!({"message_id":message,"conversation_id":conversation,"rating":rating}),
                ))
                .respond_with(ResponseTemplate::new(201).set_body_json(row(
                    conversation,
                    message,
                    id,
                    rating,
                )))
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                session
                    .message_feedback_set(conversation, message, Some(rating), "tenant")
                    .await
                    .unwrap()
                    .unwrap()
                    .rating,
                rating
            );
        }
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/message-feedback"))
            .and(query_param("message_id", message.to_string()))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"items":[row(conversation,message,id,-1)],"has_more":false}),
                ),
            )
            .mount(&server)
            .await;
        for status in [204, 404, 500] {
            Mock::given(method("DELETE"))
                .and(path(format!("/api/v1/chat/message-feedback/{id}")))
                .respond_with(ResponseTemplate::new(status))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
            let result = session
                .message_feedback_set(conversation, message, None, "tenant")
                .await;
            if status == 500 {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), None);
            }
        }
        assert!(matches!(
            session
                .message_feedback_set(conversation, message, Some(0), "tenant")
                .await,
            Err(DaChatApiError::Validation)
        ));
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn feedback_rejects_user_messages_before_writing() {
        let server = MockServer::start().await;
        let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
        let conversation = Uuid::new_v4();
        let message = Uuid::new_v4();
        Mock::given(method("GET")).and(path(format!("/api/v1/chat/messages/{message}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":message,"tenant_id":"tenant","conversation_id":conversation,"role":"user","sequence_no":1,"content":"question","created_at":"2026-09-08T00:00:00Z"})))
            .expect(1).mount(&server).await;
        assert!(matches!(
            session
                .message_feedback_set(conversation, message, Some(1), "tenant")
                .await,
            Err(DaChatApiError::Validation)
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn feedback_hydration_rejects_repeating_cursor() {
        let server = MockServer::start().await;
        let session = DaChatSession::for_test(&server.uri(), "token").unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/message-feedback"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items":[],"has_more":true,"next_cursor":"repeated"})),
            )
            .expect(2)
            .mount(&server)
            .await;
        assert!(matches!(
            session.message_feedback_list(Uuid::new_v4()).await,
            Err(DaChatApiError::InvalidResponse)
        ));
    }
}
