//! What a picked file has to be before any of it reaches DA.
//!
//! Every refusal here happens before the first network call. A mistaken pick
//! then costs a message rather than a transfer the person has to watch and
//! cancel — and DA's `/presign` enforces `content_type == _EXT_TO_MIME[ext]`,
//! so a file this module lets through with the wrong content type would be
//! refused server-side only after its bytes were sent.
//!
//! The rules are xfalcon-web's, from `src/lib/datasets/magic-bytes.ts`, with
//! one deliberate difference. Web skips the structural zip read above 64 MiB,
//! because `fflate` needs the whole archive in the browser's heap to find the
//! central directory at its tail. Reading a zip's central directory from a
//! file on disk is a seek, not an allocation, so there is no size at which
//! this module has to stop checking — and a polyglot zip above web's ceiling
//! is exactly the case a ceiling would let through.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use xfalcon_contracts::datasets::{DatasetErrorCode, DatasetFormat};

/// Enough to tell a zip archive, a compound document, and anything else
/// apart: the longest signature below is eight bytes.
const SNIFF_BYTES: usize = 8;

/// Zip local-file-header, end-of-central-directory, and spanning signatures.
/// A workbook written by any of the common tools starts with one of the three.
const ZIP_SIGNATURES: [&[u8]; 3] = [
    &[0x50, 0x4b, 0x03, 0x04],
    &[0x50, 0x4b, 0x05, 0x06],
    &[0x50, 0x4b, 0x07, 0x08],
];

/// OLE/CFBF compound-document signature. A password-protected OOXML workbook
/// is one of these rather than a zip.
const CFBF_SIGNATURE: &[u8] = &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// How far into a compound document to look for the encrypted-package stream
/// name. The directory sits near the head, so a bounded scan answers the
/// question; scanning the whole file would read a 2 GiB pick to decide a
/// message.
const ENCRYPTION_SCAN_BYTES: u64 = 256 * 1024;

/// Zip entry names to read before giving up on finding the workbook parts.
///
/// Both markers appear in the first handful of entries of any real workbook.
/// The cap is here because an archive can declare millions of entries, and
/// enumerating them all to answer a yes/no question is work an untrusted file
/// should not be able to ask for.
const MAX_ZIP_ENTRIES_SCANNED: usize = 4096;

/// The encrypted-package stream name, as a compound document stores it:
/// UTF-16LE, so every ASCII character is followed by a zero byte.
fn encrypted_package_marker() -> Vec<u8> {
    "EncryptedPackage"
        .bytes()
        .flat_map(|byte| [byte, 0])
        .collect()
}

/// The format this file's extension claims, or why the extension is not one
/// this destination accepts.
///
/// Extension, not the OS-reported type: DA's presign compares the declared
/// content type against its own extension map, so the extension is the only
/// thing that can produce an agreeing pair.
pub fn classify(path: &Path) -> Result<DatasetFormat, DatasetErrorCode> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => Ok(DatasetFormat::Csv),
        Some("xlsx") => Ok(DatasetFormat::Xlsx),
        _ => Err(DatasetErrorCode::UnsupportedFormat),
    }
}

/// Whether the file's content agrees with the format its extension claims.
///
/// A `.csv` has no signature to check: any non-empty byte sequence is a
/// candidate, and refusing one for its content would refuse the exports that
/// begin with a byte-order mark or a comment line.
pub fn verify_content(
    path: &Path,
    format: DatasetFormat,
    size: u64,
) -> Result<(), DatasetErrorCode> {
    if size == 0 {
        return Err(DatasetErrorCode::EmptyFile);
    }
    match format {
        DatasetFormat::Csv => Ok(()),
        DatasetFormat::Xlsx => verify_workbook(path),
    }
}

fn verify_workbook(path: &Path) -> Result<(), DatasetErrorCode> {
    let mut file = File::open(path).map_err(|_| DatasetErrorCode::Unreadable)?;
    let mut head = [0_u8; SNIFF_BYTES];
    let read = read_up_to(&mut file, &mut head).map_err(|_| DatasetErrorCode::Unreadable)?;
    let head = &head[..read];

    if ZIP_SIGNATURES
        .iter()
        .any(|signature| head.starts_with(signature))
    {
        // A zip prefix is not proof of a workbook: every `.zip`, and any
        // polyglot crafted to look like one, starts the same way.
        return match workbook_entries_present(&mut file) {
            WorkbookStructure::Confirmed => Ok(()),
            WorkbookStructure::NotAWorkbook => Err(DatasetErrorCode::ContentMismatch),
            // The archive could not be read as one here. DA re-validates the
            // OOXML structure on ingest, so deferring is not a hole — and
            // refusing a workbook this reader merely does not understand
            // would block a file that works.
            WorkbookStructure::Undetermined => Ok(()),
        };
    }

    if head.starts_with(CFBF_SIGNATURE) && looks_encrypted(&mut file) {
        return Err(DatasetErrorCode::EncryptedFile);
    }

    Err(DatasetErrorCode::ContentMismatch)
}

enum WorkbookStructure {
    Confirmed,
    NotAWorkbook,
    Undetermined,
}

/// Reads the archive's central directory and looks for the two parts every
/// OOXML workbook carries.
///
/// No member is ever inflated — the entry names answer the question, which
/// also means a zip bomb expands nothing here.
fn workbook_entries_present(file: &mut File) -> WorkbookStructure {
    if file.seek(SeekFrom::Start(0)).is_err() {
        return WorkbookStructure::Undetermined;
    }
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return WorkbookStructure::Undetermined;
    };
    let mut has_content_types = false;
    let mut has_workbook_part = false;
    for index in 0..archive.len().min(MAX_ZIP_ENTRIES_SCANNED) {
        let Ok(entry) = archive.by_index_raw(index) else {
            return WorkbookStructure::Undetermined;
        };
        let Ok(name) = entry.name() else {
            return WorkbookStructure::Undetermined;
        };
        if name == "[Content_Types].xml" {
            has_content_types = true;
        } else if name.starts_with("xl/") {
            has_workbook_part = true;
        }
        if has_content_types && has_workbook_part {
            return WorkbookStructure::Confirmed;
        }
    }
    if archive.len() > MAX_ZIP_ENTRIES_SCANNED {
        return WorkbookStructure::Undetermined;
    }
    WorkbookStructure::NotAWorkbook
}

/// Whether a compound document carries the stream a password-protected
/// workbook stores its contents in.
///
/// Finding the name confirms encryption. Not finding it means the file is a
/// compound document that is not an encrypted workbook, which is reported as a
/// content mismatch — the honest answer, since nothing here can read it either
/// way.
fn looks_encrypted(file: &mut File) -> bool {
    if file.seek(SeekFrom::Start(0)).is_err() {
        return false;
    }
    let mut head = Vec::new();
    if file
        .take(ENCRYPTION_SCAN_BYTES)
        .read_to_end(&mut head)
        .is_err()
    {
        return false;
    }
    let marker = encrypted_package_marker();
    head.windows(marker.len()).any(|window| window == marker)
}

/// Fills as much of `buffer` as the file has, returning how many bytes that
/// was. A file shorter than the sniff window is not an error — it is a file
/// whose signature cannot match.
fn read_up_to(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            count => filled += count,
        }
    }
    Ok(filled)
}
