//! Wire tests for the shared `mcp_servers` registry adapter.
//!
//! Every row DA returns is untrusted input at this boundary, so the cases here
//! are mostly adversarial: rows belonging to another principal, a transport or
//! auth method outside DA's own vocabulary, an oversized field, a header name
//! that would attempt header injection, a cursor that never terminates, and a
//! response body larger than the adapter will buffer.

use std::time::Duration;

use reqwest::Method;
use serde_json::{json, Value};
use wiremock::{
    matchers::{method, path, path_regex},
    Mock, MockServer, Request, ResponseTemplate,
};
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::mcp::{McpAuthType, McpPermissionMode, McpTransport};

use super::super::mcp_servers::{
    DaMcpServerRepository, McpServerPatch, McpServerRepositoryError, NewMcpServer,
};

const ROUTE: &str = "/api/v1/chat/mcp-servers";
const BEARER_CANARY: &str = "wire-bearer-mcp-registry-canary";
const ROW_ID: &str = "00000000-0000-0000-0000-0000000000aa";

fn identity() -> IdentityProjection {
    IdentityProjection::new("user-1", "tenant-1", "Wire User", None)
}

fn repository(origin: &str) -> DaMcpServerRepository {
    DaMcpServerRepository::for_test(
        origin,
        BEARER_CANARY,
        &identity(),
        Duration::from_millis(500),
    )
    .expect("valid loopback DA repository")
}

/// `ChatErrorCode` is a closed enum in the pinned DA contract, so only real
/// members deserialize — an invented code makes the generated parser fail,
/// which is a different failure than the status mapping under test.
fn error_body(code: &str) -> Value {
    json!({
        "error": {
            "code": code,
            "message": "safe wire error",
            "request_id": "request-1"
        }
    })
}

fn row(overrides: Value) -> Value {
    let mut base = json!({
        "id": ROW_ID,
        "tenant_id": "tenant-1",
        "user_id": "user-1",
        "name": "crm",
        "display_name": "Acme CRM",
        "description": null,
        "transport": "http",
        "url": "https://mcp.example.test/mcp",
        "command": null,
        "args": null,
        "icon_url": null,
        "auth_type": "bearer",
        "auth_config": { "opaque": "web-kek-ciphertext-desktop-cannot-read" },
        "is_enabled": true,
        "status": "connected",
        "status_message": null,
        "last_health_check": null,
        "permission_mode": "ask",
        "metadata": { "origin": "xfalcon-desktop" },
        "created_at": "1970-01-01T00:00:00Z",
        "updated_at": "1970-01-01T00:00:00Z",
        "visibility": "private"
    });
    let (Value::Object(base_map), Value::Object(override_map)) = (&mut base, overrides) else {
        panic!("row overrides must be a JSON object");
    };
    for (key, value) in override_map {
        base_map.insert(key, value);
    }
    base
}

fn page(items: Vec<Value>, has_more: bool, next_cursor: Option<&str>) -> Value {
    json!({
        "items": items,
        "has_more": has_more,
        "next_cursor": next_cursor,
    })
}

async fn list_mock(server: &MockServer, body: Value) {
    Mock::given(method(Method::GET))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_well_formed_row_narrows_to_the_contract_vocabulary() {
    let server = MockServer::start().await;
    list_mock(
        &server,
        page(
            vec![row(json!({
                "auth_type": "api_key",
                "metadata": { "origin": "xfalcon-desktop", "header_name": "X-Acme-Key" },
            }))],
            false,
            None,
        ),
    )
    .await;

    let records = repository(&server.uri())
        .list()
        .await
        .expect("a well-formed page lists");

    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, ROW_ID);
    assert_eq!(record.name, "crm");
    assert_eq!(record.display_name.as_deref(), Some("Acme CRM"));
    assert_eq!(record.transport, McpTransport::Http);
    assert_eq!(record.url.as_deref(), Some("https://mcp.example.test/mcp"));
    // DA spells it `api_key`; the desktop vocabulary calls it what it does.
    assert_eq!(record.auth_type, McpAuthType::Header);
    assert_eq!(record.permission_mode, McpPermissionMode::Ask);
    assert!(record.enabled);
    assert_eq!(record.header_name.as_deref(), Some("X-Acme-Key"));
}

