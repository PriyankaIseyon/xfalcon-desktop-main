use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request, ResponseTemplate,
};
use xfalcon_contracts::custom_models::{
    CustomModelCreateInput, CustomModelSampling, CustomModelUpdateInput,
};

use super::super::chat_api::{CustomModelApiError, DaChatSession};

const ROUTE: &str = "/api/v1/chat/custom-models";
const MODEL_ID: &str = "b575f456-1c79-4bc1-a80a-7892a44c4d2a";
const KEY_ID: &str = "8175d447-cfe7-4bbb-92b7-339b2ded2ab8";
const BEARER_CANARY: &str = "wire-custom-model-bearer-canary";
const TEST_SECRET: &str = "test-secret-not-a-real-provider-key";

fn session(server: &MockServer) -> DaChatSession {
    DaChatSession::for_test(&server.uri(), BEARER_CANARY).expect("valid loopback DA session")
}

fn model_row() -> Value {
    custom_model_row(MODEL_ID, true, true, Some(KEY_ID))
}

fn custom_model_row(
    id: &str,
    is_enabled: bool,
    is_active: bool,
    provider_key_id: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "model_id": format!("custom/{id}"),
        "provider": "custom_openai",
        "provider_parent": "custom_openai",
        "provider_sub": null,
        "display_name": "Private model",
        "context_length": 128_000,
        "max_output_tokens": 16_000,
        "input_price_per_m": 0.0,
        "output_price_per_m": 0.0,
        "released_at": null,
        "is_chat": true,
        "is_enabled": is_enabled,
        "supports_streaming": true,
        "supports_tools": true,
        "supports_vision": false,
        "metadata": {},
        "created_at": null,
        "updated_at": null,
        "is_active": is_active,
        "tenant_id": "tenant-1",
        "supports_reasoning": false,
        "supports_extended_thinking": false,
        "reasoning_param_style": "none",
        "default_thinking_budget_tokens": null,
        "owner_user_id": "user-1",
        "base_url": "https://models.example.com/v1",
        "upstream_model_id": "organization/private-model",
        "provider_key_id": provider_key_id
    })
}

fn product_model_row() -> Value {
    let mut row = model_row();
    row["id"] = json!("a89bf65e-cb22-4415-b876-2fb728829249");
    row["model_id"] = json!("~anthropic/claude-sonnet-latest");
    row["provider"] = json!("openrouter");
    row["provider_parent"] = json!("openrouter");
    row["owner_user_id"] = Value::Null;
    row["base_url"] = Value::Null;
    row["upstream_model_id"] = Value::Null;
    row["provider_key_id"] = Value::Null;
    row
}

fn page(items: Vec<Value>, next_cursor: Option<&str>) -> Value {
    json!({
        "items": items,
        "next_cursor": next_cursor,
        "has_more": next_cursor.is_some(),
        "limit": 200
    })
}

fn error_body(code: &str, message: &str) -> Value {
    json!({
        "error": {
            "code": code,
            "message": message,
            "request_id": "wire-request-1"
        }
    })
}

fn create_input() -> CustomModelCreateInput {
    CustomModelCreateInput {
        base_url: "https://models.example.com/v1".to_owned(),
        secret: TEST_SECRET.to_owned(),
        upstream_model_id: "organization/private-model".to_owned(),
        display_name: "Private model".to_owned(),
        context_length: 128_000,
        max_output_tokens: 16_000,
        supports_tools: true,
        supports_vision: false,
        sampling: CustomModelSampling::default(),
    }
}

fn sparse_update() -> CustomModelUpdateInput {
    CustomModelUpdateInput {
        base_url: None,
        secret: None,
        is_enabled: Some(false),
        is_active: Some(false),
        upstream_model_id: None,
        display_name: None,
        context_length: None,
        max_output_tokens: None,
        supports_tools: None,
        supports_vision: None,
        sampling: None,
    }
}

fn assert_authorized(request: &Request) {
    assert_eq!(
        request
            .headers
            .get("authorization")
            .expect("bearer attached")
            .to_str()
            .expect("header text"),
        format!("Bearer {BEARER_CANARY}")
    );
}

fn query(request: &Request) -> BTreeMap<String, String> {
    request.url.query_pairs().into_owned().collect()
}

