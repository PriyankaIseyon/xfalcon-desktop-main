use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, ResponseTemplate,
};
use xfalcon_contracts::tools::ToolSource;

use crate::agent::local_executor::{RunEventSink, ToolRoundCall, ToolRoundRecord};
use crate::tools::{ToolResult, ToolResultBudget};

use crate::clients::da::{
    chat_api::{DaChatApiError, DaChatSession},
    generated::{
        ContextCompactionActivate, PayloadMediaType, PayloadPreviewStrategy,
        PayloadSerializationFormat, ToolResultPayload, ToolResultPayloadDescriptor,
        ToolResultPayloadPrepare, UploadInstruction,
    },
};

const SHA_ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// data-analyst `MAX_PREVIEW_BYTES` (`mcp_server/api/v1/chat/models/context.py`)
/// and the `tool_result_payloads` CHECK, raised by its
/// `raise-tool-result-preview-budget` change.
const DA_DURABLE_PREVIEW_MAX_BYTES: usize = 256 * 1024;

#[test]
fn tool_preview_ceiling_matches_the_da_durable_payload_contract() {
    assert_eq!(
        ToolResultBudget::CEILING_BYTES,
        DA_DURABLE_PREVIEW_MAX_BYTES
    );
}

fn message(id: Uuid, sequence_no: i64) -> Value {
    json!({
        "content": format!("message-{sequence_no}"),
        "created_at": "2026-09-01T00:00:00Z",
        "id": id,
        "metadata": null,
        "name": null,
        "role": "user",
        "sequence_no": sequence_no,
        "token_input": null,
        "token_output": null,
        "tool_call_id": null,
        "tool_calls": null,
        "tool_result_payloads": null
    })
}

fn page(messages: Vec<Value>, has_more: bool, cursor: Option<&str>) -> Value {
    json!({
        "active_compaction": null,
        "boundary_sequence_no": 0,
        "compactable_source_message_count": 2,
        "compactable_through_sequence_no": 2,
        "has_more": has_more,
        "history_revision": 7,
        "messages": messages,
        "next_cursor": cursor,
        "snapshot_head_sequence_no": 2,
        "source_message_ids_sha256": SHA_ZERO
    })
}

#[tokio::test]
async fn context_reader_restarts_from_page_one_after_a_typed_stale_cursor() {
    let server = MockServer::start().await;
    let conversation_id = Uuid::new_v4();
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let calls = Arc::new(AtomicUsize::new(0));
    let responder_calls = calls.clone();

    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation_id}/context"
        )))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(move |request: &Request| {
            let call = responder_calls.fetch_add(1, Ordering::SeqCst);
            let cursor = request
                .url
                .query_pairs()
                .find(|(name, _)| name == "cursor")
                .map(|(_, value)| value.into_owned());
            match call {
                0 => {
                    assert_eq!(cursor, None);
                    ResponseTemplate::new(200).set_body_json(page(
                        vec![message(first_id, 1)],
                        true,
                        Some("frozen"),
                    ))
                }
                1 => {
                    assert_eq!(cursor.as_deref(), Some("frozen"));
                    ResponseTemplate::new(409).set_body_json(json!({
                        "error": {
                            "code": "context_snapshot_stale",
                            "message": "snapshot changed",
                            "request_id": "request-1"
                        }
                    }))
                }
                2 => {
                    assert_eq!(cursor, None, "restart must discard the stale cursor");
                    ResponseTemplate::new(200).set_body_json(page(
                        vec![message(first_id, 1), message(second_id, 2)],
                        false,
                        None,
                    ))
                }
                _ => ResponseTemplate::new(500),
            }
        })
        .expect(3)
        .mount(&server)
        .await;

    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let snapshot = session
        .read_context_snapshot(conversation_id)
        .await
        .expect("fresh snapshot");

    assert_eq!(snapshot.history_revision, 7);
    assert_eq!(snapshot.snapshot_head_sequence_no, 2);
    assert_eq!(snapshot.messages.len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn context_reader_rejects_an_empty_non_advancing_page_without_retrying() {
    let server = MockServer::start().await;
    let conversation_id = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation_id}/context"
        )))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            vec![],
            true,
            Some("non-advancing"),
        )))
        .expect(1)
        .mount(&server)
        .await;

    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    assert_eq!(
        session.read_context_snapshot(conversation_id).await.err(),
        Some(DaChatApiError::InvalidResponse)
    );
}