#[tokio::test]
async fn a_transport_or_auth_method_outside_das_vocabulary_becomes_other_not_an_error() {
    let server = MockServer::start().await;
    list_mock(
        &server,
        page(
            vec![row(json!({
                "transport": "websocket",
                "auth_type": "mutual_tls",
            }))],
            false,
            None,
        ),
    )
    .await;

    let records = repository(&server.uri())
        .list()
        .await
        .expect("an unknown transport must not make the list unreadable");

    assert_eq!(records[0].transport, McpTransport::Other);
    assert_eq!(records[0].auth_type, McpAuthType::Other);
    assert!(!records[0].auth_type.is_supported());
}

#[tokio::test]
async fn an_absent_or_unrecognized_permission_mode_defaults_to_ask() {
    let server = MockServer::start().await;
    list_mock(
        &server,
        page(
            vec![
                row(json!({ "permission_mode": null })),
                row(json!({ "permission_mode": "yolo" })),
                row(json!({ "permission_mode": "allow" })),
            ],
            false,
            None,
        ),
    )
    .await;

    let records = repository(&server.uri()).list().await.expect("page lists");
    assert_eq!(records[0].permission_mode, McpPermissionMode::Ask);
    assert_eq!(records[1].permission_mode, McpPermissionMode::Ask);
    assert_eq!(records[2].permission_mode, McpPermissionMode::Allow);
}

#[tokio::test]
async fn a_row_belonging_to_another_principal_is_rejected() {
    let server = MockServer::start().await;
    list_mock(
        &server,
        page(vec![row(json!({ "user_id": "someone-else" }))], false, None),
    )
    .await;

    assert_eq!(
        repository(&server.uri()).list().await,
        Err(McpServerRepositoryError::Unauthorized)
    );
}

#[tokio::test]
async fn a_row_belonging_to_another_tenant_is_rejected() {
    let server = MockServer::start().await;
    list_mock(
        &server,
        page(vec![row(json!({ "tenant_id": "tenant-2" }))], false, None),
    )
    .await;

    assert_eq!(
        repository(&server.uri()).list().await,
        Err(McpServerRepositoryError::Unauthorized)
    );
}

#[tokio::test]
async fn an_oversized_field_or_empty_name_fails_the_row_closed() {
    for overrides in [
        json!({ "name": "n".repeat(101) }),
        json!({ "name": "" }),
        json!({ "url": format!("https://example.test/{}", "p".repeat(2_100)) }),
        json!({ "description": "d".repeat(2_100) }),
    ] {
        let server = MockServer::start().await;
        list_mock(&server, page(vec![row(overrides.clone())], false, None)).await;

        let outcome = repository(&server.uri()).list().await;
        assert_eq!(
            outcome,
            Err(McpServerRepositoryError::InvalidRow),
            "row {overrides} should have failed closed"
        );
    }
}

#[tokio::test]
async fn a_header_name_that_could_inject_a_header_is_rejected() {
    for hostile in [
        "X-Bad\r\nAuthorization: Bearer stolen",
        "X Bad",
        "X-Bad:",
        "",
        &"h".repeat(65),
    ] {
        let server = MockServer::start().await;
        list_mock(
            &server,
            page(
                vec![row(json!({
                    "auth_type": "api_key",
                    "metadata": { "header_name": hostile },
                }))],
                false,
                None,
            ),
        )
        .await;

        assert_eq!(
            repository(&server.uri()).list().await,
            Err(McpServerRepositoryError::InvalidRow),
            "header name {hostile:?} should have failed closed"
        );
    }
}

#[tokio::test]
async fn a_non_string_header_name_fails_the_row_rather_than_being_coerced() {
    let server = MockServer::start().await;
    list_mock(
        &server,
        page(
            vec![row(json!({ "metadata": { "header_name": 42 } }))],
            false,
            None,
        ),
    )
    .await;

    assert_eq!(
        repository(&server.uri()).list().await,
        Err(McpServerRepositoryError::InvalidRow)
    );
}

#[tokio::test]
async fn a_cursor_that_never_terminates_is_bounded_rather_than_looping_forever() {
    let server = MockServer::start().await;
    Mock::given(method(Method::GET))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            vec![row(json!({}))],
            true,
            Some("always-more"),
        )))
        .mount(&server)
        .await;

    assert_eq!(
        repository(&server.uri()).list().await,
        Err(McpServerRepositoryError::Unavailable)
    );
}

#[tokio::test]
async fn a_page_claiming_more_without_a_cursor_is_rejected() {
    let server = MockServer::start().await;
    list_mock(&server, page(vec![row(json!({}))], true, None)).await;

    assert_eq!(
        repository(&server.uri()).list().await,
        Err(McpServerRepositoryError::Unavailable)
    );
}

