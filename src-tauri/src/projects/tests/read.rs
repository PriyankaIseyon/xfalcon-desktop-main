//! Coverage for `super::super::read`: whole-file reads (text, binary,
//! non-UTF-8, oversized), bounded partial line-range reads, the
//! sensitive-file override gate, and directory listing.
//!
//! Fixtures use a real temporary directory and `TrustedRoot::establish`,
//! mirroring `tests::path_policy`'s fixture shape.

use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tempfile::tempdir;
use xfalcon_contracts::RunId;

use crate::audit::{
    AuditAction, AuditEvent, AuditOutcome, AuditPublishStatus, AuditPublisher, AuditQueue,
    AuditSink,
};
use crate::domain::error::AppResult;
use crate::persistence::{InMemorySecretStore, Journal};
use crate::projects::{
    list_directory, read_file, read_file_with_audit, ContentKind, EntryKind, LineRange,
    MetadataOnlyReason, ProjectId, ReadError, ReadFileOutcome, ReadFileRequest,
    RelativeProjectPath, SecretClassifier, SensitiveDecision, SensitiveOverrideGrant,
    SensitiveReadScope, TrustedRoot, MAX_HASHED_FILE_BYTES, MAX_LINE_RANGE_LINES,
};

/// A model-sized page: the smallest run budget's page for a short path.
const PAGE_BYTES: usize = 27 * 1024;
const PRODUCER_DOCX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.docx");
const PRODUCER_PPTX: &[u8] = include_bytes!("../../../tests/fixtures/office/producer.pptx");
const LEGACY_DOC: &[u8] = include_bytes!("../../../tests/fixtures/office/legacy-poi/simple.doc");
const LEGACY_PPT: &[u8] =
    include_bytes!("../../../tests/fixtures/office/legacy-poi/text_shapes.ppt");

fn trusted_root(root: &std::path::Path) -> TrustedRoot {
    TrustedRoot::establish(ProjectId::new(), root).expect("establish trusted root")
}

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path")
}

fn classifier() -> SecretClassifier {
    SecretClassifier::built_in()
}

fn run_id() -> RunId {
    RunId::from("run-1")
}

/// Builds a matching `(scope, request)` pair for `read_file`, mirroring
/// `read_file`'s own documented invariant that both must name the same path.
fn read(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &str,
    line_range: Option<LineRange>,
    sensitive_override: Option<SensitiveOverrideGrant>,
) -> Result<ReadFileOutcome, ReadError> {
    let relative_path = relative(path);
    let scope = SensitiveReadScope::new(*root.project_id(), relative_path.clone(), run_id());
    read_file(
        root,
        classifier,
        scope,
        ReadFileRequest {
            relative_path,
            line_range,
            sensitive_override,
            page_bytes: PAGE_BYTES,
        },
    )
}

// ── Whole-file reads ─────────────────────────────────────────────────────

#[test]
fn whole_file_text_read_returns_full_content_and_line_span() {
    let directory = tempdir().expect("fixture root");
    fs::write(
        directory.path().join("notes.txt"),
        b"line one\nline two\nline three",
    )
    .expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "notes.txt", None, None).expect("read succeeds");
    let ReadFileOutcome::Text(text) = outcome else {
        panic!("expected Text outcome");
    };
    assert_eq!(text.content, "line one\nline two\nline three");
    assert_eq!(text.first_line, 1);
    assert_eq!(text.last_line, 3);
    assert!(!text.truncated);
}

#[test]
fn office_read_extracts_and_pages_from_the_shared_text_with_matching_line_numbers() {
    let directory = tempdir().expect("fixture root");
    let source = directory.path().join("brief.docx");
    fs::write(&source, PRODUCER_DOCX).expect("producer DOCX fixture");
    let original = fs::read(&source).expect("original bytes");
    let root = trusted_root(directory.path());

    let first = read(&root, &classifier(), "brief.docx", None, None).expect("Office first page");
    let ReadFileOutcome::Text(first) = first else {
        panic!("expected extracted text page");
    };
    assert!(first.content.contains("Office text extraction"));
    assert!(first.content.contains("Producer DOCX fixture"));
    assert_eq!(first.first_line, 1);

    let extracted = crate::documents::extract(std::path::Path::new("brief.docx"), PRODUCER_DOCX)
        .expect("fixture extracts through the shared reader");
    let unicode_line = extracted
        .text
        .lines()
        .position(|line| line.contains("Unicode café"))
        .expect("fixture Unicode paragraph") as u32
        + 1;
    let second = read(
        &root,
        &classifier(),
        "brief.docx",
        Some(LineRange::new(unicode_line, unicode_line).unwrap()),
        None,
    )
    .expect("Office line range");
    let ReadFileOutcome::Text(second) = second else {
        panic!("expected extracted text line range");
    };
    assert_eq!(second.first_line, unicode_line);
    assert!(second.content.contains("Unicode café"));
    assert_eq!(
        fs::read(source).expect("source remains unchanged"),
        original
    );
}

