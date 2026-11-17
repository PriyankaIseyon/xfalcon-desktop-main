use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use zip::ZipArchive;

use super::{
    DocumentError, MAX_DOCUMENT_ENTRIES, MAX_EXPANDED_BYTES, MAX_PART_BYTES, MAX_XML_DEPTH,
    MAX_XML_EVENTS,
};

type Parts<'a> = ArchiveParts<'a>;

struct ArchiveParts<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    by_name: HashMap<String, usize>,
    cache: HashMap<String, Vec<u8>>,
    expanded: u64,
    processed: u64,
    output_limit: usize,
}

impl<'a> ArchiveParts<'a> {
    fn new(bytes: &'a [u8], output_limit: usize) -> Result<Self, DocumentError> {
        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).map_err(|_| DocumentError::Invalid)?;
        if archive.len() > MAX_DOCUMENT_ENTRIES {
            return Err(DocumentError::ExpandedTooLarge);
        }
        let mut by_name = HashMap::with_capacity(archive.len());
        let mut declared_total = 0u64;
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|_| DocumentError::Invalid)?;
            if file.encrypted() {
                return Err(DocumentError::Encrypted);
            }
            let raw_name = file.name().map_err(|_| DocumentError::Invalid)?;
            let enclosed = file.enclosed_name().ok_or(DocumentError::Invalid)?;
            let normalized = normalize_entry_name(&enclosed).ok_or(DocumentError::Invalid)?;
            if raw_name.contains('\\') || normalized.is_empty() {
                return Err(DocumentError::Invalid);
            }
            if by_name.insert(normalized, index).is_some() {
                return Err(DocumentError::Invalid);
            }
            declared_total = declared_total
                .checked_add(file.size())
                .ok_or(DocumentError::ExpandedTooLarge)?;
            if file.size() > MAX_PART_BYTES {
                return Err(DocumentError::ExpandedTooLarge);
            }
            if declared_total > MAX_EXPANDED_BYTES {
                return Err(DocumentError::ExpandedTooLarge);
            }
        }
        Ok(Self {
            archive,
            by_name,
            cache: HashMap::new(),
            expanded: 0,
            processed: 0,
            output_limit,
        })
    }

    fn part(&mut self, name: &str) -> Result<&[u8], DocumentError> {
        let key = normalize_part_name(name)?;
        if !self.cache.contains_key(&key) {
            let index = *self.by_name.get(&key).ok_or(DocumentError::Invalid)?;
            let mut file = self
                .archive
                .by_index(index)
                .map_err(|_| DocumentError::Invalid)?;
            if file.encrypted() {
                return Err(DocumentError::Encrypted);
            }
            let declared = file.size();
            if declared > MAX_PART_BYTES {
                return Err(DocumentError::ExpandedTooLarge);
            }
            let remaining = MAX_EXPANDED_BYTES.saturating_sub(self.expanded);
            if declared > remaining {
                return Err(DocumentError::ExpandedTooLarge);
            }
            let capacity = usize::try_from(declared).unwrap_or(usize::MAX).min(1 << 20);
            let mut bytes = Vec::with_capacity(capacity);
            let mut limited = (&mut file).take(MAX_PART_BYTES + 1);
            limited
                .read_to_end(&mut bytes)
                .map_err(|_| DocumentError::Invalid)?;
            if bytes.len() as u64 > MAX_PART_BYTES {
                return Err(DocumentError::ExpandedTooLarge);
            }
            self.expanded = self
                .expanded
                .checked_add(bytes.len() as u64)
                .ok_or(DocumentError::ExpandedTooLarge)?;
            if self.expanded > MAX_EXPANDED_BYTES {
                return Err(DocumentError::ExpandedTooLarge);
            }
            self.cache.insert(key.clone(), bytes);
        }
        let part_size = self.cache.get(&key).expect("part is cached").len() as u64;
        self.processed = self
            .processed
            .checked_add(part_size)
            .ok_or(DocumentError::ExpandedTooLarge)?;
        if self.processed > MAX_EXPANDED_BYTES {
            return Err(DocumentError::ExpandedTooLarge);
        }
        Ok(self.cache.get(&key).expect("inserted above").as_slice())
    }

    fn contains(&self, name: &str) -> Result<bool, DocumentError> {
        Ok(self.by_name.contains_key(&normalize_part_name(name)?))
    }

    fn text_limit(&self) -> usize {
        self.output_limit
    }
}

fn normalize_entry_name(path: &Path) -> Option<String> {
    let mut pieces = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => {
                pieces.push(value.to_str()?.to_ascii_lowercase())
            }
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(pieces.join("/"))
}

fn normalize_part_name(name: &str) -> Result<String, DocumentError> {
    if name.is_empty() || name.contains('\\') || name.starts_with('/') || name.contains('\0') {
        return Err(DocumentError::Invalid);
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(DocumentError::Invalid);
                }
            }
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err(DocumentError::Invalid);
    }
    Ok(parts.join("/").to_ascii_lowercase())
}

#[derive(Debug, Clone)]
struct Relationship {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

pub(super) fn extract(
    extension: &str,
    bytes: &[u8],
    output_limit: usize,
) -> Result<String, DocumentError> {
    if is_cfb(bytes) {
        return Err(if is_encrypted_ooxml_cfb(bytes) {
            DocumentError::Encrypted
        } else {
            DocumentError::MismatchedFormat
        });
    }
    let mut parts = Parts::new(bytes, output_limit)?;
    let root_rels = parts.part("_rels/.rels")?;
    let root_relationships = parse_relationships(root_rels)?;
    let main_relationship = root_relationships
        .iter()
        .find(|relationship| {
            relationship_type_is(&relationship.kind, "officeDocument") && !relationship.external
        })
        .ok_or(DocumentError::MismatchedFormat)?;
    let main_part = resolve_target("", &main_relationship.target)?;
    let expected_type = match extension {
        "docx" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        }
        "pptx" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"
        }
        _ => return Err(DocumentError::MismatchedFormat),
    };
    if !content_type_matches(
        parts.part("[Content_Types].xml")?,
        &main_part,
        expected_type,
    )? {
        return Err(DocumentError::MismatchedFormat);
    }
    let relationships_path = relationships_path(&main_part);
    let document_rels = if parts.contains(&relationships_path)? {
        parse_relationships(parts.part(&relationships_path)?)?
    } else {
        Vec::new()
    };
    let mut output = TextSink::new(parts.text_limit());
    match extension {
        "docx" => extract_docx(&mut parts, &main_part, &document_rels, &mut output)?,
        "pptx" => extract_pptx(&mut parts, &main_part, &document_rels, &mut output)?,
        _ => return Err(DocumentError::MismatchedFormat),
    }
    output.finish()
}

fn is_cfb(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

fn is_encrypted_ooxml_cfb(bytes: &[u8]) -> bool {
    let Ok(file) = cfb::CompoundFile::open(Cursor::new(bytes)) else {
        return false;
    };
    file.exists("EncryptionInfo") && file.exists("EncryptedPackage")
}

fn content_type_matches(xml: &[u8], part: &str, expected: &str) -> Result<bool, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let normalized = normalize_namespaces(&selected, XmlContext::ContentTypes)?;
    let mut reader = xml_reader(&normalized)?;
    let mut buffer = Vec::new();
    let mut found = false;
    let mut events = 0;
    let mut depth = 0;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) | Event::Empty(element)
                if element_is(element.name().as_ref(), b"ct", b"Override") =>
            {
                let part_name = attribute(&element, b"PartName")?;
                let content_type = attribute(&element, b"ContentType")?;
                if part_name
                    .as_deref()
                    .is_some_and(|value| value.trim_start_matches('/').eq_ignore_ascii_case(part))
                {
                    found = content_type.as_deref() == Some(expected);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(found)
}

fn relationships_path(part: &str) -> String {
    let (directory, file) = part.rsplit_once('/').unwrap_or(("", part));
    if directory.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{directory}/_rels/{file}.rels")
    }
}

fn relationship_type_is(value: &str, name: &str) -> bool {
    [
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
        "http://purl.oclc.org/ooxml/officeDocument/relationships/",
    ]
    .iter()
    .any(|prefix| value == format!("{prefix}{name}"))
}

fn resolve_target(source: &str, target: &str) -> Result<String, DocumentError> {
    if target.is_empty()
        || target.contains('\\')
        || target.contains('\0')
        || target.contains('?')
        || target.contains('#')
    {
        return Err(DocumentError::Unsupported);
    }
    let absolute = target.starts_with('/');
    let mut path = if absolute {
        Vec::new()
    } else {
        source
            .rsplit_once('/')
            .map_or_else(Vec::new, |(directory, _)| {
                directory
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .collect()
            })
    };
    for component in target.trim_start_matches('/').split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if path.pop().is_none() {
                    return Err(DocumentError::Invalid);
                }
            }
            value => path.push(value),
        }
    }
    if path.is_empty() {
        return Err(DocumentError::Invalid);
    }
    Ok(path.join("/"))
}

fn parse_relationships(xml: &[u8]) -> Result<Vec<Relationship>, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let normalized = normalize_namespaces(&selected, XmlContext::Relationships)?;
    let mut reader = xml_reader(&normalized)?;
    let mut buffer = Vec::new();
    let mut relationships = Vec::new();
    let mut events = 0;
    let mut depth = 0;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) | Event::Empty(element)
                if element_is(element.name().as_ref(), b"package", b"Relationship") =>
            {
                let Some(id) = attribute(&element, b"Id")? else {
                    return Err(DocumentError::Invalid);
                };
                let Some(kind) = attribute(&element, b"Type")? else {
                    return Err(DocumentError::Invalid);
                };
                let Some(target) = attribute(&element, b"Target")? else {
                    return Err(DocumentError::Invalid);
                };
                relationships.push(Relationship {
                    id,
                    kind,
                    target,
                    external: attribute(&element, b"TargetMode")?
                        .is_some_and(|mode| mode.eq_ignore_ascii_case("external")),
                });
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    let mut ids = HashSet::with_capacity(relationships.len());
    if relationships
        .iter()
        .any(|relationship| !ids.insert(relationship.id.as_str()))
    {
        return Err(DocumentError::Invalid);
    }
    Ok(relationships)
}

fn extract_docx(
    parts: &mut Parts<'_>,
    main_part: &str,
    relationships: &[Relationship],
    output: &mut TextSink,
) -> Result<(), DocumentError> {
    let document_xml = parts.part(main_part)?;
    let main = parse_docx_document(document_xml, output.limit())?;
    output.push_block(&main.text)?;

    let by_id: HashMap<&str, &Relationship> = relationships
        .iter()
        .map(|rel| (rel.id.as_str(), rel))
        .collect();
    let mut referenced_header_parts = HashSet::new();
    for (id, relation_kind) in main.header_footer_refs {
        let relation = by_id.get(id.as_str()).ok_or(DocumentError::Invalid)?;
        if relation.external
            || !relationship_type_is(&relation.kind, relation_kind.trim_start_matches('/'))
        {
            return Err(DocumentError::Unsupported);
        }
        let target = resolve_target(main_part, &relation.target)?;
        if !referenced_header_parts.insert(target.clone()) {
            continue;
        }
        let text = xml_to_text(parts.part(&target)?, output.limit(), XmlFlavor::Word)?;
        output.push_block(&text)?;
    }

    for (relation_kind, referenced_ids, note_element) in [
        ("/footnotes", main.footnote_refs, "footnote"),
        ("/endnotes", main.endnote_refs, "endnote"),
    ] {
        if referenced_ids.is_empty() {
            continue;
        }
        let relation = relationships
            .iter()
            .find(|rel| relationship_type_is(&rel.kind, relation_kind.trim_start_matches('/')))
            .ok_or(DocumentError::Invalid)?;
        if relation.external {
            return Err(DocumentError::Unsupported);
        }
        let target = resolve_target(main_part, &relation.target)?;
        let xml = parts.part(&target)?;
        let text = referenced_note_text(xml, &referenced_ids, note_element, output.limit())?;
        output.push_block(&text)?;
    }
    Ok(())
}