#[tokio::test]
async fn an_oversized_list_body_is_refused_before_it_is_buffered() {
    let server = MockServer::start().await;
    let bloated = page(
        vec![row(json!({ "description": "d".repeat(1_024) })); 4_096],
        false,
        None,
    );
    Mock::given(method(Method::GET))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(bloated))
        .mount(&server)
        .await;

    assert_eq!(
        repository(&server.uri()).list().await,
        Err(McpServerRepositoryError::Unavailable)
    );
}

#[tokio::test]
async fn unauthorized_and_forbidden_listings_map_to_unauthorized() {
    for status in [401, 403] {
        let server = MockServer::start().await;
        Mock::given(method(Method::GET))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body("unauthorized")))
            .mount(&server)
            .await;

        assert_eq!(
            repository(&server.uri()).list().await,
            Err(McpServerRepositoryError::Unauthorized),
            "status {status} should map to Unauthorized"
        );
    }
}

fn new_server() -> NewMcpServer {
    NewMcpServer {
        name: "crm".to_owned(),
        display_name: Some("Acme CRM".to_owned()),
        description: None,
        url: "https://mcp.example.test/mcp".to_owned(),
        auth_type: McpAuthType::Bearer,
        header_name: None,
        permission_mode: McpPermissionMode::Ask,
        enabled: true,
    }
}

#[tokio::test]
async fn create_writes_only_the_columns_desktop_owns() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON create body");
            // The adapter must not be able to write credential material or a
            // shared status, and must always register Streamable HTTP.
            for forbidden in [
                "auth_config",
                "status",
                "status_message",
                "last_health_check",
                "command",
                "args",
            ] {
                assert!(
                    body.get(forbidden).is_none(),
                    "create body must not carry {forbidden}"
                );
            }
            assert_eq!(body["transport"], json!("http"));
            assert_eq!(body["visibility"], json!("private"));
            assert_eq!(body["auth_type"], json!("bearer"));
            assert_eq!(body["metadata"]["origin"], json!("xfalcon-desktop"));
            ResponseTemplate::new(201).set_body_json(row(json!({})))
        })
        .mount(&server)
        .await;

    let record = repository(&server.uri())
        .create(new_server())
        .await
        .expect("create returns the committed row");
    assert_eq!(record.name, "crm");
    assert_eq!(record.auth_type, McpAuthType::Bearer);
}

#[tokio::test]
async fn create_sends_the_header_name_in_metadata_for_a_header_authenticated_server() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON create body");
            assert_eq!(body["auth_type"], json!("api_key"));
            assert_eq!(body["metadata"]["header_name"], json!("X-Acme-Key"));
            ResponseTemplate::new(201).set_body_json(row(json!({
                "auth_type": "api_key",
                "metadata": { "origin": "xfalcon-desktop", "header_name": "X-Acme-Key" },
            })))
        })
        .mount(&server)
        .await;

    let record = repository(&server.uri())
        .create(NewMcpServer {
            auth_type: McpAuthType::Header,
            header_name: Some("X-Acme-Key".to_owned()),
            ..new_server()
        })
        .await
        .expect("create returns the committed row");
    assert_eq!(record.header_name.as_deref(), Some("X-Acme-Key"));
}

#[tokio::test]
async fn create_sends_oauth_without_a_grant() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON create body");
            assert_eq!(body["auth_type"], json!("oauth"));
            ResponseTemplate::new(201).set_body_json(row(json!({ "auth_type": "oauth" })))
        })
        .mount(&server)
        .await;

    let record = repository(&server.uri())
        .create(NewMcpServer {
            auth_type: McpAuthType::Oauth,
            ..new_server()
        })
        .await
        .expect("create returns the committed row");
    assert_eq!(record.auth_type, McpAuthType::Oauth);
}

#[tokio::test]
async fn create_refuses_an_auth_method_desktop_cannot_write() {
    let server = MockServer::start().await;
    // No mock is mounted: the refusal must happen before any request is sent.
    assert_eq!(
        repository(&server.uri())
            .create(NewMcpServer {
                auth_type: McpAuthType::Other,
                ..new_server()
            })
            .await,
        Err(McpServerRepositoryError::InvalidRow)
    );
    assert!(server
        .received_requests()
        .await
        .unwrap_or_default()
        .is_empty());
}

#[tokio::test]
async fn a_duplicate_name_maps_to_conflict() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(409).set_body_json(error_body("conflict")))
        .mount(&server)
        .await;

    assert_eq!(
        repository(&server.uri()).create(new_server()).await,
        Err(McpServerRepositoryError::Conflict)
    );
}