#[test]
fn office_read_fails_malformed_files_and_withholds_sensitive_office_paths() {
    let directory = tempdir().expect("fixture root");
    fs::write(
        directory.path().join("broken.docx"),
        b"not an Office package",
    )
    .expect("broken file");
    fs::write(directory.path().join(".env.docx"), PRODUCER_DOCX).expect("sensitive Office file");
    let root = trusted_root(directory.path());

    assert!(matches!(
        read(&root, &classifier(), "broken.docx", None, None),
        Err(ReadError::Office(_))
    ));
    assert!(matches!(
        read(&root, &classifier(), ".env.docx", None, None),
        Ok(ReadFileOutcome::SensitiveOverrideRequired(_))
    ));
}

#[test]
fn project_reads_all_four_office_families_without_changing_source_bytes() {
    let directory = tempdir().expect("fixture root");
    let root = trusted_root(directory.path());
    for (name, bytes) in [
        ("modern.docx", PRODUCER_DOCX),
        ("modern.PPTX", PRODUCER_PPTX),
        ("legacy.doc", LEGACY_DOC),
        ("legacy.PPT", LEGACY_PPT),
    ] {
        let source = directory.path().join(name);
        fs::write(&source, bytes).expect("producer fixture");
        let before = fs::read(&source).expect("source bytes before extraction");
        assert!(crate::documents::is_office_path(std::path::Path::new(name)));

        let extracted = crate::documents::extract(std::path::Path::new(name), bytes)
            .expect("supported Office family extracts");
        let ReadFileOutcome::Text(read) =
            read(&root, &classifier(), name, None, None).expect("project read extracts Office")
        else {
            panic!("Office extraction must produce text");
        };
        assert_eq!(
            read.content.trim_end_matches('\n'),
            extracted.text.trim_end_matches('\n'),
            "{name}"
        );
        assert!(read.content.contains("Office text extraction"), "{name}");
        assert!(
            read.content.len() > "Office text extraction".len(),
            "{name}"
        );
        assert_eq!(
            fs::read(source).expect("source bytes after extraction"),
            before,
            "{name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn project_office_read_refuses_a_symlink_outside_the_trusted_root() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    let outside = tempdir().expect("outside root");
    let outside_file = outside.path().join("brief.docx");
    fs::write(&outside_file, PRODUCER_DOCX).expect("outside document");
    symlink(&outside_file, directory.path().join("brief.docx")).expect("Office symlink");
    let root = trusted_root(directory.path());

    assert!(read(&root, &classifier(), "brief.docx", None, None).is_err());
    assert_eq!(
        fs::read(outside_file).expect("outside bytes unchanged"),
        PRODUCER_DOCX
    );
}

#[test]
fn hash_readers_refuse_a_file_that_grew_past_the_checked_limit() {
    use std::io::Write;

    let directory = tempdir().expect("fixture root");
    let path = directory.path().join("growing.txt");
    for limit in [8usize, 64 * 1024] {
        fs::write(&path, vec![b'a'; limit]).expect("fixture file");
        let buffered = fs::File::open(&path).expect("buffered reader");
        let streaming = fs::File::open(&path).expect("streaming reader");
        assert_eq!(buffered.metadata().unwrap().len(), limit as u64);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"b")
            .expect("file grows after the size check");

        let buffered = crate::projects::read::hash_and_classify(buffered, limit as u64)
            .expect_err("a partial buffer and hash must not claim a complete read");
        let streaming = crate::projects::read::hash_and_classify_streaming(streaming, limit as u64)
            .expect_err("a prefix hash must not claim to identify the whole file");
        assert_eq!(buffered.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(streaming.kind(), std::io::ErrorKind::InvalidData);
    }
}

#[test]
fn hash_readers_accept_empty_and_exact_limit_files_with_complete_hashes() {
    use sha2::{Digest, Sha256};

    let directory = tempdir().expect("fixture root");
    let path = directory.path().join("bounded.txt");
    for limit in [0usize, 8, 64 * 1024] {
        let content = vec![b'a'; limit];
        fs::write(&path, &content).expect("fixture file");
        let expected = format!("{:x}", Sha256::digest(&content));
        let (kind, hash, bytes) =
            crate::projects::read::hash_and_classify(fs::File::open(&path).unwrap(), limit as u64)
                .expect("the whole file fits");
        assert_eq!(kind, ContentKind::Text);
        assert_eq!(hash, expected);
        assert_eq!(bytes, content);
        let (kind, hash) = crate::projects::read::hash_and_classify_streaming(
            fs::File::open(&path).unwrap(),
            limit as u64,
        )
        .expect("the whole file fits");
        assert_eq!(kind, ContentKind::Text);
        assert_eq!(hash, expected);
    }
}

#[test]
fn whole_file_binary_read_returns_metadata_and_hash_never_bytes() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("binary.bin"), [0u8, 1, 2, 0, 3, 4]).expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "binary.bin", None, None).expect("read succeeds");
    let ReadFileOutcome::MetadataOnly {
        content_kind,
        sha256,
        reason,
        size_bytes,
    } = outcome
    else {
        panic!("expected MetadataOnly outcome");
    };
    assert_eq!(content_kind, ContentKind::Binary);
    assert_eq!(reason, MetadataOnlyReason::Binary);
    assert!(sha256.is_some());
    assert_eq!(size_bytes, 6);
}