fn sorted_keys(body: &Value) -> Vec<&str> {
    let mut keys = body
        .as_object()
        .expect("request body is an object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[tokio::test]
async fn picker_listing_reads_every_page_and_keeps_a_private_model_from_page_two() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method(Method::GET))
        .and(path("/api/v1/chat/models"))
        .respond_with({
            let calls = Arc::clone(&calls);
            move |_request: &Request| match calls.fetch_add(1, Ordering::SeqCst) {
                0 => ResponseTemplate::new(200)
                    .set_body_json(page(vec![product_model_row()], Some("opaque/cursor +=="))),
                1 => ResponseTemplate::new(200).set_body_json(page(vec![model_row()], None)),
                _ => panic!("model listing read beyond the terminal page"),
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let models = session(&server)
        .list_models()
        .await
        .expect("both pages load");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].provider, "openrouter");
    assert_eq!(models[1].provider, "custom_openai");
    assert_eq!(models[1].model_id, format!("custom/{MODEL_ID}"));

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.url.path(), "/api/v1/chat/models");
        assert_authorized(request);
    }
    assert_eq!(
        query(&requests[0]),
        BTreeMap::from([
            ("is_active".to_owned(), "true".to_owned()),
            ("is_enabled".to_owned(), "true".to_owned()),
            ("limit".to_owned(), "200".to_owned()),
        ])
    );
    assert_eq!(
        query(&requests[1]),
        BTreeMap::from([
            ("cursor".to_owned(), "opaque/cursor +==".to_owned()),
            ("is_active".to_owned(), "true".to_owned()),
            ("is_enabled".to_owned(), "true".to_owned()),
            ("limit".to_owned(), "200".to_owned()),
        ])
    );
}

#[tokio::test]
async fn management_listing_includes_all_live_key_states_and_excludes_removed_rows() {
    let server = MockServer::start().await;
    Mock::given(method(Method::GET))
        .and(path("/api/v1/chat/models"))
        .respond_with(|request: &Request| {
            let query = query(request);
            let is_enabled = query
                .get("is_enabled")
                .expect("enabled filter")
                .parse::<bool>()
                .expect("boolean enabled filter");
            let is_active = query
                .get("is_active")
                .expect("active filter")
                .parse::<bool>()
                .expect("boolean active filter");
            let id = match (is_enabled, is_active) {
                (true, true) => "34d36f1f-bc54-4855-9146-073aa705c164",
                (true, false) => "ced29382-71f8-4725-8a0e-a0d813749ecf",
                (false, true) => "854c8e63-e2e5-4620-a285-9c3c166ebc8f",
                (false, false) => "f8806996-f3a2-457f-a836-468674216f08",
            };
            let mut items = vec![custom_model_row(id, is_enabled, is_active, Some(KEY_ID))];
            if !is_enabled && !is_active {
                items.push(custom_model_row(
                    "7d3b153e-6c06-4997-87dc-fdbf76d97535",
                    false,
                    false,
                    None,
                ));
            }
            ResponseTemplate::new(200).set_body_json(page(items, None))
        })
        .expect(4)
        .mount(&server)
        .await;

    let models = session(&server)
        .list_custom_models_for_management()
        .await
        .expect("all management states load");
    assert_eq!(models.len(), 4, "revoked-key history must stay hidden");
    assert_eq!(
        models
            .iter()
            .map(|model| (model.is_enabled, model.is_active))
            .collect::<Vec<_>>(),
        vec![(true, true), (true, false), (false, true), (false, false)]
    );
    assert!(models.iter().all(|model| model.provider_key_id.is_some()));
    assert!(!models
        .iter()
        .any(|model| model.id.to_string() == "7d3b153e-6c06-4997-87dc-fdbf76d97535"));

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 4);
    let states = requests
        .iter()
        .map(|request| {
            assert_authorized(request);
            let query = query(request);
            assert_eq!(query.get("limit").map(String::as_str), Some("200"));
            assert_eq!(
                query.get("provider").map(String::as_str),
                Some("custom_openai")
            );
            (query["is_enabled"].clone(), query["is_active"].clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![
            ("true".to_owned(), "true".to_owned()),
            ("true".to_owned(), "false".to_owned()),
            ("false".to_owned(), "true".to_owned()),
            ("false".to_owned(), "false".to_owned()),
        ]
    );
}

#[tokio::test]
async fn management_listing_keeps_one_card_when_a_model_changes_state_between_queries() {
    let server = MockServer::start().await;
    Mock::given(method(Method::GET))
        .and(path("/api/v1/chat/models"))
        .respond_with(|request: &Request| {
            let query = query(request);
            let state = (query["is_enabled"].as_str(), query["is_active"].as_str());
            let items = match state {
                ("true", "true") => vec![model_row()],
                ("false", "true") => vec![custom_model_row(MODEL_ID, false, true, Some(KEY_ID))],
                _ => Vec::new(),
            };
            ResponseTemplate::new(200).set_body_json(page(items, None))
        })
        .expect(4)
        .mount(&server)
        .await;

    let models = session(&server)
        .list_custom_models_for_management()
        .await
        .expect("all states load");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.to_string(), MODEL_ID);
    assert!(!models[0].is_enabled, "the later snapshot wins");
}

