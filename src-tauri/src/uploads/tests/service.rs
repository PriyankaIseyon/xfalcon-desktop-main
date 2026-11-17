//! Queueing a composer file, before any byte leaves the machine.

use std::fs::File;
use std::sync::Arc;

use base64::Engine as _;
use xfalcon_contracts::auth::{IdentityProjection, SessionProjection};
use xfalcon_contracts::uploads::UploadState;

use crate::auth::AuthSessionStore;
use crate::security::temp_cleanup::TempCleanup;
use crate::uploads::domain::DatasetIngest;
use crate::uploads::kind::MAX_CHAT_ATTACHMENT_BYTES;
use crate::uploads::service::{UploadService, UploadServiceError};

fn service_with_sink(sink: crate::uploads::service::QueueSink) -> UploadService {
    let sessions = Arc::new(AuthSessionStore::new());
    sessions.set_authenticated(
        SessionProjection::authenticated(
            IdentityProjection::new("user-1", "tenant-1", "Test user", None),
            i64::MAX,
        ),
        "test-access-token",
        None,
    );
    UploadService::new(sessions, sink).expect("upload service")
}

fn service() -> UploadService {
    service_with_sink(Arc::new(|_| {}))
}

fn write_named(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

fn valid_png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .expect("valid png fixture")
}

#[tokio::test]
async fn a_composer_file_over_10_mib_is_refused_before_any_transfer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("chart.png");
    let file = File::create(&path).expect("create");
    file.set_len(MAX_CHAT_ATTACHMENT_BYTES + 1)
        .expect("sparse size");

    let error = service()
        .enqueue_attachment(path, "11111111-1111-1111-1111-111111111111".to_owned())
        .await
        .expect_err("oversize");

    assert!(matches!(error, UploadServiceError::TooLarge));
    assert!(service().queue().await.uploads.is_empty());
}

#[tokio::test]
async fn an_unknown_composer_type_is_refused_before_any_transfer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_named(&dir, "payload.exe", b"MZ");

    let error = service()
        .enqueue_attachment(path, "11111111-1111-1111-1111-111111111111".to_owned())
        .await
        .expect_err("type");

    assert!(matches!(error, UploadServiceError::UnsupportedType));
}

#[tokio::test]
async fn a_composer_file_requires_a_conversation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_named(&dir, "notes.txt", b"hello");

    let error = service()
        .enqueue_attachment(path, String::new())
        .await
        .expect_err("conversation");

    assert!(matches!(error, UploadServiceError::ConversationRequired));
}

#[tokio::test]
async fn an_allowed_composer_file_enters_the_queue_without_a_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_named(&dir, "chart.png", &valid_png());
    let conversation = "11111111-1111-1111-1111-111111111111";

    let service = service();
    let queue = service
        .enqueue_attachment(path, conversation.to_owned())
        .await
        .expect("queued");

    assert_eq!(queue.uploads.len(), 1);
    let projection = &queue.uploads[0];
    assert_eq!(projection.display_name, "chart.png");
    assert_eq!(projection.state, UploadState::Selected);
    assert_eq!(projection.conversation_id.as_deref(), Some(conversation));
    assert!(projection.resource_id.is_none());
    let serialized = serde_json::to_string(projection).expect("serialize");
    assert!(
        !serialized.contains(dir.path().to_str().unwrap_or("___")),
        "queue projection must not carry a path: {serialized}"
    );
    service
        .cancel(projection.id.as_str())
        .await
        .expect("remove snapshot");
}

