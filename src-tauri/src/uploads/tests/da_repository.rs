//! DA's upload contract, and the credentials it hands back.

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::clients::da::chat_api::DaChatApiError;
use crate::uploads::da_repository::{
    validate_presigned_url, validate_presigned_url_with, ChatAttachmentRepository, CompletedPart,
    DaUploadRepository, PersistentUploadRepository, Presigned, TransferPlan, UploadRefusal,
    UploadRefusalCode, ENFORCE_HTTPS_PRESIGNED_URLS,
};
use crate::uploads::domain::DatasetIngest;
use crate::uploads::kind::{classify_chat_attachment, AttachmentKind};

/// A URL shaped like a real presigned PUT, including the query parameters that
/// carry the signature — a canary for redaction assertions.
const CANARY_URL: &str = "https://tenant-bucket.s3.amazonaws.com/uploads/abc123\
?X-Amz-Signature=deadbeefcafe&X-Amz-Credential=AKIAEXAMPLE";
const CANARY_KEY: &str = "tenants/tenant-1/uploads/abc123.csv";

fn repository(server: &MockServer) -> DaUploadRepository {
    DaUploadRepository::for_test(&server.uri(), "bearer-token").expect("repository")
}

// ---------------------------------------------------------------------
// Presign: the three modes DA can answer with
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_single_mode_presign_yields_one_put_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": CANARY_URL,
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await
        .expect("presign");

    assert_eq!(prepared.dataset_id.as_deref(), Some("dataset-1"));
    assert_eq!(prepared.expires_in, Some(900));
    assert!(matches!(prepared.plan, TransferPlan::Single { .. }));
}

#[tokio::test]
async fn a_multipart_presign_preserves_das_part_size_and_urls_exactly() {
    // Part size is DA's decision, derived from its own storage backend's
    // constraints. Recomputing or rounding it client-side would produce parts
    // S3 rejects at complete time.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "multipart",
            "dataset_id": "dataset-1",
            "upload_id": "s3-upload-1",
            "part_size": 8_388_608,
            "parts": [
                {"part_number": 1, "url": CANARY_URL},
                {"part_number": 2, "url": CANARY_URL},
            ],
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .begin(
            "big.csv",
            "text/csv",
            16_000_000,
            "Big",
            DatasetIngest::default(),
        )
        .await
        .expect("presign");

    let TransferPlan::Multipart {
        upload_id,
        part_size,
        parts,
    } = prepared.plan
    else {
        panic!("expected a multipart plan");
    };
    assert_eq!(upload_id, "s3-upload-1");
    assert_eq!(part_size, 8_388_608);
    assert_eq!(parts.expose().len(), 2);
    assert_eq!(parts.expose()[0].part_number, 1);
}

#[tokio::test]
async fn legacy_mode_is_honoured_rather_than_treated_as_a_failure() {
    // An on-prem `file://` backend cannot presign. Rejecting this response
    // would make uploads impossible on those installs.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"mode": "legacy", "dataset_id": null})),
        )
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await
        .expect("presign");

    assert!(matches!(prepared.plan, TransferPlan::Legacy));
}

#[tokio::test]
async fn a_multipart_response_missing_its_parts_is_rejected_not_treated_as_empty() {
    // Treating it as zero parts would upload nothing and then report success.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "multipart",
            "dataset_id": "dataset-1",
            "upload_id": "s3-upload-1",
            "part_size": 8_388_608,
            "parts": [],
        })))
        .mount(&server)
        .await;

    let result = repository(&server)
        .begin(
            "big.csv",
            "text/csv",
            16_000_000,
            "Big",
            DatasetIngest::default(),
        )
        .await;

    assert!(matches!(
        result,
        Err(UploadRefusal {
            error: DaChatApiError::InvalidResponse,
            ..
        })
    ));
}

#[tokio::test]
async fn an_unknown_transfer_mode_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"mode": "quantum"})))
        .mount(&server)
        .await;

    let result = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await;

    assert!(matches!(
        result,
        Err(UploadRefusal {
            error: DaChatApiError::InvalidResponse,
            ..
        })
    ));
}

// ---------------------------------------------------------------------
// Presigned URLs are validated before any byte is sent to them
// ---------------------------------------------------------------------

#[test]
fn an_https_presigned_url_passes_validation() {
    let url = reqwest::Url::parse(CANARY_URL).expect("url");
    assert!(validate_presigned_url(&url, false).is_ok());
}

