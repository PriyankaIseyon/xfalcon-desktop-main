use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use reqwest::Method;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request, ResponseTemplate,
};
use xfalcon_contracts::{
    auth::IdentityProjection,
    credentials::{CredentialStatus, ProviderId},
};
use zeroize::Zeroizing;

use crate::{
    credentials::{
        service::{
            CredentialActor, CredentialCreateRequest, CredentialDeleteRequest,
            CredentialReplaceRequest, CredentialRepository, CredentialRepositoryError,
            CredentialService, CredentialServiceError,
        },
        vault::CredentialVault,
        verifier::{CredentialVerifier, VerificationError, VerifiedCredential},
    },
    persistence::InMemorySecretStore,
};

use super::super::provider_keys::DaCredentialRepository;

const ROUTE: &str = "/api/v1/chat/provider-keys";
const BEARER_CANARY: &str = "wire-bearer-super-secret-canary";
const PLAINTEXT_CANARY: &str = "sk-wire-plaintext-super-secret-canary";
const MAX_LIST_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROW_RESPONSE_BYTES: usize = 64 * 1024;

fn identity() -> IdentityProjection {
    IdentityProjection::new("user-1", "tenant-1", "Wire User", None)
}

fn actor() -> CredentialActor {
    CredentialActor::new("tenant-1:user-1", "tenant-1", "user-1").expect("valid actor")
}

fn repository(origin: &str) -> DaCredentialRepository {
    DaCredentialRepository::for_test(
        origin,
        BEARER_CANARY,
        &identity(),
        Duration::from_millis(100),
    )
    .expect("valid loopback DA repository")
}

fn error_body(code: &str) -> Value {
    json!({
        "error": {
            "code": code,
            "message": "safe wire error",
            "request_id": "request-1"
        }
    })
}

fn row(provider: &str, tenant_id: &str, user_id: &str, api_key: &str, metadata: Value) -> Value {
    json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "tenant_id": tenant_id,
        "user_id": user_id,
        "provider": provider,
        "label": "Work credential",
        "api_key": api_key,
        "base_url": null,
        "is_default": true,
        "is_enabled": true,
        "metadata": metadata,
        "created_at": "1970-01-01T00:00:00Z",
        "updated_at": "1970-01-01T00:00:00Z"
    })
}

fn compatible_metadata() -> Value {
    json!({
        "device_key_id": "dk-wire"
    })
}

fn da_metadata() -> Value {
    json!({
        "secret_owner": "data-analyst",
        "envelope_version": 1
    })
}

fn page(items: Vec<Value>, next_cursor: Option<&str>, has_more: bool) -> Value {
    json!({
        "items": items,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": 200
    })
}

#[derive(Default)]
struct AcceptingVerifier;

#[async_trait]
impl CredentialVerifier for AcceptingVerifier {
    async fn verify(
        &self,
        _provider: ProviderId,
        _secret: &str,
    ) -> Result<VerifiedCredential, VerificationError> {
        Ok(VerifiedCredential)
    }
}

fn service(origin: &str) -> CredentialService {
    CredentialService::new(
        Arc::new(CredentialVault::new(Arc::new(
            InMemorySecretStore::default(),
        ))),
        Arc::new(repository(origin)),
        Arc::new(AcceptingVerifier),
    )
}

fn create_request() -> CredentialCreateRequest {
    CredentialCreateRequest::new(
        ProviderId::Anthropic,
        "  Wire label  ",
        Zeroizing::new(PLAINTEXT_CANARY.to_owned()),
        true,
    )
    .expect("valid create request")
}