#[tokio::test]
async fn cancellation_wins_over_a_start_that_reaches_admission_late() {
    let directory = tempfile::tempdir().expect("tempdir");
    let cleanup_root = directory.path().join("snapshots");
    std::fs::create_dir_all(&cleanup_root).expect("snapshot root");
    let cleanup = Arc::new(TempCleanup::new([cleanup_root.clone()]));
    let service = service()
        .with_snapshot_root(cleanup_root.clone())
        .with_temp_cleanup(Arc::clone(&cleanup));
    let (ids, _) = service
        .enqueue_attachment_bytes(
            "notes.txt".to_owned(),
            b"cancel before start admission".to_vec(),
            "11111111-1111-1111-1111-111111111111".to_owned(),
        )
        .await
        .expect("queued attachment");
    let upload_id = ids[0].as_str();
    let preflight_generation = service.current_account_generation();
    let snapshot = service
        .source_path_for_test(upload_id)
        .await
        .expect("snapshot held while start preflights");

    service.cancel(upload_id).await.expect("cancel queued item");

    // This is the same admission function used by both production upload
    // paths, reached with the generation and source already captured by
    // start before cancellation removed the queued row.
    let late_start = service
        .register_start_for_test(upload_id, preflight_generation)
        .await;
    assert!(matches!(late_start, Err(UploadServiceError::NotFound)));
    assert!(!service.has_cancel_for_test(upload_id).await);
    assert!(
        !snapshot.exists(),
        "cancel releases the queued snapshot lease"
    );
    assert!(service.queue().await.uploads.is_empty());
}

#[tokio::test]
async fn renderer_bytes_are_content_checked_before_queue_admission() {
    let error = service()
        .enqueue_attachment_bytes(
            "renamed.png".to_owned(),
            b"this is text".to_vec(),
            "11111111-1111-1111-1111-111111111111".to_owned(),
        )
        .await
        .expect_err("mismatched content");

    assert!(matches!(error, UploadServiceError::ContentMismatch));
}

#[tokio::test]
async fn office_attachment_snapshots_preserve_original_bytes_and_uppercase_names() {
    let files: [(&str, &[u8]); 4] = [
        (
            "report.DOC",
            include_bytes!("../../../tests/fixtures/office/legacy-poi/simple.doc"),
        ),
        (
            "report.DOCX",
            include_bytes!("../../../tests/fixtures/office/producer.docx"),
        ),
        (
            "slides.PPT",
            include_bytes!("../../../tests/fixtures/office/legacy-poi/text_shapes.ppt"),
        ),
        (
            "slides.PPTX",
            include_bytes!("../../../tests/fixtures/office/producer.pptx"),
        ),
    ];

    for (name, original_bytes) in files {
        let service = service();
        let (ids, queue) = service
            .enqueue_attachment_bytes(
                name.to_owned(),
                original_bytes.to_vec(),
                "11111111-1111-1111-1111-111111111111".to_owned(),
            )
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(queue.uploads[0].display_name, name);
        let snapshot = service
            .source_path_for_test(ids[0].as_str())
            .await
            .expect("private attachment snapshot");
        assert_eq!(
            std::fs::read(&snapshot).expect("snapshot bytes"),
            original_bytes
        );
        service
            .cancel(ids[0].as_str())
            .await
            .expect("remove snapshot");
    }
}