#[tokio::test]
async fn compaction_activation_and_reconciliation_keep_conflict_and_absence_distinct() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let compaction_id = Uuid::new_v4();
    let activation_path =
        format!("/api/v1/chat/conversations/{conversation_id}/context-compactions:activate");
    Mock::given(method("POST"))
        .and(path(&activation_path))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": {
                "code": "context_conflict",
                "message": "history advanced",
                "request_id": "request-activation"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let error = session
        .activate_context_compaction(conversation_id, &ContextCompactionActivate::default())
        .await
        .err()
        .expect("context conflict");
    assert_eq!(error, DaChatApiError::ContextConflict);
    server.reset().await;

    let get_path =
        format!("/api/v1/chat/conversations/{conversation_id}/context-compactions/{compaction_id}");
    Mock::given(method("GET"))
        .and(path(&get_path))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {
                "code": "not_found",
                "message": "missing",
                "request_id": "request-reconcile"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    assert!(session
        .reconcile_context_compaction(conversation_id, compaction_id)
        .await
        .expect("reconciliation")
        .is_none());
}

fn prepare_request(payload_id: Uuid, tool_call_id: Uuid, exact: &[u8]) -> ToolResultPayloadPrepare {
    ToolResultPayloadPrepare {
        byte_length: exact.len() as i64,
        content_encoding: Some("identity".to_owned()),
        id: payload_id,
        inline_content_base64: None,
        media_type: PayloadMediaType::ApplicationJson,
        metadata: None,
        preview: Some("preview".to_owned()),
        preview_strategy: PayloadPreviewStrategy::Head,
        provider_tool_call_id: "provider-call".to_owned(),
        run_id: Some(Uuid::new_v4()),
        serialization_format: PayloadSerializationFormat::Json,
        serialization_version: 1,
        sha256: format!("{:x}", Sha256::digest(exact)),
        tool_call_id,
        tool_name: "query".to_owned(),
    }
}

