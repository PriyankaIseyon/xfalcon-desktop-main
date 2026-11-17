//! Composer-attachment MIME and kind, kept in lockstep with xfalcon-web and DA.
//!
//! DA's `/presign` enforces `content_type == _EXT_TO_MIME[ext]`. Guessing a
//! MIME from the OS-reported type would 415. Extension is the only source.

/// Largest composer attachment. Mirrors DA's `size_bytes` check and
/// xfalcon-web's `MAX_FILE_SIZE`.
pub const MAX_CHAT_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// Decoded image work stays bounded even when compressed input is tiny.
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_XLSX_ENTRIES: usize = 2_048;
const MAX_XLSX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;

/// Attachment family DA's `chat_attachments.kind` check accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentKind {
    Image,
    Tabular,
    Text,
}

impl AttachmentKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Tabular => "tabular",
            Self::Text => "text",
        }
    }
}

struct Spec {
    ext: &'static str,
    mime: &'static str,
    kind: AttachmentKind,
}

const SPECS: &[Spec] = &[
    Spec {
        ext: "png",
        mime: "image/png",
        kind: AttachmentKind::Image,
    },
    Spec {
        ext: "jpg",
        mime: "image/jpeg",
        kind: AttachmentKind::Image,
    },
    Spec {
        ext: "jpeg",
        mime: "image/jpeg",
        kind: AttachmentKind::Image,
    },
    Spec {
        ext: "gif",
        mime: "image/gif",
        kind: AttachmentKind::Image,
    },
    Spec {
        ext: "webp",
        mime: "image/webp",
        kind: AttachmentKind::Image,
    },
    Spec {
        ext: "csv",
        mime: "text/csv",
        kind: AttachmentKind::Tabular,
    },
    Spec {
        ext: "tsv",
        mime: "text/tab-separated-values",
        kind: AttachmentKind::Tabular,
    },
    Spec {
        ext: "xlsx",
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        kind: AttachmentKind::Tabular,
    },
    Spec {
        ext: "xls",
        mime: "application/vnd.ms-excel",
        kind: AttachmentKind::Tabular,
    },
    Spec {
        ext: "doc",
        mime: "application/msword",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "docx",
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "ppt",
        mime: "application/vnd.ms-powerpoint",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "pptx",
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "json",
        mime: "application/json",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "txt",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "md",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "markdown",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "sql",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "py",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "ts",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "tsx",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "js",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "jsx",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "rs",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "go",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "java",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "yml",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "yaml",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "toml",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "xml",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "html",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "css",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "sh",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "r",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
    Spec {
        ext: "log",
        mime: "text/plain",
        kind: AttachmentKind::Text,
    },
];

/// Canonical MIME and kind for a composer file, or `None` when DA will refuse it.
#[must_use]
pub fn classify_chat_attachment(path: &std::path::Path) -> Option<(&'static str, AttachmentKind)> {
    let ext = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)?;
    SPECS
        .iter()
        .find(|spec| spec.ext == ext)
        .map(|spec| (spec.mime, spec.kind))
}

/// Verifies that bytes agree with the allowlisted filename family.
///
/// This is intentionally a narrow signature/encoding check. DA still parses
/// the attachment as the authority; this boundary prevents a renamed binary
/// from being uploaded under an image, workbook, or text content type.
pub fn verify_chat_attachment_content(
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), ChatAttachmentContentError> {
    if bytes.is_empty() {
        return Err(ChatAttachmentContentError::Empty);
    }
    let Some((_, kind)) = classify_chat_attachment(path) else {
        return Err(ChatAttachmentContentError::UnsupportedType);
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(ChatAttachmentContentError::UnsupportedType)?;

    if crate::documents::is_office_path(path) {
        return match crate::documents::extract(path, bytes) {
            Ok(_) | Err(crate::documents::DocumentError::NoText) => Ok(()),
            Err(error) => Err(ChatAttachmentContentError::OfficeExtraction(
                error.to_string(),
            )),
        };
    }

    let valid = match extension.as_str() {
        "png" => png_dimensions(bytes).is_some_and(valid_image_dimensions),
        "jpg" | "jpeg" => jpeg_dimensions(bytes).is_some_and(valid_image_dimensions),
        "gif" => gif_dimensions(bytes).is_some_and(valid_image_dimensions),
        "webp" => webp_dimensions(bytes).is_some_and(valid_image_dimensions),
        "xls" => bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]),
        "xlsx" => is_xlsx(bytes),
        "json" => serde_json::from_slice::<serde_json::Value>(bytes).is_ok(),
        _ if matches!(kind, AttachmentKind::Text | AttachmentKind::Tabular) => {
            !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
        }
        _ => false,
    };
    valid
        .then_some(())
        .ok_or(ChatAttachmentContentError::ContentMismatch)
}