#[tokio::test]
async fn create_sends_the_explicit_model_settings() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(201).set_body_json(model_row()))
        .expect(1)
        .mount(&server)
        .await;

    let created = session(&server)
        .create_custom_model(create_input())
        .await
        .expect("create succeeds");
    assert_eq!(created.id, Uuid::parse_str(MODEL_ID).expect("fixture UUID"));
    assert_eq!(created.model_id, format!("custom/{MODEL_ID}"));

    let requests = server.received_requests().await.expect("recorded request");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.path(), ROUTE);
    assert!(request.url.query().is_none());
    assert_authorized(request);
    let body: Value = serde_json::from_slice(&request.body).expect("request JSON");
    assert_eq!(
        sorted_keys(&body),
        [
            "base_url",
            "context_length",
            "display_name",
            "max_output_tokens",
            "secret",
            "supports_tools",
            "supports_vision",
            "upstream_model_id",
        ]
    );
    assert_eq!(body["base_url"], "https://models.example.com/v1");
    assert_eq!(body["secret"], TEST_SECRET);
    assert_eq!(body["upstream_model_id"], "organization/private-model");
    assert_eq!(body["display_name"], "Private model");
    assert_eq!(body["context_length"], 128_000);
    assert_eq!(body["max_output_tokens"], 16_000);
    assert_eq!(body["supports_tools"], true);
    assert_eq!(body["supports_vision"], false);
}

fn gateway_row(id: &str) -> Value {
    let mut row = custom_model_row(id, true, true, Some(KEY_ID));
    row["base_url"] = json!("https://llm-gateway.iseyon.workers.dev/v1");
    row["upstream_model_id"] = json!("ollama/qwen3.6:35b-a3b-coding-nvfp4");
    row["context_length"] = json!(32_768);
    row["max_output_tokens"] = json!(4_096);
    row["supports_vision"] = json!(false);
    row
}

#[tokio::test]
async fn stored_gateway_rows_preserve_explicit_capacity_and_capabilities() {
    let server = MockServer::start().await;
    Mock::given(method(Method::GET))
        .and(path("/api/v1/chat/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            vec![
                gateway_row(MODEL_ID),
                model_row_with_id("34d36f1f-bc54-4855-9146-073aa705c164"),
            ],
            None,
        )))
        .mount(&server)
        .await;

    let models = session(&server).list_models().await.expect("models load");

    let gateway = &models[0];
    assert_eq!(
        gateway.upstream_model_id.as_deref(),
        Some("ollama/qwen3.6:35b-a3b-coding-nvfp4")
    );
    assert_eq!(gateway.context_length, 32_768);
    assert_eq!(gateway.max_output_tokens, Some(4_096));
    assert!(gateway.supports_tools);
    assert!(!gateway.supports_vision);
    let other = &models[1];
    assert_eq!(
        other.context_length, 128_000,
        "another endpoint is left as stored"
    );
    assert_eq!(
        other.upstream_model_id.as_deref(),
        Some("organization/private-model")
    );
    assert!(!other.supports_vision);
}

fn model_row_with_id(id: &str) -> Value {
    custom_model_row(id, true, true, Some(KEY_ID))
}

#[tokio::test]
async fn patch_targets_the_model_and_sends_only_changed_fields() {
    let server = MockServer::start().await;
    let route = format!("{ROUTE}/{MODEL_ID}");
    Mock::given(method(Method::PATCH))
        .and(path(route.as_str()))
        .respond_with(|request: &Request| {
            let patch: Value = serde_json::from_slice(&request.body).expect("patch JSON");
            let mut row = model_row();
            row["is_enabled"] = patch["is_enabled"].clone();
            row["is_active"] = patch["is_active"].clone();
            ResponseTemplate::new(200).set_body_json(row)
        })
        .expect(1)
        .mount(&server)
        .await;

    let updated = session(&server)
        .update_custom_model(
            Uuid::parse_str(MODEL_ID).expect("fixture UUID"),
            sparse_update(),
        )
        .await
        .expect("update succeeds");
    assert_eq!(updated.id, Uuid::parse_str(MODEL_ID).expect("fixture UUID"));
    assert!(!updated.is_enabled);
    assert!(!updated.is_active);

    let requests = server.received_requests().await.expect("recorded request");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::PATCH);
    assert_eq!(request.url.path(), route);
    assert!(request.url.query().is_none());
    assert_authorized(request);
    let body: Value = serde_json::from_slice(&request.body).expect("request JSON");
    assert_eq!(sorted_keys(&body), ["is_active", "is_enabled"]);
    assert_eq!(body["is_enabled"], false);
    assert_eq!(body["is_active"], false);
    assert!(body.get("secret").is_none());
    assert!(body.get("base_url").is_none());
}