#[tokio::test]
async fn valid_office_files_without_text_are_admitted_with_original_bytes() {
    let blank_legacy_ppt = crate::documents::synthetic_blank_legacy_ppt_fixture();
    let files: [(&str, &[u8]); 6] = [
        (
            "blank.doc",
            include_bytes!("../../../tests/fixtures/office/legacy-poi/empty.doc"),
        ),
        (
            "blank.docx",
            include_bytes!("../../../tests/fixtures/office/no-text.docx"),
        ),
        ("blank.ppt", &blank_legacy_ppt),
        (
            "blank.pptx",
            include_bytes!("../../../tests/fixtures/office/no-text.pptx"),
        ),
        (
            "image.docx",
            include_bytes!("../../../tests/fixtures/office/image-only.docx"),
        ),
        (
            "image.pptx",
            include_bytes!("../../../tests/fixtures/office/image-only.pptx"),
        ),
    ];

    for (name, original_bytes) in files {
        let service = service();
        let (ids, _) = service
            .enqueue_attachment_bytes(
                name.to_owned(),
                original_bytes.to_vec(),
                "11111111-1111-1111-1111-111111111111".to_owned(),
            )
            .await
            .unwrap_or_else(|error| panic!("byte admission for {name}: {error}"));
        let snapshot = service
            .source_path_for_test(ids[0].as_str())
            .await
            .expect("private attachment snapshot");
        assert_eq!(
            std::fs::read(&snapshot).expect("snapshot bytes"),
            original_bytes
        );
        service
            .cancel(ids[0].as_str())
            .await
            .expect("remove byte snapshot");

        let dir = tempfile::tempdir().expect("native files");
        let source = write_named(&dir, name, original_bytes);
        let (path_ids, _) = service
            .enqueue_snapshot_paths(
                vec![source],
                "11111111-1111-1111-1111-111111111111".to_owned(),
            )
            .await
            .unwrap_or_else(|error| panic!("path admission for {name}: {error}"));
        let snapshot = service
            .source_path_for_test(path_ids[0].as_str())
            .await
            .expect("private path snapshot");
        assert_eq!(
            std::fs::read(&snapshot).expect("snapshot bytes"),
            original_bytes
        );
        service
            .cancel(path_ids[0].as_str())
            .await
            .expect("remove path snapshot");
    }
}

#[tokio::test]
async fn corrupt_office_attachment_is_refused_with_the_parser_reason() {
    let error = service()
        .enqueue_attachment_bytes(
            "broken.docx".to_owned(),
            b"not an OOXML package".to_vec(),
            "11111111-1111-1111-1111-111111111111".to_owned(),
        )
        .await
        .expect_err("corrupt Office file");

    assert!(matches!(
        error,
        UploadServiceError::OfficeExtraction(message)
            if message.contains("corrupt or malformed")
    ));

    let mismatch = service()
        .enqueue_attachment_bytes(
            "wrong.docx".to_owned(),
            include_bytes!("../../../tests/fixtures/office/producer.pptx").to_vec(),
            "11111111-1111-1111-1111-111111111111".to_owned(),
        )
        .await
        .expect_err("mismatched Office file");
    assert!(matches!(
        mismatch,
        UploadServiceError::OfficeExtraction(message)
            if message.contains("do not match the Office extension")
    ));
}

#[tokio::test]
async fn a_native_path_is_copied_before_the_original_can_change() {
    let dir = tempfile::tempdir().expect("tempdir");
    let original = write_named(&dir, "notes.txt", b"before");
    let service = service();
    let (ids, _) = service
        .enqueue_snapshot_paths(
            vec![original.clone()],
            "11111111-1111-1111-1111-111111111111".to_owned(),
        )
        .await
        .expect("snapshot queued");
    std::fs::write(&original, b"after").expect("replace original");
    let snapshot = service
        .source_path_for_test(ids[0].as_str())
        .await
        .expect("private snapshot");

    assert_ne!(snapshot, original);
    assert_eq!(std::fs::read(&snapshot).expect("read snapshot"), b"before");
    service
        .cancel(ids[0].as_str())
        .await
        .expect("remove snapshot");
    assert!(!snapshot.exists());
}

#[tokio::test]
async fn failed_snapshots_are_bounded_and_remain_available_for_retry() {
    let service = service();
    let conversation = "11111111-1111-1111-1111-111111111111".to_owned();
    let mut ids = Vec::new();
    for index in 0..3 {
        let (admitted, _) = service
            .enqueue_attachment_bytes(
                format!("notes-{index}.txt"),
                b"retry me".to_vec(),
                conversation.clone(),
            )
            .await
            .expect("snapshot queued");
        service.fail_for_test(admitted[0].as_str()).await;
        ids.push(admitted[0].clone());
    }

    let error = service
        .enqueue_attachment_bytes(
            "one-too-many.txt".to_owned(),
            b"bounded".to_vec(),
            conversation,
        )
        .await
        .expect_err("retained snapshot cap");
    assert!(matches!(error, UploadServiceError::TooManyUploads));
    assert!(service.retry(ids[0].as_str()).await.is_ok());

    for id in ids {
        service.cancel(id.as_str()).await.expect("remove snapshot");
    }
}