#[test]
fn whole_file_non_utf8_read_returns_metadata_and_hash_never_bytes() {
    let directory = tempdir().expect("fixture root");
    // 0xFF is never valid as a standalone UTF-8 byte, and contains no NUL, so
    // this is classified `NonUtf8Text`, not `Binary`.
    fs::write(
        directory.path().join("latin1.txt"),
        [b'h', b'i', 0xFF, 0xFE],
    )
    .expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "latin1.txt", None, None).expect("read succeeds");
    let ReadFileOutcome::MetadataOnly {
        content_kind,
        sha256,
        reason,
        ..
    } = outcome
    else {
        panic!("expected MetadataOnly outcome");
    };
    assert_eq!(content_kind, ContentKind::NonUtf8Text);
    assert_eq!(reason, MetadataOnlyReason::NonUtf8Text);
    assert!(sha256.is_some());
}

#[test]
fn a_text_file_over_the_whole_read_limit_returns_its_first_page_and_where_to_continue() {
    let directory = tempdir().expect("fixture root");
    // 2 MiB of text: above `MAX_FILE_READ_BYTES` (1 MiB), so it cannot come
    // back whole -- but a long text file is readable a page at a time.
    let content = "some line of text\n".repeat(120_000);
    fs::write(directory.path().join("large.txt"), &content).expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "large.txt", None, None).expect("read succeeds");
    let ReadFileOutcome::Text(text) = outcome else {
        panic!("expected a first page of text, not a refusal");
    };
    assert_eq!(text.first_line, 1);
    assert!(text.last_line > 0, "the page carries lines");
    assert_eq!(
        text.next_line,
        Some(text.last_line + 1),
        "a bounded read names the line the next read starts at"
    );
    // The budget is checked before each line is added, and counts the page as
    // it is sent, so a page never overshoots it.
    assert!(
        crate::tools::budget::json_string_len(&text.content) <= PAGE_BYTES,
        "a page stays within its byte budget as sent"
    );
}

#[test]
fn a_file_of_one_enormous_line_returns_its_cut_start_and_then_the_next_line() {
    let directory = tempdir().expect("fixture root");
    // 3 MiB on one line, then a short one. The odd leading byte puts the line
    // bound inside a two-byte character, so the cut has to back off to one.
    let long = format!("a{}", "\u{e9}".repeat(1_536 * 1024));
    fs::write(
        directory.path().join("minified.js"),
        format!("{long}\nshort\n"),
    )
    .expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "minified.js", None, None).expect("read succeeds");
    let ReadFileOutcome::Text(text) = outcome else {
        panic!("expected the long line's first page");
    };
    assert!(text.truncated, "a cut line is marked");
    // The cut start and the next line both fit the page, so the page carries
    // them both rather than stopping at the cut.
    assert_eq!(
        (text.first_line, text.last_line, text.next_line),
        (1, 2, None),
        "the page continues past the cut line"
    );
    let (shown, rest) = text.content.split_once('\n').expect("the line is ended");
    assert_eq!(shown.len(), crate::projects::MAX_LINE_BYTES - 1);
    assert!(long.starts_with(shown));
    assert_eq!(rest, "short\n");

    let outcome = read(
        &root,
        &classifier(),
        "minified.js",
        Some(LineRange::page_from(2).expect("range")),
        None,
    )
    .expect("read succeeds");
    let ReadFileOutcome::Text(next) = outcome else {
        panic!("expected the short line");
    };
    assert_eq!(next.content, "short\n");
    assert_eq!(next.next_line, None);
    assert!(!next.truncated);
}