#[tokio::test]
async fn prepare_maps_storage_and_integrity_failures_without_string_parsing() {
    let server = MockServer::start().await;
    let conversation_id = Uuid::new_v4();
    let exact = br#"{"rows":[1]}"#;
    let request = prepare_request(Uuid::new_v4(), Uuid::new_v4(), exact);
    let endpoint =
        format!("/api/v1/chat/conversations/{conversation_id}/tool-result-payloads:prepare");
    let cases = [
        (
            "storage_quota_exceeded",
            DaChatApiError::StorageQuotaExceeded,
        ),
        ("storage_unavailable", DaChatApiError::StorageUnavailable),
        (
            "payload_integrity_failed",
            DaChatApiError::PayloadIntegrityFailed,
        ),
        ("payload_too_large", DaChatApiError::PayloadTooLarge),
        ("idempotency_conflict", DaChatApiError::IdempotencyConflict),
    ];

    let session = DaChatSession::for_test(&server.uri(), "test-token").expect("session");
    for (code, expected) in cases {
        Mock::given(method("POST"))
            .and(path(&endpoint))
            .and(header("authorization", "Bearer test-token"))
            .and(header("idempotency-key", "stable-key"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "error": {"code": code, "message": "rejected", "request_id": "request-2"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let error = session
            .prepare_tool_result_payload(conversation_id, &request, "stable-key")
            .await
            .err()
            .expect("typed rejection");
        assert_eq!(error, expected);
        server.reset().await;
    }
}

fn payload_json(
    conversation_id: Uuid,
    payload_id: Uuid,
    tool_call_id: Uuid,
    exact: &[u8],
    status: &str,
) -> Value {
    json!({
        "bound_at": null,
        "byte_length": exact.len(),
        "chat_message_id": null,
        "content_encoding": "identity",
        "conversation_id": conversation_id,
        "created_at": "2026-09-01T00:00:00Z",
        "deletion_requested_at": null,
        "expires_at": null,
        "failed_at": null,
        "failure_code": null,
        "finalized_at": null,
        "id": payload_id,
        "media_type": "application/json",
        "metadata": null,
        "preview": "preview",
        "preview_byte_length": 7,
        "preview_strategy": "head",
        "provider_tool_call_id": "provider-call",
        "retrieval_available": status == "ready",
        "run_id": null,
        "serialization_format": "json",
        "serialization_version": 1,
        "sha256": format!("{:x}", Sha256::digest(exact)),
        "status": status,
        "storage_mode": "object",
        "tenant_id": "tenant",
        "tool_call_id": tool_call_id,
        "tool_name": "query",
        "user_id": "user"
    })
}

fn descriptor_json(payload: &Value) -> Value {
    json!({
        "byte_length": payload["byte_length"],
        "content_encoding": "identity",
        "id": payload["id"],
        "media_type": "application/json",
        "preview": "preview",
        "preview_byte_length": 7,
        "preview_strategy": "head",
        "retrieval_available": true,
        "serialization_format": "json",
        "serialization_version": 1,
        "sha256": payload["sha256"],
        "status": "ready",
        "storage_mode": "object"
    })
}

fn stable_resource_id(kind: &str, conversation_id: Uuid, run_id: Uuid, suffix: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{kind}:{conversation_id}:{run_id}:{suffix}").as_bytes(),
    )
}

fn durable_payload_json(
    conversation_id: Uuid,
    payload_id: Uuid,
    tool_call_id: Uuid,
    run_id: Uuid,
    exact: &[u8],
    preview: &str,
    status: &str,
) -> Value {
    let mut payload = payload_json(conversation_id, payload_id, tool_call_id, exact, status);
    payload["run_id"] = json!(run_id);
    payload["preview"] = json!(preview);
    payload["preview_byte_length"] = json!(preview.len());
    payload["preview_strategy"] = json!("head");
    payload
}

async fn assert_payload_recovery(reconcile_ready: bool) {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "secret-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let call_id = "provider-call";
    let assistant_id = stable_resource_id("assistant-tool-round", conversation_id, run_id, "3");
    let tool_call_id = Uuid::new_v4();
    let payload_id = stable_resource_id("tool-result-payload", conversation_id, run_id, call_id);
    let result_message_id =
        stable_resource_id("tool-result-message", conversation_id, run_id, call_id);
    let exact_result = ToolResult::new(
        Vec::new(),
        Some(json!("x".repeat(70 * 1024))),
        ToolSource::Internal,
    )
    .expect("bounded exact result");
    let exact = exact_result.exact_json_bytes();
    let preview = serde_json::to_string(
        exact_result
            .model_preview(ToolResultBudget::floor())
            .value(),
    )
    .expect("preview");
    assert!(
        preview.len() <= ToolResultBudget::FLOOR_BYTES,
        "the serialized DA preview must fit its wire limit"
    );

    let message_path = format!("/api/v1/chat/conversations/{conversation_id}/messages");
    let message_calls = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let message_responder_calls = message_calls.clone();
    let ready_payload = durable_payload_json(
        conversation_id,
        payload_id,
        tool_call_id,
        run_id,
        &exact,
        &preview,
        "ready",
    );
    let result_descriptor = descriptor_json(&ready_payload);
    Mock::given(method("POST"))
        .and(path(&message_path))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("message JSON");
            let (slot, id, sequence, payloads) = if body["role"] == "assistant" {
                (0, assistant_id, 11, json!([]))
            } else {
                (1, result_message_id, 12, json!([result_descriptor]))
            };
            let call = message_responder_calls[slot].fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(201).set_body_json(json!({
                    "attachments_linked": 0,
                    "id": id,
                    "sequence_no": sequence,
                    "tool_result_payloads": payloads
                }))
            }
        })
        .expect(4)
        .mount(&server)
        .await;

    let tool_call_path = "/api/v1/chat/tool-calls";
    let tool_call_attempts = Arc::new(AtomicUsize::new(0));
    let tool_call_responder_attempts = tool_call_attempts.clone();
    Mock::given(method("POST"))
        .and(path(tool_call_path))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(move |_request: &Request| {
            if tool_call_responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(201).set_body_json(json!({
                    "conversation_id": conversation_id,
                    "id": tool_call_id,
                    "message_id": assistant_id,
                    "provider_tool_call_id": call_id,
                    "run_id": run_id,
                    "status": "completed",
                    "tenant_id": "tenant",
                    "tool_name": "query",
                    "user_id": "user"
                }))
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let prepare_path =
        format!("/api/v1/chat/conversations/{conversation_id}/tool-result-payloads:prepare");
    let prepare_attempts = Arc::new(AtomicUsize::new(0));
    let prepare_responder_attempts = prepare_attempts.clone();
    let pending_payload = durable_payload_json(
        conversation_id,
        payload_id,
        tool_call_id,
        run_id,
        &exact,
        &preview,
        "pending",
    );
    let content_path = format!(
        "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}/content"
    );
    let prepare_response_payload = pending_payload.clone();
    let prepare_content_path = content_path.clone();
    let quota_used_bytes = exact.len();
    Mock::given(method("POST"))
        .and(path(&prepare_path))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(move |_request: &Request| {
            if prepare_responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(201).set_body_json(json!({
                    "payload": prepare_response_payload,
                    "quota_limit_bytes": 20_000_000,
                    "quota_used_bytes": quota_used_bytes,
                    "upload": {
                        "expires_at": null,
                        "method": "PUT",
                        "mode": "local",
                        "required_headers": null,
                        "url": prepare_content_path
                    }
                }))
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let upload_attempts = Arc::new(AtomicUsize::new(0));
    let upload_responder_attempts = upload_attempts.clone();
    Mock::given(method("PUT"))
        .and(path(&content_path))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(move |_request: &Request| {
            if upload_responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(204)
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let finalize_path = format!(
        "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}:finalize"
    );
    let finalize_attempts = Arc::new(AtomicUsize::new(0));
    let finalize_responder_attempts = finalize_attempts.clone();
    let final_ready_payload = ready_payload.clone();
    Mock::given(method("POST"))
        .and(path(&finalize_path))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(move |_request: &Request| {
            let attempt = finalize_responder_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200).set_body_json(final_ready_payload.clone())
            }
        })
        .expect(if reconcile_ready { 1 } else { 2 })
        .mount(&server)
        .await;

    let get_payload_path =
        format!("/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}");
    let reconciled_payload = if reconcile_ready {
        ready_payload.clone()
    } else {
        durable_payload_json(
            conversation_id,
            payload_id,
            tool_call_id,
            run_id,
            &exact,
            &preview,
            "pending",
        )
    };
    Mock::given(method("GET"))
        .and(path(&get_payload_path))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reconciled_payload))
        .expect(1)
        .mount(&server)
        .await;

    let durable_progress = AtomicUsize::new(0);
    let mut on_progress = || {
        durable_progress.fetch_add(1, Ordering::SeqCst);
    };
    let durable = session
        .record_tool_round(
            conversation_id,
            ToolRoundRecord {
                run_id,
                round: 3,
                text: String::new(),
                calls: vec![ToolRoundCall {
                    call_id: call_id.to_owned(),
                    tool_name: "query".to_owned(),
                    arguments: r#"{"sql":"select 1"}"#.to_owned(),
                    result: r#"{"status":"completed","result":"preview"}"#.to_owned(),
                    exact_result: Some(exact_result),
                }],
                budget: crate::tools::ToolResultBudget::floor(),
            },
            &mut on_progress,
        )
        .await
        .expect("durable tool round");
    assert_eq!(durable.len(), 1);
    assert!(durable[0].contains(&payload_id.to_string()));
    assert_eq!(
        durable_progress.load(Ordering::SeqCst),
        6,
        "each confirmed durable write reports progress"
    );

    let requests = server.received_requests().await.expect("requests");
    let message_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == message_path)
        .collect();
    assert_eq!(message_requests.len(), 4);
    for start in [0, 2] {
        let pair = &message_requests[start..start + 2];
        assert_eq!(pair[0].body, pair[1].body, "message retry body drifted");
        assert_eq!(
            pair[0].headers.get("idempotency-key"),
            pair[1].headers.get("idempotency-key"),
            "message retry identity drifted"
        );
        let body: Value = serde_json::from_slice(&pair[0].body).expect("message body");
        let expected_key = if body["role"] == "assistant" {
            assistant_id.to_string()
        } else {
            result_message_id.to_string()
        };
        assert_eq!(
            pair[0]
                .headers
                .get("idempotency-key")
                .and_then(|value| value.to_str().ok()),
            Some(expected_key.as_str())
        );
    }
    let tool_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == tool_call_path)
        .collect();
    assert_eq!(tool_requests[0].body, tool_requests[1].body);
    assert_eq!(
        tool_requests[0].headers.get("idempotency-key"),
        tool_requests[1].headers.get("idempotency-key")
    );
    let expected_tool_key =
        stable_resource_id("tool-call", conversation_id, run_id, call_id).to_string();
    assert_eq!(
        tool_requests[0]
            .headers
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok()),
        Some(expected_tool_key.as_str())
    );
    let prepare_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == prepare_path)
        .collect();
    assert_eq!(prepare_requests[0].body, prepare_requests[1].body);
    assert_eq!(
        prepare_requests[0].headers.get("idempotency-key"),
        prepare_requests[1].headers.get("idempotency-key")
    );
    let expected_payload_key = payload_id.to_string();
    assert_eq!(
        prepare_requests[0]
            .headers
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok()),
        Some(expected_payload_key.as_str())
    );
    let prepare_body: Value =
        serde_json::from_slice(&prepare_requests[0].body).expect("prepare body");
    assert!(
        prepare_body["preview"].as_str().expect("preview").len() <= ToolResultBudget::FLOOR_BYTES
    );
    let upload_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == content_path)
        .collect();
    assert_eq!(upload_requests.len(), 2);
    assert_eq!(upload_requests[0].body, exact);
    assert_eq!(upload_requests[1].body, exact);
}