fn query(request: &Request) -> BTreeMap<String, String> {
    request.url.query_pairs().into_owned().collect()
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

#[test]
fn test_origin_seam_accepts_only_root_loopback_http_origins() {
    for valid in ["http://127.0.0.1:1234", "http://[::1]:1234/"] {
        assert!(DaCredentialRepository::for_test(
            valid,
            BEARER_CANARY,
            &identity(),
            Duration::from_millis(10)
        )
        .is_ok());
    }
    for invalid in [
        "http://example.com:1234",
        "http://user@127.0.0.1:1234",
        "http://127.0.0.1:1234/path",
        "http://127.0.0.1:1234?query=1",
        "http://127.0.0.1:1234#fragment",
    ] {
        assert!(
            DaCredentialRepository::for_test(
                invalid,
                BEARER_CANARY,
                &identity(),
                Duration::from_millis(10)
            )
            .is_err(),
            "accepted invalid test origin"
        );
    }
}

#[tokio::test]
async fn list_attaches_bearer_and_follows_the_exact_cursor_contract() {
    let server = MockServer::start().await;
    let call = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with({
            let call = call.clone();
            move |_request: &Request| {
                let body = match call.fetch_add(1, Ordering::SeqCst) {
                    0 => page(
                        vec![
                            row(
                                "future-provider",
                                "tenant-1",
                                "user-1",
                                "xfdc:v1:foreign",
                                compatible_metadata(),
                            ),
                            row(
                                "openai",
                                "tenant-1",
                                "user-1",
                                "xfdc:v1:supported",
                                compatible_metadata(),
                            ),
                        ],
                        Some("opaque/cursor +=="),
                        true,
                    ),
                    1 => page(
                        vec![row(
                            "google",
                            "tenant-1",
                            "user-1",
                            "xfdc:v1:terminal",
                            compatible_metadata(),
                        )],
                        None,
                        false,
                    ),
                    _ => panic!("list requested a page after the terminal page"),
                };
                ResponseTemplate::new(200).set_body_json(body)
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let records = repository(&server.uri())
        .list()
        .await
        .expect("list succeeds");
    assert_eq!(records.len(), 2, "unsupported row must be skipped");

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.url.path(), ROUTE);
        assert_authorized(request);
    }
    assert_eq!(
        query(&requests[0]),
        BTreeMap::from([
            ("limit".to_owned(), "200".to_owned()),
            ("sort".to_owned(), "-created_at".to_owned()),
        ])
    );
    assert_eq!(
        query(&requests[1]),
        BTreeMap::from([
            ("cursor".to_owned(), "opaque/cursor +==".to_owned()),
            ("limit".to_owned(), "200".to_owned()),
            ("sort".to_owned(), "-created_at".to_owned()),
        ])
    );
}

#[tokio::test]
async fn list_fails_closed_on_missing_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(Vec::new(), None, true)))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(
        repository(&server.uri()).list().await.unwrap_err(),
        CredentialRepositoryError::Unavailable
    );
}