#[test]
fn a_binary_file_over_the_whole_read_limit_returns_metadata_with_a_hash() {
    let directory = tempdir().expect("fixture root");
    // Binary content cannot be paged as text, so this is the case that still
    // reports metadata -- with an identity, because it is within the hash bound.
    let mut content = vec![b'a'; 2 * 1024 * 1024];
    content[0] = 0;
    fs::write(directory.path().join("large.bin"), &content).expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "large.bin", None, None).expect("read succeeds");
    let ReadFileOutcome::MetadataOnly {
        content_kind,
        sha256,
        reason,
        size_bytes,
    } = outcome
    else {
        panic!("expected MetadataOnly outcome");
    };
    assert_eq!(content_kind, ContentKind::Binary);
    assert_eq!(reason, MetadataOnlyReason::Binary);
    assert!(sha256.is_some());
    assert_eq!(size_bytes, content.len() as u64);
}

#[test]
fn a_file_beyond_the_hash_limit_returns_no_hash_at_all() {
    let directory = tempdir().expect("fixture root");
    // Sized, not written: `set_len` leaves the bytes unallocated, so this
    // reaches past the hash bound without a multi-gigabyte fixture. The read
    // decides from metadata plus a prefix sniff, and never reads the rest.
    let path = directory.path().join("huge.bin");
    let file = fs::File::create(&path).expect("fixture file");
    file.set_len(MAX_HASHED_FILE_BYTES + 1)
        .expect("a file past the hash bound");
    drop(file);
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), "huge.bin", None, None).expect("read succeeds");
    let ReadFileOutcome::MetadataOnly { sha256, reason, .. } = outcome else {
        panic!("expected MetadataOnly outcome");
    };
    // `Oversized` now means exactly this: so large that not even an identity
    // was established.
    assert_eq!(reason, MetadataOnlyReason::Oversized);
    assert!(
        sha256.is_none(),
        "reading through an unbounded file must not happen"
    );
}

#[test]
fn reading_a_missing_file_reports_not_found() {
    let directory = tempdir().expect("fixture root");
    let root = trusted_root(directory.path());

    let error = read(&root, &classifier(), "does-not-exist.txt", None, None)
        .expect_err("missing file must be rejected");
    assert!(matches!(
        error,
        ReadError::Containment(crate::projects::ContainmentError::NotFound)
    ));
}

// ── Partial (line-range) reads ────────────────────────────────────────────

#[test]
fn partial_read_returns_exactly_the_requested_line_span() {
    let directory = tempdir().expect("fixture root");
    fs::write(
        directory.path().join("lines.txt"),
        b"one\ntwo\nthree\nfour\nfive\n",
    )
    .expect("fixture file");
    let root = trusted_root(directory.path());
    let range = LineRange::new(2, 3).expect("valid range");

    let outcome =
        read(&root, &classifier(), "lines.txt", Some(range), None).expect("partial read succeeds");
    let ReadFileOutcome::Text(text) = outcome else {
        panic!("expected Text outcome");
    };
    assert_eq!(text.content, "two\nthree\n");
    assert_eq!(text.first_line, 2);
    assert_eq!(text.last_line, 3);
    assert!(!text.truncated);
}

#[test]
fn partial_read_past_the_files_actual_length_returns_what_exists() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("short.txt"), b"one\ntwo\nthree\n").expect("fixture file");
    let root = trusted_root(directory.path());
    let range = LineRange::new(2, 100).expect("valid range");

    let outcome =
        read(&root, &classifier(), "short.txt", Some(range), None).expect("partial read succeeds");
    let ReadFileOutcome::Text(text) = outcome else {
        panic!("expected Text outcome");
    };
    assert_eq!(text.content, "two\nthree\n");
    assert_eq!(text.first_line, 2);
    assert_eq!(text.last_line, 3);
}

