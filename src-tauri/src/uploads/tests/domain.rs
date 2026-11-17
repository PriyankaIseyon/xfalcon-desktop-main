//! The upload lifecycle, and the boundary it must not cross.

use std::path::PathBuf;

use xfalcon_contracts::uploads::{UploadId, UploadPurpose, UploadState};

use crate::uploads::domain::{
    sanitize_display_name, LocalUploadSource, UploadRecord, UploadTransitionError,
    FALLBACK_DISPLAY_NAME, MAX_DISPLAY_NAME_BYTES,
};

fn source(size: u64) -> LocalUploadSource {
    LocalUploadSource::new(
        PathBuf::from("/home/analyst/private/Q3 revenue.csv"),
        size,
        "text/csv",
    )
}

fn record(size: u64) -> UploadRecord {
    UploadRecord::new(
        UploadId::from("upload-1"),
        UploadPurpose::PersistentDataset,
        source(size),
    )
}

fn advance_to_uploading(record: &mut UploadRecord) {
    record.transition(UploadState::Preparing).expect("prepare");
    record.transition(UploadState::Uploading).expect("upload");
}

// ---------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------

#[test]
fn the_happy_path_runs_selected_through_ready() {
    let mut record = record(1_024);

    for next in [
        UploadState::Preparing,
        UploadState::Uploading,
        UploadState::Finalizing,
        UploadState::Ready,
    ] {
        record.transition(next).expect("legal transition");
        assert_eq!(record.state(), next);
    }
    assert!(record.state().is_terminal());
}

#[test]
fn expired_urls_resign_without_restarting_the_transfer() {
    // `Resigning` is a distinct state precisely so mid-transfer re-presigning
    // is not shown as starting over from zero.
    let mut record = record(1_024);
    advance_to_uploading(&mut record);
    record.advance(512).expect("progress");

    record.transition(UploadState::Resigning).expect("resign");
    record.transition(UploadState::Uploading).expect("resume");

    assert_eq!(record.bytes_sent(), 512);
}

#[test]
fn skipping_a_stage_is_rejected() {
    let mut record = record(1_024);

    let result = record.transition(UploadState::Uploading);

    assert!(matches!(result, Err(UploadTransitionError::Illegal { .. })));
    assert_eq!(record.state(), UploadState::Selected);
}

#[test]
fn a_ready_upload_cannot_be_moved_again() {
    let mut record = record(0);
    for next in [
        UploadState::Preparing,
        UploadState::Uploading,
        UploadState::Finalizing,
        UploadState::Ready,
    ] {
        record.transition(next).expect("legal");
    }

    assert!(record.transition(UploadState::Uploading).is_err());
    assert!(record.transition(UploadState::Failed).is_err());
}

#[test]
fn a_retry_starts_over_rather_than_resuming() {
    // The presigned URL that failed may simply have expired; resuming against
    // it would fail the same way. Showing the old byte count would also claim
    // progress the new attempt has not made.
    let mut record = record(1_024);
    advance_to_uploading(&mut record);
    record.advance(900).expect("progress");
    record.complete_part(1).expect("part");
    record.fail("The upload was interrupted.");

    record.transition(UploadState::Preparing).expect("retry");

    assert_eq!(record.bytes_sent(), 0);
    assert!(record.completed_parts().is_empty());
    assert!(record.projection().safe_error.is_none());
}

// ---------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------

#[test]
fn cancelling_twice_is_the_same_as_cancelling_once() {
    let mut record = record(1_024);
    advance_to_uploading(&mut record);

    record.cancel();
    record.cancel();

    assert_eq!(record.state(), UploadState::Cancelled);
}

#[test]
fn cancelling_a_finished_upload_does_not_unfinish_it() {
    let mut record = record(0);
    for next in [
        UploadState::Preparing,
        UploadState::Uploading,
        UploadState::Finalizing,
        UploadState::Ready,
    ] {
        record.transition(next).expect("legal");
    }

    record.cancel();

    assert_eq!(record.state(), UploadState::Ready);
}

// ---------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------

#[test]
fn progress_never_goes_backwards() {
    let mut record = record(1_024);
    advance_to_uploading(&mut record);
    record.advance(600).expect("progress");

    let result = record.advance(400);

    assert_eq!(result, Err(UploadTransitionError::ProgressWentBackwards));
    assert_eq!(record.bytes_sent(), 600);
}

#[test]
fn progress_cannot_exceed_the_size_measured_before_the_transfer() {
    // More bytes than the file has means the two sides disagree about what is
    // being sent — not something to render as a bar past 100%.
    let mut record = record(1_024);
    advance_to_uploading(&mut record);

    let result = record.advance(2_048);

    assert_eq!(result, Err(UploadTransitionError::ProgressExceedsSize));
}

#[test]
fn a_completed_part_number_cannot_repeat() {
    // S3's CompleteMultipartUpload takes one entry per part; a duplicate means
    // the caller lost track of what is outstanding.
    let mut record = record(10_000_000);
    advance_to_uploading(&mut record);
    record.complete_part(1).expect("first");

    assert_eq!(
        record.complete_part(1),
        Err(UploadTransitionError::DuplicatePart)
    );
    assert_eq!(record.completed_parts(), [1]);
}