#[tokio::test]
async fn failed_recipe_upload_keeps_its_source_leased_until_cancelled() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("upload-snapshots");
    std::fs::create_dir_all(&root).expect("snapshot root");
    let cleanup = Arc::new(TempCleanup::new([root.clone()]));
    let service = service()
        .with_snapshot_root(root.clone())
        .with_temp_cleanup(Arc::clone(&cleanup));
    let id = uuid::Uuid::new_v4();
    let lease = cleanup
        .lease_directory(&root, id)
        .expect("lease before output creation");
    let path = root.join(id.to_string()).join("sales_cleaned.csv");
    std::fs::create_dir(path.parent().expect("recipe directory")).expect("recipe output directory");
    std::fs::write(&path, b"region,revenue\nEMEA,10\n").expect("recipe output");
    let (upload_id, _) = service
        .enqueue_dataset(
            path.clone(),
            "sales.csv",
            DatasetIngest::default(),
            Some(lease),
            service.current_account_generation(),
        )
        .await
        .expect("queued recipe upload");

    service.fail_for_test(upload_id.as_str()).await;
    assert_eq!(cleanup.sweep_pass(), 0, "failed retry remains protected");
    assert!(path.exists(), "the retry source must remain available");

    service
        .cancel(upload_id.as_str())
        .await
        .expect("cancel retryable upload");
    assert!(
        !path.exists(),
        "terminal cancellation eagerly removes output"
    );
}

#[tokio::test]
async fn account_scope_clear_removes_a_retryable_recipe_source() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("upload-snapshots");
    std::fs::create_dir_all(&root).expect("snapshot root");
    let cleanup = Arc::new(TempCleanup::new([root.clone()]));
    let service = service()
        .with_snapshot_root(root.clone())
        .with_temp_cleanup(Arc::clone(&cleanup));
    let old_generation = service.current_account_generation();
    let id = uuid::Uuid::new_v4();
    let lease = cleanup
        .lease_directory(&root, id)
        .expect("lease before output creation");
    let path = root.join(id.to_string()).join("sales_cleaned.csv");
    std::fs::create_dir(path.parent().expect("recipe directory")).expect("recipe output directory");
    std::fs::write(&path, b"region,revenue\nEMEA,10\n").expect("recipe output");
    let (upload_id, _) = service
        .enqueue_dataset(
            path.clone(),
            "sales.csv",
            DatasetIngest::default(),
            Some(lease),
            old_generation,
        )
        .await
        .expect("queued recipe upload");
    service.fail_for_test(upload_id.as_str()).await;
    assert!(
        path.exists(),
        "failed upload remains retryable before reset"
    );

    service.clear_account_scope().await;

    assert!(service.current_account_generation() > old_generation);
    assert!(
        !path.exists(),
        "account boundary releases retry source data"
    );
    assert_eq!(
        cleanup.sweep_pass(),
        0,
        "the eager reset leaves no stale root"
    );
}