fn is_xlsx(bytes: &[u8]) -> bool {
    is_xlsx_with_limits(bytes, MAX_XLSX_ENTRIES, MAX_XLSX_EXPANDED_BYTES)
}

fn is_xlsx_with_limits(bytes: &[u8], max_entries: usize, max_expanded_bytes: u64) -> bool {
    let reader = std::io::Cursor::new(bytes);
    let Ok(mut archive) = zip::ZipArchive::new(reader) else {
        return false;
    };
    if archive.len() > max_entries {
        return false;
    }
    let mut expanded = 0u64;
    let mut has_content_types = false;
    let mut has_workbook = false;
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            return false;
        };
        if entry.encrypted() || entry.enclosed_name().is_none() {
            return false;
        }
        let Ok(name) = entry.name() else {
            return false;
        };
        if name.eq_ignore_ascii_case("vbaProject.bin")
            || name.to_ascii_lowercase().ends_with("/vbaproject.bin")
        {
            return false;
        }
        has_content_types |= name == "[Content_Types].xml";
        has_workbook |= name == "xl/workbook.xml";
        let Some(total) = expanded.checked_add(entry.size()) else {
            return false;
        };
        if total > max_expanded_bytes {
            return false;
        }
        expanded = total;
    }
    has_content_types && has_workbook
}

