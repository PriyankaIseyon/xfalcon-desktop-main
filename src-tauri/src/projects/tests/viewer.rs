//! Coverage for the file view's own reads (`super::super::viewer`) and the
//! backend table windows (`super::super::table`): the gates every call applies,
//! exact ranges, and row windows that honour quoting across index checkpoints.

use std::fs;

use tempfile::{tempdir, TempDir};
use xfalcon_contracts::RunId;

use crate::projects::domain::{ProjectId, TrustedRoot};
use crate::projects::path_policy::RelativeProjectPath;
use crate::projects::read::MetadataOnlyReason;
use crate::projects::sensitive::SecretClassifier;
use crate::projects::table::{table_rows, table_rows_bounded, Bounds};
use crate::projects::viewer::{
    read_office_document, read_office_source, read_preview_text, read_range, stat, ViewerError,
    ViewerStat, MAX_PROJECT_PREVIEW_BYTES, MAX_READ_RANGE_BYTES,
};

fn root(directory: &TempDir) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), directory.path()).expect("trusted root")
}

fn path(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("path")
}

fn run() -> RunId {
    RunId::from("viewer-test")
}

fn classifier() -> SecretClassifier {
    SecretClassifier::built_in()
}

const PRODUCER_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.docx");
const PRODUCER_PPTX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.pptx");
const LEGACY_DOC: &[u8] = include_bytes!("../../../tests/fixtures/office/legacy-poi/empty.doc");
const LEGACY_PPT: &[u8] =
    include_bytes!("../../../tests/fixtures/office/legacy-poi/text_shapes.ppt");
const NO_TEXT_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/no-text.docx");
const IMAGE_ONLY_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/image-only.docx");
const NO_TEXT_PPTX: &[u8] = include_bytes!("../../../tests/fixtures/office/no-text.pptx");
const IMAGE_ONLY_PPTX: &[u8] = include_bytes!("../../../tests/fixtures/office/image-only.pptx");

#[test]
fn office_source_returns_original_bytes_for_all_supported_formats() {
    let directory = tempdir().expect("root");
    let sources = [
        ("source.doc", LEGACY_DOC),
        ("source.docx", PRODUCER_DOCX),
        ("source.ppt", LEGACY_PPT),
        ("source.pptx", PRODUCER_PPTX),
    ];
    let root = root(&directory);

    for (name, bytes) in sources {
        fs::write(directory.path().join(name), bytes).expect("seed Office source");
        let source = read_office_source(&root, &classifier(), &path(name), run())
            .expect("read trusted Office source");
        assert_eq!(source.bytes, bytes, "source bytes changed for {name}");
        assert_eq!(source.size_bytes, bytes.len() as u64, "size for {name}");
        assert!(source.modified_ms.is_some(), "modified time for {name}");
    }
}

#[test]
fn valid_blank_and_image_only_office_documents_return_empty_text() {
    let directory = tempdir().expect("root");
    let blank_ppt = crate::documents::synthetic_blank_legacy_ppt_fixture();
    let sources: [(&str, &[u8]); 6] = [
        ("blank.doc", LEGACY_DOC),
        ("blank.docx", NO_TEXT_DOCX),
        ("blank.ppt", &blank_ppt),
        ("blank.pptx", NO_TEXT_PPTX),
        ("image-only.docx", IMAGE_ONLY_DOCX),
        ("image-only.pptx", IMAGE_ONLY_PPTX),
    ];
    let root = root(&directory);

    for (name, bytes) in sources {
        fs::write(directory.path().join(name), bytes).expect("seed blank Office source");
        let (text, size, modified) = read_office_document(&root, &classifier(), &path(name), run())
            .expect("valid no-text Office document");
        assert!(
            text.is_empty(),
            "expected no extracted text for {name}, got {text:?}"
        );
        assert_eq!(size, bytes.len() as u64, "source size for {name}");
        assert!(modified.is_some(), "source time for {name}");
    }
}

