use std::time::Duration;
use std::{
    io,
    sync::{Arc, Mutex},
};

use reqwest::{header, Method, Url};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tracing_subscriber::fmt::MakeWriter;
use wiremock::{
    matchers::{header as header_matcher, method, path},
    Mock, MockServer, ResponseTemplate,
};
use xfalcon_contracts::credentials::ProviderId;

use crate::{
    credentials::verifier::{CredentialVerifier, VerificationError, VerifiedCredential},
    providers::{
        FixedOriginCredentialVerifier, ANTHROPIC_MODELS_ENDPOINT, GOOGLE_MODELS_ENDPOINT,
        MAX_VERIFICATION_RESPONSE_BYTES, OPENAI_MODELS_ENDPOINT, OPENROUTER_KEY_ENDPOINT,
    },
};

const SECRET_CANARY: &str = "provider-secret-super-canary";
const VERIFIER_SOURCE: &str = include_str!("mod.rs");
const NO_RETRY_BUILDER_CHAIN_LF: &[u8] = b".redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout";
const NO_RETRY_BUILDER_CHAIN_CRLF: &[u8] = b".redirect(Policy::none())\r
            .retry(reqwest::retry::never())\r
            .connect_timeout";
const HAS_NO_RETRY_BUILDER_CHAIN: bool =
    contains_bytes(VERIFIER_SOURCE.as_bytes(), NO_RETRY_BUILDER_CHAIN_LF)
        || contains_bytes(VERIFIER_SOURCE.as_bytes(), NO_RETRY_BUILDER_CHAIN_CRLF);
const _: () = assert!(HAS_NO_RETRY_BUILDER_CHAIN);

const fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut start = 0;
    while start + needle.len() <= haystack.len() {
        let mut offset = 0;
        while offset < needle.len() && haystack[start + offset] == needle[offset] {
            offset += 1;
        }
        if offset == needle.len() {
            return true;
        }
        start += 1;
    }
    false
}