#[tokio::test]
async fn patch_sends_explicit_model_configuration_but_not_device_sampling() {
    let server = MockServer::start().await;
    let route = format!("{ROUTE}/{MODEL_ID}");
    Mock::given(method(Method::PATCH))
        .and(path(route.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(model_row()))
        .expect(1)
        .mount(&server)
        .await;
    let input = CustomModelUpdateInput {
        base_url: None,
        secret: None,
        is_enabled: None,
        is_active: None,
        upstream_model_id: Some("organization/model-v2".to_owned()),
        display_name: Some("Model V2".to_owned()),
        context_length: Some(96_000),
        max_output_tokens: Some(12_000),
        supports_tools: Some(true),
        supports_vision: Some(true),
        sampling: Some(CustomModelSampling {
            temperature: Some(0.3),
            top_p: Some(0.9),
        }),
    };

    session(&server)
        .update_custom_model(Uuid::parse_str(MODEL_ID).expect("fixture UUID"), input)
        .await
        .expect("update succeeds");

    let requests = server.received_requests().await.expect("recorded request");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("request JSON");
    assert_eq!(
        sorted_keys(&body),
        [
            "context_length",
            "display_name",
            "max_output_tokens",
            "supports_tools",
            "supports_vision",
            "upstream_model_id",
        ]
    );
    assert_eq!(body["upstream_model_id"], "organization/model-v2");
    assert_eq!(body["display_name"], "Model V2");
    assert_eq!(body["context_length"], 96_000);
    assert_eq!(body["max_output_tokens"], 12_000);
    assert_eq!(body["supports_tools"], true);
    assert_eq!(body["supports_vision"], true);
    assert!(body.get("sampling").is_none());
}

#[tokio::test]
async fn management_statuses_keep_their_distinct_error_meanings() {
    let cases = [
        (
            422,
            "validation_error",
            "base_url must be an https URL",
            "validation",
        ),
        (
            403,
            "insufficient_scope",
            "private models disabled",
            "forbidden",
        ),
        (409, "conflict", "private model limit reached", "conflict"),
    ];
    for (status, code, message, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method(Method::POST))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body(code, message)))
            .expect(1)
            .mount(&server)
            .await;

        let error = session(&server)
            .create_custom_model(create_input())
            .await
            .err()
            .expect("status must fail");
        match (expected, error) {
            ("validation", CustomModelApiError::Validation(message)) => {
                assert_eq!(message, "base_url must be an https URL")
            }
            ("forbidden", CustomModelApiError::Forbidden)
            | ("conflict", CustomModelApiError::Conflict) => {}
            (_, error) => panic!("status {status} mapped to {error:?}"),
        }
    }

    let server = MockServer::start().await;
    Mock::given(method(Method::PATCH))
        .and(path(format!("{ROUTE}/{MODEL_ID}")))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(error_body("not_found", "model not found")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = session(&server)
        .update_custom_model(
            Uuid::parse_str(MODEL_ID).expect("fixture UUID"),
            sparse_update(),
        )
        .await
        .err()
        .expect("missing model must fail");
    assert!(matches!(error, CustomModelApiError::Removed));
}

#[tokio::test]
async fn da_validation_prefix_is_removed_without_exposing_a_submitted_secret() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(422).set_body_json(error_body(
            "validation_error",
            "request failed validation: body.base_url must be an https URL",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let error = session(&server)
        .create_custom_model(create_input())
        .await
        .err()
        .expect("DA rejected the URL");
    assert!(matches!(
        error,
        CustomModelApiError::Validation(message) if message == "base_url must be an https URL"
    ));

    let secret_server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(422).set_body_json(error_body(
            "validation_error",
            &format!("request failed validation: body.secret includes {TEST_SECRET}"),
        )))
        .expect(1)
        .mount(&secret_server)
        .await;
    let error = session(&secret_server)
        .create_custom_model(create_input())
        .await
        .err()
        .expect("DA rejected the key");
    assert!(matches!(
        error,
        CustomModelApiError::Validation(message)
            if message == "Check the private model fields and try again."
    ));
}

#[tokio::test]
async fn delete_is_idempotent_when_da_acknowledges_the_removed_model() {
    let server = MockServer::start().await;
    let id = Uuid::parse_str(MODEL_ID).expect("fixture UUID");
    let route = format!("{ROUTE}/{MODEL_ID}");
    Mock::given(method(Method::DELETE))
        .and(path(route.as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": MODEL_ID, "deleted": true })),
        )
        .expect(2)
        .mount(&server)
        .await;

    let da = session(&server);
    da.delete_custom_model(id)
        .await
        .expect("first delete succeeds");
    da.delete_custom_model(id)
        .await
        .expect("repeated delete succeeds");

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.method, Method::DELETE);
        assert_eq!(request.url.path(), route);
        assert!(request.url.query().is_none());
        assert!(request.body.is_empty());
        assert_authorized(&request);
    }
}