#[tokio::test]
async fn list_rejects_each_foreign_principal_dimension_as_unauthorized() {
    for body in [
        page(
            vec![row(
                "openai",
                "tenant-other",
                "user-1",
                "xfdc:v1:foreign-tenant",
                compatible_metadata(),
            )],
            None,
            false,
        ),
        page(
            vec![row(
                "openai",
                "tenant-1",
                "user-other",
                "xfdc:v1:foreign-user",
                compatible_metadata(),
            )],
            None,
            false,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;

        let error = repository(&server.uri())
            .list()
            .await
            .expect_err("foreign list must fail");
        assert_eq!(error, CredentialRepositoryError::Unauthorized);
    }
}

#[tokio::test]
async fn real_service_create_sends_cleartext_in_the_exact_da_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(ROUTE))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("request JSON");
            ResponseTemplate::new(201).set_body_json(row(
                body["provider"].as_str().expect("provider"),
                "tenant-1",
                "user-1",
                "xfda:v1:server-ciphertext",
                da_metadata(),
            ))
        })
        .expect(1)
        .mount(&server)
        .await;

    let view = service(&server.uri())
        .create(&actor(), create_request())
        .await
        .expect("create succeeds");
    assert_eq!(view.provider, ProviderId::Anthropic);
    assert_eq!(view.status, CredentialStatus::Ready);

    let requests = server.received_requests().await.expect("recorded request");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.path(), ROUTE);
    assert!(request.url.query().is_none());
    assert_authorized(request);
    assert!(
        request
            .headers
            .keys()
            .all(|name| !name.as_str().starts_with("x-")),
        "create must not add custom DA headers"
    );
    let body: Value = serde_json::from_slice(&request.body).expect("request JSON");
    // Sorted before comparing: the assertion is about the exact key *set* DA
    // receives — no extra field may leak, none may go missing — and JSON
    // object key order carries no meaning on the wire. Comparing raw
    // iteration order instead pinned serde's struct-declaration order against
    // an alphabetically written expectation, which could never match.
    let mut keys = body
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "base_url",
            "is_default",
            "is_enabled",
            "label",
            "provider",
            "secret"
        ]
    );
    assert_eq!(body["provider"], "anthropic");
    assert_eq!(body["label"], "Wire label");
    assert_eq!(body["base_url"], Value::Null);
    assert_eq!(body["is_default"], true);
    assert_eq!(body["is_enabled"], true);
    assert_eq!(body["secret"], PLAINTEXT_CANARY);
    assert!(body.get("api_key").is_none());
    assert!(body.get("metadata").is_none());

    let projection = serde_json::to_string(&view).expect("projection JSON");
    let repository_debug = format!("{:?}", repository(&server.uri()));
    for forbidden in [PLAINTEXT_CANARY, BEARER_CANARY, "xfda:v1:server-ciphertext"] {
        assert!(!projection.contains(forbidden));
        assert!(!repository_debug.contains(forbidden));
    }
}

#[tokio::test]
async fn replacement_uses_the_secret_subresource_and_preserves_row_identity() {
    let server = MockServer::start().await;
    let id = Uuid::from_u128(1);
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            vec![row(
                "openai",
                "tenant-1",
                "user-1",
                "xfda:v1:old-server-ciphertext",
                da_metadata(),
            )],
            None,
            false,
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("{ROUTE}/{id}/secret")))
        .respond_with(ResponseTemplate::new(200).set_body_json(row(
            "openai",
            "tenant-1",
            "user-1",
            "xfda:v1:new-server-ciphertext",
            da_metadata(),
        )))
        .expect(1)
        .mount(&server)
        .await;

    let view = service(&server.uri())
        .replace_secret(
            &actor(),
            CredentialReplaceRequest::new(
                &id.to_string(),
                Zeroizing::new(PLAINTEXT_CANARY.to_owned()),
            )
            .expect("valid replacement"),
        )
        .await
        .expect("replacement succeeds");
    assert_eq!(view.id, id.to_string().into());
    assert_eq!(view.status, CredentialStatus::Ready);

    let requests = server.received_requests().await.expect("recorded requests");
    let request = requests
        .iter()
        .find(|request| request.method == Method::PUT)
        .expect("PUT replacement");
    assert_authorized(request);
    assert_eq!(request.url.path(), format!("{ROUTE}/{id}/secret"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).expect("request JSON"),
        json!({ "secret": PLAINTEXT_CANARY })
    );
    assert!(requests
        .iter()
        .all(|request| request.method != Method::PATCH));
}

fn listed_da_row() -> Value {
    page(
        vec![row(
            "openai",
            "tenant-1",
            "user-1",
            "xfda:v1:server-ciphertext",
            da_metadata(),
        )],
        None,
        false,
    )
}