#[tokio::test]
async fn payload_recovery_reconciles_a_ready_finalize_and_keeps_all_retry_ids_stable() {
    assert_payload_recovery(true).await;
}

#[tokio::test]
async fn payload_recovery_retries_finalize_only_after_reconciliation_still_shows_pending() {
    assert_payload_recovery(false).await;
}

#[tokio::test]
async fn local_upload_exact_range_and_message_binding_use_real_http_boundaries() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "secret-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let payload_id = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    let exact = br#"{"rows":[1,2,3]}"#;
    let content_path = format!(
        "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}/content"
    );
    let pending: ToolResultPayload = serde_json::from_value(payload_json(
        conversation_id,
        payload_id,
        tool_call_id,
        exact,
        "pending",
    ))
    .expect("payload");
    let upload: UploadInstruction = serde_json::from_value(json!({
        "expires_at": null,
        "method": "PUT",
        "mode": "local",
        "required_headers": null,
        "url": content_path
    }))
    .expect("upload instruction");

    Mock::given(method("PUT"))
        .and(path(&content_path))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("content-type", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    session
        .upload_tool_result_payload(conversation_id, &pending, &upload, exact)
        .await
        .expect("exact upload");
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests[0].body, exact);
    server.reset().await;

    let ready = payload_json(conversation_id, payload_id, tool_call_id, exact, "ready");
    let descriptor_value = descriptor_json(&ready);
    let descriptor: ToolResultPayloadDescriptor =
        serde_json::from_value(descriptor_value.clone()).expect("descriptor");
    Mock::given(method("GET"))
        .and(path(&content_path))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("range", format!("bytes=0-{}", exact.len() - 1)))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header(
                    "content-range",
                    format!("bytes 0-{}/{}", exact.len() - 1, exact.len()),
                )
                .set_body_bytes(exact),
        )
        .expect(1)
        .mount(&server)
        .await;
    let range = session
        .read_tool_result_range(conversation_id, &descriptor, 0, exact.len())
        .await
        .expect("exact range");
    assert_eq!(range.bytes, exact);
    assert_eq!(range.start, 0);
    assert_eq!(range.end_inclusive, exact.len() - 1);
    assert_eq!(range.total_byte_length, exact.len());
    server.reset().await;
    assert_eq!(
        session
            .read_tool_result_range(conversation_id, &descriptor, exact.len() - 1, 2)
            .await
            .err(),
        Some(DaChatApiError::Validation)
    );
    assert!(server
        .received_requests()
        .await
        .expect("requests")
        .is_empty());

    let message_path = format!("/api/v1/chat/conversations/{conversation_id}/messages");
    Mock::given(method("POST"))
        .and(path(&message_path))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("idempotency-key", "message-key"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "attachments_linked": 0,
            "id": message_id,
            "sequence_no": 9,
            "tool_result_payloads": [descriptor_value]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let inserted = session
        .create_tool_result_message_with_payloads(
            conversation_id,
            message_id,
            "provider-call",
            "query",
            "preview",
            vec![payload_id],
            "message-key",
        )
        .await
        .expect("bound message");
    assert_eq!(inserted.id, message_id);
    assert_eq!(inserted.sequence_no, 9);
    let requests = server.received_requests().await.expect("requests");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("message body");
    assert_eq!(body["id"], message_id.to_string());
    assert_eq!(body["tool_result_payload_ids"][0], payload_id.to_string());
}