#[derive(Clone)]
struct CapturedProviderLog(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedProviderLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("captured provider log lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedProviderLog {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn provider_transport_log_omits_endpoint_details() {
    use crate::providers::{openai_compat, ProviderError, ProviderStreamEvent};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local port");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    let canary = "endpoint-secret-canary";
    let endpoint = format!("http://{address}/v1/chat/completions?token={canary}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("local test client");
    let request_builder = client.post(&endpoint);
    let body = serde_json::json!({"model":"test-model","messages":[],"stream":true});
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CapturedProviderLog(bytes.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut on_event = |_event: ProviderStreamEvent| {};
    let result = openai_compat::stream_completion(
        request_builder,
        &body,
        &tokio_util::sync::CancellationToken::new(),
        &mut on_event,
    )
    .await;

    assert_eq!(result, Err(ProviderError::Unavailable));
    let log = String::from_utf8(bytes.lock().expect("captured provider log lock").clone())
        .expect("captured logs are UTF-8");
    assert!(!log.contains(canary));
    assert!(!log.contains(&endpoint));
    assert!(log.contains("error_kind"));
    assert!(log.contains("connect"));
}

fn endpoint(origin: &str, path: &str) -> Url {
    Url::parse(&format!("{origin}{path}")).expect("test endpoint")
}

fn verifier(
    openai: Url,
    anthropic: Url,
    google: Url,
    openrouter: Url,
) -> FixedOriginCredentialVerifier {
    FixedOriginCredentialVerifier::for_test(
        openai,
        anthropic,
        google,
        openrouter,
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .expect("test verifier")
}

fn verifier_for(server: &MockServer) -> FixedOriginCredentialVerifier {
    verifier(
        endpoint(&server.uri(), "/v1/models"),
        endpoint(&server.uri(), "/v1/models"),
        endpoint(&server.uri(), "/v1beta/models"),
        endpoint(&server.uri(), "/api/v1/key"),
    )
}

#[test]
fn production_model_endpoints_are_exact_fixed_origins_and_paths() {
    assert_eq!(OPENAI_MODELS_ENDPOINT, "https://api.openai.com/v1/models");
    assert_eq!(
        ANTHROPIC_MODELS_ENDPOINT,
        "https://api.anthropic.com/v1/models"
    );
    assert_eq!(
        GOOGLE_MODELS_ENDPOINT,
        "https://generativelanguage.googleapis.com/v1beta/models"
    );
    // OpenRouter's `/models` is unauthenticated and would answer 200 to a
    // garbage key; `/key` is the endpoint that actually checks the bearer.
    assert_eq!(OPENROUTER_KEY_ENDPOINT, "https://openrouter.ai/api/v1/key");
    assert!(
        !OPENROUTER_KEY_ENDPOINT.ends_with("/models"),
        "verifying against an unauthenticated endpoint would accept any key"
    );

    for endpoint in [
        OPENAI_MODELS_ENDPOINT,
        ANTHROPIC_MODELS_ENDPOINT,
        GOOGLE_MODELS_ENDPOINT,
        OPENROUTER_KEY_ENDPOINT,
    ] {
        let url = Url::parse(endpoint).expect("production endpoint");
        assert_eq!(url.scheme(), "https");
        assert!(url.username().is_empty());
        assert!(url.password().is_none());
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
    }
}

#[test]
fn verification_client_explicitly_disables_reqwest_protocol_retries() {
    // reqwest 0.13 retries some low-level protocol NACKs by default. Keeping
    // this structural assertion compile-time makes `--no-run` authoritative
    // on the Windows host where test binaries cannot currently load.
    assert!(
        contains_bytes(
            std::hint::black_box(VERIFIER_SOURCE.as_bytes()),
            NO_RETRY_BUILDER_CHAIN_LF
        ) || contains_bytes(
            std::hint::black_box(VERIFIER_SOURCE.as_bytes()),
            NO_RETRY_BUILDER_CHAIN_CRLF
        )
    );
}

#[tokio::test]
async fn each_provider_uses_exact_get_path_and_auth_contract_without_url_secrets() {
    let server = MockServer::start().await;
    let verifier = verifier_for(&server);
    let cases = [
        (
            ProviderId::OpenAi,
            "/v1/models",
            header::AUTHORIZATION.as_str(),
            format!("Bearer {SECRET_CANARY}"),
            None,
        ),
        (
            ProviderId::Anthropic,
            "/v1/models",
            "x-api-key",
            SECRET_CANARY.to_owned(),
            Some(("anthropic-version", "2023-06-01")),
        ),
        (
            ProviderId::Google,
            "/v1beta/models",
            "x-goog-api-key",
            SECRET_CANARY.to_owned(),
            None,
        ),
        (
            ProviderId::OpenRouter,
            "/api/v1/key",
            header::AUTHORIZATION.as_str(),
            format!("Bearer {SECRET_CANARY}"),
            None,
        ),
    ];

    for (provider, expected_path, auth_name, auth_value, extra_header) in cases {
        server.reset().await;
        let mut mock = Mock::given(method("GET"))
            .and(path(expected_path))
            .and(header_matcher(auth_name, auth_value.as_str()));
        if let Some((name, value)) = extra_header {
            mock = mock.and(header_matcher(name, value));
        }
        mock.respond_with(ResponseTemplate::new(200).set_body_bytes(b"{}"))
            .expect(1)
            .mount(&server)
            .await;

        assert_eq!(
            verifier.verify(provider, SECRET_CANARY).await,
            Ok(VerifiedCredential)
        );

        let requests = server
            .received_requests()
            .await
            .expect("request recording enabled");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.url.path(), expected_path);
        assert!(request.url.query().is_none());
        assert!(!request.url.as_str().contains(SECRET_CANARY));
        assert!(request.body.is_empty());
    }
}

#[tokio::test]
async fn provider_statuses_map_to_typed_verification_results() {
    let server = MockServer::start().await;
    let verifier = verifier_for(&server);
    let mappings = [
        (200, Ok(VerifiedCredential)),
        (204, Ok(VerifiedCredential)),
        (400, Err(VerificationError::InvalidCredential)),
        (401, Err(VerificationError::InvalidCredential)),
        (403, Err(VerificationError::InsufficientPermissions)),
        (429, Err(VerificationError::RateLimited)),
        (300, Err(VerificationError::ProviderUnavailable)),
        (404, Err(VerificationError::ProviderUnavailable)),
        (500, Err(VerificationError::ProviderUnavailable)),
        (503, Err(VerificationError::ProviderUnavailable)),
    ];

    for provider in [
        ProviderId::OpenAi,
        ProviderId::Anthropic,
        ProviderId::Google,
        ProviderId::OpenRouter,
    ] {
        for (status, expected) in mappings {
            server.reset().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;

            assert_eq!(
                verifier.verify(provider, SECRET_CANARY).await,
                expected,
                "provider {provider:?}, status {status}"
            );
        }
    }
}

#[tokio::test]
async fn redirects_are_not_followed_even_when_the_location_is_reachable() {
    let redirector = MockServer::start().await;
    let target = MockServer::start().await;
    let verifier = verifier_for(&redirector);
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/credential-leak", target.uri())),
        )
        .expect(1)
        .mount(&redirector)
        .await;
    Mock::given(method("GET"))
        .and(path("/credential-leak"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&target)
        .await;

    assert_eq!(
        verifier.verify(ProviderId::OpenAi, SECRET_CANARY).await,
        Err(VerificationError::ProviderUnavailable)
    );
    assert!(target
        .received_requests()
        .await
        .expect("request recording enabled")
        .is_empty());
}

#[tokio::test]
async fn invalid_secret_headers_are_rejected_before_any_request_is_sent() {
    let server = MockServer::start().await;
    let verifier = verifier_for(&server);
    let injected = "credential\r\nx-injected: true";

    for provider in [
        ProviderId::OpenAi,
        ProviderId::Anthropic,
        ProviderId::Google,
    ] {
        assert_eq!(
            verifier.verify(provider, injected).await,
            Err(VerificationError::InvalidCredential)
        );
    }
    assert!(server
        .received_requests()
        .await
        .expect("request recording enabled")
        .is_empty());
}

#[test]
fn request_debug_redacts_every_sensitive_provider_header() {
    let verifier = verifier(
        Url::parse("http://127.0.0.1:1/v1/models").unwrap(),
        Url::parse("http://127.0.0.1:1/v1/models").unwrap(),
        Url::parse("http://127.0.0.1:1/v1beta/models").unwrap(),
        Url::parse("http://127.0.0.1:1/api/v1/key").unwrap(),
    );

    for (provider, header_name) in [
        (ProviderId::OpenAi, header::AUTHORIZATION.as_str()),
        (ProviderId::Anthropic, "x-api-key"),
        (ProviderId::Google, "x-goog-api-key"),
        (ProviderId::OpenRouter, header::AUTHORIZATION.as_str()),
    ] {
        let request = verifier
            .request(provider, SECRET_CANARY)
            .expect("valid request")
            .build()
            .expect("request builds");
        let secret_header = request
            .headers()
            .get(header_name)
            .expect("provider secret header");
        assert!(secret_header.is_sensitive());
        assert!(!request.url().as_str().contains(SECRET_CANARY));
        assert!(!format!("{request:?}").contains(SECRET_CANARY));
    }
}

#[tokio::test]
async fn declared_and_streamed_oversized_bodies_fail_closed() {
    let declared = spawn_raw_response(format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_VERIFICATION_RESPONSE_BYTES + 1
    ))
    .await;
    let declared_verifier = verifier(
        declared.clone(),
        declared.clone(),
        declared.clone(),
        declared,
    );
    assert_eq!(
        declared_verifier
            .verify(ProviderId::OpenAi, SECRET_CANARY)
            .await,
        Err(VerificationError::ProviderUnavailable)
    );

    let body = "x".repeat(MAX_VERIFICATION_RESPONSE_BYTES + 1);
    let streamed = spawn_raw_response(format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{}\r\n0\r\n\r\n",
        body.len(),
        body
    ))
    .await;
    let streamed_verifier = verifier(
        streamed.clone(),
        streamed.clone(),
        streamed.clone(),
        streamed,
    );
    assert_eq!(
        streamed_verifier
            .verify(ProviderId::Google, SECRET_CANARY)
            .await,
        Err(VerificationError::ProviderUnavailable)
    );
}

#[tokio::test]
async fn timeout_and_transport_failures_map_to_provider_unavailable_without_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
        .expect(1)
        .mount(&server)
        .await;
    let timed = FixedOriginCredentialVerifier::for_test(
        endpoint(&server.uri(), "/v1/models"),
        endpoint(&server.uri(), "/v1/models"),
        endpoint(&server.uri(), "/v1beta/models"),
        endpoint(&server.uri(), "/api/v1/key"),
        Duration::from_millis(50),
        Duration::from_millis(50),
        Duration::from_millis(50),
    )
    .expect("timeout verifier");
    assert_eq!(
        timed.verify(ProviderId::Anthropic, SECRET_CANARY).await,
        Err(VerificationError::ProviderUnavailable)
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("unused port");
    let address = listener.local_addr().expect("listener address");
    drop(listener);
    let unavailable = Url::parse(&format!("http://{address}/v1/models")).unwrap();
    let broken = verifier(
        unavailable.clone(),
        unavailable.clone(),
        unavailable.clone(),
        unavailable,
    );
    assert_eq!(
        broken.verify(ProviderId::OpenAi, SECRET_CANARY).await,
        Err(VerificationError::ProviderUnavailable)
    );
}

async fn spawn_raw_response(response: String) -> Url {
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
    Url::parse(&format!("http://{address}/v1/models")).expect("raw response URL")
}
