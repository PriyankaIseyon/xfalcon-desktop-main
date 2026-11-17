use std::sync::Arc;

use serde_json::{json, Value};
use tempfile::TempDir;
use uuid::Uuid;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, ResponseTemplate,
};
use xfalcon_contracts::chat::ChatSubmitStatus;
use xfalcon_contracts::mcp::McpServerId;
use xfalcon_contracts::ConversationId;

use crate::auth::{AuthConfig, AuthSessionStore};
use crate::chat::mcp_selection::McpConversationBinding;
use crate::chat::service::{ChatService, TurnRevision};
use crate::clients::da::chat_api::SONNET_MODEL_ID;
use crate::credentials::CredentialRuntime;
use crate::persistence::InMemorySecretStore;
use crate::security::account_cleanup::CurrentAccountScope;

const TENANT: &str = "tenant-1";
const USER: &str = "user-1";
const ACCOUNT: &str = "tenant-1:user-1";
const SERVER_ID: &str = "9c7ed740-8149-4f0c-8717-e4c99897e128";
const ENDPOINT: &str = "https://mcp.example.test/mcp";
const MODEL_ID: &str = SONNET_MODEL_ID;
const REGISTRY_PATH: &str = "/api/v1/chat/mcp-servers";

fn service(server: &MockServer, root: &TempDir) -> ChatService {
    service_with_state(server, root).0
}

fn service_with_state(
    server: &MockServer,
    root: &TempDir,
) -> (ChatService, Arc<AuthSessionStore>, Arc<CurrentAccountScope>) {
    let sessions = Arc::new(AuthSessionStore::default());
    sessions.set_authenticated(
        xfalcon_contracts::auth::SessionProjection::authenticated(
            xfalcon_contracts::auth::IdentityProjection::new(USER, TENANT, "User", None),
            chrono::Utc::now().timestamp_millis() + 3_600_000,
        ),
        "access",
        None,
    );
    let accounts = Arc::new(CurrentAccountScope::default());
    accounts.bind_native(ACCOUNT).expect("account scope");
    let secrets = Arc::new(InMemorySecretStore::default());
    let service = ChatService::new(
        sessions.clone(),
        accounts.clone(),
        root.path().to_owned(),
        secrets.clone(),
        Arc::new(CredentialRuntime::new(
            sessions.clone(),
            Arc::new(InMemorySecretStore::default()),
        )),
        Arc::new(crate::clients::da::model_catalog::ModelCatalogCache::default()),
        Arc::new(crate::projects::ProjectsService::new(
            root.path().to_owned(),
            accounts.clone(),
            secrets.clone(),
        )),
        Arc::new(crate::approvals::ApprovalsService::new(
            root.path().to_owned(),
            accounts.clone(),
            secrets,
        )),
    );
    service.set_auth_config_for_test(AuthConfig {
        da_base_url: server.uri().parse().expect("DA URL"),
        casdoor_issuer: "https://auth.example".parse().expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 0,
        mcp_client_id: None,
        mcp_client_secret: None,
    });
    (service, sessions, accounts)
}

