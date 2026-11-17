use indexmap::IndexMap;
use serde_json::{json, Value};
use uuid::Uuid;
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::credentials::{CredentialStatus, ProviderId};

use super::super::{
    auth::{AuthenticatedDaClient, AuthenticatedPrincipal, BearerToken},
    generated,
    mapping::{build_hardened_client, DaRoute, TenantBaseUrl},
    provider_keys::{
        append_list_record, bounded_response, domain_record, project_provider_key,
        ProviderKeyRecord,
    },
    secret::Secret,
};

const BEARER_CANARY: &str = "bearer-super-secret-canary";
const CIPHERTEXT_CANARY: &str = "xfdc:v1:ciphertext-super-secret-canary";
const NONCE_CANARY: &str = "nonce-super-secret-canary";
const KEY_ID_CANARY: &str = "key-id-super-secret-canary";

fn provider_key(
    provider: &str,
    api_key: &str,
    metadata: IndexMap<String, Value>,
) -> generated::ProviderKey {
    generated::ProviderKey {
        id: Uuid::nil(),
        tenant_id: "tenant-secret-canary".to_owned(),
        user_id: "user-secret-canary".to_owned(),
        provider: provider.to_owned(),
        label: "Work credential".to_owned(),
        api_key: api_key.to_owned(),
        base_url: Some("https://should-not-project.example".to_owned()),
        is_default: true,
        is_enabled: true,
        metadata,
        created_at: chrono::DateTime::UNIX_EPOCH,
        updated_at: chrono::DateTime::UNIX_EPOCH,
    }
}

fn desktop_envelope_metadata() -> IndexMap<String, Value> {
    IndexMap::from([
        ("device_key_id".to_owned(), json!(KEY_ID_CANARY)),
        ("nonce".to_owned(), json!(NONCE_CANARY)),
    ])
}

#[test]
fn secret_and_bearer_debug_are_redacted() {
    let secret = Secret::new(CIPHERTEXT_CANARY.to_owned());
    let bearer = BearerToken::new(BEARER_CANARY).expect("valid bearer");

    let secret_debug = format!("{secret:?}").to_ascii_lowercase();
    let bearer_debug = format!("{bearer:?}").to_ascii_lowercase();
    for forbidden in [
        CIPHERTEXT_CANARY,
        BEARER_CANARY,
        "api_key",
        "ciphertext",
        "envelope",
        "nonce",
        "key_id",
        "key-id",
    ] {
        assert!(!secret_debug.contains(&forbidden.to_ascii_lowercase()));
        assert!(!bearer_debug.contains(&forbidden.to_ascii_lowercase()));
    }

    let header = bearer.authorization_header().expect("authorization header");
    assert!(header.is_sensitive());
    assert!(BearerToken::new("line\r\ninjection").is_err());
}

#[test]
fn provider_projection_excludes_all_secret_material_and_debug_is_safe() {
    let raw = ProviderKeyRecord::from_generated(provider_key(
        "openai",
        CIPHERTEXT_CANARY,
        desktop_envelope_metadata(),
    ));
    let raw_debug = format!("{raw:?}").to_ascii_lowercase();
    let projected = project_provider_key(&raw).expect("supported provider");
    let wire = serde_json::to_string(&projected)
        .expect("redacted projection serializes")
        .to_ascii_lowercase();

    assert_eq!(projected.provider, ProviderId::OpenAi);
    assert_eq!(projected.status, CredentialStatus::Ready);
    for forbidden in [
        CIPHERTEXT_CANARY,
        NONCE_CANARY,
        KEY_ID_CANARY,
        "api_key",
        "ciphertext",
        "envelope",
        "nonce",
        "key_id",
        "key-id",
        "tenant-secret-canary",
        "user-secret-canary",
        "should-not-project",
    ] {
        let forbidden = forbidden.to_ascii_lowercase();
        assert!(
            !raw_debug.contains(&forbidden),
            "raw Debug leaked {forbidden}"
        );
        assert!(!wire.contains(&forbidden), "projection leaked {forbidden}");
    }
}

