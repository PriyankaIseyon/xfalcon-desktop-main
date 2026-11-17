use xfalcon_contracts::datasets::{DatasetErrorCode, DatasetFormat};

use crate::datasets::tests::fixtures;
use crate::datasets::validate::{classify, verify_content};

#[test]
fn only_the_two_accepted_extensions_classify() {
    let directory = tempfile::tempdir().expect("temp dir");
    for (name, expected) in [
        ("sales.csv", DatasetFormat::Csv),
        ("sales.xlsx", DatasetFormat::Xlsx),
    ] {
        let path = fixtures::write_file(directory.path(), name, b"a,b\n1,2\n");
        assert_eq!(classify(&path), Ok(expected), "{name}");
    }
    // `.xls` is deliberately absent: DA's presign maps an extension to one
    // content type, and a legacy workbook is not one of the two it accepts.
    for name in ["sales.xls", "sales.parquet", "sales.json", "sales"] {
        let path = fixtures::write_file(directory.path(), name, b"anything");
        assert_eq!(
            classify(&path),
            Err(DatasetErrorCode::UnsupportedFormat),
            "{name}"
        );
    }
}

#[test]
fn an_extension_is_read_case_insensitively() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "QUARTER.CSV", b"a\n1\n");
    assert_eq!(classify(&path), Ok(DatasetFormat::Csv));
}

#[test]
fn an_empty_file_is_refused_before_any_content_check() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "empty.csv", b"");
    assert_eq!(
        verify_content(&path, DatasetFormat::Csv, 0),
        Err(DatasetErrorCode::EmptyFile)
    );
}

#[test]
fn a_csv_is_accepted_on_any_non_empty_content() {
    let directory = tempfile::tempdir().expect("temp dir");
    // A byte-order mark and a leading comment line are both real exports.
    let contents = "\u{feff}# generated\nregion,revenue\nEMEA,10\n".as_bytes();
    let path = fixtures::write_file(directory.path(), "export.csv", contents);
    assert_eq!(
        verify_content(&path, DatasetFormat::Csv, contents.len() as u64),
        Ok(())
    );
}

#[test]
fn a_real_workbook_is_accepted() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[fixtures::Sheet {
            name: "Data",
            rows: &[&["region", "revenue"], &["EMEA", "10"]],
        }],
    );
    let size = std::fs::metadata(&path).expect("metadata").len();
    assert_eq!(verify_content(&path, DatasetFormat::Xlsx, size), Ok(()));
}

#[test]
fn a_plain_zip_named_xlsx_is_refused() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_plain_zip(directory.path(), "archive.xlsx");
    let size = std::fs::metadata(&path).expect("metadata").len();
    assert_eq!(
        verify_content(&path, DatasetFormat::Xlsx, size),
        Err(DatasetErrorCode::ContentMismatch),
        "a zip signature is not proof of a workbook"
    );
}

#[test]
fn a_pdf_renamed_to_xlsx_is_refused() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "report.xlsx", b"%PDF-1.7\n%...");
    let size = std::fs::metadata(&path).expect("metadata").len();
    assert_eq!(
        verify_content(&path, DatasetFormat::Xlsx, size),
        Err(DatasetErrorCode::ContentMismatch)
    );
}

#[test]
fn a_password_protected_workbook_says_so() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_encrypted_workbook(directory.path(), "locked.xlsx");
    let size = std::fs::metadata(&path).expect("metadata").len();
    assert_eq!(
        verify_content(&path, DatasetFormat::Xlsx, size),
        Err(DatasetErrorCode::EncryptedFile),
        "an encrypted workbook is actionable; a mismatch is not"
    );
}

#[test]
fn a_compound_document_that_is_not_encrypted_is_a_mismatch() {
    let directory = tempfile::tempdir().expect("temp dir");
    let mut bytes: Vec<u8> = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    bytes.resize(2048, 0);
    let path = fixtures::write_file(directory.path(), "legacy.xlsx", &bytes);
    assert_eq!(
        verify_content(&path, DatasetFormat::Xlsx, bytes.len() as u64),
        Err(DatasetErrorCode::ContentMismatch)
    );
}

#[test]
fn a_file_shorter_than_the_sniff_window_is_a_mismatch_not_a_panic() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "tiny.xlsx", b"PK");
    assert_eq!(
        verify_content(&path, DatasetFormat::Xlsx, 2),
        Err(DatasetErrorCode::ContentMismatch)
    );
}
