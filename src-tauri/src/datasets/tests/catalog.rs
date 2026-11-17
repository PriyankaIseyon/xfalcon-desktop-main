//! The catalog as the destination reads it: which rows are listed, what a
//! partial list says, and where a presigned URL is allowed to go.

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use xfalcon_contracts::datasets::DatasetErrorCode;

use crate::datasets::catalog::{self, CATALOG_PAGE_SIZE, MAX_CATALOG_PAGES};
use crate::uploads::da_repository::DaUploadRepository;

fn repository(server: &MockServer) -> DaUploadRepository {
    DaUploadRepository::for_test(&server.uri(), "bearer-token").expect("repository")
}

fn row(id: &str, name: &str, status: &str) -> serde_json::Value {
    json!({
        "dataset_id": id,
        "display_name": name,
        "status": status,
        "created_at": "2026-08-20T09:30:00Z",
        "created_by": "user-1",
        "updated_at": "2026-08-21T10:00:00Z",
        "updated_by": "user-1",
    })
}

#[tokio::test]
async fn only_ready_rows_are_listed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [
                row("d-1", "Ready", "ready"),
                row("d-2", "Interrupted", "uploading"),
                row("d-3", "Broken", "failed"),
            ],
            "limit": CATALOG_PAGE_SIZE,
            "offset": 0,
            "total": 3,
        })))
        .mount(&server)
        .await;

    let result = catalog::list(&repository(&server)).await.expect("list");
    let names: Vec<&str> = result
        .datasets
        .iter()
        .map(|dataset| dataset.display_name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Ready"],
        "an interrupted or failed row is not a dataset anyone can query"
    );
    assert!(!result.truncated);
}

#[tokio::test]
async fn an_absent_uploads_enabled_flag_reads_as_enabled() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [],
            "limit": CATALOG_PAGE_SIZE,
            "offset": 0,
            "total": 0,
        })))
        .mount(&server)
        .await;

    let result = catalog::list(&repository(&server)).await.expect("list");
    assert!(
        result.uploads_enabled,
        "a server that never learned the flag is not one that turned uploads off"
    );
}

#[tokio::test]
async fn a_disabled_source_is_reported_while_the_rows_still_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [row("d-1", "Sales", "ready")],
            "limit": CATALOG_PAGE_SIZE,
            "offset": 0,
            "total": 1,
            "uploads_enabled": false,
        })))
        .mount(&server)
        .await;

    let result = catalog::list(&repository(&server)).await.expect("list");
    assert!(!result.uploads_enabled);
    assert_eq!(result.datasets.len(), 1);
}

#[tokio::test]
async fn pagination_walks_to_the_reported_total() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [row("d-1", "First", "ready"), row("d-2", "Skipped", "uploading")],
            "limit": CATALOG_PAGE_SIZE,
            "offset": 0,
            "total": CATALOG_PAGE_SIZE + 1,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .and(query_param("offset", CATALOG_PAGE_SIZE.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [row("d-3", "Second", "ready")],
            "limit": CATALOG_PAGE_SIZE,
            "offset": CATALOG_PAGE_SIZE,
            "total": CATALOG_PAGE_SIZE + 1,
        })))
        .mount(&server)
        .await;

    let result = catalog::list(&repository(&server)).await.expect("list");
    let names: Vec<&str> = result
        .datasets
        .iter()
        .map(|dataset| dataset.display_name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["First", "Second"],
        "the walk advances by the page asked for, not by the rows kept"
    );
}

#[tokio::test]
async fn a_list_longer_than_the_page_cap_says_it_is_truncated() {
    let server = MockServer::start().await;
    // Every page reports a total the walk can never reach, so the cap is what
    // stops it.
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [row("d-1", "Sales", "ready")],
            "limit": CATALOG_PAGE_SIZE,
            "offset": 0,
            "total": CATALOG_PAGE_SIZE * (MAX_CATALOG_PAGES as i64 + 10),
        })))
        .mount(&server)
        .await;

    let result = catalog::list(&repository(&server)).await.expect("list");
    assert!(
        result.truncated,
        "a partial list presented as complete is how a dataset looks lost"
    );
    assert_eq!(result.datasets.len(), MAX_CATALOG_PAGES);
}

#[tokio::test]
async fn a_refused_list_carries_das_code_and_not_its_prose() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "code": "source_disabled",
            "detail": "bucket tenant-1-uploads is deactivated; see s3://tenant-1-uploads",
        })))
        .mount(&server)
        .await;

    let error = catalog::list(&repository(&server))
        .await
        .expect_err("refused");
    assert_eq!(error, DatasetErrorCode::UploadsDisabled);
}

#[tokio::test]
async fn an_unknown_server_code_still_says_something_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "a_code_from_a_newer_server",
            "detail": "whatever",
        })))
        .mount(&server)
        .await;

    let error = catalog::list(&repository(&server))
        .await
        .expect_err("refused");
    assert_eq!(error, DatasetErrorCode::ServerRefused);
}