#[tokio::test]
async fn removal_deletes_the_row_by_id_with_the_bearer() {
    let server = MockServer::start().await;
    let id = Uuid::from_u128(1);
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed_da_row()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{ROUTE}/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(row(
            "openai",
            "tenant-1",
            "user-1",
            "xfda:v1:server-ciphertext",
            da_metadata(),
        )))
        .expect(1)
        .mount(&server)
        .await;

    service(&server.uri())
        .delete(
            &actor(),
            CredentialDeleteRequest::new(&id.to_string()).expect("valid request"),
        )
        .await
        .expect("delete succeeds");

    let requests = server.received_requests().await.expect("recorded requests");
    let request = requests
        .iter()
        .find(|request| request.method == Method::DELETE)
        .expect("DELETE request");
    assert_authorized(request);
    assert_eq!(request.url.path(), format!("{ROUTE}/{id}"));
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn removal_that_da_answers_for_another_row_asks_for_a_reload() {
    let server = MockServer::start().await;
    let id = Uuid::from_u128(1);
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed_da_row()))
        .mount(&server)
        .await;
    let mut foreign = row(
        "openai",
        "tenant-1",
        "user-1",
        "xfda:v1:server-ciphertext",
        da_metadata(),
    );
    foreign["id"] = json!("00000000-0000-0000-0000-000000000002");
    Mock::given(method("DELETE"))
        .and(path(format!("{ROUTE}/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(foreign))
        .mount(&server)
        .await;

    let outcome = service(&server.uri())
        .delete(
            &actor(),
            CredentialDeleteRequest::new(&id.to_string()).expect("valid request"),
        )
        .await;

    assert!(matches!(
        outcome,
        Err(CredentialServiceError::ReconciliationRequired { credentials: None })
    ));
}

#[tokio::test]
async fn removal_maps_da_refusals_without_guessing() {
    for (status, expected) in [
        // Gone between the fresh list and the delete: already what was asked.
        (404, Ok(())),
        (401, Err(CredentialServiceError::Unauthorized)),
        (403, Err(CredentialServiceError::Unauthorized)),
        (429, Err(CredentialServiceError::RateLimited)),
        (503, Err(CredentialServiceError::ServiceUnavailable)),
    ] {
        let server = MockServer::start().await;
        let id = Uuid::from_u128(1);
        Mock::given(method("GET"))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(200).set_body_json(listed_da_row()))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{ROUTE}/{id}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body("refused")))
            .mount(&server)
            .await;

        let outcome = service(&server.uri())
            .delete(
                &actor(),
                CredentialDeleteRequest::new(&id.to_string()).expect("valid request"),
            )
            .await;

        assert_eq!(outcome, expected, "HTTP {status}");
    }
}

#[tokio::test]
async fn a_removal_whose_answer_is_lost_asks_for_a_reload_rather_than_failing() {
    let server = MockServer::start().await;
    let id = Uuid::from_u128(1);
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed_da_row()))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{ROUTE}/{id}")))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(250)))
        .expect(1)
        .mount(&server)
        .await;

    let outcome = service(&server.uri())
        .delete(
            &actor(),
            CredentialDeleteRequest::new(&id.to_string()).expect("valid request"),
        )
        .await;

    assert_eq!(
        outcome,
        Err(CredentialServiceError::ReconciliationRequired { credentials: None })
    );
}