#[test]
fn office_source_keeps_trust_sensitivity_type_and_size_gates() {
    let directory = tempdir().expect("root");
    fs::write(directory.path().join(".env.docx"), PRODUCER_DOCX).expect("sensitive Office file");
    fs::write(directory.path().join("broken.pptx"), b"not a package").expect("bad Office file");
    fs::write(directory.path().join("notes.txt"), b"not Office").expect("text file");
    fs::create_dir(directory.path().join("folder.pptx")).expect("directory");
    fs::File::create(directory.path().join("large.docx"))
        .expect("large sparse file")
        .set_len(crate::documents::MAX_DOCUMENT_BYTES + 1)
        .expect("set Office size");
    let root = root(&directory);

    assert!(matches!(
        read_office_source(&root, &classifier(), &path(".env.docx"), run()),
        Err(ViewerError::Sensitive)
    ));
    assert!(matches!(
        read_office_source(&root, &classifier(), &path("notes.txt"), run()),
        Err(ViewerError::NotOfficeDocument)
    ));
    assert!(matches!(
        read_office_source(&root, &classifier(), &path("folder.pptx"), run()),
        Err(ViewerError::NotAFile)
    ));
    assert!(matches!(
        read_office_source(&root, &classifier(), &path("large.docx"), run()),
        Err(ViewerError::TooLarge { limit }) if limit == crate::documents::MAX_DOCUMENT_BYTES
    ));
    assert!(matches!(
        read_office_document(&root, &classifier(), &path("broken.pptx"), run()),
        Err(ViewerError::Office(_))
    ));
}

#[cfg(unix)]
#[test]
fn office_source_rejects_symlinks_that_escape_the_trusted_root() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("root");
    let outside = tempdir().expect("outside");
    let source = outside.path().join("source.docx");
    fs::write(&source, PRODUCER_DOCX).expect("outside Office file");
    symlink(&source, directory.path().join("escape.docx")).expect("symlink");
    let root = root(&directory);

    assert!(matches!(
        read_office_source(&root, &classifier(), &path("escape.docx"), run()),
        Err(ViewerError::Containment(_))
    ));
}

#[test]
fn office_preview_extracts_read_only_text_and_respects_sensitive_and_invalid_files() {
    let directory = tempdir().expect("root");
    let original_path = directory.path().join("brief.docx");
    fs::write(&original_path, PRODUCER_DOCX).expect("producer DOCX");
    let original = fs::read(&original_path).expect("original bytes");
    fs::write(directory.path().join("broken.pptx"), b"not a package").expect("bad Office file");
    fs::write(directory.path().join(".env.docx"), PRODUCER_DOCX).expect("sensitive Office file");
    let root = root(&directory);

    let (text, size, modified) =
        read_office_document(&root, &classifier(), &path("brief.docx"), run())
            .expect("read-only Office preview");
    assert!(text.contains("Producer DOCX fixture"));
    assert!(text.contains("images are not OCR-scanned"));
    assert_eq!(size, PRODUCER_DOCX.len() as u64);
    assert!(modified.is_some());
    assert_eq!(
        fs::read(original_path).expect("original remains intact"),
        original
    );
    assert!(matches!(
        read_office_document(&root, &classifier(), &path(".env.docx"), run()),
        Err(ViewerError::Sensitive)
    ));
    assert!(matches!(
        read_office_document(&root, &classifier(), &path("broken.pptx"), run()),
        Err(ViewerError::Office(_))
    ));
}