#[tokio::test]
async fn staging_inline_payload_moves_from_pending_to_ready_without_a_second_upload() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "secret-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let payload_id = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let exact = br#"{"rows":[1,2,3]}"#;
    let request = ToolResultPayloadPrepare {
        id: payload_id,
        run_id: None,
        tool_call_id,
        provider_tool_call_id: "provider-call".to_owned(),
        tool_name: "query".to_owned(),
        media_type: PayloadMediaType::ApplicationJson,
        serialization_format: PayloadSerializationFormat::Json,
        serialization_version: 1,
        content_encoding: Some("identity".to_owned()),
        byte_length: i64::try_from(exact.len()).expect("small fixture"),
        sha256: format!("{:x}", Sha256::digest(exact)),
        preview: Some("preview".to_owned()),
        preview_strategy: PayloadPreviewStrategy::Head,
        inline_content_base64: Some(base64::engine::general_purpose::STANDARD.encode(exact)),
        metadata: None,
    };
    let mut pending_payload =
        payload_json(conversation_id, payload_id, tool_call_id, exact, "pending");
    pending_payload["storage_mode"] = json!("inline");
    let mut ready_payload = payload_json(conversation_id, payload_id, tool_call_id, exact, "ready");
    ready_payload["storage_mode"] = json!("inline");
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads:prepare"
        )))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("idempotency-key", "payload-key"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "payload": pending_payload,
            "upload": {
                "expires_at": null,
                "method": null,
                "mode": "inline",
                "required_headers": null,
                "url": null
            },
            "quota_limit_bytes": 1024,
            "quota_used_bytes": exact.len()
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/chat/conversations/{conversation_id}/tool-result-payloads/{payload_id}:finalize"
        )))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ready_payload))
        .expect(1)
        .mount(&server)
        .await;

    let prepared = session
        .prepare_tool_result_payload(conversation_id, &request, "payload-key")
        .await
        .expect("staging returns pending inline payloads before finalize");

    assert!(matches!(
        prepared.payload.status,
        crate::clients::da::generated::PayloadStatus::Pending
    ));
    assert!(matches!(
        prepared.upload.mode,
        crate::clients::da::generated::UploadInstructionMode::Inline
    ));
    session
        .upload_tool_result_payload(conversation_id, &prepared.payload, &prepared.upload, exact)
        .await
        .expect("prepare already transferred inline bytes");
    let finalized = session
        .finalize_tool_result_payload_reconciled(conversation_id, payload_id)
        .await
        .expect("inline payload finalizes to ready");
    assert!(matches!(
        finalized.status,
        crate::clients::da::generated::PayloadStatus::Ready
    ));

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        requests.len(),
        2,
        "prepare and finalize are the only writes"
    );
}