#[tokio::test]
async fn inference_resolves_once_with_no_store_and_returns_a_zeroizing_secret() {
    let server = MockServer::start().await;
    let id = Uuid::from_u128(1);
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            vec![row(
                "anthropic",
                "tenant-1",
                "user-1",
                "xfda:v1:server-ciphertext",
                da_metadata(),
            )],
            None,
            false,
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{ROUTE}/{id}:resolve")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "private, no-store")
                .set_body_json(json!({
                    "id": id,
                    "provider": "anthropic",
                    "base_url": null,
                    "secret": PLAINTEXT_CANARY
                })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let resolved = service(&server.uri())
        .resolve_for_inference(&actor(), ProviderId::Anthropic)
        .await
        .expect("resolve succeeds");
    assert_eq!(resolved.credential_id, id.to_string());
    assert_eq!(resolved.secret.as_str(), PLAINTEXT_CANARY);
    assert!(!format!("{resolved:?}").contains(PLAINTEXT_CANARY));

    let requests = server.received_requests().await.expect("recorded requests");
    let resolve = requests
        .iter()
        .find(|request| request.url.path().ends_with(":resolve"))
        .expect("resolve request");
    assert_eq!(resolve.method, Method::POST);
    assert!(resolve.body.is_empty());
    assert_authorized(resolve);
}

#[tokio::test]
async fn resolve_conflict_is_actionable_and_missing_no_store_fails_closed() {
    for (status, expected) in [
        (409, CredentialServiceError::NoUsableCredential),
        (200, CredentialServiceError::ServiceUnavailable),
    ] {
        let server = MockServer::start().await;
        let id = Uuid::from_u128(1);
        Mock::given(method("GET"))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(
                vec![row(
                    "google",
                    "tenant-1",
                    "user-1",
                    "xfda:v1:server-ciphertext",
                    da_metadata(),
                )],
                None,
                false,
            )))
            .mount(&server)
            .await;
        let response = if status == 200 {
            ResponseTemplate::new(status).set_body_json(json!({
                "id": id,
                "provider": "google",
                "base_url": null,
                "secret": PLAINTEXT_CANARY
            }))
        } else {
            ResponseTemplate::new(status).set_body_json(error_body("conflict"))
        };
        Mock::given(method("POST"))
            .and(path(format!("{ROUTE}/{id}:resolve")))
            .respond_with(response)
            .mount(&server)
            .await;

        assert_eq!(
            service(&server.uri())
                .resolve_for_inference(&actor(), ProviderId::Google)
                .await
                .expect_err("resolve must fail"),
            expected
        );
    }
}

#[tokio::test]
async fn typed_http_statuses_preserve_mutation_certainty() {
    for status in [401, 403] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body("unauthorized")))
            .mount(&server)
            .await;
        assert_eq!(
            repository(&server.uri()).list().await.unwrap_err(),
            CredentialRepositoryError::Unauthorized
        );
    }

    for (status, code, expected) in [
        (401, "unauthorized", CredentialServiceError::Unauthorized),
        (403, "unauthorized", CredentialServiceError::Unauthorized),
        (409, "conflict", CredentialServiceError::Conflict),
        (
            422,
            "validation_error",
            CredentialServiceError::InvalidRequest,
        ),
        (429, "rate_limited", CredentialServiceError::RateLimited),
        (503, "internal", CredentialServiceError::ServiceUnavailable),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body(code)))
            .expect(1)
            .mount(&server)
            .await;
        let error = service(&server.uri())
            .create(&actor(), create_request())
            .await
            .expect_err("status rejects create");
        assert_eq!(error, expected);
        let requests = server.received_requests().await.expect("recorded requests");
        assert_eq!(requests.len(), 1, "explicit rejection must not reconcile");
        assert_eq!(requests[0].method, Method::POST);
    }

    for status in [429, 503] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body("rate_limited")))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            repository(&server.uri()).list().await.unwrap_err(),
            CredentialRepositoryError::Unavailable
        );
    }
}

#[tokio::test]
async fn list_transport_timeout_and_unreachable_are_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(250)))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        repository(&server.uri()).list().await.unwrap_err(),
        CredentialRepositoryError::Unavailable
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    assert_eq!(
        repository(&format!("http://{address}"))
            .list()
            .await
            .unwrap_err(),
        CredentialRepositoryError::Unavailable
    );
}

#[tokio::test]
async fn create_timeout_reconciles_once_without_retrying_the_post() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_millis(250)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(Vec::new(), None, false)))
        .expect(1)
        .mount(&server)
        .await;

    let error = service(&server.uri())
        .create(&actor(), create_request())
        .await
        .expect_err("timed out create requires reconciliation");
    assert_eq!(
        error,
        CredentialServiceError::ReconciliationRequired {
            credentials: Some(Vec::new())
        }
    );
    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == Method::POST)
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == Method::GET)
            .count(),
        1
    );
    let error_debug = format!("{error:?}");
    assert!(!error_debug.contains(PLAINTEXT_CANARY));
    assert!(!error_debug.contains("xfdc:v1:"));
    assert!(!error_debug.contains(BEARER_CANARY));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    assert!(matches!(
        service(&format!("http://{address}"))
            .create(&actor(), create_request())
            .await,
        Err(CredentialServiceError::ReconciliationRequired { .. })
    ));
}

