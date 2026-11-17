//! The two reads that resolve a conversation's model before a send: the access
//! check (`ensure_conversation_access`, a conversation bootstrap through the
//! sync service) and the conversation read (`get_conversation`). They run
//! together, so these tests pin what that overlap may and may not change, and
//! the diagnostic measures how much it saves.
//!
//! The diagnostic is ignored by default. Run it with:
//! `cargo test --lib chat::tests::send_latency -- --ignored --nocapture`
//!
//! Every request goes to one loopback wiremock that answers after
//! `DELAY_MS`. The report counts requests per path and divides the elapsed
//! time by the delay to give the number of serial tenant round trips.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

use super::service::catalog_row;
use crate::auth::{AuthConfig, AuthSessionStore};
use crate::chat::service::{ChatService, ChatServiceError};
use crate::clients::da::chat_api::{DaChatSession, SONNET_MODEL_ID};
use crate::clients::da::model_catalog::ModelCatalogCache;
use crate::credentials::CredentialRuntime;
use crate::persistence::InMemorySecretStore;
use crate::security::account_cleanup::CurrentAccountScope;
use xfalcon_contracts::ConversationId;

const DELAY_MS: u64 = 100;
const TENANT: &str = "tenant-1";
const MODEL_ID: &str = SONNET_MODEL_ID;

/// The auth configuration a test service talks to `da_origin` with. It is
/// injected per service, so the process environment is never written.
fn test_auth_config(da_origin: &str) -> AuthConfig {
    AuthConfig {
        da_base_url: da_origin.parse().expect("DA URL"),
        casdoor_issuer: "https://auth.test.invalid".parse().expect("auth URL"),
        client_id: "test-client".to_owned(),
        organization: "test-org".to_owned(),
        loopback_port: 0,
        mcp_client_id: None,
        mcp_client_secret: None,
    }
}

/// A service with no session, so every call stops at the access check.
fn signed_out_service(da_origin: &str) -> ChatService {
    let root = std::env::temp_dir();
    build_service(
        Arc::new(AuthSessionStore::new()),
        Arc::new(CurrentAccountScope::default()),
        &root,
        da_origin,
    )
}

fn signed_in_stores() -> (Arc<AuthSessionStore>, Arc<CurrentAccountScope>) {
    let sessions = Arc::new(AuthSessionStore::new());
    sessions.set_authenticated(
        xfalcon_contracts::auth::SessionProjection::authenticated(
            xfalcon_contracts::auth::IdentityProjection::new("user-1", TENANT, "User", None),
            chrono::Utc::now().timestamp_millis() + 3_600_000,
        ),
        "access",
        None,
    );
    let accounts = Arc::new(CurrentAccountScope::default());
    accounts
        .bind_native("tenant-1:user-1")
        .expect("bind account");
    (sessions, accounts)
}

/// A signed-in service whose tenant calls go to `da_origin`. The projection
/// store lives under `root`, so concurrent tests never share a database.
fn tenant_service(da_origin: &str, root: &TempDir) -> ChatService {
    let (sessions, accounts) = signed_in_stores();
    build_service(sessions, accounts, root.path(), da_origin)
}

fn build_service(
    sessions: Arc<AuthSessionStore>,
    accounts: Arc<CurrentAccountScope>,
    root: &std::path::Path,
    da_origin: &str,
) -> ChatService {
    let secrets = Arc::new(InMemorySecretStore::default());
    let service = ChatService::new(
        sessions.clone(),
        accounts.clone(),
        root.to_owned(),
        secrets.clone(),
        Arc::new(CredentialRuntime::new(
            sessions,
            Arc::new(InMemorySecretStore::default()),
        )),
        Arc::new(ModelCatalogCache::default()),
        Arc::new(crate::projects::ProjectsService::new(
            root.to_owned(),
            accounts.clone(),
            secrets.clone(),
        )),
        Arc::new(crate::approvals::ApprovalsService::new(
            root.to_owned(),
            accounts,
            secrets,
        )),
    );
    service.set_auth_config_for_test(test_auth_config(da_origin));
    service
}

/// `GET /api/v1/chat/bootstrap/conversations/{id}`, the access check's read.
async fn mount_bootstrap(server: &MockServer, id: Uuid, tenant_id: &str, delay: Duration) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/chat/bootstrap/conversations/{id}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(json!({
                    "conversation": {
                        "id": id,
                        "tenant_id": tenant_id,
                        "title": "Locked",
                        "model": MODEL_ID,
                    },
                    "chats": [],
                    "tool_calls": [],
                    "artifacts": [],
                    "attached_server_ids": [],
                })),
        )
        .mount(server)
        .await;
}