#[test]
fn partial_read_of_binary_content_returns_metadata_never_a_misleading_empty_text() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("binary.bin"), [0u8, 1, 2, 3]).expect("fixture file");
    let root = trusted_root(directory.path());
    let range = LineRange::new(1, 10).expect("valid range");

    let outcome =
        read(&root, &classifier(), "binary.bin", Some(range), None).expect("read succeeds");
    let ReadFileOutcome::MetadataOnly {
        content_kind,
        reason,
        ..
    } = outcome
    else {
        panic!("expected MetadataOnly outcome, not an empty/misleading Text result");
    };
    assert_eq!(content_kind, ContentKind::Binary);
    assert_eq!(reason, MetadataOnlyReason::Binary);
}

#[test]
fn line_range_boundary_is_enforced() {
    assert!(LineRange::new(1, MAX_LINE_RANGE_LINES).is_ok());
    assert!(matches!(
        LineRange::new(1, MAX_LINE_RANGE_LINES + 1),
        Err(ReadError::InvalidLineRange(limit)) if limit == MAX_LINE_RANGE_LINES
    ));
    assert!(matches!(
        LineRange::new(0, 1),
        Err(ReadError::InvalidLineRange(limit)) if limit == MAX_LINE_RANGE_LINES
    ));
    assert!(matches!(
        LineRange::new(5, 4),
        Err(ReadError::InvalidLineRange(limit)) if limit == MAX_LINE_RANGE_LINES
    ));
}

#[test]
fn partial_read_rejects_binary_in_discarded_long_line_suffixes() {
    for (content, start_line, end_line) in [
        (format!("{}\0\nvisible\n", "a".repeat(20_003)), 1, 1),
        (format!("{}\0\nvisible\n", "a".repeat(20_003)), 2, 2),
        (format!("visible\n{}\0\n", "a".repeat(20_003)), 1, 1),
    ] {
        let directory = tempdir().expect("fixture root");
        fs::write(directory.path().join("long-binary.txt"), content).expect("seed file");
        let root = trusted_root(directory.path());
        let outcome = read(
            &root,
            &classifier(),
            "long-binary.txt",
            Some(LineRange::new(start_line, end_line).expect("valid range")),
            None,
        )
        .expect("classify bounded read");
        assert!(
            matches!(
                outcome,
                ReadFileOutcome::MetadataOnly {
                    content_kind: ContentKind::Binary,
                    reason: MetadataOnlyReason::Binary,
                    ..
                }
            ),
            "line range {start_line}..{end_line} must classify consumed suffix bytes"
        );
    }
}

// ── Sensitive-file override gate ──────────────────────────────────────────

#[test]
fn reading_a_sensitive_file_without_an_override_requires_one_and_returns_no_content() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join(".env"), b"SECRET=top-secret-value").expect("fixture file");
    let root = trusted_root(directory.path());

    let outcome = read(&root, &classifier(), ".env", None, None).expect("read call succeeds");
    let ReadFileOutcome::SensitiveOverrideRequired(request) = outcome else {
        panic!("expected SensitiveOverrideRequired, content must never be returned unasked");
    };
    assert_eq!(request.scope().relative_path().as_str(), ".env");
}

#[test]
fn reading_a_sensitive_file_with_a_matching_override_returns_its_content() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join(".env"), b"SECRET=top-secret-value").expect("fixture file");
    let root = trusted_root(directory.path());
    let classifier = classifier();

    let ReadFileOutcome::SensitiveOverrideRequired(request) =
        read(&root, &classifier, ".env", None, None).expect("read call succeeds")
    else {
        panic!("expected SensitiveOverrideRequired on the first call");
    };
    let grant = request.grant();

    let outcome =
        read(&root, &classifier, ".env", None, Some(grant)).expect("overridden read succeeds");
    let ReadFileOutcome::Text(text) = outcome else {
        panic!("expected Text outcome once a matching override is presented");
    };
    assert_eq!(text.content, "SECRET=top-secret-value");
}