struct DocxMain {
    text: String,
    header_footer_refs: Vec<(String, &'static str)>,
    footnote_refs: HashSet<String>,
    endnote_refs: HashSet<String>,
}

fn parse_docx_document(xml: &[u8], max: usize) -> Result<DocxMain, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let normalized = normalize_namespaces(&selected, XmlContext::Word)?;
    let mut reader = xml_reader(&normalized)?;
    let mut buffer = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut deletion_depth = 0usize;
    let mut collector = TextCollector::new(max);
    let mut header_footer_refs = Vec::new();
    let mut footnote_refs = HashSet::new();
    let mut endnote_refs = HashSet::new();
    let mut saw_root = false;
    let mut saw_body = false;
    let mut events = 0;
    let mut depth = 0;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) => {
                let name = element.name().as_ref().to_vec();
                if stack.is_empty() {
                    if saw_root || name.as_slice() != b"w__document" {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                if name.as_slice() == b"w__body" {
                    saw_body = true;
                }
                if matches!(name.as_slice(), b"w__del" | b"w__moveFrom") {
                    deletion_depth = deletion_depth.saturating_add(1);
                }
                if deletion_depth == 0 {
                    match name.as_slice() {
                        b"w__headerReference" => {
                            let id = required_reference_id(relationship_id(&element)?)?;
                            header_footer_refs.push((id, "/header"));
                        }
                        b"w__footerReference" => {
                            let id = required_reference_id(relationship_id(&element)?)?;
                            header_footer_refs.push((id, "/footer"));
                        }
                        b"w__footnoteReference" => {
                            let id =
                                required_reference_id(office_attribute(&element, b"word", b"id")?)?;
                            footnote_refs.insert(id);
                        }
                        b"w__endnoteReference" => {
                            let id =
                                required_reference_id(office_attribute(&element, b"word", b"id")?)?;
                            endnote_refs.insert(id);
                        }
                        b"w__altChunk" => return Err(DocumentError::Unsupported),
                        b"w__tab" => collector.separator('\t')?,
                        b"w__br" | b"w__cr" => collector.separator('\n')?,
                        b"w__p" | b"w__tc" | b"w__tr" | b"a__p" | b"a__tc" | b"a__tr" => {
                            collector.start(tag_local(&name))?
                        }
                        _ => {}
                    }
                }
                stack.push(name);
            }
            Event::Empty(element) => {
                let name = element.name().as_ref().to_vec();
                if stack.is_empty() {
                    if saw_root || name.as_slice() != b"w__document" {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                if name.as_slice() == b"w__body" {
                    saw_body = true;
                }
                if deletion_depth == 0 {
                    match name.as_slice() {
                        b"w__headerReference" => {
                            let id = required_reference_id(relationship_id(&element)?)?;
                            header_footer_refs.push((id, "/header"));
                        }
                        b"w__footerReference" => {
                            let id = required_reference_id(relationship_id(&element)?)?;
                            header_footer_refs.push((id, "/footer"));
                        }
                        b"w__footnoteReference" => {
                            let id =
                                required_reference_id(office_attribute(&element, b"word", b"id")?)?;
                            footnote_refs.insert(id);
                        }
                        b"w__endnoteReference" => {
                            let id =
                                required_reference_id(office_attribute(&element, b"word", b"id")?)?;
                            endnote_refs.insert(id);
                        }
                        b"w__altChunk" => return Err(DocumentError::Unsupported),
                        b"w__tab" => collector.separator('\t')?,
                        b"w__br" | b"w__cr" => collector.separator('\n')?,
                        _ => {}
                    }
                }
            }
            Event::Text(text)
                if deletion_depth == 0
                    && stack.last().is_some_and(|name| {
                        matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t")
                    }) =>
            {
                let decoded = text
                    .xml_content(quick_xml::XmlVersion::Explicit1_0)
                    .map_err(|_| DocumentError::Invalid)?;
                collector.text(&decoded)?;
            }
            Event::GeneralRef(reference)
                if deletion_depth == 0
                    && stack.last().is_some_and(|name| {
                        matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t")
                    }) =>
            {
                let reference = reference.decode().map_err(|_| DocumentError::Invalid)?;
                collector.text(&resolve_xml_reference(&reference)?)?;
            }
            Event::CData(text)
                if deletion_depth == 0
                    && stack.last().is_some_and(|name| {
                        matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t")
                    }) =>
            {
                let decoded = text.xml10_content().map_err(|_| DocumentError::Invalid)?;
                collector.text(&decoded)?;
            }
            Event::End(element) => {
                let name = element.name().as_ref().to_vec();
                if deletion_depth > 0 && matches!(name.as_slice(), b"w__del" | b"w__moveFrom") {
                    deletion_depth -= 1;
                }
                if deletion_depth == 0 && (name.starts_with(b"w__") || name.starts_with(b"a__")) {
                    collector.end(tag_local(&name))?;
                }
                stack.pop().ok_or(DocumentError::Invalid)?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !stack.is_empty() || !saw_root || !saw_body {
        return Err(DocumentError::Invalid);
    }
    Ok(DocxMain {
        text: collector.finish(),
        header_footer_refs,
        footnote_refs,
        endnote_refs,
    })
}

fn referenced_note_text(
    xml: &[u8],
    referenced_ids: &HashSet<String>,
    element_name: &str,
    max: usize,
) -> Result<String, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let normalized = normalize_namespaces(&selected, XmlContext::Word)?;
    let mut reader = xml_reader(&normalized)?;
    let mut deletion_depth = 0usize;
    let mut buffer = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut active = false;
    let mut found_ids = HashSet::new();
    let mut saw_root = false;
    let expected_root = match element_name {
        "footnote" => b"footnotes".as_slice(),
        "endnote" => b"endnotes".as_slice(),
        _ => return Err(DocumentError::Unsupported),
    };
    let mut collector = TextCollector::new(max);
    let mut events = 0;
    let mut depth = 0;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) => {
                let name = element.name().as_ref().to_vec();
                if stack.is_empty() {
                    let expected_root = if expected_root == b"footnotes" {
                        b"w__footnotes".as_slice()
                    } else {
                        b"w__endnotes".as_slice()
                    };
                    if saw_root || name.as_slice() != expected_root {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                if element_is(&name, b"w", element_name.as_bytes()) {
                    let id = office_attribute(&element, b"word", b"id")?.unwrap_or_default();
                    active = referenced_ids.contains(&id) && !matches!(id.as_str(), "-1" | "0");
                    if active {
                        if !found_ids.insert(id) {
                            return Err(DocumentError::Invalid);
                        }
                        collector.start(tag_local(&name))?;
                    }
                } else if active
                    && matches!(
                        name.as_slice(),
                        b"w__p" | b"w__tc" | b"w__tr" | b"a__p" | b"a__tc" | b"a__tr"
                    )
                {
                    collector.start(tag_local(&name))?;
                }
                if active && matches!(name.as_slice(), b"w__del" | b"w__moveFrom") {
                    deletion_depth += 1;
                }
                if active && deletion_depth == 0 && name.as_slice() == b"w__tab" {
                    collector.separator('\t')?;
                } else if active
                    && deletion_depth == 0
                    && matches!(name.as_slice(), b"w__br" | b"w__cr")
                {
                    collector.separator('\n')?;
                }
                stack.push(name);
            }
            Event::Empty(element) if active && deletion_depth == 0 => {
                match element.name().as_ref() {
                    b"w__tab" => collector.separator('\t')?,
                    b"w__br" | b"w__cr" => collector.separator('\n')?,
                    _ => {}
                }
            }
            Event::Text(text)
                if active
                    && deletion_depth == 0
                    && stack.last().is_some_and(|name| {
                        matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t")
                    }) =>
            {
                let decoded = text
                    .xml_content(quick_xml::XmlVersion::Explicit1_0)
                    .map_err(|_| DocumentError::Invalid)?;
                collector.text(&decoded)?;
            }
            Event::GeneralRef(reference)
                if active
                    && deletion_depth == 0
                    && stack.last().is_some_and(|name| {
                        matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t")
                    }) =>
            {
                let reference = reference.decode().map_err(|_| DocumentError::Invalid)?;
                collector.text(&resolve_xml_reference(&reference)?)?;
            }
            Event::CData(text)
                if active
                    && deletion_depth == 0
                    && stack.last().is_some_and(|name| {
                        matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t")
                    }) =>
            {
                let decoded = text.xml10_content().map_err(|_| DocumentError::Invalid)?;
                collector.text(&decoded)?;
            }
            Event::End(element) => {
                let name = element.name().as_ref().to_vec();
                if active
                    && deletion_depth == 0
                    && (name.starts_with(b"w__") || name.starts_with(b"a__"))
                {
                    collector.end(tag_local(&name))?;
                }
                if deletion_depth > 0 && matches!(name.as_slice(), b"w__del" | b"w__moveFrom") {
                    deletion_depth -= 1;
                }
                if element_is(&name, b"w", element_name.as_bytes()) {
                    active = false;
                }
                stack.pop().ok_or(DocumentError::Invalid)?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !saw_root || !referenced_ids.is_subset(&found_ids) {
        return Err(DocumentError::Invalid);
    }
    Ok(collector.finish())
}

fn extract_pptx(
    parts: &mut Parts<'_>,
    main_part: &str,
    presentation_rels: &[Relationship],
    output: &mut TextSink,
) -> Result<(), DocumentError> {
    let presentation_xml = parts.part(main_part)?;
    let slide_rel_ids = presentation_slide_ids(presentation_xml)?;
    let relationships: HashMap<&str, &Relationship> = presentation_rels
        .iter()
        .map(|rel| (rel.id.as_str(), rel))
        .collect();
    if slide_rel_ids.is_empty() {
        return Err(DocumentError::NoText);
    }
    let mut unique_slide_ids = HashSet::with_capacity(slide_rel_ids.len());
    if slide_rel_ids
        .iter()
        .any(|id| !unique_slide_ids.insert(id.as_str()))
    {
        return Err(DocumentError::Invalid);
    }
    let mut unique_slide_parts = HashSet::with_capacity(slide_rel_ids.len());
    let mut has_text = false;
    let mut pending_empty_slides = 0usize;
    for (slide_index, id) in slide_rel_ids.into_iter().enumerate() {
        let relation = relationships
            .get(id.as_str())
            .ok_or(DocumentError::Invalid)?;
        if relation.external || !relationship_type_is(&relation.kind, "slide") {
            return Err(DocumentError::Invalid);
        }
        let slide_part = resolve_target(main_part, &relation.target)?;
        if !unique_slide_parts.insert(slide_part.clone()) {
            return Err(DocumentError::Invalid);
        }
        let slide_xml = parts.part(&slide_part)?;
        let referenced_visual_relation_ids = slide_visual_relation_ids(slide_xml)?;
        let mut slide_content = Vec::new();
        match xml_to_text(slide_xml, output.limit(), XmlFlavor::Drawing) {
            Ok(text) if !text.trim().is_empty() => slide_content.push(text),
            Ok(_) | Err(DocumentError::NoText) => {}
            Err(error) => return Err(error),
        }
        let rels_path = relationships_path(&slide_part);
        let slide_rels = if parts.contains(&rels_path)? {
            parse_relationships(parts.part(&rels_path)?)?
        } else {
            Vec::new()
        };
        // Ids are unique (`parse_relationships`), so index them once instead of
        // scanning the list for every reference.
        let slide_rels_by_id: HashMap<&str, &Relationship> = slide_rels
            .iter()
            .map(|relation| (relation.id.as_str(), relation))
            .collect();
        for relation_id in &referenced_visual_relation_ids {
            let relation = slide_rels_by_id
                .get(relation_id.as_str())
                .copied()
                .ok_or(DocumentError::Invalid)?;
            if !relationship_type_is(&relation.kind, "chart")
                && !relationship_type_is(&relation.kind, "diagramData")
                && !relationship_type_is(&relation.kind, "diagramDrawing")
            {
                return Err(DocumentError::Invalid);
            }
        }
        let mut related_text = Vec::new();
        for rel in &slide_rels {
            if rel.external
                && (relationship_type_is(&rel.kind, "notesSlide")
                    || (referenced_visual_relation_ids.contains(&rel.id)
                        && (relationship_type_is(&rel.kind, "chart")
                            || relationship_type_is(&rel.kind, "diagramData")
                            || relationship_type_is(&rel.kind, "diagramDrawing"))))
            {
                return Err(DocumentError::Unsupported);
            }
            if rel.external {
                continue;
            }
            if relationship_type_is(&rel.kind, "notesSlide") {
                let note_part = resolve_target(&slide_part, &rel.target)?;
                let note_xml = parts.part(&note_part)?;
                match xml_to_text(note_xml, output.limit(), XmlFlavor::Notes) {
                    Ok(text) if !text.trim().is_empty() => {
                        related_text.push(format!("Speaker notes\n{text}"))
                    }
                    Ok(_) | Err(DocumentError::NoText) => {}
                    Err(error) => return Err(error),
                }
            } else if referenced_visual_relation_ids.contains(&rel.id)
                && (relationship_type_is(&rel.kind, "chart")
                    || relationship_type_is(&rel.kind, "diagramData"))
            {
                let data_part = resolve_target(&slide_part, &rel.target)?;
                let data_xml = parts.part(&data_part)?;
                match xml_to_text(data_xml, output.limit(), XmlFlavor::Chart) {
                    Ok(text) if !text.trim().is_empty() => {
                        let label = if relationship_type_is(&rel.kind, "chart") {
                            "Chart data"
                        } else {
                            "Diagram data"
                        };
                        related_text.push(format!("{label}\n{text}"))
                    }
                    Ok(_) | Err(DocumentError::NoText) => {}
                    Err(error) => return Err(error),
                }
            } else if referenced_visual_relation_ids.contains(&rel.id)
                && relationship_type_is(&rel.kind, "diagramDrawing")
            {
                let drawing_part = resolve_target(&slide_part, &rel.target)?;
                let drawing_xml = parts.part(&drawing_part)?;
                let referenced_data_ids = slide_visual_relation_ids(drawing_xml)?;
                let drawing_rels_path = relationships_path(&drawing_part);
                if !referenced_data_ids.is_empty() && !parts.contains(&drawing_rels_path)? {
                    return Err(DocumentError::Invalid);
                }
                if parts.contains(&drawing_rels_path)? {
                    let drawing_rels = parse_relationships(parts.part(&drawing_rels_path)?)?;
                    let drawing_rels_by_id: HashMap<&str, &Relationship> = drawing_rels
                        .iter()
                        .map(|relation| (relation.id.as_str(), relation))
                        .collect();
                    for relation_id in &referenced_data_ids {
                        let data_rel = drawing_rels_by_id
                            .get(relation_id.as_str())
                            .copied()
                            .ok_or(DocumentError::Invalid)?;
                        if data_rel.external || !relationship_type_is(&data_rel.kind, "diagramData")
                        {
                            return Err(if data_rel.external {
                                DocumentError::Unsupported
                            } else {
                                DocumentError::Invalid
                            });
                        }
                        let data_part = resolve_target(&drawing_part, &data_rel.target)?;
                        let data_xml = parts.part(&data_part)?;
                        match xml_to_text(data_xml, output.limit(), XmlFlavor::Chart) {
                            Ok(text) if !text.trim().is_empty() => {
                                related_text.push(format!("Diagram data\n{text}"))
                            }
                            Ok(_) | Err(DocumentError::NoText) => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
        }
        slide_content.extend(related_text);
        if slide_content.is_empty() {
            if has_text {
                output.push_block(&format!("Slide {}", slide_index + 1))?;
            } else {
                pending_empty_slides += 1;
            }
            continue;
        }
        if !has_text {
            for prior_slide in 0..pending_empty_slides {
                output.push_block(&format!("Slide {}", prior_slide + 1))?;
            }
            has_text = true;
        }
        output.push_block(&format!("Slide {}", slide_index + 1))?;
        for text in slide_content {
            output.push_block(&text)?;
        }
    }
    Ok(())
}

fn presentation_slide_ids(xml: &[u8]) -> Result<Vec<String>, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let normalized = normalize_namespaces(&selected, XmlContext::Presentation)?;
    let mut reader = xml_reader(&normalized)?;
    let mut buffer = Vec::new();
    let mut ids = Vec::new();
    let mut events = 0;
    let mut depth = 0;
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut saw_root = false;
    let mut saw_slide_list = false;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) => {
                let name = element.name().as_ref().to_vec();
                if stack.is_empty() {
                    if saw_root || name.as_slice() != b"p__presentation" {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                if name.as_slice() == b"p__sldIdLst" {
                    saw_slide_list = true;
                }
                if name.as_slice() == b"p__sldId" {
                    ids.push(relationship_id(&element)?.ok_or(DocumentError::Invalid)?);
                }
                stack.push(name);
            }
            Event::Empty(element) => {
                let name = element.name().as_ref().to_vec();
                if stack.is_empty() {
                    if saw_root || name.as_slice() != b"p__presentation" {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                if name.as_slice() == b"p__sldIdLst" {
                    saw_slide_list = true;
                }
                if name.as_slice() == b"p__sldId" {
                    ids.push(relationship_id(&element)?.ok_or(DocumentError::Invalid)?);
                }
            }
            Event::End(_) => {
                stack.pop().ok_or(DocumentError::Invalid)?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !saw_root || !saw_slide_list || !stack.is_empty() {
        return Err(DocumentError::Invalid);
    }
    Ok(ids)
}

fn slide_visual_relation_ids(xml: &[u8]) -> Result<HashSet<String>, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let normalized = normalize_namespaces(&selected, XmlContext::Drawing)?;
    let mut reader = xml_reader(&normalized)?;
    let mut buffer = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut ids = HashSet::new();
    let mut events = 0;
    let mut depth = 0;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) => {
                let qualified_name = element.name();
                let name = qualified_name.as_ref();
                if name == b"c__chart" {
                    if let Some(id) = relationship_id(&element)? {
                        ids.insert(id);
                    } else {
                        return Err(DocumentError::Invalid);
                    }
                } else if name == b"p__relIds" {
                    for attribute in element.attributes().with_checks(true) {
                        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
                        if attribute.key.as_ref() == b"xfrel:dm" {
                            let id = attribute
                                .decoded_and_normalized_value(
                                    quick_xml::XmlVersion::Explicit1_0,
                                    element.decoder(),
                                )
                                .map_err(|_| DocumentError::Invalid)?
                                .into_owned();
                            ids.insert(id);
                        }
                    }
                }
                stack.push(element.name().as_ref().to_vec());
            }
            Event::Empty(element) => {
                let qualified_name = element.name();
                let name = qualified_name.as_ref();
                if name == b"c__chart" {
                    ids.insert(relationship_id(&element)?.ok_or(DocumentError::Invalid)?);
                } else if name == b"p__relIds" {
                    for attribute in element.attributes().with_checks(true) {
                        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
                        if attribute.key.as_ref() == b"xfrel:dm" {
                            ids.insert(
                                attribute
                                    .decoded_and_normalized_value(
                                        quick_xml::XmlVersion::Explicit1_0,
                                        element.decoder(),
                                    )
                                    .map_err(|_| DocumentError::Invalid)?
                                    .into_owned(),
                            );
                        }
                    }
                }
            }
            Event::End(_) => {
                stack.pop().ok_or(DocumentError::Invalid)?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(ids)
}

#[derive(Clone, Copy)]
enum XmlFlavor {
    Word,
    Drawing,
    Notes,
    Chart,
}

fn xml_to_text(xml: &[u8], max: usize, flavor: XmlFlavor) -> Result<String, DocumentError> {
    let selected = select_alternate_content(xml)?;
    let context = match flavor {
        XmlFlavor::Word => XmlContext::Word,
        XmlFlavor::Drawing => XmlContext::Drawing,
        XmlFlavor::Notes => XmlContext::Notes,
        XmlFlavor::Chart => XmlContext::Chart,
    };
    let normalized = normalize_namespaces(&selected, context)?;
    let mut reader = xml_reader(&normalized)?;
    let mut buffer = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut collector = TextCollector::new(max);
    let mut deletion_depth = 0usize;
    let mut shape_depth: Option<usize> = None;
    let mut skip_shape = false;
    let mut events = 0;
    let mut depth = 0;
    let mut saw_root = false;
    loop {
        match read_event(&mut reader, &mut buffer, &mut events, &mut depth)? {
            Event::Start(element) => {
                let name = element.name().as_ref().to_vec();
                if stack.is_empty() {
                    if saw_root {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                if matches!(name.as_slice(), b"w__altChunk" | b"p__oleObj") {
                    return Err(DocumentError::Unsupported);
                }
                if matches!(
                    name.as_slice(),
                    b"w__del" | b"w__moveFrom" | b"p__del" | b"p__moveFrom"
                ) {
                    deletion_depth += 1;
                }
                if matches!(flavor, XmlFlavor::Notes) && name.as_slice() == b"p__sp" {
                    shape_depth = Some(stack.len() + 1);
                    skip_shape = false;
                }
                if matches!(flavor, XmlFlavor::Notes) && name.as_slice() == b"p__ph" {
                    let kind = attribute_local(&element, b"type")?;
                    if kind
                        .as_deref()
                        .is_some_and(|kind| !matches!(kind, "body" | "obj"))
                    {
                        skip_shape = true;
                    }
                }
                if !skip_shape && matches!(flavor, XmlFlavor::Word) {
                    if name.as_slice() == b"w__tab" {
                        collector.separator('\t')?;
                    }
                    if matches!(name.as_slice(), b"w__br" | b"w__cr") {
                        collector.separator('\n')?;
                    }
                }
                let starts_text_block = match flavor {
                    XmlFlavor::Word => matches!(
                        name.as_slice(),
                        b"w__p" | b"w__tc" | b"w__tr" | b"a__p" | b"a__tc" | b"a__tr"
                    ),
                    XmlFlavor::Drawing | XmlFlavor::Notes | XmlFlavor::Chart => {
                        matches!(name.as_slice(), b"a__p" | b"a__tc" | b"a__tr")
                    }
                };
                if deletion_depth == 0 && !skip_shape && starts_text_block {
                    collector.start(tag_local(&name))?;
                }
                stack.push(name);
            }
            Event::Empty(element) => {
                if stack.is_empty() {
                    if saw_root {
                        return Err(DocumentError::Invalid);
                    }
                    saw_root = true;
                }
                let name = element.name().as_ref().to_vec();
                if matches!(name.as_slice(), b"w__altChunk" | b"p__oleObj") {
                    return Err(DocumentError::Unsupported);
                }
                if matches!(flavor, XmlFlavor::Notes) && name.as_slice() == b"p__ph" {
                    let kind = attribute_local(&element, b"type")?;
                    if kind
                        .as_deref()
                        .is_some_and(|kind| !matches!(kind, "body" | "obj"))
                    {
                        skip_shape = true;
                    }
                }
                if matches!(flavor, XmlFlavor::Word) && name.as_slice() == b"w__tab" {
                    collector.separator('\t')?;
                } else if (matches!(flavor, XmlFlavor::Word)
                    && matches!(name.as_slice(), b"w__br" | b"w__cr"))
                    || matches!(name.as_slice(), b"w__br" | b"a__br")
                {
                    collector.separator('\n')?;
                } else if name.as_slice() == b"a__p" && matches!(flavor, XmlFlavor::Drawing) {
                    collector.end(b"p")?;
                }
            }
            Event::Text(text)
                if deletion_depth == 0
                    && !skip_shape
                    && stack.last().is_some_and(|name| match flavor {
                        XmlFlavor::Word => matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t"),
                        XmlFlavor::Drawing | XmlFlavor::Notes => name.as_slice() == b"a__t",
                        XmlFlavor::Chart => matches!(name.as_slice(), b"a__t" | b"c__v"),
                    }) =>
            {
                let decoded = text
                    .xml_content(quick_xml::XmlVersion::Explicit1_0)
                    .map_err(|_| DocumentError::Invalid)?;
                collector.text(&decoded)?;
            }
            Event::GeneralRef(reference)
                if deletion_depth == 0
                    && !skip_shape
                    && stack.last().is_some_and(|name| match flavor {
                        XmlFlavor::Word => matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t"),
                        XmlFlavor::Drawing | XmlFlavor::Notes => name.as_slice() == b"a__t",
                        XmlFlavor::Chart => matches!(name.as_slice(), b"a__t" | b"c__v"),
                    }) =>
            {
                let reference = reference.decode().map_err(|_| DocumentError::Invalid)?;
                collector.text(&resolve_xml_reference(&reference)?)?;
            }
            Event::CData(text)
                if deletion_depth == 0
                    && !skip_shape
                    && stack.last().is_some_and(|name| match flavor {
                        XmlFlavor::Word => matches!(name.as_slice(), b"w__t" | b"a__t" | b"m__t"),
                        XmlFlavor::Drawing | XmlFlavor::Notes => name.as_slice() == b"a__t",
                        XmlFlavor::Chart => matches!(name.as_slice(), b"a__t" | b"c__v"),
                    }) =>
            {
                let decoded = text.xml10_content().map_err(|_| DocumentError::Invalid)?;
                collector.text(&decoded)?;
            }
            Event::End(element) => {
                let name = element.name().as_ref().to_vec();
                if deletion_depth == 0 && !skip_shape {
                    let ends_text_block = match flavor {
                        XmlFlavor::Word => matches!(
                            name.as_slice(),
                            b"w__p" | b"w__tc" | b"w__tr" | b"a__p" | b"a__tc" | b"a__tr"
                        ),
                        XmlFlavor::Drawing | XmlFlavor::Notes | XmlFlavor::Chart => {
                            matches!(name.as_slice(), b"a__p" | b"a__tc" | b"a__tr")
                        }
                    };
                    if ends_text_block {
                        collector.end(tag_local(&name))?;
                    }
                }
                if deletion_depth > 0
                    && matches!(
                        name.as_slice(),
                        b"w__del" | b"w__moveFrom" | b"p__del" | b"p__moveFrom"
                    )
                {
                    deletion_depth -= 1;
                }
                if shape_depth == Some(stack.len()) {
                    shape_depth = None;
                    skip_shape = false;
                }
                stack.pop().ok_or(DocumentError::Invalid)?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if depth != 0 || !stack.is_empty() || !saw_root {
        return Err(DocumentError::Invalid);
    }
    Ok(collector.finish())
}

struct TextCollector {
    out: String,
    max: usize,
}

impl TextCollector {
    fn new(max: usize) -> Self {
        Self {
            out: String::new(),
            max,
        }
    }

    fn text(&mut self, text: &str) -> Result<(), DocumentError> {
        self.push(text.trim_matches(['\r', '\n']))
    }

    fn start(&mut self, _name: &[u8]) -> Result<(), DocumentError> {
        Ok(())
    }

    fn end(&mut self, name: &[u8]) -> Result<(), DocumentError> {
        match name {
            b"p" | b"pt" | b"tr" => self.separator('\n'),
            b"tc" => self.separator('\t'),
            _ => Ok(()),
        }
    }

    fn separator(&mut self, ch: char) -> Result<(), DocumentError> {
        if self.out.is_empty() {
            return Ok(());
        }
        if ch == '\t' && self.out.ends_with('\n') {
            self.out.pop();
        }
        if ch == '\n' && self.out.ends_with('\t') {
            self.out.pop();
        }
        if self.out.ends_with('\n') || self.out.ends_with('\t') {
            return Ok(());
        }
        if self.out.len().saturating_add(ch.len_utf8()) > self.max {
            return Err(DocumentError::TextTooLarge);
        }
        self.out.push(ch);
        Ok(())
    }

    fn push(&mut self, text: &str) -> Result<(), DocumentError> {
        if text.is_empty() {
            return Ok(());
        }
        if !text.chars().all(is_xml10_character) {
            return Err(DocumentError::Invalid);
        }
        if self.out.len().saturating_add(text.len()) > self.max {
            return Err(DocumentError::TextTooLarge);
        }
        self.out.push_str(text);
        Ok(())
    }

    fn finish(self) -> String {
        self.out.trim().to_owned()
    }
}

struct TextSink {
    out: String,
    max: usize,
}

impl TextSink {
    fn new(max: usize) -> Self {
        Self {
            out: String::new(),
            max,
        }
    }
    fn limit(&self) -> usize {
        self.max.saturating_sub(self.out.len())
    }
    fn push_block(&mut self, block: &str) -> Result<(), DocumentError> {
        let block = block.trim();
        if block.is_empty() {
            return Ok(());
        }
        let separator = if self.out.is_empty() { 0 } else { 2 };
        let needed = self
            .out
            .len()
            .saturating_add(separator)
            .saturating_add(block.len());
        if needed > self.max {
            return Err(DocumentError::TextTooLarge);
        }
        if separator > 0 {
            self.out.push_str("\n\n");
        }
        self.out.push_str(block);
        Ok(())
    }
    fn finish(self) -> Result<String, DocumentError> {
        if self.out.trim().is_empty() {
            Err(DocumentError::NoText)
        } else {
            Ok(self.out)
        }
    }
}

fn xml_reader(xml: &[u8]) -> Result<Reader<&[u8]>, DocumentError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    Ok(reader)
}

fn read_resolved_event<'a, 'b, R: std::io::BufRead>(
    reader: &'a mut quick_xml::reader::NsReader<R>,
    buffer: &'b mut Vec<u8>,
    events: &mut usize,
    depth: &mut usize,
) -> Result<(quick_xml::name::ResolveResult<'a>, Event<'b>), DocumentError> {
    *events = events.checked_add(1).ok_or(DocumentError::Unsupported)?;
    if *events > MAX_XML_EVENTS {
        return Err(DocumentError::Unsupported);
    }
    let (resolved, event) = reader
        .read_resolved_event_into(buffer)
        .map_err(|_| DocumentError::Invalid)?;
    if matches!(event, Event::DocType(_)) {
        return Err(DocumentError::Unsupported);
    }
    if let Event::Start(element) | Event::Empty(element) = &event {
        let mut count = 0usize;
        for attribute in element.attributes().with_checks(true) {
            attribute.map_err(|_| DocumentError::Invalid)?;
            count += 1;
            if count > MAX_XML_ATTRIBUTES_PER_ELEMENT {
                return Err(DocumentError::Unsupported);
            }
        }
    }
    match &event {
        Event::Start(_) => {
            *depth = depth.checked_add(1).ok_or(DocumentError::Unsupported)?;
            if *depth > MAX_XML_DEPTH {
                return Err(DocumentError::Unsupported);
            }
        }
        Event::End(_) => *depth = depth.checked_sub(1).ok_or(DocumentError::Invalid)?,
        Event::Eof if *depth != 0 => return Err(DocumentError::Invalid),
        _ => {}
    }
    Ok((resolved, event))
}

#[derive(Clone, Copy, Debug)]
enum XmlContext {
    ContentTypes,
    Relationships,
    Word,
    Presentation,
    Drawing,
    Notes,
    Chart,
}

const WORD_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    b"http://purl.oclc.org/ooxml/wordprocessingml/main",
];
const PRESENTATION_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/presentationml/2006/main",
    b"http://purl.oclc.org/ooxml/presentationml/main",
];
const DRAWING_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/drawingml/2006/main",
    b"http://purl.oclc.org/ooxml/drawingml/main",
];
const CHART_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/drawingml/2006/chart",
    b"http://purl.oclc.org/ooxml/drawingml/chart",
];
const OFFICE_MATH_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/officeDocument/2006/math",
    b"http://purl.oclc.org/ooxml/officeDocument/math",
];
const RELATIONSHIP_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    b"http://purl.oclc.org/ooxml/officeDocument/relationships",
];
const PACKAGE_RELATIONSHIP_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/package/2006/relationships",
    b"http://purl.oclc.org/ooxml/package/relationships",
];
const CONTENT_TYPE_NAMESPACES: &[&[u8]] = &[
    b"http://schemas.openxmlformats.org/package/2006/content-types",
    b"http://purl.oclc.org/ooxml/package/content-types",
];

fn attribute_namespace_allowed(context: XmlContext, namespace: &[u8]) -> bool {
    match context {
        XmlContext::ContentTypes => CONTENT_TYPE_NAMESPACES.contains(&namespace),
        XmlContext::Relationships => PACKAGE_RELATIONSHIP_NAMESPACES.contains(&namespace),
        XmlContext::Word => {
            WORD_NAMESPACES.contains(&namespace)
                || DRAWING_NAMESPACES.contains(&namespace)
                || OFFICE_MATH_NAMESPACES.contains(&namespace)
        }
        XmlContext::Presentation => PRESENTATION_NAMESPACES.contains(&namespace),
        XmlContext::Drawing => {
            PRESENTATION_NAMESPACES.contains(&namespace)
                || DRAWING_NAMESPACES.contains(&namespace)
                || CHART_NAMESPACES.contains(&namespace)
        }
        XmlContext::Notes => {
            PRESENTATION_NAMESPACES.contains(&namespace) || DRAWING_NAMESPACES.contains(&namespace)
        }
        XmlContext::Chart => {
            CHART_NAMESPACES.contains(&namespace) || DRAWING_NAMESPACES.contains(&namespace)
        }
    }
}

fn element_namespace_allowed(context: XmlContext, namespace: &[u8], local: &[u8]) -> bool {
    if namespace == MC_NAMESPACE {
        return matches!(local, b"AlternateContent" | b"Choice" | b"Fallback");
    }
    match context {
        XmlContext::ContentTypes => {
            CONTENT_TYPE_NAMESPACES.contains(&namespace)
                && matches!(local, b"Types" | b"Default" | b"Override")
        }
        XmlContext::Relationships => {
            PACKAGE_RELATIONSHIP_NAMESPACES.contains(&namespace)
                && matches!(local, b"Relationships" | b"Relationship")
        }
        XmlContext::Word => {
            if WORD_NAMESPACES.contains(&namespace) {
                true
            } else if DRAWING_NAMESPACES.contains(&namespace) {
                !matches!(
                    local,
                    b"document"
                        | b"body"
                        | b"del"
                        | b"moveFrom"
                        | b"headerReference"
                        | b"footerReference"
                        | b"footnoteReference"
                        | b"endnoteReference"
                        | b"altChunk"
                )
            } else if OFFICE_MATH_NAMESPACES.contains(&namespace) {
                !matches!(local, b"document" | b"body" | b"del" | b"moveFrom")
            } else {
                false
            }
        }
        XmlContext::Presentation => PRESENTATION_NAMESPACES.contains(&namespace),
        XmlContext::Drawing => {
            if PRESENTATION_NAMESPACES.contains(&namespace) {
                !matches!(local, b"t" | b"v" | b"chart" | b"relIds")
            } else if DRAWING_NAMESPACES.contains(&namespace) {
                true
            } else if CHART_NAMESPACES.contains(&namespace) {
                matches!(local, b"chart" | b"chartSpace")
            } else {
                false
            }
        }
        XmlContext::Notes => {
            if PRESENTATION_NAMESPACES.contains(&namespace) {
                !matches!(local, b"t" | b"v" | b"p" | b"tc" | b"tr")
            } else {
                DRAWING_NAMESPACES.contains(&namespace)
            }
        }
        XmlContext::Chart => {
            if CHART_NAMESPACES.contains(&namespace) {
                true
            } else if DRAWING_NAMESPACES.contains(&namespace) {
                matches!(
                    local,
                    b"p" | b"r" | b"t" | b"tc" | b"tr" | b"tab" | b"br" | b"cr"
                )
            } else {
                false
            }
        }
    }
}

fn root_namespace_allowed(context: XmlContext, namespace: &[u8]) -> bool {
    match context {
        XmlContext::ContentTypes => CONTENT_TYPE_NAMESPACES.contains(&namespace),
        XmlContext::Relationships => PACKAGE_RELATIONSHIP_NAMESPACES.contains(&namespace),
        XmlContext::Word => WORD_NAMESPACES.contains(&namespace),
        XmlContext::Presentation | XmlContext::Drawing | XmlContext::Notes => {
            PRESENTATION_NAMESPACES.contains(&namespace)
        }
        XmlContext::Chart => CHART_NAMESPACES.contains(&namespace),
    }
}

fn root_expected(context: XmlContext, local: &[u8]) -> bool {
    match context {
        XmlContext::ContentTypes => local == b"Types",
        XmlContext::Relationships => local == b"Relationships",
        XmlContext::Word => matches!(
            local,
            b"document" | b"hdr" | b"ftr" | b"footnotes" | b"endnotes"
        ),
        XmlContext::Presentation => local == b"presentation",
        XmlContext::Drawing => local == b"sld",
        XmlContext::Notes => local == b"notes",
        XmlContext::Chart => local == b"chartSpace",
    }
}

fn element_family(namespace: &[u8]) -> Option<&'static str> {
    if WORD_NAMESPACES.contains(&namespace) {
        Some("w")
    } else if PRESENTATION_NAMESPACES.contains(&namespace) {
        Some("p")
    } else if DRAWING_NAMESPACES.contains(&namespace) {
        Some("a")
    } else if OFFICE_MATH_NAMESPACES.contains(&namespace) {
        Some("m")
    } else if CHART_NAMESPACES.contains(&namespace) {
        Some("c")
    } else if RELATIONSHIP_NAMESPACES.contains(&namespace) {
        Some("r")
    } else if PACKAGE_RELATIONSHIP_NAMESPACES.contains(&namespace) {
        Some("package")
    } else if CONTENT_TYPE_NAMESPACES.contains(&namespace) {
        Some("ct")
    } else if namespace == MC_NAMESPACE {
        Some("mc")
    } else {
        None
    }
}

fn element_is(name: &[u8], family: &[u8], local: &[u8]) -> bool {
    name.strip_prefix(family)
        .is_some_and(|suffix| suffix.strip_prefix(b"__") == Some(local))
}

fn tag_local(name: &[u8]) -> &[u8] {
    name.windows(2)
        .position(|pair| pair == b"__")
        .map_or(name, |index| &name[index + 2..])
}

/// Rewrites recognized elements to stable family-qualified names such as
/// `w__t` and `a__t`, so consumers keep the expanded-name distinction after
/// parsing. Foreign elements receive inert names. Namespace resolution rejects
/// undeclared prefixes and uses each element's active namespace scope.
fn normalize_namespaces(xml: &[u8], context: XmlContext) -> Result<Vec<u8>, DocumentError> {
    use quick_xml::name::ResolveResult;
    use quick_xml::reader::NsReader;

    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut events = 0usize;
    let mut root_seen = false;
    let mut foreign_stack = Vec::new();
    loop {
        let (resolved, event) =
            read_resolved_event(&mut reader, &mut buffer, &mut events, &mut depth)?;
        let is_empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let namespace = match &resolved {
                    ResolveResult::Bound(namespace) => Some(namespace.as_ref()),
                    ResolveResult::Unbound => None,
                    ResolveResult::Unknown(_) => return Err(DocumentError::Invalid),
                };
                let local = element.local_name();
                let parent_foreign = foreign_stack.last().copied().unwrap_or(false);
                let allowed = !parent_foreign
                    && namespace.is_some_and(|namespace| {
                        element_namespace_allowed(context, namespace, local.as_ref())
                    });
                if !root_seen {
                    if !namespace
                        .is_some_and(|namespace| root_namespace_allowed(context, namespace))
                        || !root_expected(context, local.as_ref())
                    {
                        return Err(DocumentError::Invalid);
                    }
                    root_seen = true;
                } else if foreign_stack.is_empty() {
                    return Err(DocumentError::Invalid);
                }
                let name = if allowed {
                    let namespace = namespace.ok_or(DocumentError::Invalid)?;
                    let family = element_family(namespace).ok_or(DocumentError::Invalid)?;
                    format!("{family}__{}", String::from_utf8_lossy(local.as_ref()))
                } else {
                    format!("foreign_{}", String::from_utf8_lossy(local.as_ref()))
                };
                let mut normalized = BytesStart::new(name);
                for attribute in element.attributes().with_checks(true) {
                    let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
                    let key = attribute.key;
                    let key_bytes = key.as_ref();
                    if key_bytes == b"xmlns" || key_bytes.starts_with(b"xmlns:") {
                        continue;
                    }
                    let (attribute_namespace, attribute_local) =
                        reader.resolver().resolve_attribute(key);
                    let attribute_name = match attribute_namespace {
                        ResolveResult::Unbound => {
                            String::from_utf8(attribute_local.as_ref().to_vec())
                                .map_err(|_| DocumentError::Invalid)?
                        }
                        ResolveResult::Unknown(_) => return Err(DocumentError::Invalid),
                        ResolveResult::Bound(namespace)
                            if RELATIONSHIP_NAMESPACES.contains(&namespace.as_ref()) =>
                        {
                            match attribute_local.as_ref() {
                                b"id" => "xfrel:id".to_string(),
                                b"dm" => "xfrel:dm".to_string(),
                                _ => format!(
                                    "foreign_attr_{}",
                                    String::from_utf8_lossy(attribute_local.as_ref())
                                ),
                            }
                        }
                        ResolveResult::Bound(namespace)
                            if attribute_namespace_allowed(context, namespace.as_ref()) =>
                        {
                            let family = if WORD_NAMESPACES.contains(&namespace.as_ref()) {
                                "word"
                            } else if PRESENTATION_NAMESPACES.contains(&namespace.as_ref()) {
                                "presentation"
                            } else if DRAWING_NAMESPACES.contains(&namespace.as_ref()) {
                                "drawing"
                            } else if OFFICE_MATH_NAMESPACES.contains(&namespace.as_ref()) {
                                "math"
                            } else if CHART_NAMESPACES.contains(&namespace.as_ref()) {
                                "chart"
                            } else if CONTENT_TYPE_NAMESPACES.contains(&namespace.as_ref()) {
                                "content_type"
                            } else {
                                "package"
                            };
                            format!(
                                "xf{family}:{}",
                                String::from_utf8_lossy(attribute_local.as_ref())
                            )
                        }
                        ResolveResult::Bound(_) => format!(
                            "foreign_attr_{}",
                            String::from_utf8_lossy(attribute_local.as_ref())
                        ),
                    };
                    let value = attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Explicit1_0,
                            element.decoder(),
                        )
                        .map_err(|_| DocumentError::Invalid)?;
                    normalized.push_attribute((attribute_name.as_str(), value.as_ref()));
                }
                let rewritten = if is_empty {
                    Event::Empty(normalized)
                } else {
                    Event::Start(normalized)
                };
                writer
                    .write_event(rewritten)
                    .map_err(|_| DocumentError::Invalid)?;
                if !is_empty {
                    foreign_stack.push(!allowed);
                }
            }
            Event::End(element) => {
                let was_foreign = foreign_stack.pop().ok_or(DocumentError::Invalid)?;
                let name = match resolved {
                    ResolveResult::Bound(namespace)
                        if !was_foreign
                            && element_namespace_allowed(
                                context,
                                namespace.as_ref(),
                                element.local_name().as_ref(),
                            ) =>
                    {
                        let family =
                            element_family(namespace.as_ref()).ok_or(DocumentError::Invalid)?;
                        format!(
                            "{family}__{}",
                            String::from_utf8_lossy(element.local_name().as_ref())
                        )
                    }
                    ResolveResult::Bound(_) => format!(
                        "foreign_{}",
                        String::from_utf8_lossy(element.local_name().as_ref())
                    ),
                    ResolveResult::Unbound if was_foreign => format!(
                        "foreign_{}",
                        String::from_utf8_lossy(element.local_name().as_ref())
                    ),
                    ResolveResult::Unbound | ResolveResult::Unknown(_) => {
                        return Err(DocumentError::Invalid)
                    }
                };
                writer
                    .write_event(Event::End(quick_xml::events::BytesEnd::new(name)))
                    .map_err(|_| DocumentError::Invalid)?;
            }
            Event::Eof => break,
            other => writer
                .write_event(other.into_owned())
                .map_err(|_| DocumentError::Invalid)?,
        }
        buffer.clear();
    }
    if !root_seen || !foreign_stack.is_empty() {
        return Err(DocumentError::Invalid);
    }
    Ok(writer.into_inner())
}

const MC_NAMESPACE: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";
const MAX_XML_ATTRIBUTES_PER_ELEMENT: usize = 128;
const MAX_CHOICE_PREFIXES: usize = 128;
const MAX_REQUIRES_BYTES: usize = 4096;
const SUPPORTED_MC_NAMESPACES: &[&[u8]] = &[
    MC_NAMESPACE,
    b"http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    b"http://purl.oclc.org/ooxml/wordprocessingml/main",
    b"http://schemas.openxmlformats.org/presentationml/2006/main",
    b"http://purl.oclc.org/ooxml/presentationml/main",
    b"http://schemas.openxmlformats.org/drawingml/2006/main",
    b"http://purl.oclc.org/ooxml/drawingml/main",
    b"http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    b"http://purl.oclc.org/ooxml/officeDocument/relationships",
    b"http://schemas.openxmlformats.org/package/2006/relationships",
    b"http://purl.oclc.org/ooxml/package/relationships",
    b"http://schemas.openxmlformats.org/package/2006/content-types",
    b"http://purl.oclc.org/ooxml/package/content-types",
    b"http://schemas.openxmlformats.org/drawingml/2006/chart",
    b"http://purl.oclc.org/ooxml/drawingml/chart",
    b"http://schemas.openxmlformats.org/officeDocument/2006/math",
    b"http://purl.oclc.org/ooxml/officeDocument/math",
];
struct XmlFrame {
    name: Vec<u8>,
    namespace_changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    skip: bool,
    alternate: Option<AlternateState>,
}

type NamespaceChange = (Vec<u8>, Option<Vec<u8>>);

struct AlternateState {
    saw_choice: bool,
    saw_fallback: bool,
    selected: bool,
}

fn select_alternate_content(xml: &[u8]) -> Result<Vec<u8>, DocumentError> {
    let mut reader = xml_reader(xml)?;
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut buffer = Vec::new();
    let mut stack: Vec<XmlFrame> = Vec::new();
    let mut namespace_bindings: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let mut events = 0usize;
    let mut depth = 0usize;
    loop {
        let event = read_event(&mut reader, &mut buffer, &mut events, &mut depth)?;
        match event {
            Event::Start(element) => {
                let qname = element.name().as_ref().to_vec();
                let name = local_name(&qname).to_vec();
                let namespace_changes = bind_element_namespaces(&element, &mut namespace_bindings)?;
                let is_alternate = name.as_slice() == b"AlternateContent"
                    && qname_namespace(&qname, &namespace_bindings) == Some(MC_NAMESPACE);
                let is_choice = is_mc_element(&qname, &namespace_bindings, b"Choice");
                let is_fallback = is_mc_element(&qname, &namespace_bindings, b"Fallback");
                let choice_supported = if is_choice {
                    choice_is_supported(&element, &namespace_bindings)?
                } else {
                    false
                };
                let parent_skip = stack.last().is_some_and(|frame| frame.skip);
                if !parent_skip
                    && stack.last().is_some_and(|frame| frame.alternate.is_some())
                    && !is_choice
                    && !is_fallback
                {
                    return Err(DocumentError::Invalid);
                }
                let mut skip = parent_skip;
                let alternate = if !parent_skip && (is_choice || is_fallback) {
                    stack
                        .last_mut()
                        .and_then(|parent| parent.alternate.as_mut())
                } else {
                    None
                };
                if let Some(state) = alternate {
                    if is_choice {
                        if state.saw_fallback {
                            return Err(DocumentError::Invalid);
                        }
                        state.saw_choice = true;
                        skip = state.selected || !choice_supported;
                        if !skip {
                            state.selected = true;
                        }
                    } else {
                        if state.saw_fallback {
                            return Err(DocumentError::Invalid);
                        }
                        state.saw_fallback = true;
                        skip = state.selected;
                        if !skip {
                            state.selected = true;
                        }
                    }
                }
                if !skip {
                    writer
                        .write_event(Event::Start(element.to_owned()))
                        .map_err(|_| DocumentError::Invalid)?;
                }
                stack.push(XmlFrame {
                    name,
                    namespace_changes,
                    skip,
                    alternate: (is_alternate && !skip).then_some(AlternateState {
                        saw_choice: false,
                        saw_fallback: false,
                        selected: false,
                    }),
                });
            }
            Event::Empty(element) => {
                let qname = element.name().as_ref().to_vec();
                let namespace_changes = bind_element_namespaces(&element, &mut namespace_bindings)?;
                let is_choice = is_mc_element(&qname, &namespace_bindings, b"Choice");
                let is_fallback = is_mc_element(&qname, &namespace_bindings, b"Fallback");
                let choice_supported = if is_choice {
                    choice_is_supported(&element, &namespace_bindings)?
                } else {
                    false
                };
                let parent_skip = stack.last().is_some_and(|frame| frame.skip);
                if !parent_skip
                    && stack.last().is_some_and(|frame| frame.alternate.is_some())
                    && !is_choice
                    && !is_fallback
                {
                    return Err(DocumentError::Invalid);
                }
                let mut skip = parent_skip;
                let alternate = if !parent_skip && (is_choice || is_fallback) {
                    stack
                        .last_mut()
                        .and_then(|parent| parent.alternate.as_mut())
                } else {
                    None
                };
                if let Some(state) = alternate {
                    if is_choice {
                        if state.saw_fallback {
                            return Err(DocumentError::Invalid);
                        }
                        state.saw_choice = true;
                        skip = state.selected || !choice_supported;
                        if !skip {
                            state.selected = true;
                        }
                    } else {
                        if state.saw_fallback {
                            return Err(DocumentError::Invalid);
                        }
                        state.saw_fallback = true;
                        skip = state.selected;
                        if !skip {
                            state.selected = true;
                        }
                    }
                }
                if !skip {
                    writer
                        .write_event(Event::Empty(element.to_owned()))
                        .map_err(|_| DocumentError::Invalid)?;
                }
                restore_namespace_bindings(&mut namespace_bindings, namespace_changes);
            }
            Event::End(element) => {
                let frame = stack.pop().ok_or(DocumentError::Invalid)?;
                if frame.name != local_name(element.name().as_ref()) {
                    return Err(DocumentError::Invalid);
                }
                if frame
                    .alternate
                    .is_some_and(|state| !state.saw_choice || !state.selected)
                {
                    return Err(DocumentError::Unsupported);
                }
                if !frame.skip {
                    writer
                        .write_event(Event::End(element.to_owned()))
                        .map_err(|_| DocumentError::Invalid)?;
                }
                restore_namespace_bindings(&mut namespace_bindings, frame.namespace_changes);
            }
            Event::Text(text) => {
                let decoded = text
                    .xml_content(quick_xml::XmlVersion::Explicit1_0)
                    .map_err(|_| DocumentError::Invalid)?;
                if stack.last().is_some_and(|frame| frame.alternate.is_some())
                    && !decoded.trim().is_empty()
                {
                    return Err(DocumentError::Invalid);
                }
                if !decoded.chars().all(is_xml10_character) {
                    return Err(DocumentError::Invalid);
                }
                if !stack.last().is_some_and(|frame| frame.skip) {
                    writer
                        .write_event(Event::Text(text.to_owned()))
                        .map_err(|_| DocumentError::Invalid)?;
                }
            }
            Event::CData(text) => {
                let decoded = text.xml10_content().map_err(|_| DocumentError::Invalid)?;
                if !decoded.chars().all(is_xml10_character) {
                    return Err(DocumentError::Invalid);
                }
                if !stack.last().is_some_and(|frame| frame.skip) {
                    writer
                        .write_event(Event::CData(text.to_owned()))
                        .map_err(|_| DocumentError::Invalid)?;
                }
            }
            Event::GeneralRef(reference) => {
                let decoded = reference.decode().map_err(|_| DocumentError::Invalid)?;
                resolve_xml_reference(&decoded)?;
                if !stack.last().is_some_and(|frame| frame.skip) {
                    writer
                        .write_event(Event::GeneralRef(reference.to_owned()))
                        .map_err(|_| DocumentError::Invalid)?;
                }
            }
            Event::Eof => break,
            other => {
                if !stack.last().is_some_and(|frame| frame.skip) {
                    writer
                        .write_event(other.into_owned())
                        .map_err(|_| DocumentError::Invalid)?;
                }
            }
        }
        if writer.get_ref().len() > MAX_PART_BYTES as usize {
            return Err(DocumentError::ExpandedTooLarge);
        }
        buffer.clear();
    }
    if !stack.is_empty() {
        return Err(DocumentError::Invalid);
    }
    Ok(writer.into_inner())
}

fn bind_element_namespaces(
    element: &BytesStart<'_>,
    active: &mut HashMap<Vec<u8>, Vec<u8>>,
) -> Result<Vec<NamespaceChange>, DocumentError> {
    let mut changes = Vec::new();
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
        let key = attribute.key.as_ref();
        let decoded = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, element.decoder())
            .map_err(|_| DocumentError::Invalid)?;
        if !decoded.chars().all(is_xml10_character) {
            return Err(DocumentError::Invalid);
        }
        let prefix = if key == b"xmlns" {
            Some(Vec::new())
        } else {
            key.strip_prefix(b"xmlns:").map(ToOwned::to_owned)
        };
        if let Some(prefix) = prefix {
            let uri = decoded.into_owned().into_bytes();
            let previous = active.insert(prefix.clone(), uri);
            changes.push((prefix, previous));
        }
    }
    Ok(changes)
}

fn choice_is_supported(
    element: &BytesStart<'_>,
    namespaces: &HashMap<Vec<u8>, Vec<u8>>,
) -> Result<bool, DocumentError> {
    let mut requires = None;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
        if attribute.key.as_ref() == b"Requires" {
            if attribute.value.as_ref().len() > MAX_REQUIRES_BYTES {
                return Err(DocumentError::Unsupported);
            }
            requires = Some(
                attribute
                    .decoded_and_normalized_value(
                        quick_xml::XmlVersion::Explicit1_0,
                        element.decoder(),
                    )
                    .map_err(|_| DocumentError::Invalid)?
                    .into_owned(),
            );
            break;
        }
    }
    let required = requires.ok_or(DocumentError::Invalid)?;
    let mut count = 0usize;
    let mut supported = true;
    for prefix in required.split_ascii_whitespace() {
        count += 1;
        if count > MAX_CHOICE_PREFIXES {
            return Err(DocumentError::Unsupported);
        }
        if !namespaces
            .get(prefix.as_bytes())
            .is_some_and(|uri| SUPPORTED_MC_NAMESPACES.contains(&uri.as_slice()))
        {
            supported = false;
        }
    }
    if count == 0 {
        return Err(DocumentError::Invalid);
    }
    Ok(supported)
}

fn restore_namespace_bindings(
    active: &mut HashMap<Vec<u8>, Vec<u8>>,
    changes: Vec<NamespaceChange>,
) {
    for (prefix, previous) in changes.into_iter().rev() {
        if let Some(previous) = previous {
            active.insert(prefix, previous);
        } else {
            active.remove(&prefix);
        }
    }
}

fn qname_namespace<'a>(
    qname: &[u8],
    namespaces: &'a HashMap<Vec<u8>, Vec<u8>>,
) -> Option<&'a [u8]> {
    let prefix = qname
        .iter()
        .position(|byte| *byte == b':')
        .map_or(&b""[..], |index| &qname[..index]);
    namespaces.get(prefix).map(Vec::as_slice)
}