#[test]
fn plain_http_to_a_public_host_is_refused_unless_the_dev_backend_allows_it() {
    for raw in [
        "http://tenant-bucket.example.com:9000/bucket/key?X-Amz-Signature=abc",
        "http://203.0.113.10:9000/bucket/key",
    ] {
        let url = reqwest::Url::parse(raw).expect("url");
        assert_eq!(
            validate_presigned_url_with(&url, false, true),
            Err(DaChatApiError::InvalidResponse),
            "{raw}"
        );
        assert!(
            validate_presigned_url_with(&url, true, true).is_ok(),
            "{raw}"
        );
    }
}

#[test]
fn plain_http_to_on_prem_object_storage_is_accepted() {
    for raw in [
        "http://minio:9000/bucket/key?X-Amz-Signature=abc",
        "http://minio.local:9000/bucket/key",
        "http://storage.corp/bucket/key",
        "http://10.1.2.3:9000/bucket/key",
        "http://192.168.0.20/bucket/key",
        "http://[fd00::1]:9000/bucket/key",
    ] {
        let url = reqwest::Url::parse(raw).expect("url");
        assert!(
            validate_presigned_url_with(&url, false, true).is_ok(),
            "{raw}"
        );
    }
}

#[test]
fn a_suffix_lookalike_is_not_a_private_host() {
    for raw in [
        "http://notlocal.example.com/key",
        "http://corp.example.com/key",
        "http://internal.example.org/key",
    ] {
        let url = reqwest::Url::parse(raw).expect("url");
        assert_eq!(
            validate_presigned_url_with(&url, false, true),
            Err(DaChatApiError::InvalidResponse),
            "{raw}"
        );
    }
}

#[test]
fn with_enforcement_off_plain_http_is_accepted_to_any_host() {
    for raw in [
        "http://tenant-bucket.example.com:9000/b/k?X-Amz-Signature=abc",
        "http://203.0.113.10:9000/b/k",
        "http://minio.company.com/b/k",
        "http://10.1.2.3:9000/b/k",
        "http://localhost:9000/b/k",
        "http://[fd00::1]:9000/b/k",
    ] {
        let url = reqwest::Url::parse(raw).expect("url");
        assert!(
            validate_presigned_url_with(&url, false, false).is_ok(),
            "{raw}"
        );
    }
}

#[test]
fn with_enforcement_off_userinfo_and_non_web_schemes_are_still_refused() {
    for raw in [
        "https://user:secret@x.example/k",
        "http://user@10.0.0.5/k",
        "ftp://tenant-bucket.example/key",
        "file:///etc/passwd",
    ] {
        let url = reqwest::Url::parse(raw).expect("url");
        assert_eq!(
            validate_presigned_url_with(&url, true, false),
            Err(DaChatApiError::InvalidResponse),
            "{raw}"
        );
    }
}

#[test]
fn the_default_wrapper_follows_the_enforcement_constant() {
    let url = reqwest::Url::parse("http://minio.company.com/b/k").expect("url");
    assert_eq!(
        validate_presigned_url(&url, false).is_ok(),
        !ENFORCE_HTTPS_PRESIGNED_URLS
    );
}

#[test]
fn a_presigned_url_with_userinfo_is_refused() {
    for raw in [
        "https://user:secret@tenant-bucket.example/key",
        "https://user@tenant-bucket.example/key",
    ] {
        let url = reqwest::Url::parse(raw).expect("url");
        assert_eq!(
            validate_presigned_url(&url, true),
            Err(DaChatApiError::InvalidResponse)
        );
    }
}

#[test]
fn a_non_web_scheme_is_refused_even_in_dev() {
    let url = reqwest::Url::parse("ftp://tenant-bucket.example/key").expect("url");
    assert_eq!(
        validate_presigned_url(&url, true),
        Err(DaChatApiError::InvalidResponse)
    );
}

#[tokio::test]
async fn a_single_presign_with_userinfo_in_its_url_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": "https://user:secret@tenant-bucket.example/uploads/abc",
        })))
        .mount(&server)
        .await;

    let result = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await;

    assert!(matches!(
        result,
        Err(UploadRefusal {
            error: DaChatApiError::InvalidResponse,
            ..
        })
    ));
}

#[tokio::test]
async fn a_single_presign_with_no_host_is_refused() {
    // The URL parser does not accept an https URL without a host, so this is
    // refused as malformed rather than reaching the host check.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": "https://",
        })))
        .mount(&server)
        .await;

    let result = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await;

    assert!(matches!(
        result,
        Err(UploadRefusal {
            error: DaChatApiError::InvalidResponse,
            ..
        })
    ));
}

