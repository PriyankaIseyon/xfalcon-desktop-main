//! Moving bytes: what gets sent, what gets counted, and what never gets sent.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reqwest::Url;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::clients::da::generated::{
    DatasetDetailResponse, DatasetListResponse, PresignedPart, UploadResponse,
};
use crate::uploads::da_repository::{
    CompletedPart, PersistentUploadRepository, PreparedDataset, Presigned, TransferPlan,
    UploadRefusal,
};
use crate::uploads::domain::DatasetIngest;
use crate::uploads::transfer::{HttpObjectStore, ObjectStorePut, TransferError, UploadTransfer};

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

fn url(suffix: &str) -> Url {
    Url::parse(&format!("https://storage.example/object?sig={suffix}")).expect("url")
}

/// Records every PUT and answers from a script.
#[derive(Default)]
struct ScriptedStore {
    /// Per call: `Some(etag)` succeeds, `None` answers 403 (expired-or-denied).
    script: Mutex<Vec<Option<String>>>,
    seen: Mutex<Vec<(Url, usize)>>,
}

#[async_trait]
impl ObjectStorePut for ScriptedStore {
    async fn put_part(
        &self,
        url: &Url,
        body: Vec<u8>,
        _content_type: &str,
    ) -> Result<String, TransferError> {
        self.seen
            .lock()
            .expect("lock")
            .push((url.clone(), body.len()));
        let mut script = self.script.lock().expect("lock");
        let next = if script.is_empty() {
            Some("\"etag-default\"".to_owned())
        } else {
            script.remove(0)
        };
        next.ok_or(TransferError::Rejected)
    }
}

/// A repository whose only live method is `resign`.
#[derive(Default)]
struct ResigningRepository {
    resigned: Mutex<Vec<i64>>,
    fresh_url: Mutex<Option<String>>,
}

#[async_trait]
impl PersistentUploadRepository for ResigningRepository {
    async fn begin(
        &self,
        _filename: &str,
        _content_type: &str,
        _size_bytes: u64,
        _display_name: &str,
        _ingest: DatasetIngest,
    ) -> Result<PreparedDataset, UploadRefusal> {
        unreachable!("the transfer never presigns; it is handed a prepared plan")
    }

    async fn finalize(
        &self,
        _dataset_id: &str,
        _upload_id: Option<&str>,
        _parts: &[CompletedPart],
        _display_name: &str,
        _filename: &str,
        _ingest: DatasetIngest,
    ) -> Result<UploadResponse, UploadRefusal> {
        unreachable!("finalize belongs to the caller, not the transfer")
    }

    async fn abort(&self, _upload_id: &str, _dataset_id: &str) -> Result<(), UploadRefusal> {
        unreachable!("abort belongs to the caller")
    }

    async fn resign(
        &self,
        _upload_id: &str,
        _dataset_id: &str,
        part_numbers: &[i64],
    ) -> Result<(Presigned<Vec<PresignedPart>>, i64), UploadRefusal> {
        self.resigned
            .lock()
            .expect("lock")
            .extend_from_slice(part_numbers);
        let fresh = self
            .fresh_url
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or_else(|| url("fresh").to_string());
        Ok((
            Presigned::new(
                part_numbers
                    .iter()
                    .map(|number| PresignedPart {
                        part_number: *number,
                        url: fresh.clone(),
                    })
                    .collect(),
            ),
            900,
        ))
    }

    async fn list(&self, _limit: i64, _offset: i64) -> Result<DatasetListResponse, UploadRefusal> {
        unreachable!("listing belongs to the caller")
    }

    async fn get(&self, _dataset_id: &str) -> Result<DatasetDetailResponse, UploadRefusal> {
        unreachable!("detail belongs to the caller")
    }

    async fn delete(&self, _dataset_id: &str) -> Result<(), UploadRefusal> {
        unreachable!("deleting belongs to the caller")
    }

    async fn download_url(
        &self,
        _dataset_id: &str,
        _file_id: &str,
    ) -> Result<(Presigned<String>, String), UploadRefusal> {
        unreachable!("downloading belongs to the caller")
    }
}

struct Fixture {
    directory: TempDir,
    path: PathBuf,
}

fn file_of(size: usize) -> Fixture {
    let directory = TempDir::new().expect("tempdir");
    let path = directory.path().join("data.csv");
    // Distinct byte values so a wrongly-offset part is visible, not just
    // wrongly-sized.
    let contents: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    std::fs::write(&path, contents).expect("write");
    Fixture { directory, path }
}