/// `GET /api/v1/chat/conversations/{id}`, the model read.
async fn mount_conversation(server: &MockServer, id: Uuid, tenant_id: &str, delay: Duration) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/chat/conversations/{id}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(json!({
                    "id": id,
                    "tenant_id": tenant_id,
                    "user_id": "user-1",
                    "title": "Locked",
                    "model": MODEL_ID,
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z",
                })),
        )
        .mount(server)
        .await;
}

/// `GET /api/v1/chat/chat-attachments`, read after the bootstrap inside the
/// access check. Empty: these tests cover the model, not the file labels.
async fn mount_attachments(server: &MockServer, delay: Duration) {
    Mock::given(method("GET"))
        .and(path("/api/v1/chat/chat-attachments"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(json!({ "items": [], "next_cursor": null, "has_more": false })),
        )
        .mount(server)
        .await;
}

async fn mount_failure(server: &MockServer, request_path: String, status: u16, delay: Duration) {
    Mock::given(method("GET"))
        .and(path(request_path))
        .respond_with(ResponseTemplate::new(status).set_delay(delay))
        .mount(server)
        .await;
}

async fn locked_model(
    service: &ChatService,
    server: &MockServer,
    id: Uuid,
) -> Result<(Uuid, String), ChatServiceError> {
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");
    let models = vec![catalog_row(MODEL_ID, "Test model")];
    let config = test_auth_config(&server.uri());
    let account = crate::security::account_cleanup::ValidatedAccountId::parse("tenant-1:user-1")
        .expect("account");
    let expected_auth = crate::chat::service::CapturedChatAuth {
        config: &config,
        token: "token",
        account: &account,
        generation: service.session_generation(),
    };
    service
        .locked_conversation_model(
            &da,
            TENANT,
            &expected_auth,
            &ConversationId::from(id.to_string()),
            &models,
            None,
        )
        .await
        .map(|(uuid, model, _, _)| (uuid, model.model_id))
}

#[tokio::test]
#[ignore = "diagnostic: prints read-overlap latency; run with --ignored --nocapture"]
async fn read_overlap_latency_under_tenant_delay() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    let delay = Duration::from_millis(DELAY_MS);
    mount_bootstrap(&server, id, TENANT, delay).await;
    mount_conversation(&server, id, TENANT, delay).await;
    mount_attachments(&server, delay).await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let started = Instant::now();
    let result = locked_model(&service, &server, id).await;
    let elapsed = started.elapsed();

    let requests = server.received_requests().await.unwrap_or_default();
    eprintln!("send_latency: result={result:?}");
    eprintln!("send_latency: locked_conversation_model elapsed={elapsed:?}");
    eprintln!("send_latency: requests received={}", requests.len());
    for request in &requests {
        eprintln!("send_latency:   {} {}", request.method, request.url.path());
    }
    eprintln!(
        "send_latency: serial delays={:.2} (elapsed / {DELAY_MS} ms)",
        elapsed.as_secs_f64() * 1000.0 / DELAY_MS as f64
    );
}

/// Both reads succeed: the uuid and the model come back as the conversation
/// locked them.
#[tokio::test]
async fn both_reads_return_the_conversation_and_its_locked_model() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    mount_bootstrap(&server, id, TENANT, Duration::ZERO).await;
    mount_conversation(&server, id, TENANT, Duration::ZERO).await;
    mount_attachments(&server, Duration::ZERO).await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let (uuid, model_id) = locked_model(&service, &server, id)
        .await
        .expect("both reads succeed");

    assert_eq!(uuid, id);
    assert_eq!(model_id, MODEL_ID);
}

#[tokio::test]
async fn a_conversation_of_another_tenant_is_refused_after_both_reads() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    mount_bootstrap(&server, id, TENANT, Duration::ZERO).await;
    mount_conversation(&server, id, "tenant-other", Duration::ZERO).await;
    mount_attachments(&server, Duration::ZERO).await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let error = locked_model(&service, &server, id)
        .await
        .expect_err("tenant mismatch");

    assert!(matches!(
        error,
        ChatServiceError::Message(ref message)
            if message == "The conversation does not belong to this account."
    ));
}

/// The access check fails and so does the model read: the access failure is
/// the one reported.
#[tokio::test]
async fn a_failed_access_check_is_reported_over_a_failed_model_read() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    mount_failure(
        &server,
        format!("/api/v1/chat/bootstrap/conversations/{id}"),
        404,
        Duration::ZERO,
    )
    .await;
    mount_failure(
        &server,
        format!("/api/v1/chat/conversations/{id}"),
        500,
        Duration::ZERO,
    )
    .await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let error = locked_model(&service, &server, id)
        .await
        .expect_err("access fails");

    assert!(matches!(error, ChatServiceError::NotFound));
}