#[tokio::test]
async fn a_multipart_presign_with_one_bad_part_url_is_refused_as_a_whole() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "multipart",
            "dataset_id": "dataset-1",
            "upload_id": "s3-upload-1",
            "part_size": 8_388_608,
            "parts": [
                {"part_number": 1, "url": CANARY_URL},
                {"part_number": 2, "url": "https://user:secret@tenant-bucket.example/key"},
            ],
        })))
        .mount(&server)
        .await;

    let result = repository(&server)
        .begin(
            "big.csv",
            "text/csv",
            16_000_000,
            "Big",
            DatasetIngest::default(),
        )
        .await;

    assert!(matches!(
        result,
        Err(UploadRefusal {
            error: DaChatApiError::InvalidResponse,
            ..
        })
    ));
}

#[tokio::test]
async fn an_attachment_presign_with_a_non_web_url_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/chat-attachments/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "s3",
            "storage_key": CANARY_KEY,
            "url": "ftp://tenant-bucket.example/key",
        })))
        .mount(&server)
        .await;

    let result = repository(&server)
        .presign_attachment("conversation-1", "chart.png", "image/png", 2_048, "image")
        .await;

    assert!(matches!(result, Err(DaChatApiError::InvalidResponse)));
}

#[tokio::test]
async fn a_refused_presigned_url_never_appears_in_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": "https://user:secret@tenant-bucket.example/uploads/abc?X-Amz-Signature=deadbeefcafe",
        })))
        .mount(&server)
        .await;

    let refusal = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await
        .expect_err("refused");

    for rendered in [format!("{refusal:?}"), refusal.error.to_string()] {
        assert!(!rendered.contains("deadbeefcafe"), "{rendered}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(!rendered.contains("tenant-bucket"), "{rendered}");
    }
}

// ---------------------------------------------------------------------
// Finalize
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_single_put_finalize_sends_null_parts_not_an_empty_list() {
    // DA distinguishes "no multipart" from "multipart that completed zero
    // parts"; the latter asks it to complete an upload that sent nothing.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dataset_id": "dataset-1",
            "table_name": "q3",
            "status": "ready",
            "row_count": 10,
            "column_count": 3,
        })))
        .mount(&server)
        .await;

    repository(&server)
        .finalize(
            "dataset-1",
            None,
            &[],
            "Q3",
            "q3.csv",
            DatasetIngest::default(),
        )
        .await
        .expect("finalize");

    let request = &server.received_requests().await.expect("requests")[0];
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json");
    assert!(body["parts"].is_null(), "{body}");
    assert!(body["upload_id"].is_null(), "{body}");
}

#[tokio::test]
async fn a_multipart_finalize_sends_every_completed_part() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dataset_id": "dataset-1",
            "table_name": "big",
            "status": "ready",
            "row_count": 1_000,
            "column_count": 5,
        })))
        .mount(&server)
        .await;

    repository(&server)
        .finalize(
            "dataset-1",
            Some("s3-upload-1"),
            &[
                CompletedPart {
                    part_number: 1,
                    etag: "\"etag-1\"".to_owned(),
                },
                CompletedPart {
                    part_number: 2,
                    etag: "\"etag-2\"".to_owned(),
                },
            ],
            "Big",
            "big.csv",
            DatasetIngest::default(),
        )
        .await
        .expect("finalize");

    let request = &server.received_requests().await.expect("requests")[0];
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json");
    assert_eq!(body["parts"].as_array().expect("parts").len(), 2);
    assert_eq!(body["parts"][1]["etag"], "\"etag-2\"");
}

#[tokio::test]
async fn an_already_finalized_upload_reports_a_conflict_not_a_generic_failure() {
    // A retry after a response that was lost in flight is the ordinary case.
    // The caller can treat a conflict as "already done"; it cannot treat an
    // opaque failure that way.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/finalize"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;

    let result = repository(&server)
        .finalize(
            "dataset-1",
            None,
            &[],
            "Q3",
            "q3.csv",
            DatasetIngest::default(),
        )
        .await;

    assert!(matches!(
        result,
        Err(UploadRefusal {
            error: DaChatApiError::Conflict,
            ..
        })
    ));
}

// ---------------------------------------------------------------------
// Resign, abort, list
// ---------------------------------------------------------------------