fn progress_sink() -> (Arc<Mutex<Vec<u64>>>, crate::uploads::transfer::ProgressSink) {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&recorded);
    (
        recorded,
        Arc::new(move |bytes: u64| sink.lock().expect("lock").push(bytes)),
    )
}

fn multipart_plan(part_count: i64) -> PreparedDataset {
    PreparedDataset {
        dataset_id: Some("dataset-1".to_owned()),
        plan: TransferPlan::Multipart {
            upload_id: "s3-upload-1".to_owned(),
            part_size: 8,
            parts: Presigned::new(
                (1..=part_count)
                    .map(|number| PresignedPart {
                        part_number: number,
                        url: url(&format!("part{number}")).to_string(),
                    })
                    .collect(),
            ),
        },
        expires_in: Some(900),
    }
}

// ---------------------------------------------------------------------
// Multipart: offsets, sizes, progress
// ---------------------------------------------------------------------

#[tokio::test]
async fn every_part_is_read_from_its_own_offset_at_the_agreed_size() {
    // A part read from the wrong offset produces an object whose bytes are not
    // the file's — and nothing downstream would notice until a user opened it.
    let fixture = file_of(20);
    let store = ScriptedStore::default();
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let outcome = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &multipart_plan(3),
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await
    .expect("transfer");

    let seen = store.seen.lock().expect("lock");
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].1, 8, "first part is a full part");
    assert_eq!(seen[1].1, 8, "second part is a full part");
    assert_eq!(seen[2].1, 4, "the last part is the remainder, not padded");
    assert_eq!(outcome.parts.len(), 3);
    assert_eq!(outcome.upload_id.as_deref(), Some("s3-upload-1"));
    let _ = &fixture.directory;
}

#[tokio::test]
async fn progress_is_reported_only_after_a_part_is_confirmed() {
    // Counting bytes handed to a socket makes the bar jump and then stall on a
    // retry. Counting confirmed parts makes it monotone and true.
    let fixture = file_of(20);
    let store = ScriptedStore::default();
    let repository = ResigningRepository::default();
    let (recorded, on_progress) = progress_sink();

    UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &multipart_plan(3),
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await
    .expect("transfer");

    assert_eq!(recorded.lock().expect("lock").as_slice(), [8, 16, 20]);
    let _ = &fixture.directory;
}

#[tokio::test]
async fn the_etag_of_every_part_is_carried_to_the_finalize_payload() {
    // Multipart completion is keyed on ETags; losing one makes the object
    // uncompletable and the bytes already uploaded unreachable.
    let fixture = file_of(16);
    let store = ScriptedStore::default();
    *store.script.lock().expect("lock") =
        vec![Some("\"etag-a\"".to_owned()), Some("\"etag-b\"".to_owned())];
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let outcome = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &multipart_plan(2),
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await
    .expect("transfer");

    assert_eq!(
        outcome.parts,
        vec![
            CompletedPart {
                part_number: 1,
                etag: "\"etag-a\"".to_owned()
            },
            CompletedPart {
                part_number: 2,
                etag: "\"etag-b\"".to_owned()
            },
        ]
    );
    let _ = &fixture.directory;
}

#[tokio::test]
async fn a_file_that_shrank_since_it_was_measured_fails_rather_than_sending_an_empty_part() {
    // Completing an upload whose object is not what the user picked is worse
    // than failing: the dataset would look ready and be wrong.
    let fixture = file_of(4);
    let store = ScriptedStore::default();
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let result = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        // The plan was made for three 8-byte parts; the file now holds four
        // bytes, so parts two and three read empty.
        &multipart_plan(3),
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(TransferError::SourceUnreadable)));
    let _ = &fixture.directory;
}

// ---------------------------------------------------------------------
// Expiry and re-signing
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_expired_part_url_is_resigned_once_and_the_part_retried() {
    let fixture = file_of(8);
    let store = ScriptedStore::default();
    *store.script.lock().expect("lock") = vec![None, Some("\"etag-after-resign\"".to_owned())];
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let outcome = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &multipart_plan(1),
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await
    .expect("transfer");

    assert_eq!(
        repository.resigned.lock().expect("lock").as_slice(),
        [1],
        "only the outstanding part is re-signed"
    );
    assert_eq!(outcome.parts[0].etag, "\"etag-after-resign\"");
    let _ = &fixture.directory;
}