fn is_mc_element(qname: &[u8], namespaces: &HashMap<Vec<u8>, Vec<u8>>, wanted: &[u8]) -> bool {
    local_name(qname) == wanted && qname_namespace(qname, namespaces) == Some(MC_NAMESPACE)
}

fn read_event<'buf>(
    reader: &mut Reader<&[u8]>,
    buffer: &'buf mut Vec<u8>,
    count: &mut usize,
    depth: &mut usize,
) -> Result<Event<'buf>, DocumentError> {
    *count = count.checked_add(1).ok_or(DocumentError::Unsupported)?;
    if *count > MAX_XML_EVENTS {
        return Err(DocumentError::Unsupported);
    }
    let event = reader
        .read_event_into(buffer)
        .map_err(|_| DocumentError::Invalid)?;
    match &event {
        Event::DocType(_) => return Err(DocumentError::Unsupported),
        Event::Start(element) | Event::Empty(element) => {
            let mut count = 0usize;
            for attribute in element.attributes().with_checks(true) {
                attribute.map_err(|_| DocumentError::Invalid)?;
                count += 1;
                if count > MAX_XML_ATTRIBUTES_PER_ELEMENT {
                    return Err(DocumentError::Unsupported);
                }
            }
        }
        _ => {}
    }
    match &event {
        Event::Start(_) => {
            *depth = depth.checked_add(1).ok_or(DocumentError::Unsupported)?;
            if *depth > MAX_XML_DEPTH {
                return Err(DocumentError::Unsupported);
            }
        }
        Event::End(_) => {
            *depth = depth.checked_sub(1).ok_or(DocumentError::Invalid)?;
        }
        Event::Eof if *depth != 0 => return Err(DocumentError::Invalid),
        _ => {}
    }
    Ok(event)
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn resolve_xml_reference(reference: &str) -> Result<String, DocumentError> {
    if let Some(character) = quick_xml::events::BytesRef::new(reference)
        .resolve_char_ref()
        .map_err(|_| DocumentError::Invalid)?
    {
        if !is_xml10_character(character) {
            return Err(DocumentError::Invalid);
        }
        return Ok(character.to_string());
    }
    quick_xml::escape::resolve_predefined_entity(reference)
        .map(str::to_owned)
        .ok_or(DocumentError::Unsupported)
}

