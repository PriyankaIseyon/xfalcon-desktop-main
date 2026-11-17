use xfalcon_contracts::datasets::{DatasetErrorCode, DatasetFormat, StagedDatasetId};

use crate::datasets::preview::read_sample;
use crate::datasets::staging::{Staging, MAX_STAGED_DATASETS};
use crate::datasets::tests::fixtures;

#[test]
fn a_staged_file_is_described_without_its_path() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "sales.csv", b"region,revenue\nEMEA,10\n");
    let staging = Staging::new();

    let view = staging.stage(path.clone()).expect("staged");
    let serialized = serde_json::to_string(&view).expect("serialize");
    assert!(
        !serialized.contains(directory.path().to_str().unwrap_or("___")),
        "the staged view must not carry a path: {serialized}"
    );
    assert_eq!(view.display_name, "sales.csv");
    assert_eq!(view.format, DatasetFormat::Csv);
    assert_eq!(view.size, 23);
    assert!(view.worksheets.is_empty(), "a CSV has no worksheets");
}

#[test]
fn a_staged_workbook_names_its_worksheets() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[
            fixtures::Sheet {
                name: "Q3",
                rows: &[&["a"], &["1"]],
            },
            fixtures::Sheet {
                name: "Notes",
                rows: &[&["b"], &["2"]],
            },
        ],
    );
    let staging = Staging::new();
    let view = staging.stage(path).expect("staged");
    assert_eq!(view.worksheets, vec!["Q3", "Notes"]);
}

#[test]
fn a_file_that_fails_validation_is_never_staged() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "notes.txt", b"hello");
    let staging = Staging::new();

    assert_eq!(
        staging.stage(path),
        Err(DatasetErrorCode::UnsupportedFormat)
    );
    assert!(
        staging.is_empty(),
        "an id must never name a file DA has no chance of accepting"
    );
}

#[test]
fn an_empty_file_is_refused() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "empty.csv", b"");
    assert_eq!(Staging::new().stage(path), Err(DatasetErrorCode::EmptyFile));
}

#[test]
fn a_directory_is_not_a_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let nested = directory.path().join("folder.csv");
    std::fs::create_dir(&nested).expect("directory");
    assert_eq!(
        Staging::new().stage(nested),
        Err(DatasetErrorCode::Unreadable)
    );
}

#[test]
fn the_stage_cap_holds_and_discarding_makes_room() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = Staging::new();
    let mut staged = Vec::new();
    for index in 0..MAX_STAGED_DATASETS {
        let path = fixtures::write_file(directory.path(), &format!("sales-{index}.csv"), b"a\n1\n");
        staged.push(staging.stage(path).expect("staged"));
    }
    let extra = fixtures::write_file(directory.path(), "one-more.csv", b"a\n1\n");
    assert_eq!(
        staging.stage(extra.clone()),
        Err(DatasetErrorCode::StageLimitReached)
    );

    // A bound, not a wall: the person discards one and picks again.
    staging.discard(&staged[0].id);
    staging.stage(extra).expect("staged after discarding one");
    assert_eq!(staging.len(), MAX_STAGED_DATASETS);
}

#[test]
fn a_discarded_id_is_unknown() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "sales.csv", b"a\n1\n");
    let staging = Staging::new();
    let view = staging.stage(path).expect("staged");

    staging.discard(&view.id);
    assert_eq!(
        staging.source(&view.id).err(),
        Some(DatasetErrorCode::StageNotFound)
    );
    assert!(staging.is_empty());
}

#[test]
fn leaving_an_account_releases_every_staged_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let first = fixtures::write_file(directory.path(), "first.csv", b"a\n1\n");
    let second = fixtures::write_file(directory.path(), "second.csv", b"b\n2\n");
    let staging = Staging::new();
    let first = staging.stage(first).expect("first staged");
    staging.stage(second).expect("second staged");

    staging.clear_account_scope();

    assert!(staging.is_empty());
    assert_eq!(
        staging.source(&first.id).err(),
        Some(DatasetErrorCode::StageNotFound)
    );
}

#[test]
fn a_picker_result_from_the_previous_account_cannot_be_staged() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "late.csv", b"a\n1\n");
    let staging = Staging::new();
    let account_a_generation = staging.current_account_generation();

    staging.clear_account_scope();

    assert_eq!(
        staging.stage_for_generation(path, account_a_generation),
        Err(DatasetErrorCode::AuthRequired)
    );
    assert!(staging.is_empty());
}

#[test]
fn an_id_that_was_never_staged_is_unknown() {
    let staging = Staging::new();
    assert_eq!(
        staging
            .source(&StagedDatasetId::from(
                "00000000-0000-0000-0000-000000000000"
            ))
            .err(),
        Some(DatasetErrorCode::StageNotFound),
        "discarded, uploaded, and invented ids answer identically"
    );
}

#[test]
fn the_cached_sample_is_returned_only_for_the_worksheet_it_came_from() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[
            fixtures::Sheet {
                name: "First",
                rows: &[&["a"], &["1"]],
            },
            fixtures::Sheet {
                name: "Second",
                rows: &[&["b"], &["2"]],
            },
        ],
    );
    let staging = Staging::new();
    let view = staging.stage(path.clone()).expect("staged");
    let source = staging.source(&view.id).expect("source");

    assert!(staging.cached_sample(&view.id, 0).is_none());
    let sample = read_sample(&source.path, source.format, source.size, 0).expect("sample");
    staging.cache_sample(&view.id, 0, sample);

    assert!(staging.cached_sample(&view.id, 0).is_some());
    assert!(
        staging.cached_sample(&view.id, 1).is_none(),
        "the cache holds one worksheet, so a switch re-reads rather than lying"
    );
}

#[test]
fn caching_against_a_discarded_id_is_dropped_rather_than_resurrecting_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "sales.csv", b"a\n1\n");
    let staging = Staging::new();
    let view = staging.stage(path.clone()).expect("staged");
    let source = staging.source(&view.id).expect("source");
    let sample = read_sample(&source.path, source.format, source.size, 0).expect("sample");

    staging.discard(&view.id);
    staging.cache_sample(&view.id, 0, sample);
    assert!(staging.is_empty());
}