#[tokio::test]
async fn resigning_asks_only_for_the_parts_still_outstanding() {
    // Re-signing every part would discard URLs that are still valid and, for
    // a large upload, multiply the work DA does for no benefit.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/s3-upload-1/resign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "parts": [{"part_number": 7, "url": CANARY_URL}],
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let (parts, expires_in) = repository(&server)
        .resign("s3-upload-1", "dataset-1", &[7])
        .await
        .expect("resign");

    assert_eq!(expires_in, 900);
    assert_eq!(parts.expose().len(), 1);
    let request = &server.received_requests().await.expect("requests")[0];
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json");
    assert_eq!(body["part_numbers"], json!([7]));
}

#[tokio::test]
async fn aborting_targets_the_upload_and_its_dataset() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/s3-upload-1/abort"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    repository(&server)
        .abort("s3-upload-1", "dataset-1")
        .await
        .expect("abort");

    let request = &server.received_requests().await.expect("requests")[0];
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json");
    assert_eq!(body["dataset_id"], "dataset-1");
}

#[tokio::test]
async fn listing_passes_pagination_through_unchanged() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .and(query_param("limit", "25"))
        .and(query_param("offset", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [],
            "limit": 25,
            "offset": 50,
            "total": 0,
        })))
        .mount(&server)
        .await;

    let listed = repository(&server).list(25, 50).await.expect("list");

    assert_eq!(listed.limit, 25);
    assert_eq!(listed.offset, 50);
}

// ---------------------------------------------------------------------
// Chat attachments
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_attachment_presign_returns_the_key_to_echo_back_on_finalize() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/chat-attachments/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "s3",
            "storage_key": CANARY_KEY,
            "url": CANARY_URL,
            "expires_in": 600,
        })))
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .presign_attachment("conversation-1", "chart.png", "image/png", 2_048, "image")
        .await
        .expect("presign");

    assert_eq!(prepared.storage_key.expose(), CANARY_KEY);
    assert!(prepared.url.is_some());
}

#[tokio::test]
async fn office_attachment_presign_carries_canonical_mime_and_text_kind() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/chat-attachments/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "s3",
            "storage_key": CANARY_KEY,
            "url": CANARY_URL,
            "expires_in": 600,
        })))
        .expect(4)
        .mount(&server)
        .await;

    let office_types = [
        ("report.doc", "application/msword"),
        (
            "report.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        ("slides.ppt", "application/vnd.ms-powerpoint"),
        (
            "slides.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
    ];
    for (filename, expected_mime) in office_types {
        let (mime, kind) = classify_chat_attachment(std::path::Path::new(filename))
            .expect("Office MIME classification");
        assert_eq!(mime, expected_mime);
        assert_eq!(kind, AttachmentKind::Text);
        repository(&server)
            .presign_attachment("conversation-1", filename, mime, 2_048, kind.as_str())
            .await
            .expect("Office presign");
    }

    let requests = server.received_requests().await.expect("requests");
    for (request, (filename, mime)) in requests.iter().zip(office_types) {
        let body: serde_json::Value = serde_json::from_slice(&request.body).expect("JSON");
        assert_eq!(body["filename"], filename);
        assert_eq!(body["content_type"], mime);
        assert_eq!(body["kind"], "text");
        assert_eq!(body["size_bytes"], 2_048);
    }
}

#[tokio::test]
async fn an_s3_mode_attachment_presign_with_no_url_is_rejected() {
    // Guessing in-band delivery here would silently upload nothing.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/chat-attachments/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "s3",
            "storage_key": CANARY_KEY,
            "url": null,
        })))
        .mount(&server)
        .await;

    let result = repository(&server)
        .presign_attachment("conversation-1", "chart.png", "image/png", 2_048, "image")
        .await;

    assert!(matches!(result, Err(DaChatApiError::InvalidResponse)));
}

#[tokio::test]
async fn a_legacy_attachment_presign_carries_no_url_and_is_still_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/chat-attachments/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "legacy",
            "storage_key": CANARY_KEY,
            "url": null,
        })))
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .presign_attachment("conversation-1", "chart.png", "image/png", 2_048, "image")
        .await
        .expect("presign");

    assert!(prepared.url.is_none());
}

// ---------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------

#[tokio::test]
async fn every_request_carries_the_da_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"mode": "legacy"})))
        .mount(&server)
        .await;

    let _ = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await;

    let request = &server.received_requests().await.expect("requests")[0];
    assert_eq!(
        request
            .headers
            .get("authorization")
            .expect("authorization header"),
        "Bearer bearer-token"
    );
}