#[tokio::test]
async fn account_scope_clear_defers_removal_while_a_transfer_holds_its_lease() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("upload-snapshots");
    std::fs::create_dir_all(&root).expect("snapshot root");
    let cleanup = Arc::new(TempCleanup::new([root.clone()]));
    let service = service()
        .with_snapshot_root(root.clone())
        .with_temp_cleanup(Arc::clone(&cleanup));
    let id = uuid::Uuid::new_v4();
    let lease = cleanup
        .lease_directory(&root, id)
        .expect("lease before output creation");
    let path = root.join(id.to_string()).join("sales_cleaned.csv");
    std::fs::create_dir(path.parent().expect("recipe directory")).expect("recipe output directory");
    std::fs::write(&path, b"region,revenue\nEMEA,10\n").expect("recipe output");
    let (upload_id, _) = service
        .enqueue_dataset(
            path.clone(),
            "sales.csv",
            DatasetIngest::default(),
            Some(lease),
            service.current_account_generation(),
        )
        .await
        .expect("queued recipe upload");
    let active_transfer = service
        .operation_lease_for_test(upload_id.as_str())
        .await
        .expect("active operation lease")
        .expect("temporary source has a lease");

    service.clear_account_scope().await;

    assert!(
        path.exists(),
        "reset must not unlink an active transfer source"
    );
    drop(active_transfer);
    assert_eq!(cleanup.sweep_pass(), 1, "released source becomes sweepable");
    assert!(!path.exists());
}

#[tokio::test]
async fn a_ready_snapshot_is_deleted_before_the_queue_can_forget_it() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    let watched_snapshot = Arc::new(Mutex::new(None::<std::path::PathBuf>));
    let ready_observed = Arc::new(AtomicBool::new(false));
    let sink_path = Arc::clone(&watched_snapshot);
    let sink_ready = Arc::clone(&ready_observed);
    let service = service_with_sink(Arc::new(move |queue| {
        if queue
            .uploads
            .iter()
            .any(|upload| upload.state == UploadState::Ready)
        {
            let path = sink_path.lock().expect("watched snapshot lock");
            let path = path.as_ref().expect("snapshot path before Ready");
            assert!(
                !path.exists(),
                "Ready was published before snapshot cleanup"
            );
            sink_ready.store(true, Ordering::SeqCst);
        }
    }));
    let attachment_id = uuid::Uuid::new_v4();
    let (ids, _) = service
        .enqueue_attachment_bytes(
            "notes.txt".to_owned(),
            b"ready".to_vec(),
            "11111111-1111-1111-1111-111111111111".to_owned(),
        )
        .await
        .expect("snapshot queued");
    let snapshot = service
        .source_path_for_test(ids[0].as_str())
        .await
        .expect("private snapshot");
    *watched_snapshot.lock().expect("watched snapshot lock") = Some(snapshot.clone());

    service
        .complete_chat_attachment_for_test(
            ids[0].as_str(),
            attachment_id.to_string(),
            b"ready".len() as u64,
        )
        .await;

    assert!(!snapshot.exists());
    assert!(ready_observed.load(Ordering::SeqCst));
    assert_eq!(service.queue().await.uploads[0].state, UploadState::Ready);
    assert!(service
        .forget_by_resource_ids(&[attachment_id])
        .await
        .uploads
        .is_empty());
    assert!(!snapshot.exists());
}

#[tokio::test]
async fn leaving_an_account_clears_its_queue_and_invalidates_its_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_named(&dir, "account-a.txt", b"private account-a attachment");
    let service = service();
    let queue = service
        .enqueue_attachment(path, "11111111-1111-1111-1111-111111111111".to_owned())
        .await
        .expect("queued");
    let upload_id = queue.uploads[0].id.as_str().to_owned();

    service.clear_account_scope().await;

    assert!(service.queue().await.uploads.is_empty());
    assert!(matches!(
        service.retry(&upload_id).await,
        Err(UploadServiceError::NotFound)
    ));
}

#[tokio::test]
async fn a_picker_result_from_the_previous_account_cannot_reenter_the_queue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_named(&dir, "late-account-a.txt", b"late private attachment");
    let service = service();
    let account_a_generation = service.current_account_generation();

    service.clear_account_scope().await;
    let error = service
        .enqueue_attachment_for_generation(
            path,
            "11111111-1111-1111-1111-111111111111".to_owned(),
            account_a_generation,
        )
        .await
        .expect_err("stale picker result");

    assert!(matches!(error, UploadServiceError::AuthRequired));
    assert!(service.queue().await.uploads.is_empty());
}