#[test]
fn stat_reports_size_time_and_kind_without_reading_the_file() {
    let directory = tempdir().expect("root");
    fs::write(directory.path().join("notes.txt"), "hello\n").expect("text");
    fs::write(directory.path().join("image.bin"), [0u8, 1, 2, 3, 0, 0]).expect("binary");
    fs::write(
        directory.path().join("latin1.txt"),
        [b'h', b'i', 0xFF, 0xFE],
    )
    .expect("latin1");
    fs::write(directory.path().join(".env"), "TOKEN=1").expect("secret");
    fs::create_dir(directory.path().join("folder")).expect("dir");
    let root = root(&directory);

    match stat(&root, &classifier(), &path("notes.txt"), run()).expect("stat") {
        ViewerStat::Text {
            size_bytes,
            modified_ms,
        } => {
            assert_eq!(size_bytes, 6);
            assert!(modified_ms.is_some());
        }
        other => panic!("expected text, got {other:?}"),
    }
    assert!(matches!(
        stat(&root, &classifier(), &path("image.bin"), run()).expect("stat"),
        ViewerStat::NotText {
            reason: MetadataOnlyReason::Binary,
            ..
        }
    ));
    assert!(matches!(
        stat(&root, &classifier(), &path("latin1.txt"), run()).expect("stat"),
        ViewerStat::NotText {
            reason: MetadataOnlyReason::NonUtf8Text,
            ..
        }
    ));
    // Decided from the path alone: no size, no time.
    assert!(matches!(
        stat(&root, &classifier(), &path(".env"), run()).expect("stat"),
        ViewerStat::Sensitive(_)
    ));
    assert!(matches!(
        stat(&root, &classifier(), &path("folder"), run()),
        Err(ViewerError::NotAFile)
    ));
}

/// A sparse file past the cap costs no disk. Unix only: NTFS allocates the
/// length it is extended to, and a 10 GiB test fixture is not a fair ask of a
/// build machine. The comparison under test is platform-neutral.
#[cfg(unix)]
#[test]
fn a_file_past_the_ten_gib_cap_is_reported_and_never_read() {
    let directory = tempdir().expect("root");
    let file = fs::File::create(directory.path().join("huge.log")).expect("create");
    file.set_len(crate::projects::MAX_VIEWABLE_FILE_BYTES + 1)
        .expect("sparse length");
    let root = root(&directory);

    assert!(matches!(
        stat(&root, &classifier(), &path("huge.log"), run()).expect("stat"),
        ViewerStat::TooLarge { size_bytes } if size_bytes == crate::projects::MAX_VIEWABLE_FILE_BYTES + 1
    ));
    assert!(matches!(
        read_range(&root, &classifier(), &path("huge.log"), run(), 0, 16),
        Err(ViewerError::TooLarge { .. })
    ));
}

#[test]
fn a_range_returns_exactly_the_bytes_asked_for_and_stops_at_the_end() {
    let directory = tempdir().expect("root");
    let content: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
    fs::write(directory.path().join("data.txt"), &content).expect("seed");
    let root = root(&directory);
    let read = |offset, length| {
        read_range(
            &root,
            &classifier(),
            &path("data.txt"),
            run(),
            offset,
            length,
        )
    };

    assert_eq!(read(100, 50).expect("middle"), content[100..150]);
    assert_eq!(read(9_990, 64).expect("tail"), content[9_990..]);
    assert!(read(10_000, 64).expect("at the end").is_empty());
    assert!(matches!(
        read(10_001, 1),
        Err(ViewerError::OffsetPastEnd {
            offset: 10_001,
            size: 10_000
        })
    ));
    assert!(matches!(
        read(0, MAX_READ_RANGE_BYTES + 1),
        Err(ViewerError::RangeTooLong)
    ));
}

#[test]
fn a_sensitive_path_is_never_read_by_range_or_preview() {
    let directory = tempdir().expect("root");
    fs::write(directory.path().join(".env"), "TOKEN=1").expect("secret");
    let root = root(&directory);
    assert!(matches!(
        read_range(&root, &classifier(), &path(".env"), run(), 0, 4),
        Err(ViewerError::Sensitive)
    ));
    assert!(matches!(
        read_preview_text(&root, &classifier(), &path(".env"), run()),
        Err(ViewerError::Sensitive)
    ));
}

#[test]
fn a_preview_reads_the_whole_document_up_to_its_cap() {
    let directory = tempdir().expect("root");
    let html = "<!doctype html><html><body>chart</body></html>";
    fs::write(directory.path().join("q3.html"), html).expect("html");
    fs::write(directory.path().join("latin1.html"), [b'<', 0xFF]).expect("latin1");
    let big = fs::File::create(directory.path().join("big.html")).expect("big");
    big.set_len(MAX_PROJECT_PREVIEW_BYTES + 1).expect("length");
    let root = root(&directory);

    assert_eq!(
        read_preview_text(&root, &classifier(), &path("q3.html"), run()).expect("preview"),
        html
    );
    assert!(matches!(
        read_preview_text(&root, &classifier(), &path("latin1.html"), run()),
        Err(ViewerError::NotText)
    ));
    assert!(matches!(
        read_preview_text(&root, &classifier(), &path("big.html"), run()),
        Err(ViewerError::TooLarge { limit }) if limit == MAX_PROJECT_PREVIEW_BYTES
    ));
}