#[tokio::test]
async fn create_503_is_known_unwritten_while_unknown_completion_reconciles() {
    let unavailable = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_json(error_body("internal")))
        .expect(1)
        .mount(&unavailable)
        .await;
    assert_eq!(
        service(&unavailable.uri())
            .create(&actor(), create_request())
            .await,
        Err(CredentialServiceError::ServiceUnavailable)
    );
    assert_eq!(
        unavailable
            .received_requests()
            .await
            .expect("recorded requests")
            .len(),
        1
    );

    let unknown = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(599).set_body_json(error_body("internal")))
        .expect(1)
        .mount(&unknown)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            Vec::new(),
            Some("must-not-be-followed"),
            true,
        )))
        .expect(1)
        .mount(&unknown)
        .await;

    let error = service(&unknown.uri())
        .create(&actor(), create_request())
        .await
        .expect_err("ambiguous create");
    assert!(matches!(
        error,
        CredentialServiceError::ReconciliationRequired { .. }
    ));
    assert_eq!(
        unknown
            .received_requests()
            .await
            .expect("recorded requests")
            .len(),
        2
    );
}

#[tokio::test]
async fn reconciliation_repository_operation_reads_only_the_newest_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(
            Vec::new(),
            Some("must-not-be-followed"),
            true,
        )))
        .expect(1)
        .mount(&server)
        .await;

    assert!(repository(&server.uri())
        .reconcile_after_create()
        .await
        .expect("single reconciliation page")
        .is_empty());
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .len(),
        1
    );
}

#[tokio::test]
async fn malformed_and_oversized_list_responses_fail_closed() {
    let malformed = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"{not-json"))
        .mount(&malformed)
        .await;
    assert_eq!(
        repository(&malformed.uri()).list().await.unwrap_err(),
        CredentialRepositoryError::Unavailable
    );

    for response in [
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_LIST_RESPONSE_BYTES + 1
        ),
        {
            let body = "x".repeat(MAX_LIST_RESPONSE_BYTES + 1);
            format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{}\r\n0\r\n\r\n",
                body.len(),
                body
            )
        },
    ] {
        let origin = spawn_raw_origin(response).await;
        assert_eq!(
            repository(&origin).list().await.unwrap_err(),
            CredentialRepositoryError::Unavailable
        );
    }
}

#[tokio::test]
async fn malformed_or_oversized_created_rows_require_reconciliation() {
    let malformed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201).set_body_bytes(b"{not-json"))
        .mount(&malformed)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(Vec::new(), None, false)))
        .mount(&malformed)
        .await;
    assert!(matches!(
        service(&malformed.uri())
            .create(&actor(), create_request())
            .await,
        Err(CredentialServiceError::ReconciliationRequired { .. })
    ));

    let oversized = spawn_raw_origin(format!(
        "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_ROW_RESPONSE_BYTES + 1
    ))
    .await;
    let direct = service(&oversized)
        .create(&actor(), create_request())
        .await
        .expect_err("oversized create response");
    assert!(matches!(
        direct,
        CredentialServiceError::ReconciliationRequired { .. }
    ));

    let body = "x".repeat(MAX_ROW_RESPONSE_BYTES + 1);
    let streamed = spawn_raw_origin(format!(
        "HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{}\r\n0\r\n\r\n",
        body.len(),
        body
    ))
    .await;
    assert!(matches!(
        service(&streamed).create(&actor(), create_request()).await,
        Err(CredentialServiceError::ReconciliationRequired { .. })
    ));
}