#[test]
fn da_owned_credentials_are_ready_without_device_metadata() {
    let raw = ProviderKeyRecord::from_generated(provider_key(
        "openrouter",
        "xfda:v1:server-ciphertext",
        IndexMap::from([("secret_owner".to_owned(), json!("data-analyst"))]),
    ));

    let projected = project_provider_key(&raw).expect("supported provider");

    assert_eq!(projected.status, CredentialStatus::Ready);
}

#[test]
fn foreign_providers_fail_closed_and_foreign_envelopes_require_reentry() {
    let foreign_provider = ProviderKeyRecord::from_generated(provider_key(
        "openai-compatible",
        CIPHERTEXT_CANARY,
        desktop_envelope_metadata(),
    ));
    assert!(project_provider_key(&foreign_provider).is_err());

    for (ciphertext, metadata) in [
        ("plain-provider-key", IndexMap::new()),
        ("enc:v1:web-ciphertext", IndexMap::new()),
        (
            CIPHERTEXT_CANARY,
            IndexMap::from([("device_key_id".to_owned(), json!(""))]),
        ),
        (
            "enc:v1:client-ciphertext",
            IndexMap::from([("secret_owner".to_owned(), json!("data-analyst"))]),
        ),
    ] {
        let raw =
            ProviderKeyRecord::from_generated(provider_key("anthropic", ciphertext, metadata));
        let view = project_provider_key(&raw).expect("provider itself is supported");
        assert_eq!(view.provider, ProviderId::Anthropic);
        assert_eq!(view.status, CredentialStatus::NeedsReentry);
    }
}

#[test]
fn disabled_compatible_credentials_remain_disabled() {
    let mut dto = provider_key("google", CIPHERTEXT_CANARY, desktop_envelope_metadata());
    dto.is_enabled = false;
    let raw = ProviderKeyRecord::from_generated(dto);

    let view = project_provider_key(&raw).expect("supported provider");

    assert_eq!(view.provider, ProviderId::Google);
    assert_eq!(view.status, CredentialStatus::Disabled);
}

#[test]
fn tenant_urls_are_https_fixed_origin_and_cannot_redirect_authority() {
    for invalid in [
        "http://tenant.example",
        "https://user@tenant.example",
        "https://tenant.example?query=1",
        "https://tenant.example#fragment",
        "https://tenant.example/base",
        "https://tenant.example\\@evil.example",
    ] {
        assert!(
            TenantBaseUrl::parse(invalid).is_err(),
            "accepted invalid configured origin {invalid}"
        );
    }

    let base = TenantBaseUrl::parse("https://tenant.example:8443/").expect("valid origin");
    let provider_keys = base.url_for(DaRoute::ProviderKeys);
    let models = base.url_for(DaRoute::Models);
    let audit_events = base.url_for(DaRoute::AuditEvents);

    for url in [provider_keys, models, audit_events] {
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("tenant.example"));
        assert_eq!(url.port(), Some(8443));
        assert!(url.username().is_empty());
        assert!(url.password().is_none());
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
    }

    let client = build_hardened_client().expect("hardened reqwest client");
    let identity = IdentityProjection::new("user-1", "tenant-1", "Test User", None);
    let principal =
        AuthenticatedPrincipal::from_verified_identity(&identity).expect("verified principal");
    let transport = AuthenticatedDaClient::new(
        base,
        client,
        BearerToken::new(BEARER_CANARY).unwrap(),
        principal,
    );
    let request = transport
        .get(DaRoute::ProviderKeys)
        .expect("fixed-route request")
        .build()
        .expect("request builds");
    assert_eq!(request.url().host_str(), Some("tenant.example"));
    assert!(
        !format!("{request:?}").contains(BEARER_CANARY),
        "request Debug exposed the bearer"
    );

    let request = transport
        .post(DaRoute::AuditEvents)
        .expect("fixed-route audit request")
        .build()
        .expect("audit request builds");
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://tenant.example:8443/api/v1/chat/audit-events"
    );
    assert!(
        !format!("{request:?}").contains(BEARER_CANARY),
        "audit request Debug exposed the bearer"
    );
}