fn selected_binding(endpoint: &str) -> Value {
    McpConversationBinding::server(McpServerId::from(SERVER_ID), endpoint.to_owned())
        .expect("binding")
        .metadata()
        .into_iter()
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn server_row(endpoint: &str) -> Value {
    json!({
        "id": SERVER_ID,
        "tenant_id": TENANT,
        "user_id": USER,
        "name": "crm",
        "display_name": "CRM",
        "description": null,
        "transport": "http",
        "url": endpoint,
        "command": null,
        "args": null,
        "icon_url": null,
        "auth_type": "none",
        "auth_config": null,
        "is_enabled": true,
        "status": "connected",
        "status_message": null,
        "last_health_check": null,
        "permission_mode": "ask",
        "metadata": {"origin": "xfalcon-desktop"},
        "created_at": "1970-01-01T00:00:00Z",
        "updated_at": "1970-01-01T00:00:00Z",
        "visibility": "private"
    })
}

async fn mount_registry(server: &MockServer, endpoint: &str) {
    let payload = json!({"items": [server_row(endpoint)], "has_more": false, "next_cursor": null});
    Mock::given(method("GET"))
        .and(path(REGISTRY_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(server)
        .await;
}

async fn mount_models(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/chat/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{
                "id": "00000000-0000-0000-0000-0000000000aa",
                "model_id": MODEL_ID,
                "provider": "openrouter",
                "provider_parent": "openrouter",
                "provider_sub": null,
                "display_name": "Sonnet",
                "context_length": crate::clients::da::chat_api::MIN_NATIVE_CONTEXT_TOKENS,
                "max_output_tokens": 8192,
                "input_price_per_m": 1.0,
                "output_price_per_m": 2.0,
                "released_at": null,
                "is_chat": true,
                "is_enabled": true,
                "supports_streaming": true,
                "supports_tools": true,
                "supports_vision": false,
                "metadata": {},
                "created_at": null,
                "updated_at": null,
                "is_active": true,
                "tenant_id": TENANT,
                "supports_reasoning": false,
                "supports_extended_thinking": false,
                "reasoning_param_style": "none",
                "default_thinking_budget_tokens": null,
                "owner_user_id": null,
                "base_url": null,
                "upstream_model_id": null,
                "provider_key_id": null
            }]
        })))
        .mount(server)
        .await;
}

async fn mount_existing_conversation(
    server: &MockServer,
    id: Uuid,
    metadata: Value,
    user_id: &str,
) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/chat/bootstrap/conversations/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "conversation": {
                "id": id,
                "tenant_id": TENANT,
                "title": "Selected",
                "model": MODEL_ID
            },
            "chats": [],
            "tool_calls": [],
            "artifacts": [],
            "attached_server_ids": []
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/chat/chat-attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [], "next_cursor": null, "has_more": false
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/chat/conversations/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": id,
            "tenant_id": TENANT,
            "user_id": user_id,
            "title": "Selected",
            "model": MODEL_ID,
            "metadata": metadata,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn selection_get_reads_persisted_metadata_and_rejects_corruption_or_foreign_owner() {
    let da = MockServer::start().await;
    let root = TempDir::new().expect("local data");
    let id = Uuid::new_v4();
    mount_existing_conversation(&da, id, selected_binding(ENDPOINT), USER).await;
    let chat_service = service(&da, &root);

    let selection = chat_service
        .mcp_selection_get(ConversationId::from(id.to_string()))
        .await
        .expect("selection reads from DA metadata");
    assert_eq!(
        selection.server_id.as_ref().map(McpServerId::as_str),
        Some(SERVER_ID)
    );

    let corrupt_da = MockServer::start().await;
    let corrupt_id = Uuid::new_v4();
    mount_existing_conversation(
        &corrupt_da,
        corrupt_id,
        json!({"xfalcon_desktop_mcp": {"version": 1, "server_id": SERVER_ID}}),
        USER,
    )
    .await;
    let corrupt = service(&corrupt_da, &root)
        .mcp_selection_get(ConversationId::from(corrupt_id.to_string()))
        .await;
    assert!(corrupt.is_err(), "corrupt metadata must not become All");

    let foreign_da = MockServer::start().await;
    let foreign_id = Uuid::new_v4();
    mount_existing_conversation(
        &foreign_da,
        foreign_id,
        selected_binding(ENDPOINT),
        "other-user",
    )
    .await;
    let foreign = service(&foreign_da, &root)
        .mcp_selection_get(ConversationId::from(foreign_id.to_string()))
        .await;
    assert!(foreign.is_err(), "a foreign owner cannot read selection");
}