fn is_xml10_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{a}' | '\u{d}')
        || ('\u{20}'..='\u{d7ff}').contains(&character)
        || ('\u{e000}'..='\u{fffd}').contains(&character)
        || ('\u{10000}'..='\u{10ffff}').contains(&character)
}

fn attribute(element: &BytesStart<'_>, wanted: &[u8]) -> Result<Option<String>, DocumentError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
        if attribute.key.as_ref() == wanted {
            let decoded = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, element.decoder())
                .map_err(|_| DocumentError::Invalid)?;
            return Ok(Some(decoded.into_owned()));
        }
    }
    Ok(None)
}

fn attribute_local(
    element: &BytesStart<'_>,
    wanted: &[u8],
) -> Result<Option<String>, DocumentError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
        if attribute.key.as_ref() == wanted {
            let decoded = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, element.decoder())
                .map_err(|_| DocumentError::Invalid)?;
            return Ok(Some(decoded.into_owned()));
        }
    }
    Ok(None)
}

fn office_attribute(
    element: &BytesStart<'_>,
    family: &[u8],
    wanted: &[u8],
) -> Result<Option<String>, DocumentError> {
    let mut key = b"xf".to_vec();
    key.extend_from_slice(family);
    key.push(b':');
    key.extend_from_slice(wanted);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
        if attribute.key.as_ref() == key {
            let decoded = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, element.decoder())
                .map_err(|_| DocumentError::Invalid)?;
            return Ok(Some(decoded.into_owned()));
        }
    }
    Ok(None)
}