#[tokio::test]
async fn a_second_rejection_after_resigning_is_a_real_denial_not_another_retry() {
    // A URL minted seconds ago that is still refused is an authorization
    // failure. Retrying it forever would hang the upload with no way out.
    let fixture = file_of(8);
    let store = ScriptedStore::default();
    *store.script.lock().expect("lock") = vec![None, None];
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let result = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &multipart_plan(1),
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(TransferError::Rejected)));
    assert_eq!(
        store.seen.lock().expect("lock").len(),
        2,
        "exactly one retry"
    );
    let _ = &fixture.directory;
}

#[tokio::test]
async fn a_rejected_single_put_is_not_resigned_because_there_is_no_part_to_resign() {
    // DA's resign endpoint is part-based. A whole-file PUT has to restart from
    // a fresh presign, which is the caller's retry path.
    let fixture = file_of(8);
    let store = ScriptedStore::default();
    *store.script.lock().expect("lock") = vec![None];
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let result = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &PreparedDataset {
            dataset_id: Some("dataset-1".to_owned()),
            plan: TransferPlan::Single {
                url: Presigned::new(url("single")),
            },
            expires_in: Some(900),
        },
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(TransferError::Rejected)));
    assert!(repository.resigned.lock().expect("lock").is_empty());
    let _ = &fixture.directory;
}

// ---------------------------------------------------------------------
// Cancellation and legacy
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_already_cancelled_transfer_sends_nothing() {
    let fixture = file_of(20);
    let store = ScriptedStore::default();
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(&multipart_plan(3), &fixture.path, "text/csv", &cancel)
    .await;

    assert!(matches!(result, Err(TransferError::Cancelled)));
    assert!(store.seen.lock().expect("lock").is_empty());
    let _ = &fixture.directory;
}

#[tokio::test]
async fn legacy_mode_uploads_nothing_directly_and_leaves_finalize_to_the_caller() {
    let fixture = file_of(8);
    let store = ScriptedStore::default();
    let repository = ResigningRepository::default();
    let (_recorded, on_progress) = progress_sink();

    let outcome = UploadTransfer {
        repository: &repository,
        store: &store,
        on_progress,
    }
    .run(
        &PreparedDataset {
            dataset_id: None,
            plan: TransferPlan::Legacy,
            expires_in: None,
        },
        &fixture.path,
        "text/csv",
        &CancellationToken::new(),
    )
    .await
    .expect("transfer");

    assert!(store.seen.lock().expect("lock").is_empty());
    assert!(outcome.parts.is_empty());
    let _ = &fixture.directory;
}

// ---------------------------------------------------------------------
// The object-store client never carries the DA bearer
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_presigned_put_carries_no_authorization_header() {
    // The presigned URL is already the credential for this request. Adding the
    // tenant bearer would hand a far more powerful one to whatever host the
    // URL names.
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path_matcher("/object"))
        .respond_with(ResponseTemplate::new(200).insert_header("etag", "\"etag-1\""))
        .mount(&server)
        .await;
    let target = Url::parse(&format!("{}/object", server.uri())).expect("url");

    HttpObjectStore::new()
        .expect("client")
        .put_part(&target, b"payload".to_vec(), "text/csv")
        .await
        .expect("put");

    let request = &server.received_requests().await.expect("requests")[0];
    assert!(
        request.headers.get("authorization").is_none(),
        "a presigned PUT must never carry the tenant bearer"
    );
}

#[tokio::test]
async fn a_put_with_no_etag_fails_rather_than_reporting_a_part_that_cannot_be_completed() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path_matcher("/object"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let target = Url::parse(&format!("{}/object", server.uri())).expect("url");

    let result = HttpObjectStore::new()
        .expect("client")
        .put_part(&target, b"payload".to_vec(), "text/csv")
        .await;

    assert!(matches!(result, Err(TransferError::MissingEtag)));
}

#[tokio::test]
async fn a_redirect_is_refused_rather_than_replaying_the_body_elsewhere() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path_matcher("/object"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "https://elsewhere.example/steal"),
        )
        .mount(&server)
        .await;
    let target = Url::parse(&format!("{}/object", server.uri())).expect("url");

    let result = HttpObjectStore::new()
        .expect("client")
        .put_part(&target, b"payload".to_vec(), "text/csv")
        .await;

    assert!(
        matches!(result, Err(TransferError::Unreachable)),
        "a 302 must not be followed: it would replay the body somewhere nobody presigned"
    );
}