#[test]
fn a_zero_byte_file_reads_as_complete_only_once_ready() {
    let mut record = record(0);
    assert_eq!(record.projection().progress(), 0.0);

    for next in [
        UploadState::Preparing,
        UploadState::Uploading,
        UploadState::Finalizing,
        UploadState::Ready,
    ] {
        record.transition(next).expect("legal");
    }

    assert_eq!(record.projection().progress(), 1.0);
}

// ---------------------------------------------------------------------
// The projection boundary
// ---------------------------------------------------------------------

#[test]
fn the_projection_carries_no_local_path() {
    let mut record = record(1_024);
    record.set_da_upload_id("s3://tenant-bucket/uploads/abc123");

    let serialized = serde_json::to_string(&record.projection()).expect("serialize");

    assert!(
        !serialized.contains("/home/analyst/private"),
        "the webview has no business knowing where on disk a file came from: {serialized}"
    );
    assert!(
        !serialized.contains("s3://"),
        "a storage key describes where other tenants' objects live too: {serialized}"
    );
    assert!(serialized.contains("Q3 revenue.csv"), "{serialized}");
}

#[test]
fn the_projection_has_no_field_that_could_hold_a_credential() {
    // A presigned URL is a bearer credential for tenant object storage. This
    // asserts the *shape*: there is nowhere in the wire type to put one, so
    // leaking one is not a one-line mistake away.
    let record = record(1_024);

    let value = serde_json::to_value(record.projection()).expect("serialize");
    let object = value.as_object().expect("object");

    for forbidden in ["url", "path", "token", "key", "storage_key", "signature"] {
        assert!(
            !object.contains_key(forbidden),
            "UploadProjectionV1 must have no {forbidden} field"
        );
    }
}

#[test]
fn a_failure_message_reaches_the_ui_but_the_state_says_it_is_retryable() {
    let mut record = record(1_024);
    advance_to_uploading(&mut record);

    record.fail("The upload was interrupted. You can try again.");

    let projection = record.projection();
    assert_eq!(projection.state, UploadState::Failed);
    assert!(!projection.state.is_terminal());
    assert_eq!(
        projection.safe_error.as_deref(),
        Some("The upload was interrupted. You can try again.")
    );
}

// ---------------------------------------------------------------------
// Display names
// ---------------------------------------------------------------------

#[test]
fn a_display_name_drops_path_separators() {
    // A name is attacker-influenced: the file may have come from anywhere.
    assert_eq!(sanitize_display_name("../../etc/passwd"), "....etcpasswd");
    assert_eq!(
        sanitize_display_name(r"..\..\windows\system32"),
        "....windowssystem32"
    );
}

#[test]
fn a_display_name_drops_bidi_overrides_that_disguise_an_extension() {
    // `report\u{202E}fdp.exe` renders as `reportexe.pdf` with the override in
    // place — the classic filename spoof. `char::is_control` does not cover
    // these, so they are filtered explicitly.
    let spoofed = "report\u{202E}fdp.exe";

    let safe = sanitize_display_name(spoofed);

    assert_eq!(safe, "reportfdp.exe");
    assert!(!safe.contains('\u{202E}'));
}

#[test]
fn a_display_name_drops_control_characters() {
    assert_eq!(
        sanitize_display_name("quarterly\nreport\u{0}.csv"),
        "quarterlyreport.csv"
    );
}

#[test]
fn a_display_name_is_bounded() {
    let long = format!("{}.csv", "a".repeat(500));

    let safe = sanitize_display_name(&long);

    assert!(safe.len() <= MAX_DISPLAY_NAME_BYTES);
}

#[test]
fn a_multibyte_name_is_truncated_on_a_character_boundary() {
    let long = "é".repeat(200);

    let safe = sanitize_display_name(&long);

    assert!(safe.len() <= MAX_DISPLAY_NAME_BYTES);
    assert!(std::str::from_utf8(safe.as_bytes()).is_ok());
}

#[test]
fn a_name_that_sanitizes_away_gets_a_readable_fallback() {
    // An empty label in the UI is worse than an honest placeholder.
    assert_eq!(
        sanitize_display_name("\u{202E}\u{0}///"),
        FALLBACK_DISPLAY_NAME
    );
}

#[test]
fn the_display_name_comes_from_the_file_name_not_the_whole_path() {
    let record = record(1_024);

    assert_eq!(record.projection().display_name, "Q3 revenue.csv");
}

#[test]
fn a_ready_chat_attachment_projects_its_da_id_and_never_a_path() {
    let mut record = UploadRecord::new(
        UploadId::from("upload-1"),
        UploadPurpose::ChatAttachment,
        source(1_024),
    );
    record.set_conversation_id("11111111-1111-1111-1111-111111111111");
    record.set_da_upload_id("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
    for next in [
        UploadState::Preparing,
        UploadState::Uploading,
        UploadState::Finalizing,
        UploadState::Ready,
    ] {
        record.transition(next).expect("legal");
    }

    let projection = record.projection();
    assert_eq!(
        projection.resource_id.as_deref(),
        Some("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")
    );
    assert_eq!(
        projection.conversation_id.as_deref(),
        Some("11111111-1111-1111-1111-111111111111")
    );
    let serialized = serde_json::to_string(&projection).expect("serialize");
    assert!(
        !serialized.contains("/home/analyst/private"),
        "ready projection must not carry a path: {serialized}"
    );
    assert!(
        !serialized.contains("s3://"),
        "ready projection must not carry a storage key: {serialized}"
    );
}