#[tokio::test]
async fn selected_reservation_persists_binding_metadata_in_the_first_da_write() {
    let da = MockServer::start().await;
    let root = TempDir::new().expect("local data");
    let id = Uuid::new_v4();
    mount_models(&da).await;
    mount_registry(&da, ENDPOINT).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/conversations"))
        .and(body_json(json!({
            "model": MODEL_ID,
            "metadata": selected_binding(ENDPOINT),
            "auto_attach": false
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": id,
            "tenant_id": TENANT,
            "user_id": USER,
            "title": "Selected",
            "model": MODEL_ID,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&da)
        .await;

    let conversation_id = service(&da, &root)
        .conversation_reserve(Some(McpServerId::from(SERVER_ID)))
        .await
        .expect("reservation writes the selected scope");

    assert_eq!(conversation_id.as_str(), id.to_string());
    da.verify().await;
}

#[tokio::test]
async fn selection_mismatch_and_repointed_endpoint_fail_before_revision_mutation() {
    let da = MockServer::start().await;
    let root = TempDir::new().expect("local data");
    let id = Uuid::new_v4();
    mount_models(&da).await;
    mount_existing_conversation(&da, id, selected_binding(ENDPOINT), USER).await;
    mount_registry(&da, "https://changed.example.test/mcp").await;
    let chat_service = service(&da, &root);

    for revision in [
        TurnRevision::None,
        TurnRevision::Replace {
            cutoff: Uuid::new_v4(),
        },
        TurnRevision::Rerun {
            cutoff: Uuid::new_v4(),
        },
    ] {
        let mismatch = chat_service
            .message_send_inner(
                Some(ConversationId::from(id.to_string())),
                "edited text",
                revision,
                None,
                Vec::new(),
                None,
            )
            .await;
        assert_eq!(mismatch.status, ChatSubmitStatus::Failed);
        assert!(
            mismatch.message.contains("bound to a different MCP server"),
            "unexpected failure: {}",
            mismatch.message
        );
    }
    assert!(da
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method != "POST" && request.method != "PATCH"));

    let matching_da = MockServer::start().await;
    let matching_root = TempDir::new().expect("local data");
    let matching_id = Uuid::new_v4();
    mount_models(&matching_da).await;
    mount_existing_conversation(&matching_da, matching_id, selected_binding(ENDPOINT), USER).await;
    mount_registry(&matching_da, "https://MCP.example.test:443/mcp").await;
    let upload_mismatch = service(&matching_da, &matching_root)
        .mcp_selection_validate(ConversationId::from(matching_id.to_string()), None)
        .await;
    assert!(upload_mismatch
        .expect_err("an All upload target cannot enter a selected chat")
        .to_string()
        .contains("bound to a different MCP server"));
    let validate_matching = service(&matching_da, &matching_root)
        .mcp_selection_validate(
            ConversationId::from(matching_id.to_string()),
            Some(McpServerId::from(SERVER_ID)),
        )
        .await;
    assert!(
        validate_matching.is_ok(),
        "the unchanged selected endpoint is accepted: {validate_matching:?}"
    );

    let changed_da = MockServer::start().await;
    let changed_root = TempDir::new().expect("local data");
    let changed_id = Uuid::new_v4();
    mount_models(&changed_da).await;
    mount_existing_conversation(&changed_da, changed_id, selected_binding(ENDPOINT), USER).await;
    mount_registry(&changed_da, "https://changed.example.test/mcp").await;
    let upload_target = service(&changed_da, &changed_root)
        .mcp_selection_validate(
            ConversationId::from(changed_id.to_string()),
            Some(McpServerId::from(SERVER_ID)),
        )
        .await;
    assert!(upload_target
        .expect_err("repointed upload targets must be refused")
        .to_string()
        .contains("changed or is unavailable"));
    let matching = service(&changed_da, &changed_root)
        .message_send_inner(
            Some(ConversationId::from(changed_id.to_string())),
            "edited text",
            TurnRevision::Replace {
                cutoff: Uuid::new_v4(),
            },
            None,
            Vec::new(),
            Some(McpServerId::from(SERVER_ID)),
        )
        .await;
    assert_eq!(matching.status, ChatSubmitStatus::Failed);
    assert!(matching.message.contains("changed or is unavailable"));
    assert!(changed_da
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method != "POST" && request.method != "PATCH"));
}

#[tokio::test]
async fn retry_checks_scope_before_failed_or_completed_run_handling() {
    for status in ["failed", "completed"] {
        let da = MockServer::start().await;
        let root = TempDir::new().expect("local data");
        let conversation = Uuid::new_v4();
        let run = Uuid::new_v4();
        mount_existing_conversation(&da, conversation, selected_binding(ENDPOINT), USER).await;
        mount_registry(&da, ENDPOINT).await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/chat/runs/{run}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": run,
                "conversation_id": conversation,
                "tenant_id": TENANT,
                "user_id": USER,
                "status": status,
                "input_message": "original question"
            })))
            .mount(&da)
            .await;
        let service = service(&da, &root);
        let rejected = service
            .run_retry(None, run.to_string(), None)
            .await
            .expect_err("retry cannot widen a selected chat to All");
        assert!(rejected
            .to_string()
            .contains("bound to a different MCP server"));

        if status == "completed" {
            let settled = service
                .run_retry(None, run.to_string(), Some(McpServerId::from(SERVER_ID)))
                .await
                .expect("matching completed retry is settled");
            assert_eq!(settled.status, ChatSubmitStatus::Submitted);
            assert!(settled.run_id.is_none());
            assert_eq!(
                settled.conversation_id,
                Some(ConversationId::from(conversation.to_string()))
            );
        }
        assert!(da
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method != "POST" && request.method != "PATCH"));
    }
}