#[tokio::test]
async fn malformed_created_identity_provider_or_da_envelope_reconciles() {
    for mutation in ["id", "tenant", "user", "provider", "owner", "envelope"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).expect("request JSON");
                let mut tenant = "tenant-1";
                let mut user = "user-1";
                let mut provider = body["provider"].as_str().expect("provider");
                match mutation {
                    "tenant" => tenant = "tenant-other",
                    "user" => user = "user-other",
                    "provider" => provider = "google",
                    "id" | "owner" | "envelope" => {}
                    _ => unreachable!("all mutations are enumerated by the test"),
                }
                let mut response = row(
                    provider,
                    tenant,
                    user,
                    "xfda:v1:server-ciphertext",
                    da_metadata(),
                );
                if mutation == "id" {
                    response["id"] = json!(Uuid::nil());
                }
                if mutation == "owner" {
                    response["metadata"]["secret_owner"] = json!("other-client");
                }
                if mutation == "envelope" {
                    response["api_key"] = json!("enc:v1:client-ciphertext");
                }
                ResponseTemplate::new(201).set_body_json(response)
            })
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(Vec::new(), None, false)))
            .expect(1)
            .mount(&server)
            .await;

        assert!(matches!(
            service(&server.uri())
                .create(&actor(), create_request())
                .await,
            Err(CredentialServiceError::ReconciliationRequired { .. })
        ));
        let requests = server.received_requests().await.expect("recorded requests");
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == Method::POST)
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == Method::GET)
                .count(),
            1
        );
    }
}

async fn spawn_raw_origin(response: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("raw response listener");
    let address = listener.local_addr().expect("raw response address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("raw response connection");
        let mut request = vec![0_u8; 8 * 1024];
        let _ = socket.read(&mut request).await.expect("read request");
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.shutdown().await.expect("close response");
    });
    format!("http://{address}")
}

#[tokio::test]
async fn ambiguous_create_confirms_the_stored_secret_without_repeating_the_write() {
    for (provider, provider_name) in [
        (ProviderId::OpenAi, "openai"),
        (ProviderId::Anthropic, "anthropic"),
        (ProviderId::Google, "google"),
        (ProviderId::OpenRouter, "openrouter"),
    ] {
        for confirmed in [true, false] {
            let server = MockServer::start().await;
            let mut stored = row(
                provider_name,
                "tenant-1",
                "user-1",
                "xfda:v1:ciphertext",
                da_metadata(),
            );
            stored["label"] = json!("Wire label");
            Mock::given(method("POST"))
                .and(path(ROUTE))
                .respond_with(ResponseTemplate::new(201).set_body_bytes(b"{lost-response"))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(ROUTE))
                .respond_with(ResponseTemplate::new(200).set_body_json(page(
                    vec![stored],
                    None,
                    false,
                )))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path(format!(
                    "{ROUTE}/00000000-0000-0000-0000-000000000001:resolve"
                )))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("cache-control", "no-store")
                        .set_body_json(json!({
                            "id": "00000000-0000-0000-0000-000000000001",
                            "provider": provider_name,
                            "base_url": null,
                            "secret": if confirmed { PLAINTEXT_CANARY } else { "sk-different-key" },
                        })),
                )
                .expect(1)
                .mount(&server)
                .await;
            let result = service(&server.uri())
                .create(
                    &actor(),
                    CredentialCreateRequest::new(
                        provider,
                        "Wire label",
                        Zeroizing::new(PLAINTEXT_CANARY.to_owned()),
                        true,
                    )
                    .unwrap(),
                )
                .await;
            if confirmed {
                let view = result.expect("the saved secret was verified");
                assert_eq!(view.provider, provider);
                assert_eq!(view.status, CredentialStatus::Ready);
            } else {
                assert!(matches!(
                    result,
                    Err(CredentialServiceError::ReconciliationRequired {
                        credentials: Some(_)
                    })
                ));
            }
            server.verify().await;
        }
    }
}