fn relationship_id(element: &BytesStart<'_>) -> Result<Option<String>, DocumentError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
        let key = attribute.key.as_ref();
        if key == b"xfrel:id" {
            let decoded = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, element.decoder())
                .map_err(|_| DocumentError::Invalid)?;
            return Ok(Some(decoded.into_owned()));
        }
    }
    Ok(None)
}

fn required_reference_id(id: Option<String>) -> Result<String, DocumentError> {
    id.filter(|id| !id.is_empty()).ok_or(DocumentError::Invalid)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;

    use super::*;

    const PRODUCER_DOCX: &[u8] = include_bytes!("../../tests/fixtures/office/producer.docx");
    const PRODUCER_PPTX: &[u8] = include_bytes!("../../tests/fixtures/office/producer.pptx");

    #[test]
    fn producer_docx_keeps_text_order_tables_headers_and_ignores_deleted_text() {
        let text = extract("docx", PRODUCER_DOCX, 4 * 1024 * 1024).unwrap();
        assert!(text.contains("Unicode café — split text and entities <tag> & ampersand."));
        assert!(
            text.contains("Header A\tHeader B"),
            "table header is missing: {text}"
        );
        assert!(
            text.contains("Cell 1\tCell 2"),
            "table row is missing: {text}"
        );
        assert!(text.contains("Referenced DOCX header"));
        assert!(text.contains("Referenced DOCX footer"));
        assert!(
            !text.contains("REMOVE_THIS_TEXT"),
            "tracked deletion leaked: {text}"
        );
    }

    #[test]
    fn drawingml_paragraphs_keep_boundaries_in_docx_body_header_and_referenced_note() {
        let package = minimal_docx_with_notes();
        let package = replace_zip_part(
            &package,
            "word/document.xml",
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:body><w:p><w:r><w:t>Body text</w:t><w:footnoteReference w:id="4"/></w:r><w:drawing><a:graphic><a:graphicData><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Body drawing one</a:t></a:r></a:p><a:p><a:r><a:t>Body drawing two</a:t></a:r></a:p></a:txBody></a:graphicData></a:graphic></w:drawing></w:p><w:sectPr><w:headerReference w:type="default" r:id="head"/></w:sectPr></w:body></w:document>"#,
        );
        let package = replace_zip_part(
            &package,
            "word/header1.xml",
            br#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:p><w:r><w:t>Header text</w:t></w:r><w:drawing><a:txBody><a:p><a:r><a:t>Header drawing one</a:t></a:r></a:p><a:p><a:r><a:t>Header drawing two</a:t></a:r></a:p></a:txBody><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Header cell one</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Header cell two</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></w:drawing></w:p></w:hdr>"#,
        );
        let package = replace_zip_part(
            &package,
            "word/footnotes.xml",
            br#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:footnote w:id="-1"><w:p><w:r><w:t>Separator</w:t></w:r></w:p></w:footnote><w:footnote w:id="4"><w:p><w:r><w:t>Referenced note</w:t><w:drawing><a:txBody><a:p><a:r><a:t>Note drawing one</a:t></a:r></a:p><a:p><a:r><a:t>Note drawing two</a:t></a:r></a:p></a:txBody></w:drawing></w:r></w:p></w:footnote></w:footnotes>"#,
        );

        let text = extract("docx", &package, 4096).unwrap();
        for paragraph_pair in [
            "Body drawing one\nBody drawing two",
            "Header drawing one\nHeader drawing two",
            "Header cell one\tHeader cell two",
            "Note drawing one\nNote drawing two",
        ] {
            assert!(text.contains(paragraph_pair), "{paragraph_pair:?}: {text}");
        }
    }

    #[test]
    fn docx_references_require_nonempty_ids_in_start_and_empty_events() {
        let cases = [
            (
                "header Start without id",
                "<w:sectPr><w:headerReference w:type=\"default\"></w:headerReference></w:sectPr>",
            ),
            (
                "header Empty without id",
                "<w:sectPr><w:headerReference w:type=\"default\"/></w:sectPr>",
            ),
            (
                "header Start with empty id",
                "<w:sectPr><w:headerReference w:type=\"default\" r:id=\"\"></w:headerReference></w:sectPr>",
            ),
            (
                "header Empty with empty id",
                "<w:sectPr><w:headerReference w:type=\"default\" r:id=\"\"/></w:sectPr>",
            ),
            (
                "footer Start without id",
                "<w:sectPr><w:footerReference w:type=\"default\"></w:footerReference></w:sectPr>",
            ),
            (
                "footer Empty without id",
                "<w:sectPr><w:footerReference w:type=\"default\"/></w:sectPr>",
            ),
            (
                "footer Start with empty id",
                "<w:sectPr><w:footerReference w:type=\"default\" r:id=\"\"></w:footerReference></w:sectPr>",
            ),
            (
                "footer Empty with empty id",
                "<w:sectPr><w:footerReference w:type=\"default\" r:id=\"\"/></w:sectPr>",
            ),
            (
                "footnote Start without id",
                "<w:r><w:footnoteReference></w:footnoteReference></w:r>",
            ),
            (
                "footnote Empty without id",
                "<w:r><w:footnoteReference/></w:r>",
            ),
            (
                "footnote Start with empty id",
                "<w:r><w:footnoteReference w:id=\"\"></w:footnoteReference></w:r>",
            ),
            (
                "footnote Empty with empty id",
                "<w:r><w:footnoteReference w:id=\"\"/></w:r>",
            ),
            (
                "endnote Start without id",
                "<w:r><w:endnoteReference></w:endnoteReference></w:r>",
            ),
            (
                "endnote Empty without id",
                "<w:r><w:endnoteReference/></w:r>",
            ),
            (
                "endnote Start with empty id",
                "<w:r><w:endnoteReference w:id=\"\"></w:endnoteReference></w:r>",
            ),
            (
                "endnote Empty with empty id",
                "<w:r><w:endnoteReference w:id=\"\"/></w:r>",
            ),
        ];

        for (case, reference) in cases {
            let xml = format!(
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>Visible body text</w:t></w:r></w:p>{reference}</w:body></w:document>"#
            );
            let package = replace_zip_part(
                &minimal_docx_with_notes(),
                "word/document.xml",
                xml.as_bytes(),
            );
            assert_eq!(
                extract("docx", &package, 1024),
                Err(DocumentError::Invalid),
                "{case} was silently ignored"
            );
        }
    }

    #[test]
    fn producer_pptx_uses_relationship_order_and_extracts_tables_notes_and_chart_cache() {
        let text = extract("pptx", PRODUCER_PPTX, 4 * 1024 * 1024).unwrap();
        let first = text.find("Slide relationship order first").unwrap();
        let second = text.find("Slide relationship order second").unwrap();
        assert!(
            first < second,
            "slides are not in presentation order: {text}"
        );
        for expected in [
            "Table A",
            "Table B",
            "Table C",
            "Table D",
            "Speaker note for slide one",
            "Q1",
            "Q2",
            "Sales",
            "42",
            "57",
            "Speaker notes",
            "Chart data",
        ] {
            assert!(text.contains(expected), "{expected:?} is missing: {text}");
        }
        assert!(
            !text.contains("Click to add notes"),
            "notes master placeholder leaked: {text}"
        );
    }

    #[test]
    fn docx_requires_exact_word_content_types_and_expanded_namespaces() {
        let valid = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Valid text</w:t></w:r></w:p></w:body></w:document>"#,
        );
        assert!(extract("docx", &valid, 1024)
            .unwrap()
            .contains("Valid text"));

        for xml in [
            br#"<w:document xmlns:w="urn:attacker:fake-wordprocessingml"><w:body><w:p><w:r><w:t>Fake namespace</w:t></w:r></w:p></w:body></w:document>"#.as_slice(),
            br#"<a:document xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:body><a:p><a:r><a:t>Wrong known namespace</a:t></a:r></a:p></a:body></a:document>"#,
            br#"<w:document><w:body><w:p><w:r><w:t>Undeclared prefix</w:t></w:r></w:p></w:body></w:document>"#,
        ] {
            assert_eq!(extract("docx", &minimal_docx(xml), 1024), Err(DocumentError::Invalid));
        }

        let shadowed = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>Visible</w:t></w:r></w:p><w:p xmlns:w="urn:attacker:shadow"><w:r><w:t>Must not leak</w:t></w:r></w:p><r:body><r:p><r:t>Relationship namespace text</r:t></r:p></r:body><a:body><a:p><a:r><a:t>Drawing structure collision</a:t></a:r></a:p></a:body></w:body></w:document>"#,
        );
        let shadowed_text = extract("docx", &shadowed, 1024).unwrap();
        assert!(shadowed_text.contains("Visible"));
        assert!(!shadowed_text.contains("Must not leak"));
        assert!(!shadowed_text.contains("Relationship namespace text"));
        assert!(!shadowed_text.contains("Drawing structure collision"));

        let extension = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:ext="urn:vendor:extension"><w:body><w:p><w:r><w:t>Visible</w:t></w:r><ext:t>Extension text</ext:t></w:p></w:body></w:document>"#,
        );
        let text = extract("docx", &extension, 1024).unwrap();
        assert!(text.contains("Visible"));
        assert!(!text.contains("Extension text"));

        let wrong_content_type = replace_zip_part(
            &valid,
            "[Content_Types].xml",
            br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.attacker-wordprocessingml.document.main+xml"/></Types>"#,
        );
        assert_eq!(
            extract("docx", &wrong_content_type, 1024),
            Err(DocumentError::MismatchedFormat)
        );

        let wrong_relationship_namespace = replace_zip_part(
            &valid,
            "_rels/.rels",
            br#"<Relationships xmlns="urn:attacker:package-relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        );
        assert_eq!(
            extract("docx", &wrong_relationship_namespace, 1024),
            Err(DocumentError::Invalid)
        );

        let wrong_content_namespace = replace_zip_part(
            &valid,
            "[Content_Types].xml",
            br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><x:Override xmlns:x="urn:attacker:content-types" PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        );
        assert_eq!(
            extract("docx", &wrong_content_namespace, 1024),
            Err(DocumentError::MismatchedFormat)
        );
    }

    #[test]
    fn docx_accepts_prefix_aliases_default_namespaces_and_strict_word_namespace() {
        let aliased = minimal_docx(
            br#"<word:document xmlns:word="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><word:body><word:p><word:r><word:t>Aliased text</word:t></word:r></word:p></word:body></word:document>"#,
        );
        assert!(extract("docx", &aliased, 1024)
            .unwrap()
            .contains("Aliased text"));

        let default_ns = minimal_docx(
            br#"<document xmlns="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><body><p><r><t>Default namespace</t></r></p></body></document>"#,
        );
        assert!(extract("docx", &default_ns, 1024)
            .unwrap()
            .contains("Default namespace"));

        let strict = minimal_docx(
            br#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"><w:body><w:p><w:r><w:t>Strict text</w:t></w:r></w:p></w:body></w:document>"#,
        );
        assert!(extract("docx", &strict, 1024)
            .unwrap()
            .contains("Strict text"));
    }

    #[test]
    fn pptx_requires_presentation_namespaces_and_exact_main_content_type() {
        let valid = minimal_pptx(&["PPTX text"]);
        assert!(extract("pptx", &valid, 1024).unwrap().contains("PPTX text"));

        let fake_main = replace_zip_part(
            &valid,
            "ppt/presentation.xml",
            br#"<p:presentation xmlns:p="urn:attacker:fake-presentationml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="slide1"/></p:sldIdLst></p:presentation>"#,
        );
        assert_eq!(
            extract("pptx", &fake_main, 1024),
            Err(DocumentError::Invalid)
        );

        let fake_relationship_id = replace_zip_part(
            &valid,
            "ppt/presentation.xml",
            br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:evil="urn:attacker:relationships"><p:sldIdLst><p:sldId id="256" evil:id="slide1"/></p:sldIdLst></p:presentation>"#,
        );
        assert_eq!(
            extract("pptx", &fake_relationship_id, 1024),
            Err(DocumentError::Invalid)
        );

        let unqualified_relationship_id = replace_zip_part(
            &valid,
            "ppt/presentation.xml",
            br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" relationship_id="slide1"/></p:sldIdLst></p:presentation>"#,
        );
        assert_eq!(
            extract("pptx", &unqualified_relationship_id, 1024),
            Err(DocumentError::Invalid)
        );

        let fake_relationship_type = replace_zip_part(
            &valid,
            "_rels/.rels",
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="root" Type="http://attacker.invalid/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#,
        );
        assert_eq!(
            extract("pptx", &fake_relationship_type, 1024),
            Err(DocumentError::MismatchedFormat)
        );

        let fake_slide = replace_zip_part(
            &valid,
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="urn:attacker:drawing"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Must not leak</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
        );
        assert_eq!(
            extract("pptx", &fake_slide, 1024),
            Err(DocumentError::NoText)
        );

        let wrong_content_type = replace_zip_part(
            &valid,
            "[Content_Types].xml",
            br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml-extra"/></Types>"#,
        );
        assert_eq!(
            extract("pptx", &wrong_content_type, 1024),
            Err(DocumentError::MismatchedFormat)
        );
    }

    #[test]
    fn pptx_accepts_namespace_aliases_default_namespaces_and_strict_ooxml() {
        let base = minimal_pptx(&["Presentation text"]);
        let aliased = replace_zip_part(
            &replace_zip_part(
                &base,
                "ppt/presentation.xml",
                br#"<deck:presentation xmlns:deck="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:rel="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><deck:sldIdLst><deck:sldId id="256" rel:id="slide1"/></deck:sldIdLst></deck:presentation>"#,
            ),
            "ppt/slides/slide1.xml",
            br#"<slide:sld xmlns:slide="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:draw="http://schemas.openxmlformats.org/drawingml/2006/main"><slide:cSld><slide:spTree><slide:sp><slide:txBody><draw:p><draw:r><draw:t>Alias slide text</draw:t></draw:r></draw:p></slide:txBody></slide:sp></slide:spTree></slide:cSld></slide:sld>"#,
        );
        assert!(extract("pptx", &aliased, 1024)
            .unwrap()
            .contains("Alias slide text"));

        let strict = replace_zip_part(
            &replace_zip_part(
                &replace_zip_part(
                    &replace_zip_part(
                        &replace_zip_part(
                            &replace_zip_part(
                                &base,
                                "[Content_Types].xml",
                                br#"<Types xmlns="http://purl.oclc.org/ooxml/package/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#,
                            ),
                            "_rels/.rels",
                            br#"<Relationships xmlns="http://purl.oclc.org/ooxml/package/relationships"><Relationship Id="root" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#,
                        ),
                        "ppt/presentation.xml",
                        br#"<p:presentation xmlns:p="http://purl.oclc.org/ooxml/presentationml/main" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships"><p:sldIdLst><p:sldId id="256" r:id="slide1"/></p:sldIdLst></p:presentation>"#,
                    ),
                    "ppt/_rels/presentation.xml.rels",
                    br#"<Relationships xmlns="http://purl.oclc.org/ooxml/package/relationships"><Relationship Id="slide1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#,
                ),
                "ppt/slides/slide1.xml",
                br#"<p:sld xmlns:p="http://purl.oclc.org/ooxml/presentationml/main" xmlns:a="http://purl.oclc.org/ooxml/drawingml/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Strict slide text</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
            ),
            "ppt/slides/_rels/slide1.xml.rels",
            br#"<Relationships xmlns="http://purl.oclc.org/ooxml/package/relationships"></Relationships>"#,
        );
        assert!(extract("pptx", &strict, 1024)
            .unwrap()
            .contains("Strict slide text"));
    }

    #[test]
    fn blank_slides_do_not_create_text_and_keep_positions_when_other_slides_have_text() {
        let blank = minimal_pptx(&["", ""]);
        assert_eq!(extract("pptx", &blank, 1024), Err(DocumentError::NoText));

        let mixed = minimal_pptx(&["", "Visible content", ""]);
        let text = extract("pptx", &mixed, 1024).unwrap();
        assert!(text.contains("Slide 1"));
        assert!(text.contains("Slide 2"));
        assert!(text.contains("Slide 3"));
        assert!(text.contains("Visible content"));
    }

    #[test]
    fn image_only_ooxml_documents_have_no_extractable_text() {
        assert_eq!(
            extract(
                "docx",
                include_bytes!("../../tests/fixtures/office/image-only.docx"),
                4096,
            ),
            Err(DocumentError::NoText)
        );
        assert_eq!(
            extract(
                "pptx",
                include_bytes!("../../tests/fixtures/office/image-only.pptx"),
                4096,
            ),
            Err(DocumentError::NoText)
        );
    }

    #[test]
    fn docx_includes_only_referenced_notes_and_referenced_header_parts() {
        let archive = minimal_docx_with_notes();
        let text = extract("docx", &archive, 1024).unwrap();
        assert!(text.contains("Body text"));
        assert!(text.contains("Header text"));
        assert!(text.contains("Referenced footnote"));
        assert!(!text.contains("Unreferenced footnote"));
        assert!(!text.contains("Deleted phrase"));
        assert!(!text.contains("Deleted note"));
        assert!(!text.contains("Moved note"));
        assert!(text.contains("Referenced footnote\tsecond\nthird"));
    }

    #[test]
    fn alternate_content_selects_one_supported_or_fallback_branch_recursively() {
        let word_xml = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:alias="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:future="urn:unknown"><w:body><w:p><mc:AlternateContent><mc:Choice Requires="future"><w:r><w:t>wrong future branch</w:t></w:r></mc:Choice><mc:Choice Requires="alias"><w:r><w:t>chosen branch</w:t></w:r><mc:AlternateContent><mc:Choice Requires="future"><w:r><w:t>nested wrong</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>nested fallback</w:t></w:r></mc:Fallback></mc:AlternateContent></mc:Choice><mc:Fallback><w:r><w:t>outer fallback</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"#;
        let docx = minimal_docx(word_xml);
        let text = extract("docx", &docx, 4096).unwrap();
        assert!(text.contains("chosen branch"), "{text}");
        assert!(text.contains("nested fallback"), "{text}");
        assert!(!text.contains("wrong future"), "{text}");
        assert!(!text.contains("outer fallback"), "{text}");

        let fallback = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x="urn:unknown"><w:body><w:p><mc:AlternateContent><mc:Choice Requires="x"><w:r><w:t>unsupported</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>supported fallback</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"#,
        );
        assert!(extract("docx", &fallback, 4096)
            .unwrap()
            .contains("supported fallback"));

        let no_branch = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x="urn:unknown"><w:body><w:p><mc:AlternateContent><mc:Choice Requires="x"><w:r><w:t>unsupported</w:t></w:r></mc:Choice></mc:AlternateContent></w:p></w:body></w:document>"#,
        );
        assert_eq!(
            extract("docx", &no_branch, 4096),
            Err(DocumentError::Unsupported)
        );
    }

    #[test]
    fn pptx_only_extracts_live_chart_references_and_rejects_duplicate_slide_targets() {
        let slide = br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:future="urn:unknown"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Visible slide text</a:t></a:r></a:p></p:txBody></p:sp><mc:AlternateContent><mc:Choice Requires="future"><c:chart r:id="stale"/></mc:Choice><mc:Fallback><c:chart r:id="live"/></mc:Fallback></mc:AlternateContent></p:spTree></p:cSld></p:sld>"#;
        let archive = minimal_pptx_with_chart_refs(slide, false);
        let text = extract("pptx", &archive, 4096).unwrap();
        assert!(text.contains("Visible slide text"), "{text}");
        assert!(text.contains("Live chart value"), "{text}");
        assert!(!text.contains("Stale orphan chart"), "{text}");
        assert!(text.contains("Chart data"), "{text}");

        for chart_value in [
            "<c:v>Live chart value</c:v>",
            "<c:v>Live chart&#x20;value</c:v>",
            "<c:v><![CDATA[Live chart value]]></c:v>",
        ] {
            let chart = format!(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:tx>{chart_value}</c:tx></c:chartSpace>"#
            );
            let chart_archive = replace_zip_part(&archive, "ppt/charts/live.xml", chart.as_bytes());
            assert_eq!(extract("pptx", &chart_archive, 4096).unwrap(), text);
        }

        for forged_chart in [
            String::from_utf8_lossy(slide)
                .replace("<c:chart", "<a:chart")
                .into_bytes(),
            String::from_utf8_lossy(slide)
                .replace("<c:chart", "<p:chart")
                .into_bytes(),
        ] {
            let forged_archive = minimal_pptx_with_chart_refs(&forged_chart, false);
            let forged_text = extract("pptx", &forged_archive, 4096).unwrap();
            assert!(forged_text.contains("Visible slide text"), "{forged_text}");
            assert!(!forged_text.contains("Live chart value"), "{forged_text}");
            assert!(!forged_text.contains("Chart data"), "{forged_text}");
        }

        let duplicate_target = minimal_pptx_with_chart_refs(slide, true);
        assert_eq!(
            extract("pptx", &duplicate_target, 4096),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn rejects_invalid_xml_characters_and_duplicate_referenced_note_ids() {
        let invalid_character = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>bad &#0; character</w:t></w:r></w:p></w:body></w:document>"#,
        );
        assert_eq!(
            extract("docx", &invalid_character, 4096),
            Err(DocumentError::Invalid)
        );

        let duplicate = br#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnote w:id="4"><w:p><w:r><w:t>one</w:t></w:r></w:p></w:footnote><w:footnote w:id="4"><w:p><w:r><w:t>two</w:t></w:r></w:p></w:footnote></w:footnotes>"#;
        assert_eq!(
            referenced_note_text(
                duplicate,
                &HashSet::from(["4".to_string()]),
                "footnote",
                4096
            ),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn namespace_declarations_are_scoped_without_copying_the_full_map_per_element() {
        let mut declarations = String::new();
        for index in 0..127 {
            declarations.push_str(&format!(" xmlns:n{index}=\"urn:test:{index}\""));
        }
        let mut children = String::new();
        for _ in 0..4_000 {
            children.push_str("<w:r><w:t>x</w:t></w:r>");
        }
        let xml = format!(
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"{declarations}><w:body><w:p>{children}</w:p></w:body></w:document>"
        );
        let document = minimal_docx(xml.as_bytes());
        let text = extract("docx", &document, 16_000).unwrap();
        assert_eq!(text.matches('x').count(), 4_000);
    }

    #[test]
    fn rejects_excessive_attributes_and_requires_prefixes_through_docx_extraction() {
        let mut attributes = String::new();
        for index in 0..MAX_XML_ATTRIBUTES_PER_ELEMENT {
            attributes.push_str(&format!(" x{index}=\"value\""));
        }
        let too_many_attributes = minimal_docx(
            format!(
                "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"{attributes}><w:body><w:p><w:r><w:t>text</w:t></w:r></w:p></w:body></w:document>"
            )
            .as_bytes(),
        );
        assert_eq!(
            extract("docx", &too_many_attributes, 4096),
            Err(DocumentError::Unsupported)
        );

        let requires = std::iter::repeat_n("x", MAX_CHOICE_PREFIXES + 1)
            .collect::<Vec<_>>()
            .join(" ");
        let too_many_prefixes = minimal_docx(
            format!(
                "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:x=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><mc:AlternateContent><mc:Choice Requires=\"{requires}\"><w:r><w:t>text</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>fallback</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"
            )
            .as_bytes(),
        );
        assert_eq!(
            extract("docx", &too_many_prefixes, 4096),
            Err(DocumentError::Unsupported)
        );
    }

    #[test]
    fn preserves_literal_entity_sequences_and_word_breaks() {
        let document = minimal_docx(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Literal &amp;lt; tag</w:t><w:tab/><w:t>second</w:t><w:br/><w:t>third</w:t></w:r></w:p></w:body></w:document>"#,
        );
        let text = extract("docx", &document, 1024).unwrap();
        assert!(
            text.contains("Literal &lt; tag\tsecond\nthird"),
            "unexpected output: {text}"
        );
    }

    #[test]
    fn rejects_wrong_family_malformed_xml_and_encrypted_ooxml() {
        assert_eq!(
            extract("docx", PRODUCER_PPTX, 1024),
            Err(DocumentError::MismatchedFormat)
        );
        let malformed = minimal_docx(b"<w:document><w:body><w:p><w:r><w:t>open");
        assert_eq!(
            extract("docx", &malformed, 1024),
            Err(DocumentError::Invalid)
        );
        assert_eq!(
            extract("docx", &minimal_docx(b"<w:wrong/>"), 1024),
            Err(DocumentError::Invalid)
        );
        assert_eq!(
            extract("docx", &minimal_docx(b""), 1024),
            Err(DocumentError::Invalid)
        );
        assert_eq!(
            xml_to_text(b"", 1024, XmlFlavor::Drawing),
            Err(DocumentError::Invalid)
        );
        let encrypted = encrypted_package();
        assert_eq!(
            extract("docx", &encrypted, 1024),
            Err(DocumentError::Encrypted)
        );
    }

    #[test]
    fn rejects_dtd_custom_entities_duplicate_parts_and_relationship_ids() {
        let dtd = minimal_docx(
            br#"<!DOCTYPE w:document [<!ENTITY x "unsafe">]><w:document><w:body><w:p><w:r><w:t>&x;</w:t></w:r></w:p></w:body></w:document>"#,
        );
        assert_eq!(extract("docx", &dtd, 1024), Err(DocumentError::Unsupported));

        let duplicate_parts = zip_entries([
            ("[Content_Types].xml".into(), content_types_docx()),
            ("_rels/.rels".into(), root_relationships_docx()),
            ("word/document.xml".into(), docx_body("first")),
            ("word/./document.xml".into(), docx_body("second")),
        ]);
        assert_eq!(
            extract("docx", &duplicate_parts, 1024),
            Err(DocumentError::Invalid)
        );

        let duplicate_relationship_ids = zip_entries([
            ("[Content_Types].xml".into(), content_types_docx()),
            (
                "_rels/.rels".into(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="dup" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="dup" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/other" Target="other.xml"/></Relationships>"#.to_vec(),
            ),
            ("word/document.xml".into(), docx_body("text")),
        ]);
        assert_eq!(
            extract("docx", &duplicate_relationship_ids, 1024),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn rejects_missing_referenced_parts_and_xml_depth_over_limit() {
        let missing_header = zip_entries([
            ("[Content_Types].xml".into(), content_types_docx()),
            ("_rels/.rels".into(), root_relationships_docx()),
            (
                "word/_rels/document.xml.rels".into(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="h" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="missing-header.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "word/document.xml".into(),
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:pPr><w:headerReference r:id="h"/></w:pPr><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#.to_vec(),
            ),
        ]);
        assert_eq!(
            extract("docx", &missing_header, 1024),
            Err(DocumentError::Invalid)
        );

        let nested = format!(
            "<w:document><w:body>{}<w:t>deep</w:t>{}</w:body></w:document>",
            "<x>".repeat(MAX_XML_DEPTH),
            "</x>".repeat(MAX_XML_DEPTH),
        );
        assert_eq!(
            extract("docx", &minimal_docx(nested.as_bytes()), 1024),
            Err(DocumentError::Unsupported)
        );
    }

    #[test]
    fn enforces_archive_entry_part_and_text_limits() {
        let too_many_entries =
            zip_entries((0..=MAX_DOCUMENT_ENTRIES).map(|i| (format!("x/{i}"), vec![])));
        assert_eq!(
            extract("docx", &too_many_entries, 1024),
            Err(DocumentError::ExpandedTooLarge)
        );

        let oversized_part = zip_entries([(
            "[Content_Types].xml".to_string(),
            vec![b'x'; MAX_PART_BYTES as usize + 1],
        )]);
        assert_eq!(
            extract("docx", &oversized_part, 1024),
            Err(DocumentError::ExpandedTooLarge)
        );

        let large_text = minimal_docx(format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:body></w:document>"#,
            "x".repeat(200),
        ).as_bytes());
        assert_eq!(
            extract("docx", &large_text, 64),
            Err(DocumentError::TextTooLarge)
        );
    }

    fn minimal_docx(document: &[u8]) -> Vec<u8> {
        zip_entries([
            (
                "[Content_Types].xml".to_string(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec(),
            ),
            (
                "_rels/.rels".to_string(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec(),
            ),
            ("word/document.xml".to_string(), document.to_vec()),
        ])
    }

    fn content_types_docx() -> Vec<u8> {
        br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec()
    }

    fn root_relationships_docx() -> Vec<u8> {
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec()
    }

    fn docx_body(text: &str) -> Vec<u8> {
        format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
        )
        .into_bytes()
    }

    fn minimal_pptx(slides: &[&str]) -> Vec<u8> {
        let mut entries = vec![
            (
                "[Content_Types].xml".to_string(),
                format!(
                    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>{}</Types>"#,
                    (1..=slides.len()).map(|index| format!(r#"<Override PartName="/ppt/slides/slide{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#)).collect::<String>(),
                ).into_bytes(),
            ),
            (
                "_rels/.rels".to_string(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="root" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "ppt/presentation.xml".to_string(),
                format!(
                    r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst>{}</p:sldIdLst></p:presentation>"#,
                    (1..=slides.len()).map(|index| format!(r#"<p:sldId id="{}" r:id="slide{index}"/>"#, 255 + index)).collect::<String>(),
                ).into_bytes(),
            ),
            (
                "ppt/_rels/presentation.xml.rels".to_string(),
                format!(
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#,
                    (1..=slides.len()).map(|index| format!(r#"<Relationship Id="slide{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{index}.xml"/>"#)).collect::<String>(),
                ).into_bytes(),
            ),
        ];
        entries.extend(slides.iter().enumerate().map(|(index, text)| (
            format!("ppt/slides/slide{}.xml", index + 1),
            format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).into_bytes(),
        )));
        zip_entries(entries)
    }

    fn minimal_pptx_with_chart_refs(slide_xml: &[u8], duplicate_target: bool) -> Vec<u8> {
        let slide_ids = if duplicate_target {
            r#"<p:sldId id="256" r:id="slide1"/><p:sldId id="257" r:id="slide2"/>"#
        } else {
            r#"<p:sldId id="256" r:id="slide1"/>"#
        };
        let slide_relationships = if duplicate_target {
            r#"<Relationship Id="slide1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide.xml"/><Relationship Id="slide2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/./slide.xml"/>"#
        } else {
            r#"<Relationship Id="slide1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide.xml"/>"#
        };
        zip_entries([
            (
                "[Content_Types].xml".into(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#.to_vec(),
            ),
            (
                "_rels/.rels".into(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="root" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "ppt/presentation.xml".into(),
                format!(r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst>{slide_ids}</p:sldIdLst></p:presentation>"#).into_bytes(),
            ),
            (
                "ppt/_rels/presentation.xml.rels".into(),
                format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{slide_relationships}</Relationships>"#).into_bytes(),
            ),
            ("ppt/slides/slide.xml".into(), slide_xml.to_vec()),
            (
                "ppt/slides/_rels/slide.xml.rels".into(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="live" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/live.xml"/><Relationship Id="stale" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/stale.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "ppt/charts/live.xml".into(),
                br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:tx><c:v>Live chart value</c:v></c:tx></c:chartSpace>"#.to_vec(),
            ),
            (
                "ppt/charts/stale.xml".into(),
                br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:tx><c:v>Stale orphan chart</c:v></c:tx></c:chartSpace>"#.to_vec(),
            ),
        ])
    }

    fn minimal_docx_with_notes() -> Vec<u8> {
        zip_entries([
            (
                "[Content_Types].xml".to_string(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#.to_vec(),
            ),
            (
                "_rels/.rels".to_string(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "word/_rels/document.xml.rels".to_string(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="head" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="fn" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "word/document.xml".to_string(),
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>Body text</w:t></w:r><w:r><w:footnoteReference w:id="4"/></w:r></w:p><w:del><w:r><w:delText>Deleted phrase</w:delText></w:r></w:del><w:sectPr><w:headerReference w:type="default" r:id="head"/></w:sectPr></w:body></w:document>"#.to_vec(),
            ),
            (
                "word/header1.xml".to_string(),
                br#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Header text</w:t></w:r></w:p></w:hdr>"#.to_vec(),
            ),
            (
                "word/footnotes.xml".to_string(),
                br#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnote w:id="-1"><w:p><w:r><w:t>Separator</w:t></w:r></w:p></w:footnote><w:footnote w:id="4"><w:p><w:r><w:t>Referenced footnote</w:t><w:del><w:r><w:delText>Deleted note</w:delText></w:r></w:del><w:moveFrom><w:r><w:t>Moved note</w:t></w:r></w:moveFrom><w:tab/><w:t>second</w:t><w:br/><w:t>third</w:t></w:r></w:p></w:footnote><w:footnote w:id="8"><w:p><w:r><w:t>Unreferenced footnote</w:t></w:r></w:p></w:footnote></w:footnotes>"#.to_vec(),
            ),
        ])
    }

    fn encrypted_package() -> Vec<u8> {
        let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        compound
            .create_stream("/EncryptionInfo")
            .unwrap()
            .write_all(b"info")
            .unwrap();
        compound
            .create_stream("/EncryptedPackage")
            .unwrap()
            .write_all(b"package")
            .unwrap();
        compound.into_inner().into_inner()
    }

    fn zip_entries(entries: impl IntoIterator<Item = (String, Vec<u8>)>) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();
        for (name, bytes) in entries {
            archive.start_file(name, options).unwrap();
            archive.write_all(&bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    fn replace_zip_part(bytes: &[u8], name: &str, replacement: &[u8]) -> Vec<u8> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut entries = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let entry_name = file
                .name()
                .expect("fixture entry name is valid")
                .into_owned();
            let mut content = Vec::new();
            file.read_to_end(&mut content).unwrap();
            if entry_name == name {
                content = replacement.to_vec();
            }
            entries.push((entry_name, content));
        }
        zip_entries(entries)
    }
}