fn valid_image_dimensions((width, height): (u32, u32)) -> bool {
    width > 0
        && height > 0
        && width <= MAX_IMAGE_DIMENSION
        && height <= MAX_IMAGE_DIMENSION
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    Some((
        u16::from_le_bytes(bytes[6..8].try_into().ok()?).into(),
        u16::from_le_bytes(bytes[8..10].try_into().ok()?).into(),
    ))
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut cursor = 2usize;
    while cursor + 4 <= bytes.len() {
        while cursor < bytes.len() && bytes[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *bytes.get(cursor)?;
        cursor += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let length = u16::from_be_bytes(bytes.get(cursor..cursor + 2)?.try_into().ok()?) as usize;
        if length < 2 || cursor.checked_add(length)? > bytes.len() {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 7 {
                return None;
            }
            let height = u16::from_be_bytes(bytes[cursor + 3..cursor + 5].try_into().ok()?);
            let width = u16::from_be_bytes(bytes[cursor + 5..cursor + 7].try_into().ok()?);
            return Some((width.into(), height.into()));
        }
        cursor += length;
    }
    None
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 20 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" if bytes.len() >= 30 => Some((
            1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]),
            1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]),
        )),
        b"VP8 " if bytes.len() >= 30 && bytes[23..26] == [0x9d, 0x01, 0x2a] => Some((
            u32::from(u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3fff),
            u32::from(u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3fff),
        )),
        b"VP8L" if bytes.len() >= 25 && bytes[20] == 0x2f => {
            let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ChatAttachmentContentError {
    #[error("that file is empty")]
    Empty,
    #[error("that file type cannot be attached to a chat message")]
    UnsupportedType,
    #[error("that file's contents do not match its filename")]
    ContentMismatch,
    #[error("{0}")]
    OfficeExtraction(String),
}

#[cfg(test)]
mod tests {
    use super::{
        classify_chat_attachment, is_xlsx_with_limits, verify_chat_attachment_content,
        AttachmentKind, ChatAttachmentContentError,
    };
    use base64::Engine as _;
    use std::path::Path;

    fn xlsx(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write as _;
        use zip::write::SimpleFileOptions;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("xlsx entry");
            writer.write_all(content).expect("xlsx content");
        }
        writer.finish().expect("xlsx archive").into_inner()
    }

    fn replace_zip_part(bytes: &[u8], name: &str, replacement: &[u8]) -> Vec<u8> {
        use std::io::{Read as _, Write as _};
        use zip::write::SimpleFileOptions;

        let mut source = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("Office package");
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for index in 0..source.len() {
            let mut entry = source.by_index(index).expect("package entry");
            let entry_name = entry
                .name()
                .expect("fixture entry name is valid")
                .into_owned();
            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .expect("read package entry");
            writer
                .start_file(&entry_name, SimpleFileOptions::default())
                .expect("copy entry");
            writer
                .write_all(if entry_name == name {
                    replacement
                } else {
                    &contents
                })
                .expect("write package entry");
        }
        writer.finish().expect("finish Office package").into_inner()
    }

    #[test]
    fn an_allowed_image_is_classified_from_its_extension() {
        let (mime, kind) = classify_chat_attachment(Path::new("chart.PNG")).expect("png");
        assert_eq!(mime, "image/png");
        assert_eq!(kind, AttachmentKind::Image);
    }

    #[test]
    fn an_unknown_extension_is_refused() {
        assert!(classify_chat_attachment(Path::new("payload.exe")).is_none());
    }

    #[test]
    fn image_content_must_match_its_extension() {
        assert_eq!(
            verify_chat_attachment_content(Path::new("chart.png"), b"not a png"),
            Err(ChatAttachmentContentError::ContentMismatch)
        );
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
            .expect("valid png fixture");
        assert!(verify_chat_attachment_content(Path::new("chart.png"), &png).is_ok());
    }

    #[test]
    fn text_must_be_nonempty_utf8_without_nul_bytes() {
        assert_eq!(
            verify_chat_attachment_content(Path::new("notes.txt"), b""),
            Err(ChatAttachmentContentError::Empty)
        );
        assert_eq!(
            verify_chat_attachment_content(Path::new("notes.txt"), b"hello\0world"),
            Err(ChatAttachmentContentError::ContentMismatch)
        );
        assert!(verify_chat_attachment_content(Path::new("notes.txt"), b"hello").is_ok());
    }

    #[test]
    fn office_attachments_use_canonical_mime_and_extract_case_insensitively() {
        let files: [(&str, &str, &str, &[u8]); 4] = [
            (
                "report.DOC",
                "application/msword",
                "This is a simple file",
                include_bytes!("../../tests/fixtures/office/legacy-poi/simple.doc"),
            ),
            (
                "report.DOCX",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "Unicode café",
                include_bytes!("../../tests/fixtures/office/producer.docx"),
            ),
            (
                "slides.PPT",
                "application/vnd.ms-powerpoint",
                "Slide 1",
                include_bytes!("../../tests/fixtures/office/legacy-poi/text_shapes.ppt"),
            ),
            (
                "slides.PPTX",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "Slide relationship order first",
                include_bytes!("../../tests/fixtures/office/producer.pptx"),
            ),
        ];

        for (name, expected_mime, expected_text, bytes) in files {
            let (mime, kind) = classify_chat_attachment(Path::new(name)).expect("Office type");
            assert_eq!(mime, expected_mime, "{name}");
            assert_eq!(kind, AttachmentKind::Text, "{name}");
            verify_chat_attachment_content(Path::new(name), bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let text = crate::documents::extract(Path::new(name), bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(text.text.contains(expected_text), "{name}: {}", text.text);
        }
    }

    #[test]
    fn office_attachment_validation_reports_mismatched_corrupt_and_encrypted_files() {
        let pptx = include_bytes!("../../tests/fixtures/office/producer.pptx");
        assert!(matches!(
            verify_chat_attachment_content(Path::new("wrong.docx"), pptx),
            Err(ChatAttachmentContentError::OfficeExtraction(message))
                if message.contains("do not match the Office extension")
        ));
        assert!(matches!(
            verify_chat_attachment_content(Path::new("broken.docx"), b"not a package"),
            Err(ChatAttachmentContentError::OfficeExtraction(message))
                if message.contains("corrupt or malformed")
        ));

        use std::io::Write as _;
        let mut compound = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new()))
            .expect("encrypted package container");
        compound
            .create_stream("/EncryptionInfo")
            .expect("encryption metadata")
            .write_all(b"info")
            .expect("write encryption metadata");
        compound
            .create_stream("/EncryptedPackage")
            .expect("encrypted body")
            .write_all(b"package")
            .expect("write encrypted body");
        let encrypted = compound.into_inner().into_inner();
        assert!(matches!(
            verify_chat_attachment_content(Path::new("encrypted.docx"), &encrypted),
            Err(ChatAttachmentContentError::OfficeExtraction(message))
                if message.contains("Password-protected")
        ));
    }

    #[test]
    fn office_admission_rejects_namespace_and_content_type_spoofing() {
        let docx = include_bytes!("../../tests/fixtures/office/producer.docx");
        let pptx = include_bytes!("../../tests/fixtures/office/producer.pptx");
        let doc_xml = b"<w:document xmlns:w=\"urn:attacker:wordprocessingml\"><w:body><w:p><w:r><w:t>Injected</w:t></w:r></w:p></w:body></w:document>";
        let spoofed_docx = replace_zip_part(docx, "word/document.xml", doc_xml);
        assert!(matches!(
            verify_chat_attachment_content(Path::new("spoof.docx"), &spoofed_docx),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        let wrong_known_namespace = b"<a:document xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:body><a:p><a:r><a:t>Injected</a:t></a:r></a:p></a:body></a:document>";
        let wrong_namespace_docx =
            replace_zip_part(docx, "word/document.xml", wrong_known_namespace);
        assert!(matches!(
            verify_chat_attachment_content(Path::new("wrong-known.docx"), &wrong_namespace_docx),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        let undeclared_prefix =
            b"<w:document><w:body><w:p><w:r><w:t>Injected</w:t></w:r></w:p></w:body></w:document>";
        let undeclared_docx = replace_zip_part(docx, "word/document.xml", undeclared_prefix);
        assert!(matches!(
            verify_chat_attachment_content(Path::new("undeclared.docx"), &undeclared_docx),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        let shadowed_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>Visible</w:t></w:r></w:p><w:p xmlns:w=\"urn:attacker:shadow\"><w:r><w:t>Injected</w:t></w:r></w:p></w:body></w:document>";
        let shadowed_docx = replace_zip_part(docx, "word/document.xml", shadowed_xml);
        assert!(verify_chat_attachment_content(Path::new("shadowed.docx"), &shadowed_docx).is_ok());

        let types = b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.attacker-wordprocessingml.document.main+xml\"/></Types>";
        let spoofed_type = replace_zip_part(docx, "[Content_Types].xml", types);
        assert!(matches!(
            verify_chat_attachment_content(Path::new("spoof.docx"), &spoofed_type),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        let wrong_content_namespace = b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><x:Override xmlns:x=\"urn:attacker:content-types\" PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>";
        let wrong_content_namespace_docx =
            replace_zip_part(docx, "[Content_Types].xml", wrong_content_namespace);
        assert!(matches!(
            verify_chat_attachment_content(
                Path::new("wrong-content-namespace.docx"),
                &wrong_content_namespace_docx
            ),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        let ppt_xml = b"<p:presentation xmlns:p=\"urn:attacker:presentationml\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"slide1\"/></p:sldIdLst></p:presentation>";
        let spoofed_pptx = replace_zip_part(pptx, "ppt/presentation.xml", ppt_xml);
        assert!(matches!(
            verify_chat_attachment_content(Path::new("spoof.pptx"), &spoofed_pptx),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        let wrong_relationships = b"<Relationships xmlns=\"urn:attacker:package-relationships\"><Relationship Id=\"main\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
        let spoofed_relationships = replace_zip_part(docx, "_rels/.rels", wrong_relationships);
        assert!(matches!(
            verify_chat_attachment_content(Path::new("spoof.docx"), &spoofed_relationships),
            Err(ChatAttachmentContentError::OfficeExtraction(_))
        ));

        assert!(verify_chat_attachment_content(Path::new("valid.docx"), docx).is_ok());
        assert!(verify_chat_attachment_content(Path::new("valid.pptx"), pptx).is_ok());
    }

    #[test]
    fn truncated_image_headers_are_refused_without_panicking() {
        let samples: [(&str, &[u8]); 4] = [
            (
                "image.png",
                b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01",
            ),
            ("image.gif", b"GIF89a\x01\0\x01\0"),
            ("image.jpg", b"\xff\xd8\xff\xc0\0\x07\x08\0\x01\0\x01"),
            (
                "image.webp",
                b"RIFF\x16\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0\0\0\0\0\0\0",
            ),
        ];
        for (name, bytes) in samples {
            for length in 0..bytes.len() {
                let _ = verify_chat_attachment_content(Path::new(name), &bytes[..length]);
            }
        }
    }

    #[test]
    fn oversized_image_dimensions_are_refused() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&20_000u32.to_be_bytes());
        png.extend_from_slice(&20_000u32.to_be_bytes());
        assert_eq!(
            verify_chat_attachment_content(Path::new("large.png"), &png),
            Err(ChatAttachmentContentError::ContentMismatch)
        );
    }

    #[test]
    fn xlsx_packages_reject_macros_unsafe_paths_and_expansion() {
        let base = [
            ("[Content_Types].xml", b"types".as_slice()),
            ("xl/workbook.xml", b"book".as_slice()),
        ];
        assert!(is_xlsx_with_limits(&xlsx(&base), 8, 64));

        let macros = xlsx(&[base[0], base[1], ("xl/vbaProject.bin", b"macro".as_slice())]);
        assert!(!is_xlsx_with_limits(&macros, 8, 64));

        let traversal = xlsx(&[base[0], base[1], ("../escape.xml", b"x".as_slice())]);
        assert!(!is_xlsx_with_limits(&traversal, 8, 64));

        assert!(!is_xlsx_with_limits(&xlsx(&base), 8, 7));
    }
}