#[tokio::test]
async fn a_detail_keeps_the_servers_column_order_and_its_unknown_counts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads/d-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dataset_id": "d-1",
            "display_name": "Sales",
            "status": "ready",
            "created_at": "2026-08-20T09:30:00Z",
            "created_by": "user-1",
            "updated_at": "2026-08-21T10:00:00Z",
            "updated_by": "user-1",
            "files": [{
                "file_id": "f-1",
                "table_name": "sales",
                "row_count": null,
                "column_count": 3,
                "inferred_schema": {
                    "region": "VARCHAR",
                    "month": "VARCHAR",
                    "revenue": "DOUBLE",
                },
                "size_bytes": 4096,
                "original_filename": "sales.csv",
                "created_at": "2026-08-20T09:30:00Z",
                "created_by": "user-1",
                "updated_at": "2026-08-20T09:30:00Z",
                "updated_by": null,
            }],
        })))
        .mount(&server)
        .await;

    let detail = catalog::detail(&repository(&server), "d-1")
        .await
        .expect("detail");
    let file = &detail.files[0];
    assert_eq!(
        file.row_count, None,
        "a count the server never recorded is unknown, not zero"
    );
    assert_eq!(file.column_count, Some(3));
    let columns: Vec<&str> = file
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    assert_eq!(
        columns,
        vec!["region", "month", "revenue"],
        "the order is the server's, not a hash's"
    );
    assert_eq!(file.columns[2].inferred_type, "DOUBLE");
    assert!(detail.uploads_enabled);
}

#[tokio::test]
async fn timestamps_cross_as_epoch_milliseconds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [
                row("d-1", "Offset form", "ready"),
                json!({
                    "dataset_id": "d-2",
                    "display_name": "Naive form",
                    "status": "ready",
                    "created_at": "2026-08-20T09:30:00",
                    "created_by": "user-1",
                    "updated_at": "2026-08-20 09:30:00.500",
                    "updated_by": null,
                }),
                json!({
                    "dataset_id": "d-3",
                    "display_name": "Unreadable form",
                    "status": "ready",
                    "created_at": "last tuesday",
                    "created_by": "user-1",
                    "updated_at": "",
                    "updated_by": null,
                }),
            ],
            "limit": CATALOG_PAGE_SIZE,
            "offset": 0,
            "total": 3,
        })))
        .mount(&server)
        .await;

    let result = catalog::list(&repository(&server)).await.expect("list");
    // 2026-08-20T09:30:00Z
    assert_eq!(result.datasets[0].created_at_ms, 1_787_218_200_000);
    assert_eq!(
        result.datasets[1].created_at_ms, 1_787_218_200_000,
        "a naive timestamp is read as UTC, which is what DA documents"
    );
    assert_eq!(result.datasets[1].updated_at_ms, 1_787_218_200_500);
    assert_eq!(
        result.datasets[2].created_at_ms, 0,
        "an unreadable timestamp sorts to the end of a newest-first list"
    );
}

#[tokio::test]
async fn a_delete_takes_no_content_for_an_answer() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/user/uploads/d-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    catalog::delete(&repository(&server), "d-1")
        .await
        .expect("deleted");
}

#[tokio::test]
async fn deleting_something_that_is_not_there_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/user/uploads/d-1"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let error = catalog::delete(&repository(&server), "d-1")
        .await
        .expect_err("missing");
    assert_eq!(error, DatasetErrorCode::NotFound);
}

#[tokio::test]
async fn a_download_streams_to_the_destination_and_never_returns_the_url() {
    let storage = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/object"))
        .respond_with(ResponseTemplate::new(200).set_body_string("region,revenue\nEMEA,10\n"))
        .mount(&storage)
        .await;

    let server = MockServer::start().await;
    let presigned = format!("{}/object?X-Amz-Signature=deadbeef", storage.uri());
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads/d-1/files/f-1/download"))
        .and(query_param("as", "url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": presigned,
            "filename": "sales.csv",
            "content_type": "text/csv",
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let repository = repository(&server);
    let suggested = catalog::download_name(&repository, "d-1", "f-1")
        .await
        .expect("name");
    assert_eq!(
        suggested, "sales.csv",
        "the dialog's default comes from the server, not from the renderer"
    );

    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("sales.csv");
    catalog::download_to(&repository, "d-1", "f-1", &destination)
        .await
        .expect("downloaded");
    assert_eq!(
        std::fs::read_to_string(&destination).expect("written"),
        "region,revenue\nEMEA,10\n"
    );
}

#[tokio::test]
async fn a_download_that_object_storage_refuses_leaves_no_file() {
    let storage = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/object"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&storage)
        .await;

    let server = MockServer::start().await;
    let presigned = format!("{}/object", storage.uri());
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads/d-1/files/f-1/download"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": presigned,
            "filename": "sales.csv",
            "content_type": "text/csv",
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("sales.csv");
    let error = catalog::download_to(&repository(&server), "d-1", "f-1", &destination)
        .await
        .expect_err("refused");
    assert_eq!(error, DatasetErrorCode::ServerRefused);
    assert!(
        !destination.exists(),
        "a file with the chosen name and none of the data reads as the data"
    );
}

#[tokio::test]
async fn an_interrupted_download_preserves_the_existing_destination() {
    let storage = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/object"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-length", "100")
                .set_body_string("partial"),
        )
        .mount(&storage)
        .await;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/uploads/d-1/files/f-1/download"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": format!("{}/object", storage.uri()),
            "filename": "sales.csv",
            "content_type": "text/csv",
            "expires_in": 900,
        })))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("sales.csv");
    std::fs::write(&destination, "trusted existing data").expect("existing file");

    let error = catalog::download_to(&repository(&server), "d-1", "f-1", &destination)
        .await
        .expect_err("interrupted");

    assert_eq!(error, DatasetErrorCode::Network);
    assert_eq!(
        std::fs::read_to_string(&destination).expect("existing destination"),
        "trusted existing data"
    );
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("directory")
            .count(),
        1,
        "the incomplete sibling must be removed"
    );
}