#[test]
fn a_grant_for_a_different_run_does_not_satisfy_the_same_project_and_path() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join(".env"), b"SECRET=1").expect("fixture file");
    let root = trusted_root(directory.path());
    let classifier = classifier();
    let relative_path = relative(".env");

    // A grant obtained for a *different* run id must not satisfy this scope
    // -- construct it directly via `evaluate`, mirroring
    // `tests::sensitive`'s own exact-scoping tests, rather than through
    // `read_file` (whose own scope is fixed to `run_id()` by the `read`
    // helper above).
    let other_run_scope = SensitiveReadScope::new(
        *root.project_id(),
        relative_path.clone(),
        RunId::from("run-2"),
    );
    let SensitiveDecision::RequiresOverride(request) =
        crate::projects::evaluate(&classifier, other_run_scope, None)
    else {
        panic!("expected a pending override request");
    };
    let grant_for_other_run = request.grant();

    let outcome = read(&root, &classifier, ".env", None, Some(grant_for_other_run))
        .expect("read call succeeds");
    assert!(matches!(
        outcome,
        ReadFileOutcome::SensitiveOverrideRequired(_)
    ));
}

// ── Directory listing ─────────────────────────────────────────────────────

#[test]
fn list_directory_lists_the_root_and_flags_a_sensitive_child() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("notes.txt"), b"notes").expect("fixture file");
    fs::write(directory.path().join(".env"), b"SECRET=1").expect("fixture file");
    fs::create_dir(directory.path().join("subdir")).expect("fixture subdir");
    let root = trusted_root(directory.path());

    let entries = list_directory(&root, &classifier(), None).expect("list_directory succeeds");
    let notes = entries
        .iter()
        .find(|entry| entry.name == "notes.txt")
        .expect("notes.txt listed");
    assert_eq!(notes.kind, EntryKind::File);
    assert!(!notes.is_sensitive);

    let env = entries
        .iter()
        .find(|entry| entry.name == ".env")
        .expect(".env listed");
    assert!(env.is_sensitive);

    let subdir = entries
        .iter()
        .find(|entry| entry.name == "subdir")
        .expect("subdir listed");
    assert_eq!(subdir.kind, EntryKind::Directory);
}

#[test]
fn list_directory_lists_a_named_subdirectorys_children() {
    let directory = tempdir().expect("fixture root");
    fs::create_dir(directory.path().join("subdir")).expect("fixture subdir");
    fs::write(directory.path().join("subdir/inner.txt"), b"inner").expect("fixture file");
    let root = trusted_root(directory.path());

    let entries = list_directory(&root, &classifier(), Some(relative("subdir")))
        .expect("list_directory succeeds");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "inner.txt");
}

#[test]
fn list_directory_flags_a_bare_secret_directory_name_as_sensitive() {
    let directory = tempdir().expect("fixture root");
    fs::create_dir(directory.path().join(".aws")).expect("fixture .aws dir");
    fs::write(directory.path().join(".aws/credentials"), b"AKIA...").expect("fixture file");
    fs::create_dir(directory.path().join("plain")).expect("fixture plain dir");
    let root = trusted_root(directory.path());

    let entries = list_directory(&root, &classifier(), None).expect("list_directory succeeds");
    let aws_dir = entries
        .iter()
        .find(|entry| entry.name == ".aws")
        .expect(".aws listed");
    assert_eq!(aws_dir.kind, EntryKind::Directory);
    assert!(
        aws_dir.is_sensitive,
        "a bare secret-class directory name must itself be flagged sensitive"
    );

    let plain_dir = entries
        .iter()
        .find(|entry| entry.name == "plain")
        .expect("plain listed");
    assert!(!plain_dir.is_sensitive);
}

#[cfg(unix)]
#[test]
fn list_directory_reports_a_symlink_child_as_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("real.txt"), b"real").expect("fixture file");
    symlink(
        directory.path().join("real.txt"),
        directory.path().join("alias.txt"),
    )
    .expect("internal symlink");
    let root = trusted_root(directory.path());

    let entries = list_directory(&root, &classifier(), None).expect("list_directory succeeds");
    let alias = entries
        .iter()
        .find(|entry| entry.name == "alias.txt")
        .expect("alias.txt listed");
    assert_eq!(alias.kind, EntryKind::Symlink);
}

// ── `read_file_with_audit` (sensitive-read decision audit — Task 7) ────────

struct RecordingAuditSink {
    events: Mutex<Vec<AuditEvent>>,
}

#[async_trait]
impl AuditSink for RecordingAuditSink {
    async fn publish(&self, event: &AuditEvent) -> AppResult<AuditPublishStatus> {
        self.events.lock().unwrap().push(event.clone());
        Ok(AuditPublishStatus::Published)
    }
}

