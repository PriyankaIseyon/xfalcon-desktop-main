//! Bounded, text-only extraction for Office files.
//!
//! This module does not execute macros, resolve external relationships, or
//! perform OCR. It uses the ZIP/XML package model for OOXML and a separate
//! native reader for legacy OLE documents. The 32 MiB source bound and the
//! expanded-package limits below apply before content reaches callers.
//!
//! References: ECMA-376 / ISO 29500 (OOXML package, relationships, WordprocessingML
//! and PresentationML); MS-DOC (Word binary piece tables); MS-PPT (PowerPoint
//! persist object directory); MS-CFB (OLE compound files). The crate review found
//! that Kreuzberg 4.10.4 scans the whole legacy PPT stream and office_oxide 0.1.15
//! can fall back to stream-wide PPT scanning when the current persist graph is
//! unavailable. Neither is used here because that fallback can expose deleted
//! slide text.

use std::path::Path;

mod legacy;
mod ooxml;

pub(crate) const MAX_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_DOCUMENT_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_DOCUMENT_ENTRIES: usize = 4096;
pub(crate) const MAX_PART_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_XML_DEPTH: usize = 128;
pub(crate) const MAX_XML_EVENTS: usize = 1_000_000;

const LIMITATIONS_HEADER: &str = "[Office text extraction: text and table contents only. Images, layout, and visual styling are not interpreted; images are not OCR-scanned.]\n\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentError {
    Empty,
    InputTooLarge,
    ExpandedTooLarge,
    TextTooLarge,
    Invalid,
    MismatchedFormat,
    Encrypted,
    Unsupported,
    NoText,
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Empty => "The Office file is empty.",
            Self::InputTooLarge => "The Office file exceeds the 32 MiB input limit.",
            Self::ExpandedTooLarge => "The Office file exceeds the expanded-content limit.",
            Self::TextTooLarge => "The extracted Office text exceeds the 4 MiB limit.",
            Self::Invalid => "The Office file is corrupt or malformed.",
            Self::MismatchedFormat => "The file contents do not match the Office extension.",
            Self::Encrypted => "Password-protected Office files are not supported.",
            Self::Unsupported => "This Office file uses an unsupported structure.",
            Self::NoText => "The Office file contains no extractable text.",
        };
        f.write_str(message)
    }
}

impl std::error::Error for DocumentError {}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExtractedDocument {
    pub text: String,
}

impl ExtractedDocument {
    /// Returns source text and the number of generated header lines that must
    /// be added to body-relative search results to match `text` line numbers.
    pub(crate) fn search_body(&self) -> (&str, u32) {
        let body = self
            .text
            .strip_prefix(LIMITATIONS_HEADER)
            .expect("extracted Office text starts with its limitations header");
        let header_lines = LIMITATIONS_HEADER
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u32;
        (body, header_lines)
    }
}

pub(crate) fn is_office_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "doc" | "docx" | "ppt" | "pptx"
            )
        })
}

pub(crate) fn extract(path: &Path, bytes: &[u8]) -> Result<ExtractedDocument, DocumentError> {
    if bytes.is_empty() {
        return Err(DocumentError::Empty);
    }
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::InputTooLarge);
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(DocumentError::MismatchedFormat)?;
    if !matches!(extension.as_str(), "doc" | "docx" | "ppt" | "pptx") {
        return Err(DocumentError::MismatchedFormat);
    }

    let max_body_bytes = MAX_DOCUMENT_TEXT_BYTES.saturating_sub(LIMITATIONS_HEADER.len());
    let body = match extension.as_str() {
        "docx" | "pptx" => ooxml::extract(&extension, bytes, max_body_bytes)?,
        "doc" | "ppt" => legacy::extract_legacy(&extension, bytes, max_body_bytes)?,
        _ => unreachable!("the extension was checked above"),
    };
    if body.trim().is_empty() {
        return Err(DocumentError::NoText);
    }
    let mut text = String::with_capacity(LIMITATIONS_HEADER.len() + body.len());
    text.push_str(LIMITATIONS_HEADER);
    text.push_str(&body);
    if text.len() > MAX_DOCUMENT_TEXT_BYTES {
        return Err(DocumentError::TextTooLarge);
    }
    Ok(ExtractedDocument { text })
}

#[cfg(test)]
pub(crate) fn synthetic_blank_legacy_ppt_fixture() -> Vec<u8> {
    legacy::synthetic_blank_ppt_fixture()
}