#[tokio::test]
async fn a_rejected_caller_maps_to_unauthorized_rather_than_a_generic_failure() {
    // A disabled user or a wrong-role caller needs to be told to sign in
    // again, not offered a retry that will fail identically.
    for status in [401_u16, 403] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/user/uploads/presign"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;

        let result = repository(&server)
            .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
            .await;

        assert!(
            matches!(
                result,
                Err(UploadRefusal {
                    error: DaChatApiError::Unauthorized,
                    ..
                })
            ),
            "status {status} must map to Unauthorized"
        );
    }
}

// ---------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------

#[test]
fn debugging_a_presigned_value_prints_nothing_about_it() {
    // `tracing::error!(?prepared)` on a failed transfer is an ordinary thing
    // to write. Without this, it puts a live credential in a log file.
    let wrapped = Presigned::new(CANARY_URL.to_owned());

    let rendered = format!("{wrapped:?}");

    assert_eq!(rendered, "Presigned(<redacted>)");
    assert!(!rendered.contains("X-Amz-Signature"));
}

#[tokio::test]
async fn debugging_a_whole_prepared_upload_leaks_neither_url_nor_signature() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": CANARY_URL,
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await
        .expect("presign");

    let rendered = format!("{prepared:?}");
    assert!(!rendered.contains("X-Amz-Signature"), "{rendered}");
    assert!(!rendered.contains("s3.amazonaws.com"), "{rendered}");
}

#[tokio::test]
async fn debugging_a_prepared_attachment_leaks_neither_key_nor_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/chat-attachments/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "s3",
            "storage_key": CANARY_KEY,
            "url": CANARY_URL,
            "expires_in": 600,
        })))
        .mount(&server)
        .await;

    let prepared = repository(&server)
        .presign_attachment("conversation-1", "chart.png", "image/png", 2_048, "image")
        .await
        .expect("presign");

    let rendered = format!("{prepared:?}");
    assert!(
        !rendered.contains("tenants/tenant-1"),
        "a storage key describes where other tenants' objects live: {rendered}"
    );
    assert!(!rendered.contains("X-Amz-Signature"), "{rendered}");
}

// ---------------------------------------------------------------------
// Ingest options, and what a refusal is allowed to carry
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_chosen_worksheet_and_header_row_reach_presign_and_finalize_alike() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": CANARY_URL,
            "expires_in": 900,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dataset_id": "dataset-1",
            "table_name": "q3",
            "status": "ready",
            "row_count": 10,
            "column_count": 3,
        })))
        .mount(&server)
        .await;

    let ingest = DatasetIngest {
        worksheet_index: Some(2),
        header_row_index: Some(1),
    };
    let repository = repository(&server);
    repository
        .begin("q3.xlsx", "text/csv", 1_024, "Q3", ingest)
        .await
        .expect("presign");
    repository
        .finalize("dataset-1", None, &[], "Q3", "q3.xlsx", ingest)
        .await
        .expect("finalize");

    let requests = server.received_requests().await.expect("requests");
    for request in &requests {
        let body: serde_json::Value = request.body_json().expect("body");
        assert_eq!(
            body["selected_sheet_index"], 2,
            "DA reads the pair from whichever call created the row it completes"
        );
        assert_eq!(body["header_row_index"], 1);
    }
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn an_unchosen_worksheet_and_header_row_are_omitted_rather_than_defaulted_here() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "mode": "single",
            "dataset_id": "dataset-1",
            "url": CANARY_URL,
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await
        .expect("presign");

    let request = &server.received_requests().await.expect("requests")[0];
    let body: serde_json::Value = request.body_json().expect("body");
    assert!(
        body.get("selected_sheet_index").is_none() || body["selected_sheet_index"].is_null(),
        "DA applies its own default; this app does not assert one on the person's behalf"
    );
    assert!(body.get("header_row_index").is_none() || body["header_row_index"].is_null());
}