/// With both reads delayed by 1000 ms, the pair completes in about one delay.
/// Serial reads would take about 2000 ms.
#[tokio::test]
async fn the_access_check_and_the_model_read_overlap() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    let delay = Duration::from_millis(1000);
    mount_bootstrap(&server, id, TENANT, delay).await;
    mount_conversation(&server, id, TENANT, delay).await;
    // The attachment list is the bootstrap's own second request, read after it
    // inside the access check. The overlap under test is between the access
    // check and the model read, so the list answers at once here.
    mount_attachments(&server, Duration::ZERO).await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let started = Instant::now();
    locked_model(&service, &server, id)
        .await
        .expect("both reads succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(1600),
        "reads ran one after the other: {elapsed:?}"
    );
}

/// A failed access check is returned at once, not after the model read finishes.
#[tokio::test]
async fn a_failed_access_check_does_not_wait_for_a_slow_model_read() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    mount_failure(
        &server,
        format!("/api/v1/chat/bootstrap/conversations/{id}"),
        404,
        Duration::ZERO,
    )
    .await;
    mount_conversation(&server, id, TENANT, Duration::from_secs(3)).await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let started = Instant::now();
    let error = locked_model(&service, &server, id)
        .await
        .expect_err("access fails");
    let elapsed = started.elapsed();

    assert!(matches!(error, ChatServiceError::NotFound));
    assert!(
        elapsed < Duration::from_millis(1500),
        "the access error waited for the model read: {elapsed:?}"
    );
}

/// A read failure that arrives first does not outrank a slower access failure.
#[tokio::test]
async fn a_fast_failed_read_does_not_outrank_a_slower_failed_access_check() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    mount_failure(
        &server,
        format!("/api/v1/chat/bootstrap/conversations/{id}"),
        404,
        Duration::from_millis(200),
    )
    .await;
    mount_failure(
        &server,
        format!("/api/v1/chat/conversations/{id}"),
        500,
        Duration::ZERO,
    )
    .await;
    let root = TempDir::new().expect("root");
    let service = tenant_service(&server.uri(), &root);

    let error = locked_model(&service, &server, id)
        .await
        .expect_err("access fails");

    assert!(matches!(error, ChatServiceError::NotFound));
}

/// The signed-out case: the access check fails before any tenant request, so
/// the device is asked to sign in. This does not establish a failed model read;
/// the failed-read precedence is covered by
/// `a_failed_access_check_is_reported_over_a_failed_model_read`.
#[tokio::test]
async fn a_signed_out_device_is_asked_to_sign_in() {
    let server = MockServer::start().await;
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");
    let service = signed_out_service("https://da.test.invalid");
    let config = test_auth_config(&server.uri());
    let account = crate::security::account_cleanup::ValidatedAccountId::parse("tenant-1:user-1")
        .expect("account");
    let expected_auth = crate::chat::service::CapturedChatAuth {
        config: &config,
        token: "token",
        account: &account,
        generation: service.session_generation(),
    };

    let error = service
        .locked_conversation_model(
            &da,
            "tenant-1",
            &expected_auth,
            &ConversationId::from(Uuid::new_v4().to_string()),
            &[],
            None,
        )
        .await
        .map(|_| ())
        .expect_err("signed out");

    assert!(matches!(error, ChatServiceError::AuthRequired));
}

/// A malformed id still reports the access failure first, as it always has.
#[tokio::test]
async fn access_failure_outranks_a_malformed_conversation_id() {
    let da = DaChatSession::for_test("http://127.0.0.1:9", "token").expect("session");
    let service = signed_out_service("https://da.test.invalid");
    let config = test_auth_config("https://da.test.invalid");
    let account = crate::security::account_cleanup::ValidatedAccountId::parse("tenant-1:user-1")
        .expect("account");
    let expected_auth = crate::chat::service::CapturedChatAuth {
        config: &config,
        token: "token",
        account: &account,
        generation: service.session_generation(),
    };

    let error = service
        .locked_conversation_model(
            &da,
            "tenant-1",
            &expected_auth,
            &ConversationId::from("not-a-uuid"),
            &[],
            None,
        )
        .await;
    let error = error.map(|_| ()).expect_err("signed out");

    assert!(matches!(error, ChatServiceError::AuthRequired));
}