#[tokio::test]
async fn pending_inline_payload_needs_no_second_upload_request() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "secret-token").expect("session");
    let conversation_id = Uuid::new_v4();
    let exact = br#"{"rows":[1,2,3]}"#;
    let mut payload = payload_json(
        conversation_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        exact,
        "pending",
    );
    payload["storage_mode"] = json!("inline");
    let payload: ToolResultPayload =
        serde_json::from_value(payload).expect("pending inline payload");
    let upload: UploadInstruction = serde_json::from_value(json!({
        "expires_at": null,
        "method": null,
        "mode": "inline",
        "required_headers": null,
        "url": null
    }))
    .expect("inline instruction");

    session
        .upload_tool_result_payload(conversation_id, &payload, &upload, exact)
        .await
        .expect("inline bytes were already transferred during prepare");
    assert!(server
        .received_requests()
        .await
        .expect("requests")
        .is_empty());
}

#[tokio::test]
async fn presigned_upload_never_leaks_the_da_bearer() {
    let server = MockServer::start().await;
    let session = DaChatSession::for_test(&server.uri(), "do-not-forward").expect("session");
    let conversation_id = Uuid::new_v4();
    let payload_id = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let exact = b"exact object bytes";
    let payload: ToolResultPayload = serde_json::from_value(payload_json(
        conversation_id,
        payload_id,
        tool_call_id,
        exact,
        "pending",
    ))
    .expect("payload");
    let target_path = format!("/object-store/{payload_id}");
    let upload: UploadInstruction = serde_json::from_value(json!({
        "expires_at": "2099-09-01T01:00:00Z",
        "method": "PUT",
        "mode": "presigned",
        "required_headers": {"x-upload-key": "bound-value"},
        "url": format!("{}{target_path}", server.uri())
    }))
    .expect("upload instruction");
    Mock::given(method("PUT"))
        .and(path(&target_path))
        .and(header("x-upload-key", "bound-value"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    session
        .upload_tool_result_payload(conversation_id, &payload, &upload, exact)
        .await
        .expect("presigned upload");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(requests[0].body, exact);
}