#[tokio::test]
async fn selected_admission_rejects_account_switch_during_registry_read() {
    let da = MockServer::start().await;
    let root = TempDir::new().expect("local data");
    let id = Uuid::new_v4();
    mount_existing_conversation(&da, id, selected_binding(ENDPOINT), USER).await;
    Mock::given(method("GET"))
        .and(path(REGISTRY_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(300))
                .set_body_json(json!({
                    "items": [server_row(ENDPOINT)],
                    "has_more": false,
                    "next_cursor": null
                })),
        )
        .expect(1)
        .mount(&da)
        .await;
    let (service, sessions, accounts) = service_with_state(&da, &root);
    let service = Arc::new(service);
    let generation = service.session_generation();
    let config = AuthConfig {
        da_base_url: da.uri().parse().expect("DA URL"),
        casdoor_issuer: "https://auth.example".parse().expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 0,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    let account =
        crate::security::account_cleanup::ValidatedAccountId::parse(ACCOUNT).expect("account");
    let conversation = ConversationId::from(id.to_string());
    let da_session = crate::clients::da::chat_api::DaChatSession::for_test(&da.uri(), "access")
        .expect("DA session");
    let task_service = Arc::clone(&service);
    let task = tokio::spawn(async move {
        let expected = crate::chat::service::CapturedChatAuth {
            config: &config,
            token: "access",
            account: &account,
            generation,
        };
        task_service
            .locked_conversation_model(
                &da_session,
                TENANT,
                &expected,
                &conversation,
                &[],
                Some(&McpServerId::from(SERVER_ID)),
            )
            .await
    });

    let mut registry_started = false;
    for _ in 0..100 {
        if da
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.method == "GET" && request.url.path() == REGISTRY_PATH)
        {
            registry_started = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        registry_started,
        "selected resolution reached the delayed DA read"
    );
    accounts
        .bind_native("tenant-2:user-2")
        .expect("switch account scope");
    sessions.set_authenticated(
        xfalcon_contracts::auth::SessionProjection::authenticated(
            xfalcon_contracts::auth::IdentityProjection::new(
                "user-2",
                "tenant-2",
                "Other user",
                None,
            ),
            chrono::Utc::now().timestamp_millis() + 3_600_000,
        ),
        "other-access",
        None,
    );

    let error = task
        .await
        .expect("admission task")
        .map(|_| ())
        .expect_err("account change invalidates selected admission");
    assert!(
        error.to_string().contains("active account changed")
            || error
                .to_string()
                .contains("selected MCP server changed or is unavailable"),
        "unexpected error before the account guard: {error}"
    );
    assert!(da
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method != "POST" && request.method != "PATCH"));
}