// ── Table windows ────────────────────────────────────────────────────────

fn rows_csv(count: usize) -> String {
    let mut csv = String::from("\u{feff}id,note\n");
    for index in 0..count {
        // Every tenth row carries a quoted delimiter and a quoted newline, so a
        // reader that split on bare newlines would lose count.
        if index % 10 == 0 {
            csv.push_str(&format!("{index},\"multi, line\nnote {index}\"\n"));
        } else {
            csv.push_str(&format!("{index},plain {index}\n"));
        }
    }
    csv
}

#[test]
fn windows_page_through_a_quoted_csv_across_index_checkpoints() {
    let directory = tempdir().expect("root");
    fs::write(directory.path().join("rows.csv"), rows_csv(5_000)).expect("csv");
    let root = root(&directory);
    let window = |start, limit| {
        table_rows(
            &root,
            &classifier(),
            &path("rows.csv"),
            run(),
            b',',
            start,
            limit,
        )
    };

    let first = window(0, 1_000).expect("first window");
    assert_eq!(
        first.headers,
        vec!["id", "note"],
        "the byte-order mark is not a header"
    );
    assert_eq!(first.rows.len(), 1_000);
    assert_eq!(first.rows[0], vec!["0", "multi, line\nnote 0"]);
    assert_eq!(first.rows[999], vec!["999", "plain 999"]);
    assert_eq!(first.next_row, Some(1_000));
    assert_eq!(first.total_rows, None, "the end has not been read yet");

    // Past several checkpoints, then back before them.
    let deep = window(4_000, 1_000).expect("deep window");
    assert_eq!(deep.rows[0][0], "4000");
    assert_eq!(deep.rows[999][0], "4999");
    assert_eq!(deep.next_row, None);
    assert_eq!(deep.total_rows, Some(5_000));
    let back = window(2_050, 3).expect("back window");
    assert_eq!(
        back.rows
            .iter()
            .map(|row| row[0].as_str())
            .collect::<Vec<_>>(),
        vec!["2050", "2051", "2052"]
    );
    assert_eq!(back.total_rows, Some(5_000), "the index remembers the end");

    let at_end = window(5_000, 10).expect("at the end");
    assert!(at_end.rows.is_empty());
    assert_eq!(at_end.next_row, None);
    assert!(matches!(
        window(5_001, 10),
        Err(ViewerError::OffsetPastEnd { .. })
    ));
}

#[test]
fn a_changed_file_gets_a_fresh_index() {
    let directory = tempdir().expect("root");
    let file = directory.path().join("live.tsv");
    fs::write(&file, "a\tb\n1\t2\n").expect("tsv");
    let root = root(&directory);
    let read = || table_rows(&root, &classifier(), &path("live.tsv"), run(), b'\t', 0, 10);
    assert_eq!(read().expect("first").rows, vec![vec!["1", "2"]]);

    fs::write(&file, "a\tb\n7\t8\n9\t10\n").expect("rewrite");
    let second = read().expect("second");
    assert_eq!(second.rows, vec![vec!["7", "8"], vec!["9", "10"]]);
    assert_eq!(second.total_rows, Some(2));
}

#[test]
fn an_oversized_cell_is_cut_for_display_and_the_window_says_so() {
    let directory = tempdir().expect("root");
    let long = "x".repeat(40_000);
    fs::write(directory.path().join("wide.csv"), format!("h\n{long}\n")).expect("csv");
    let root = root(&directory);
    let window =
        table_rows(&root, &classifier(), &path("wide.csv"), run(), b',', 0, 10).expect("window");
    assert!(window.truncated);
    assert!(window.rows[0][0].len() < long.len());
    assert!(window.rows[0][0].ends_with('…'));
}