#[tokio::test]
async fn a_refusal_keeps_das_code_and_drops_its_prose() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(413).set_body_json(json!({
            "code": "file_too_large",
            // The prose names a bucket and a key. That is exactly why it is
            // never forwarded.
            "detail": "object s3://tenant-1-uploads/tenants/tenant-1/abc.csv exceeds 2GiB",
            "issues": ["size_bytes"],
        })))
        .mount(&server)
        .await;

    let refusal = repository(&server)
        .begin(
            "big.csv",
            "text/csv",
            1_024,
            "Big",
            DatasetIngest::default(),
        )
        .await
        .expect_err("refused");
    assert_eq!(refusal.code, Some(UploadRefusalCode::FileTooLarge));

    let message = crate::uploads::da_repository::refusal_message(&refusal);
    assert_eq!(message, "That file is larger than this workspace accepts.");
    assert!(!message.contains("s3://"));
    assert!(!message.contains("tenant-1"));
    assert!(
        !format!("{refusal:?}").contains("tenant-1"),
        "the refusal itself must not carry the prose either: {refusal:?}"
    );
}

#[tokio::test]
async fn a_refusal_with_no_body_is_still_a_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/uploads/presign"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let refusal = repository(&server)
        .begin("q3.csv", "text/csv", 1_024, "Q3", DatasetIngest::default())
        .await
        .expect_err("refused");
    assert_eq!(refusal.code, None);
    assert_eq!(refusal.error, DaChatApiError::RequestFailed);
}

// ---------------------------------------------------------------------
// Download and sign-get URLs get the same validation as upload URLs
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_valid_download_url_is_returned_unchanged_with_its_filename() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads/dataset-1/files/file-1/download"))
        .and(query_param("as", "url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": CANARY_URL,
            "filename": "q3.csv",
            "content_type": "text/csv",
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let (url, filename) = repository(&server)
        .download_url("dataset-1", "file-1")
        .await
        .expect("download url");

    assert_eq!(url.expose(), CANARY_URL);
    assert_eq!(filename, "q3.csv");
}

#[tokio::test]
async fn a_download_url_that_is_unparsable_or_not_a_bare_web_url_is_refused() {
    for raw in [
        "https://user:secret@tenant-bucket.example/key",
        "ftp://tenant-bucket.example/key",
        "not a url",
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/uploads/dataset-1/files/file-1/download"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "url": raw,
                "filename": "q3.csv",
                "content_type": "text/csv",
                "expires_in": 900,
            })))
            .mount(&server)
            .await;

        let refusal = repository(&server)
            .download_url("dataset-1", "file-1")
            .await
            .expect_err("refused");

        assert_eq!(refusal.error, DaChatApiError::InvalidResponse, "{raw}");
        assert_eq!(refusal.code, None, "{raw}");
    }
}

#[tokio::test]
async fn a_refused_download_url_never_appears_in_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads/dataset-1/files/file-1/download"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": "https://user:secret@tenant-bucket.example/uploads/abc?X-Amz-Signature=deadbeefcafe",
            "filename": "q3.csv",
            "content_type": "text/csv",
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let refusal = repository(&server)
        .download_url("dataset-1", "file-1")
        .await
        .expect_err("refused");

    for rendered in [format!("{refusal:?}"), refusal.error.to_string()] {
        assert!(!rendered.contains("deadbeefcafe"), "{rendered}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(!rendered.contains("tenant-bucket"), "{rendered}");
    }
}

#[tokio::test]
async fn a_valid_sign_get_url_is_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/chat-attachments/sign-get"))
        .and(query_param("storage_key", CANARY_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": CANARY_URL,
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let signed = repository(&server)
        .sign_get(CANARY_KEY)
        .await
        .expect("sign-get");

    assert_eq!(signed.expose().as_str(), CANARY_URL);
}

#[tokio::test]
async fn a_sign_get_url_with_userinfo_or_a_non_web_scheme_is_refused_without_echoing_it() {
    for raw in [
        "https://user:secret@tenant-bucket.example/uploads/abc?X-Amz-Signature=deadbeefcafe",
        "ftp://tenant-bucket.example/uploads/abc?X-Amz-Signature=deadbeefcafe",
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/chat-attachments/sign-get"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "url": raw,
                "expires_in": 900,
            })))
            .mount(&server)
            .await;

        let error = repository(&server)
            .sign_get(CANARY_KEY)
            .await
            .expect_err("refused");

        assert_eq!(error, DaChatApiError::InvalidResponse, "{raw}");
        for rendered in [format!("{error:?}"), error.to_string()] {
            assert!(!rendered.contains("deadbeefcafe"), "{rendered}");
            assert!(!rendered.contains("secret"), "{rendered}");
            assert!(!rendered.contains("tenant-bucket"), "{rendered}");
        }
    }
}