#[tokio::test]
async fn a_create_that_returns_a_different_row_demands_reconciliation() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(201).set_body_json(row(json!({ "name": "not-mine" }))))
        .mount(&server)
        .await;

    assert_eq!(
        repository(&server.uri()).create(new_server()).await,
        Err(McpServerRepositoryError::ReconciliationRequired)
    );
}

#[tokio::test]
async fn update_targets_the_scoped_row_and_omits_untouched_fields() {
    let server = MockServer::start().await;
    Mock::given(method(Method::PATCH))
        .and(path(format!("{ROUTE}/{ROW_ID}")))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON patch body");
            let fields = body.as_object().expect("patch body is an object");
            // Only what the caller actually set may travel.
            assert_eq!(fields.len(), 1, "unexpected patch body: {body}");
            assert_eq!(body["permission_mode"], json!("deny"));
            ResponseTemplate::new(200).set_body_json(row(json!({ "permission_mode": "deny" })))
        })
        .mount(&server)
        .await;

    let record = repository(&server.uri())
        .update(
            ROW_ID,
            McpServerPatch {
                permission_mode: Some(McpPermissionMode::Deny),
                ..McpServerPatch::default()
            },
        )
        .await
        .expect("update returns the committed row");
    assert_eq!(record.permission_mode, McpPermissionMode::Deny);
}

#[tokio::test]
async fn an_id_carrying_path_syntax_cannot_reshape_the_request() {
    let server = MockServer::start().await;
    // Every DELETE is answered, so the assertion below is about which path the
    // adapter actually requested, not about a 404 from a missing mock.
    Mock::given(method(Method::DELETE))
        .and(path_regex(".*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "deleted": true })))
        .mount(&server)
        .await;

    repository(&server.uri())
        .delete("../provider-keys/00000000-0000-0000-0000-000000000001")
        .await
        .expect("the request is sent, just not where the id asked");

    let requests = server
        .received_requests()
        .await
        .expect("the mock server records requests");
    assert_eq!(requests.len(), 1);
    let requested = requests[0].url.clone();
    let segments: Vec<&str> = requested
        .path_segments()
        .expect("an absolute URL has path segments")
        .collect();
    // The id stays exactly one segment: percent-encoding leaves its literal
    // text visible but inert, so DA resolves it as an id that does not exist
    // rather than as a path into another route.
    assert_eq!(
        segments,
        vec![
            "api",
            "v1",
            "chat",
            "mcp-servers",
            "..%2Fprovider-keys%2F00000000-0000-0000-0000-000000000001"
        ],
        "a traversing id must not add or escape path segments"
    );
}

#[tokio::test]
async fn delete_maps_missing_rows_and_denials() {
    for (status, code, expected) in [
        (404, "not_found", McpServerRepositoryError::NotFound),
        (401, "unauthorized", McpServerRepositoryError::Unauthorized),
        (
            403,
            "insufficient_scope",
            McpServerRepositoryError::Unauthorized,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method(Method::DELETE))
            .and(path(format!("{ROUTE}/{ROW_ID}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body(code)))
            .mount(&server)
            .await;

        assert_eq!(
            repository(&server.uri()).delete(ROW_ID).await,
            Err(expected),
            "status {status} mapped wrongly"
        );
    }
}

#[tokio::test]
async fn a_successful_delete_reports_success() {
    let server = MockServer::start().await;
    Mock::given(method(Method::DELETE))
        .and(path(format!("{ROUTE}/{ROW_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "deleted": true })))
        .mount(&server)
        .await;

    assert_eq!(repository(&server.uri()).delete(ROW_ID).await, Ok(()));
}

#[tokio::test]
async fn every_request_carries_the_session_bearer_and_never_leaks_it_into_a_body() {
    let server = MockServer::start().await;
    Mock::given(method(Method::POST))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(201).set_body_json(row(json!({}))))
        .mount(&server)
        .await;

    repository(&server.uri())
        .create(new_server())
        .await
        .expect("create succeeds");

    let requests = server
        .received_requests()
        .await
        .expect("the mock server records requests");
    let request = &requests[0];
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some(format!("Bearer {BEARER_CANARY}").as_str())
    );
    let body = String::from_utf8_lossy(&request.body);
    assert!(
        !body.contains(BEARER_CANARY),
        "the session bearer must never appear in a request body"
    );
}