#[test]
fn da_provider_rows_must_match_the_verified_principal() {
    let base = TenantBaseUrl::parse("https://tenant.example").expect("valid origin");
    let client = build_hardened_client().expect("hardened client");
    let identity = IdentityProjection::new(
        "user-secret-canary",
        "tenant-secret-canary",
        "Test User",
        None,
    );
    let principal =
        AuthenticatedPrincipal::from_verified_identity(&identity).expect("verified principal");
    let transport = AuthenticatedDaClient::new(
        base,
        client,
        BearerToken::new(BEARER_CANARY).unwrap(),
        principal,
    );

    let matching = provider_key("openai", CIPHERTEXT_CANARY, desktop_envelope_metadata());
    assert!(domain_record(&transport, matching).is_ok());

    let mut wrong_tenant = provider_key("openai", CIPHERTEXT_CANARY, desktop_envelope_metadata());
    wrong_tenant.tenant_id = "tenant-other".to_owned();
    assert!(domain_record(&transport, wrong_tenant).is_err());

    let mut wrong_user = provider_key("openai", CIPHERTEXT_CANARY, desktop_envelope_metadata());
    wrong_user.user_id = "user-other".to_owned();
    assert!(domain_record(&transport, wrong_user).is_err());
}

#[test]
fn unsupported_provider_rows_do_not_block_supported_list_rows() {
    let base = TenantBaseUrl::parse("https://tenant.example").expect("valid origin");
    let client = build_hardened_client().expect("hardened client");
    let identity = IdentityProjection::new(
        "user-secret-canary",
        "tenant-secret-canary",
        "Test User",
        None,
    );
    let principal =
        AuthenticatedPrincipal::from_verified_identity(&identity).expect("verified principal");
    let transport = AuthenticatedDaClient::new(
        base,
        client,
        BearerToken::new(BEARER_CANARY).unwrap(),
        principal,
    );
    let mut records = Vec::new();

    append_list_record(
        &transport,
        &mut records,
        provider_key(
            "openai-compatible",
            CIPHERTEXT_CANARY,
            desktop_envelope_metadata(),
        ),
    )
    .expect("foreign provider is safely skipped");
    append_list_record(
        &transport,
        &mut records,
        provider_key("openai", CIPHERTEXT_CANARY, desktop_envelope_metadata()),
    )
    .expect("supported provider remains visible");

    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn provider_response_reader_rejects_declared_and_streamed_oversize_bodies() {
    let declared = http::Response::builder()
        .status(200)
        .header(http::header::CONTENT_LENGTH, "9")
        .body(reqwest::Body::from(vec![0_u8; 9]))
        .expect("declared response");
    assert!(bounded_response(reqwest::Response::from(declared), 8)
        .await
        .is_err());

    let streamed = http::Response::builder()
        .status(200)
        .body(reqwest::Body::from(vec![0_u8; 9]))
        .expect("streamed response");
    assert!(bounded_response(reqwest::Response::from(streamed), 8)
        .await
        .is_err());
}

#[test]
fn provider_rows_reject_oversized_secret_class_fields() {
    let base = TenantBaseUrl::parse("https://tenant.example").expect("valid origin");
    let client = build_hardened_client().expect("hardened client");
    let identity = IdentityProjection::new(
        "user-secret-canary",
        "tenant-secret-canary",
        "Test User",
        None,
    );
    let principal =
        AuthenticatedPrincipal::from_verified_identity(&identity).expect("verified principal");
    let transport = AuthenticatedDaClient::new(
        base,
        client,
        BearerToken::new(BEARER_CANARY).unwrap(),
        principal,
    );
    let mut row = provider_key("openai", CIPHERTEXT_CANARY, desktop_envelope_metadata());
    row.api_key = "x".repeat(32 * 1024 + 1);

    assert!(domain_record(&transport, row).is_err());
}