fn audit_queue_for_test() -> Arc<AuditQueue> {
    let directory = tempdir().expect("audit dir");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key("account-1", [0x55; 32]).expect("key");
    let journal = Arc::new(
        Journal::open(directory.path().join("audit.sqlite3"), secrets).expect("open journal"),
    );
    Arc::new(AuditQueue::new("account-1", 16, journal).expect("queue"))
}

#[tokio::test]
async fn read_file_with_audit_publishes_nothing_for_an_ordinary_non_sensitive_read() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join("notes.txt"), b"hello").expect("fixture file");
    let root = trusted_root(directory.path());
    let classifier = classifier();
    let sink = Arc::new(RecordingAuditSink {
        events: Mutex::new(Vec::new()),
    });
    let audit = AuditPublisher::new(
        "actor-1",
        "tenant-1",
        "device-1",
        sink.clone(),
        audit_queue_for_test(),
    );

    let relative_path = relative("notes.txt");
    let scope = SensitiveReadScope::new(*root.project_id(), relative_path.clone(), run_id());
    let outcome = read_file_with_audit(
        &root,
        &classifier,
        scope,
        ReadFileRequest {
            relative_path,
            line_range: None,
            sensitive_override: None,
            page_bytes: PAGE_BYTES,
        },
        Some(&audit),
    )
    .await
    .expect("read succeeds");
    assert!(matches!(outcome, ReadFileOutcome::Text(_)));
    assert!(
        sink.events.lock().unwrap().is_empty(),
        "an ordinary non-sensitive read must not be audited — this is a decision log for the \
         sensitive-file gate, not a blanket access log"
    );
}

#[tokio::test]
async fn read_file_with_audit_publishes_approval_required_for_an_unoverridden_sensitive_read() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join(".env"), b"SECRET=1").expect("fixture file");
    let root = trusted_root(directory.path());
    let classifier = classifier();
    let sink = Arc::new(RecordingAuditSink {
        events: Mutex::new(Vec::new()),
    });
    let audit = AuditPublisher::new(
        "actor-1",
        "tenant-1",
        "device-1",
        sink.clone(),
        audit_queue_for_test(),
    );

    let relative_path = relative(".env");
    let scope = SensitiveReadScope::new(*root.project_id(), relative_path.clone(), run_id());
    let outcome = read_file_with_audit(
        &root,
        &classifier,
        scope,
        ReadFileRequest {
            relative_path,
            line_range: None,
            sensitive_override: None,
            page_bytes: PAGE_BYTES,
        },
        Some(&audit),
    )
    .await
    .expect("read call succeeds");
    assert!(matches!(
        outcome,
        ReadFileOutcome::SensitiveOverrideRequired(_)
    ));

    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action(), AuditAction::ProjectRead);
    assert_eq!(events[0].outcome(), AuditOutcome::ApprovalRequired);
    assert_eq!(events[0].resource_id(), ".env");
}

#[tokio::test]
async fn read_file_with_audit_publishes_allowed_for_an_overridden_sensitive_read() {
    let directory = tempdir().expect("fixture root");
    fs::write(directory.path().join(".env"), b"SECRET=1").expect("fixture file");
    let root = trusted_root(directory.path());
    let classifier = classifier();
    let sink = Arc::new(RecordingAuditSink {
        events: Mutex::new(Vec::new()),
    });
    let audit = AuditPublisher::new(
        "actor-1",
        "tenant-1",
        "device-1",
        sink.clone(),
        audit_queue_for_test(),
    );

    let ReadFileOutcome::SensitiveOverrideRequired(request) =
        read(&root, &classifier, ".env", None, None).expect("first read call succeeds")
    else {
        panic!("expected SensitiveOverrideRequired on the first call");
    };
    let grant = request.grant();

    let relative_path = relative(".env");
    let scope = SensitiveReadScope::new(*root.project_id(), relative_path.clone(), run_id());
    let outcome = read_file_with_audit(
        &root,
        &classifier,
        scope,
        ReadFileRequest {
            relative_path,
            line_range: None,
            sensitive_override: Some(grant),
            page_bytes: PAGE_BYTES,
        },
        Some(&audit),
    )
    .await
    .expect("overridden read succeeds");
    assert!(matches!(outcome, ReadFileOutcome::Text(_)));

    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action(), AuditAction::ProjectRead);
    assert_eq!(events[0].outcome(), AuditOutcome::Allowed);
}