#[test]
fn a_row_past_the_row_bound_is_refused_rather_than_held_whole() {
    // Stands in for an unclosed quote in a 10 GiB file: without the bound the
    // parser would hold everything after it as one row.
    let directory = tempdir().expect("root");
    let unclosed = format!("h\n\"{}\n", "x".repeat(500));
    fs::write(directory.path().join("broken.csv"), unclosed).expect("csv");
    fs::write(directory.path().join("exact.csv"), "h\nabcd").expect("csv");
    let root = root(&directory);
    let bounds = Bounds {
        row_bytes: 64,
        window_bytes: usize::MAX,
    };
    let read = |name: &str| {
        table_rows_bounded(
            &root,
            &classifier(),
            &path(name),
            run(),
            b',',
            0,
            10,
            bounds,
        )
    };

    assert!(matches!(
        read("broken.csv"),
        Err(ViewerError::RowTooLong { limit: 64 })
    ));
    // A row that ends exactly at the bound, at the end of the file, is whole.
    let exact = table_rows_bounded(
        &root,
        &classifier(),
        &path("exact.csv"),
        run(),
        b',',
        0,
        10,
        Bounds {
            row_bytes: 4,
            window_bytes: usize::MAX,
        },
    )
    .expect("a row at the bound");
    assert_eq!(exact.rows, vec![vec!["abcd"]]);
    assert_eq!(exact.total_rows, Some(1));
}

#[test]
fn a_window_stops_at_its_byte_budget_and_the_next_one_continues() {
    let directory = tempdir().expect("root");
    let mut csv = String::from("id\n");
    for index in 0..20 {
        csv.push_str(&format!("row-{index:05}\n"));
    }
    fs::write(directory.path().join("rows.csv"), csv).expect("csv");
    let root = root(&directory);
    // Each row displays as nine bytes; a 25-byte budget holds three.
    let window = |start| {
        table_rows_bounded(
            &root,
            &classifier(),
            &path("rows.csv"),
            run(),
            b',',
            start,
            10,
            Bounds {
                row_bytes: 1024,
                window_bytes: 25,
            },
        )
        .expect("window")
    };

    let first = window(0);
    assert_eq!(
        first.rows,
        vec![vec!["row-00000"], vec!["row-00001"], vec!["row-00002"]]
    );
    assert_eq!(first.next_row, Some(3));
    assert!(!first.truncated, "a short window is paging, not a cut");
    let second = window(3);
    assert_eq!(second.rows[0], vec!["row-00003"]);
    assert_eq!(second.next_row, Some(6));
    let last = window(18);
    assert_eq!(last.rows, vec![vec!["row-00018"], vec!["row-00019"]]);
    assert_eq!(last.next_row, None);
    assert_eq!(last.total_rows, Some(20));
}

#[test]
fn a_sensitive_or_binary_path_never_yields_table_rows() {
    let directory = tempdir().expect("root");
    fs::write(directory.path().join(".env"), "A,B\n1,2\n").expect("secret");
    fs::write(directory.path().join("blob.csv"), [b'a', b',', 0, b'\n']).expect("binary");
    let root = root(&directory);
    assert!(matches!(
        table_rows(&root, &classifier(), &path(".env"), run(), b',', 0, 10),
        Err(ViewerError::Sensitive)
    ));
    assert!(matches!(
        table_rows(&root, &classifier(), &path("blob.csv"), run(), b',', 0, 10),
        Err(ViewerError::NotText)
    ));
}

#[test]
fn a_start_row_near_the_top_of_the_range_does_not_overflow() {
    let directory = tempdir().expect("root");
    fs::write(directory.path().join("small.csv"), "h\n1\n").expect("csv");
    let root = root(&directory);
    assert!(matches!(
        table_rows(
            &root,
            &classifier(),
            &path("small.csv"),
            run(),
            b',',
            u64::MAX - 1,
            1_000,
        ),
        Err(ViewerError::OffsetPastEnd { size: 1, .. })
    ));
}
