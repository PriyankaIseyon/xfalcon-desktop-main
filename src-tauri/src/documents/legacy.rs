//! Strict, bounded extraction for the legacy OLE Word and PowerPoint formats.
//!
//! The parsers follow the current document structures. They do not scan arbitrary
//! CFB streams for strings, since those streams can contain deleted or superseded
//! content.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use super::{DocumentError, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_ENTRIES, MAX_DOCUMENT_TEXT_BYTES};

const MAX_RECORDS: usize = 1_000_000;
const MAX_RECURSION: usize = 128;
const MAX_EDIT_CHAIN: usize = 512;
const PPT_STREAM_LIMIT: u64 = 32 * 1024 * 1024;
const WORD_STREAM_LIMIT: u64 = 32 * 1024 * 1024;

pub(super) fn extract_legacy(
    extension: &str,
    bytes: &[u8],
    max_output: usize,
) -> Result<String, DocumentError> {
    if bytes.is_empty() {
        return Err(DocumentError::Empty);
    }
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::InputTooLarge);
    }
    let max_output = max_output.min(MAX_DOCUMENT_TEXT_BYTES);
    match extension {
        "ppt" => extract_ppt(bytes, max_output),
        "doc" => {
            const REVISION_LIMITATION: &str =
                "[Legacy Word limitation: tracked deletions may appear in extracted text.]\n\n";
            let body = extract_doc(bytes, max_output.saturating_sub(REVISION_LIMITATION.len()))?;
            let mut output = BoundedText::new(max_output);
            output.push(REVISION_LIMITATION)?;
            output.push(&body)?;
            Ok(output.text)
        }
        _ => Err(DocumentError::MismatchedFormat),
    }
}

fn open_cfb(bytes: &[u8]) -> Result<cfb::CompoundFile<Cursor<&[u8]>>, DocumentError> {
    let cfb = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|_| DocumentError::Invalid)?;
    if cfb.walk().take(MAX_DOCUMENT_ENTRIES + 1).count() > MAX_DOCUMENT_ENTRIES {
        return Err(DocumentError::ExpandedTooLarge);
    }
    Ok(cfb)
}

fn read_stream(
    cfb: &mut cfb::CompoundFile<Cursor<&[u8]>>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, DocumentError> {
    if !cfb.is_stream(name) {
        return Err(DocumentError::Invalid);
    }
    let entry = cfb.entry(name).map_err(|_| DocumentError::Invalid)?;
    let len = entry.len();
    if len > limit {
        return Err(DocumentError::ExpandedTooLarge);
    }
    let cap = usize::try_from(len).map_err(|_| DocumentError::ExpandedTooLarge)?;
    let mut stream = cfb.open_stream(name).map_err(|_| DocumentError::Invalid)?;
    let mut data = Vec::with_capacity(cap);
    (&mut stream)
        .take(limit.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(|_| DocumentError::Invalid)?;
    if data.len() as u64 != len {
        return Err(DocumentError::Invalid);
    }
    Ok(data)
}

// -----------------------------------------------------------------------------
// PowerPoint 97-2003 (MS-PPT)

#[derive(Clone, Copy)]
struct Record<'a> {
    kind: u16,
    instance: u16,
    version: u16,
    body: &'a [u8],
    start: usize,
    end: usize,
}

fn record_at(bytes: &[u8], offset: usize, end: usize) -> Result<Record<'_>, DocumentError> {
    let header_end = offset.checked_add(8).ok_or(DocumentError::Invalid)?;
    if header_end > end || end > bytes.len() {
        return Err(DocumentError::Invalid);
    }
    let options = u16le(bytes, offset)?;
    let kind = u16le(bytes, offset + 2)?;
    let len = usize::try_from(u32le(bytes, offset + 4)?).map_err(|_| DocumentError::Invalid)?;
    let body_end = header_end.checked_add(len).ok_or(DocumentError::Invalid)?;
    if body_end > end {
        return Err(DocumentError::Invalid);
    }
    Ok(Record {
        kind,
        instance: options >> 4,
        version: options & 0xF,
        body: &bytes[header_end..body_end],
        start: offset,
        end: body_end,
    })
}

fn children<'a>(bytes: &'a [u8], parent: Record<'a>) -> Result<Vec<Record<'a>>, DocumentError> {
    let mut result = Vec::new();
    let mut cursor = parent.start + 8;
    while cursor < parent.end {
        let record = record_at(bytes, cursor, parent.end)?;
        if record.end <= cursor {
            return Err(DocumentError::Invalid);
        }
        result.push(record);
        if result.len() > MAX_RECORDS {
            return Err(DocumentError::ExpandedTooLarge);
        }
        cursor = record.end;
    }
    if cursor != parent.end {
        return Err(DocumentError::Invalid);
    }
    Ok(result)
}

#[derive(Default)]
struct PersistDirectory {
    offsets: HashMap<u32, usize>,
}

fn extract_ppt(bytes: &[u8], max_output: usize) -> Result<String, DocumentError> {
    let mut cfb = open_cfb(bytes)?;
    let mut presentation = read_stream(&mut cfb, "PowerPoint Document", PPT_STREAM_LIMIT)?;
    let current_user = read_stream(&mut cfb, "Current User", 1 << 20)?;
    if current_user.len() < 28 {
        return Err(DocumentError::Invalid);
    }
    let current = record_at(&current_user, 0, current_user.len())?;
    if current.kind != 0x0FF6 || current.version != 0 || current.body.len() < 20 {
        return Err(DocumentError::MismatchedFormat);
    }
    if u32le(current.body, 0)? != 20 {
        return Err(DocumentError::Invalid);
    }
    let header_token = u32le(current.body, 4)?;
    if header_token == 0xF3D1_C4DF {
        return Err(DocumentError::Encrypted);
    }
    if header_token != 0xE391_C05F {
        return Err(DocumentError::MismatchedFormat);
    }
    let mut edit_offset =
        usize::try_from(u32le(current.body, 8)?).map_err(|_| DocumentError::Invalid)?;
    let mut seen_edits = HashSet::new();
    let mut seen_directories = HashSet::new();
    let mut total_directory_bytes = 0u64;
    let mut persist = PersistDirectory::default();
    let mut document_id = None;
    for _ in 0..MAX_EDIT_CHAIN {
        if edit_offset == 0 {
            break;
        }
        if edit_offset >= presentation.len() || !seen_edits.insert(edit_offset) {
            return Err(DocumentError::Invalid);
        }
        let edit = record_at(&presentation, edit_offset, presentation.len())?;
        if edit.kind != 0x0FF5 || edit.version != 0 || edit.body.len() < 24 {
            return Err(DocumentError::Invalid);
        }
        let previous = usize::try_from(u32le(edit.body, 8)?).map_err(|_| DocumentError::Invalid)?;
        let directory_offset =
            usize::try_from(u32le(edit.body, 12)?).map_err(|_| DocumentError::Invalid)?;
        let this_document = u32le(edit.body, 16)?;
        if document_id.is_none() {
            document_id = Some(this_document);
        }
        let directory = record_at(&presentation, directory_offset, presentation.len())?;
        if directory.kind != 0x1772 || directory.version != 0 {
            return Err(DocumentError::Invalid);
        }
        if !seen_directories.insert(directory_offset) {
            return Err(DocumentError::Invalid);
        }
        total_directory_bytes = total_directory_bytes
            .checked_add(directory.body.len() as u64)
            .ok_or(DocumentError::ExpandedTooLarge)?;
        if total_directory_bytes > super::MAX_EXPANDED_BYTES {
            return Err(DocumentError::ExpandedTooLarge);
        }
        let mut pos = 0usize;
        while pos < directory.body.len() {
            let entry = u32le(directory.body, pos)?;
            pos += 4;
            let id = entry & 0x000F_FFFF;
            let count = (entry >> 20) as usize;
            if count == 0 || count > 4096 || id.checked_add(count as u32).is_none() {
                return Err(DocumentError::Invalid);
            }
            let bytes_needed = count.checked_mul(4).ok_or(DocumentError::Invalid)?;
            if pos
                .checked_add(bytes_needed)
                .ok_or(DocumentError::Invalid)?
                > directory.body.len()
            {
                return Err(DocumentError::Invalid);
            }
            for i in 0..count {
                let object_id = id + i as u32;
                let object_offset = usize::try_from(u32le(directory.body, pos + i * 4)?)
                    .map_err(|_| DocumentError::Invalid)?;
                if object_offset >= presentation.len() {
                    return Err(DocumentError::Invalid);
                }
                // Newer edit records take precedence over older versions.
                persist.offsets.entry(object_id).or_insert(object_offset);
            }
            pos += bytes_needed;
        }
        if previous == 0 {
            edit_offset = 0;
        } else {
            edit_offset = previous;
        }
    }
    if edit_offset != 0 || seen_edits.is_empty() {
        return Err(DocumentError::Invalid);
    }
    let document_offset = *persist
        .offsets
        .get(&document_id.ok_or(DocumentError::Invalid)?)
        .ok_or(DocumentError::Invalid)?;
    let document = record_at(&presentation, document_offset, presentation.len())?;
    if document.kind != 0x03E8 || document.version != 0xF {
        return Err(DocumentError::Invalid);
    }
    let document_children = children(&presentation, document)?;
    let mut slide_refs = Vec::new();
    let mut note_refs = Vec::new();
    for child in document_children {
        if child.kind == 0x0FF0 && child.version == 0xF {
            match child.instance {
                0 => slide_refs = parse_slide_list(&presentation, child, max_output)?,
                2 => note_refs = parse_slide_list(&presentation, child, max_output)?,
                _ => {}
            }
        }
    }
    if slide_refs.is_empty() {
        return Err(DocumentError::NoText);
    }
    let mut unique_slide_ids = HashSet::new();
    let mut unique_slide_persist_ids = HashSet::new();
    if slide_refs.iter().any(|slide| {
        !unique_slide_ids.insert(slide.slide_id)
            || !unique_slide_persist_ids.insert(slide.persist_id)
    }) {
        return Err(DocumentError::Invalid);
    }
    let mut unique_note_persist_ids = HashSet::new();
    if note_refs
        .iter()
        .any(|note| !unique_note_persist_ids.insert(note.persist_id))
    {
        return Err(DocumentError::Invalid);
    }
    let mut sink = BoundedText::new(max_output);
    let mut slide_ids = HashSet::new();
    let mut slide_notes = HashMap::new();
    // Every slide and notes page is its own persist object. Ids that share one
    // offset would make the walks below repeat once per alias.
    let mut used_offsets = HashSet::new();
    for (slide_index, slide) in slide_refs.iter().enumerate() {
        slide_ids.insert(slide.slide_id);
        let offset = *persist
            .offsets
            .get(&slide.persist_id)
            .ok_or(DocumentError::Invalid)?;
        if !used_offsets.insert(offset) {
            return Err(DocumentError::Invalid);
        }
        let record = record_at(&presentation, offset, presentation.len())?;
        if record.kind != 0x03EE || record.version != 0xF {
            return Err(DocumentError::Invalid);
        }
        let notes_id = find_nested_atom_u32(&presentation, record, 0x03EF, 16)?.unwrap_or(0);
        slide_notes.insert(slide.slide_id, notes_id);
        let references = find_outline_text_indices(&presentation, record)?;
        let cached = slide
            .cached_text
            .iter()
            .filter(|(index, _)| references.contains(index))
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let inline = collect_ppt_text(&presentation, record, sink.remaining())?;
        let text = normalize_legacy_text(&join_text(&cached, &inline, sink.remaining())?);
        if !text.trim().is_empty() {
            sink.label(&format!("Slide {}", slide_index + 1))?;
            sink.push(&text)?;
        }
    }
    for note in note_refs {
        let offset = *persist
            .offsets
            .get(&note.persist_id)
            .ok_or(DocumentError::Invalid)?;
        if !used_offsets.insert(offset) {
            return Err(DocumentError::Invalid);
        }
        let record = record_at(&presentation, offset, presentation.len())?;
        if record.kind != 0x03F0 || record.version != 0xF {
            return Err(DocumentError::Invalid);
        }
        let Some(source_slide_id) = find_nested_atom_u32(&presentation, record, 0x03F1, 0)? else {
            return Err(DocumentError::Invalid);
        };
        if !slide_ids.contains(&source_slide_id) {
            continue;
        }
        if slide_notes.get(&source_slide_id).copied().unwrap_or(0) != note.slide_id {
            return Err(DocumentError::Invalid);
        }
        let references = find_outline_text_indices(&presentation, record)?;
        let cached = note
            .cached_text
            .iter()
            .filter(|(index, _)| references.contains(index))
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let inline = collect_ppt_text(&presentation, record, sink.remaining())?;
        let text = normalize_legacy_text(&join_text(&cached, &inline, sink.remaining())?);
        if !text.trim().is_empty() {
            let slide_index = slide_refs
                .iter()
                .position(|slide| slide.slide_id == source_slide_id)
                .ok_or(DocumentError::Invalid)?;
            sink.label(&format!("Notes for slide {}", slide_index + 1))?;
            sink.push(&text)?;
        }
    }
    // Avoid retaining the input allocation while returning the extracted text.
    presentation.clear();
    sink.finish()
}

#[derive(Clone)]
struct SlideReference {
    persist_id: u32,
    slide_id: u32,
    expected_text_count: usize,
    cached_text: Vec<(u16, String)>,
}

fn parse_slide_list(
    bytes: &[u8],
    list: Record<'_>,
    output_limit: usize,
) -> Result<Vec<SlideReference>, DocumentError> {
    let records = children(bytes, list)?;
    let mut result = Vec::new();
    let mut total_cached = 0usize;
    let mut active_group = None;
    for record in records {
        if record.kind == 0x03F3 {
            if record.version != 0 || record.body.len() != 20 {
                return Err(DocumentError::Invalid);
            }
            if result.last().is_some_and(|previous: &SlideReference| {
                previous.cached_text.len() != previous.expected_text_count
            }) {
                return Err(DocumentError::Invalid);
            }
            let expected_text_count =
                usize::try_from(u32le(record.body, 8)?).map_err(|_| DocumentError::Invalid)?;
            if expected_text_count > 8 {
                return Err(DocumentError::Invalid);
            }
            result.push(SlideReference {
                persist_id: u32le(record.body, 0)?,
                slide_id: u32le(record.body, 12)?,
                expected_text_count,
                cached_text: Vec::with_capacity(expected_text_count),
            });
            active_group = None;
            if result.len() > MAX_RECORDS {
                return Err(DocumentError::ExpandedTooLarge);
            }
        } else if record.kind == 0x0F9F {
            if record.version != 0 || record.body.len() != 4 || record.instance > 5 {
                return Err(DocumentError::Invalid);
            }
            let Some(current) = result.last_mut() else {
                return Err(DocumentError::Invalid);
            };
            if current
                .cached_text
                .iter()
                .any(|(index, _)| *index == record.instance)
            {
                return Err(DocumentError::Invalid);
            }
            current.cached_text.push((record.instance, String::new()));
            active_group = Some(record.instance);
        } else if matches!(record.kind, 0x0FA0 | 0x0FA8) {
            let Some(group_index) = active_group else {
                return Err(DocumentError::Invalid);
            };
            let atom_text =
                decode_ppt_text_atom(record, output_limit.saturating_sub(total_cached))?;
            total_cached = total_cached
                .checked_add(atom_text.len().saturating_add(1))
                .ok_or(DocumentError::TextTooLarge)?;
            if total_cached > output_limit {
                return Err(DocumentError::TextTooLarge);
            }
            let current = result.last_mut().ok_or(DocumentError::Invalid)?;
            let group = current
                .cached_text
                .iter_mut()
                .find(|(index, _)| *index == group_index)
                .ok_or(DocumentError::Invalid)?;
            group.1.push_str(&atom_text);
            group.1.push('\n');
        }
    }
    if result
        .last()
        .is_some_and(|last| last.cached_text.len() != last.expected_text_count)
    {
        return Err(DocumentError::Invalid);
    }
    Ok(result)
}

fn find_outline_text_indices(
    bytes: &[u8],
    root: Record<'_>,
) -> Result<HashSet<u16>, DocumentError> {
    let mut indices = HashSet::new();
    let mut stack = vec![(root, 0usize)];
    let mut records_seen = 0usize;
    while let Some((record, depth)) = stack.pop() {
        if depth > MAX_RECURSION {
            return Err(DocumentError::ExpandedTooLarge);
        }
        records_seen += 1;
        if records_seen > MAX_RECORDS {
            return Err(DocumentError::ExpandedTooLarge);
        }
        if record.kind == 0x0F9E {
            if record.version != 0 || record.body.len() != 4 {
                return Err(DocumentError::Invalid);
            }
            let index =
                i32::from_le_bytes(record.body.try_into().map_err(|_| DocumentError::Invalid)?);
            if !(0..=5).contains(&index) || !indices.insert(index as u16) {
                return Err(DocumentError::Invalid);
            }
        } else if record.version == 0xF {
            for child in children(bytes, record)?.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    Ok(indices)
}

fn join_text(cached: &str, inline: &str, limit: usize) -> Result<String, DocumentError> {
    let mut sink = BoundedText::new(limit);
    if !cached.trim().is_empty() {
        sink.push(cached)?;
    }
    if !cached.trim().is_empty() && !inline.trim().is_empty() {
        sink.push("\n")?;
    }
    if !inline.trim().is_empty() {
        sink.push(inline)?;
    }
    Ok(sink.text)
}

/// `TextBytesAtom` holds ANSI text, which LibreOffice reads as Windows-1252
/// (`svdfppt.cxx`, `PPT_PST_TextBytesAtom`). Latin-1 would turn the curly
/// quotes, dashes and euro sign in 0x80-0x9F into control characters, and
/// `normalize_legacy_text` would then replace each of them with a space.
fn windows_1252_char(byte: u8) -> char {
    const C1: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}',
        '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    match byte {
        0x80..=0x9F => C1[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

fn decode_ppt_text_atom(record: Record<'_>, limit: usize) -> Result<String, DocumentError> {
    let mut text = BoundedText::new(limit);
    match record.kind {
        0x0FA0 => {
            if !record.body.len().is_multiple_of(2) {
                return Err(DocumentError::Invalid);
            }
            for decoded in char::decode_utf16(
                record
                    .body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
            ) {
                let character = decoded.map_err(|_| DocumentError::Invalid)?;
                if character == '\0' {
                    return Err(DocumentError::Invalid);
                }
                text.push_char(character)?;
            }
            Ok(text.text)
        }
        0x0FA8 => {
            for &byte in record.body {
                if byte == 0 {
                    return Err(DocumentError::Invalid);
                }
                text.push_char(windows_1252_char(byte))?;
            }
            Ok(text.text)
        }
        _ => Err(DocumentError::Invalid),
    }
}

fn find_nested_atom_u32(
    bytes: &[u8],
    root: Record<'_>,
    kind: u16,
    offset: usize,
) -> Result<Option<u32>, DocumentError> {
    let mut stack = vec![(root, 0usize)];
    let mut seen = 0usize;
    while let Some((record, depth)) = stack.pop() {
        if depth > MAX_RECURSION {
            return Err(DocumentError::ExpandedTooLarge);
        }
        seen += 1;
        if seen > MAX_RECORDS {
            return Err(DocumentError::ExpandedTooLarge);
        }
        if record.kind == kind && record.version != 0xF {
            return Ok(Some(u32le(record.body, offset)?));
        }
        if record.version == 0xF {
            for child in children(bytes, record)?.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    Ok(None)
}

fn collect_ppt_text(
    bytes: &[u8],
    root: Record<'_>,
    output_limit: usize,
) -> Result<String, DocumentError> {
    let mut sink = BoundedText::new(output_limit);
    let mut stack = vec![(root, 0usize)];
    let mut records_seen = 0usize;
    while let Some((record, depth)) = stack.pop() {
        if depth > MAX_RECURSION {
            return Err(DocumentError::ExpandedTooLarge);
        }
        records_seen += 1;
        if records_seen > MAX_RECORDS {
            return Err(DocumentError::ExpandedTooLarge);
        }
        if record.version == 0xF {
            let nested = children(bytes, record)?;
            for child in nested.into_iter().rev() {
                stack.push((child, depth + 1));
            }
            continue;
        }
        match record.kind {
            0x0FA0 | 0x0FA8 => {
                sink.push(&decode_ppt_text_atom(record, sink.remaining())?)?;
                sink.push_char('\n')?;
            }
            _ => {}
        }
    }
    sink.finish_or_empty()
}

// -----------------------------------------------------------------------------
// Word 97-2003 (MS-DOC)

fn extract_doc(bytes: &[u8], max_output: usize) -> Result<String, DocumentError> {
    let mut cfb = open_cfb(bytes)?;
    let word = read_stream(&mut cfb, "WordDocument", WORD_STREAM_LIMIT)?;
    if word.len() < 0x1AE {
        return Err(DocumentError::Invalid);
    }
    if u16le(&word, 0)? != 0xA5EC {
        return Err(DocumentError::MismatchedFormat);
    }
    let n_fib = u16le(&word, 2)?;
    if !(0x00C1..=0x0112).contains(&n_fib) {
        return Err(DocumentError::Unsupported);
    }
    let flags = u16le(&word, 10)?;
    if flags & 0x0100 != 0 || flags & 0x8000 != 0 {
        return Err(DocumentError::Encrypted);
    }
    let table_name = if flags & 0x0200 != 0 {
        "1Table"
    } else {
        "0Table"
    };
    let table = read_stream(&mut cfb, table_name, WORD_STREAM_LIMIT)?;
    let (stories, fc_clx, lcb_clx) = parse_fib(&word)?;
    let clx_end = fc_clx.checked_add(lcb_clx).ok_or(DocumentError::Invalid)?;
    let clx = table.get(fc_clx..clx_end).ok_or(DocumentError::Invalid)?;
    let pieces = parse_clx(clx, stories.total()?)?;
    let mut sink = BoundedText::new(max_output);
    let mut story_start = 0u32;
    for (label, count, include) in stories.ordered() {
        let story_end = story_start
            .checked_add(count)
            .ok_or(DocumentError::Invalid)?;
        if include {
            let text = decode_word_range(
                &word,
                &pieces,
                story_start,
                story_end,
                MAX_DOCUMENT_TEXT_BYTES,
            )?;
            let visible = normalize_legacy_text(&strip_field_instructions(&text)?);
            if !visible.trim().is_empty() {
                sink.label(label)?;
                sink.push(&visible)?;
            }
        }
        story_start = story_end;
    }
    sink.finish()
}

#[derive(Clone, Copy)]
struct Stories {
    main: u32,
    footnotes: u32,
    headers: u32,
    macro_text: u32,
    annotations: u32,
    endnotes: u32,
    textboxes: u32,
    header_textboxes: u32,
}

impl Stories {
    fn total(self) -> Result<u32, DocumentError> {
        self.ordered().into_iter().try_fold(0u32, |sum, (_, n, _)| {
            sum.checked_add(n).ok_or(DocumentError::Invalid)
        })
    }

    fn ordered(self) -> [(&'static str, u32, bool); 8] {
        [
            ("Body", self.main, true),
            ("Footnotes", self.footnotes, true),
            ("Headers and footers", self.headers, true),
            ("Macro text", self.macro_text, false),
            ("Comments", self.annotations, false),
            ("Endnotes", self.endnotes, true),
            ("Text boxes", self.textboxes, true),
            ("Header text boxes", self.header_textboxes, true),
        ]
    }
}

fn parse_fib(word: &[u8]) -> Result<(Stories, usize, usize), DocumentError> {
    let csw = usize::from(u16le(word, 32)?);
    if csw > 128 {
        return Err(DocumentError::Unsupported);
    }
    let rg_w_end = 34usize
        .checked_add(csw.checked_mul(2).ok_or(DocumentError::Invalid)?)
        .ok_or(DocumentError::Invalid)?;
    if rg_w_end + 2 > word.len() {
        return Err(DocumentError::Invalid);
    }
    let cslw = usize::from(u16le(word, rg_w_end)?);
    if cslw > 128 {
        return Err(DocumentError::Unsupported);
    }
    let rg_lw_start = rg_w_end + 2;
    let rg_lw_end = rg_lw_start
        .checked_add(cslw.checked_mul(4).ok_or(DocumentError::Invalid)?)
        .ok_or(DocumentError::Invalid)?;
    if rg_lw_end + 2 > word.len() || cslw < 11 {
        return Err(DocumentError::Invalid);
    }
    let ccp = |index: usize| -> Result<u32, DocumentError> {
        if index >= cslw {
            return Err(DocumentError::Invalid);
        }
        u32le(word, rg_lw_start + index * 4)
    };
    let stories = Stories {
        main: ccp(3)?,
        footnotes: ccp(4)?,
        headers: ccp(5)?,
        macro_text: ccp(6)?,
        annotations: ccp(7)?,
        endnotes: ccp(8)?,
        textboxes: ccp(9)?,
        header_textboxes: ccp(10)?,
    };
    let cb_rg_fc_lcb = usize::from(u16le(word, rg_lw_end)?);
    if cb_rg_fc_lcb > 256 {
        return Err(DocumentError::Unsupported);
    }
    let pairs_start = rg_lw_end + 2;
    let pairs_end = pairs_start
        .checked_add(cb_rg_fc_lcb.checked_mul(8).ok_or(DocumentError::Invalid)?)
        .ok_or(DocumentError::Invalid)?;
    if pairs_end > word.len() || cb_rg_fc_lcb <= 33 {
        return Err(DocumentError::Invalid);
    }
    let fc_clx =
        usize::try_from(u32le(word, pairs_start + 33 * 8)?).map_err(|_| DocumentError::Invalid)?;
    let lcb_clx = usize::try_from(u32le(word, pairs_start + 33 * 8 + 4)?)
        .map_err(|_| DocumentError::Invalid)?;
    if lcb_clx == 0 || lcb_clx > 16 * 1024 * 1024 {
        return Err(DocumentError::Invalid);
    }
    Ok((stories, fc_clx, lcb_clx))
}

#[derive(Clone, Copy)]
struct Piece {
    cp_start: u32,
    cp_end: u32,
    file_start: usize,
    compressed: bool,
}

fn parse_clx(clx: &[u8], expected_cps: u32) -> Result<Vec<Piece>, DocumentError> {
    let mut cursor = 0usize;
    while cursor < clx.len() && clx[cursor] == 0x01 {
        if cursor.checked_add(3).ok_or(DocumentError::Invalid)? > clx.len() {
            return Err(DocumentError::Invalid);
        }
        let len = usize::from(u16le(clx, cursor + 1)?);
        cursor = cursor.checked_add(3 + len).ok_or(DocumentError::Invalid)?;
    }
    if clx.get(cursor) != Some(&0x02) {
        return Err(DocumentError::Unsupported);
    }
    let pcdt_len = usize::try_from(u32le(clx, cursor + 1)?).map_err(|_| DocumentError::Invalid)?;
    let start = cursor.checked_add(5).ok_or(DocumentError::Invalid)?;
    let end = start.checked_add(pcdt_len).ok_or(DocumentError::Invalid)?;
    if end != clx.len() || pcdt_len < 16 || (pcdt_len - 4) % 12 != 0 {
        return Err(DocumentError::Invalid);
    }
    let count = (pcdt_len - 4) / 12;
    if count == 0 || count > MAX_DOCUMENT_ENTRIES * 16 {
        return Err(DocumentError::ExpandedTooLarge);
    }
    let cp_count = count + 1;
    let cp_bytes = cp_count.checked_mul(4).ok_or(DocumentError::Invalid)?;
    let mut cps = Vec::with_capacity(cp_count);
    for index in 0..cp_count {
        cps.push(u32le(clx, start + index * 4)?);
    }
    if cps.first() != Some(&0) || cps.last() != Some(&expected_cps) {
        // MS-DOC permits one terminal CP for the final paragraph mark.
        if cps.first() != Some(&0) || cps.last().copied() != expected_cps.checked_add(1) {
            return Err(DocumentError::Invalid);
        }
    }
    if cps.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(DocumentError::Invalid);
    }
    let pcds_start = start.checked_add(cp_bytes).ok_or(DocumentError::Invalid)?;
    if pcds_start + count * 8 != end {
        return Err(DocumentError::Invalid);
    }
    let mut pieces = Vec::with_capacity(count);
    for index in 0..count {
        let offset = pcds_start + index * 8;
        let fc_raw = u32le(clx, offset + 2)?;
        let compressed = fc_raw & 0x4000_0000 != 0;
        let raw_offset = fc_raw & 0x3FFF_FFFF;
        let file_start = usize::try_from(if compressed {
            raw_offset / 2
        } else {
            raw_offset
        })
        .map_err(|_| DocumentError::Invalid)?;
        pieces.push(Piece {
            cp_start: cps[index],
            cp_end: cps[index + 1],
            file_start,
            compressed,
        });
    }
    Ok(pieces)
}

fn decode_word_range(
    word: &[u8],
    pieces: &[Piece],
    start: u32,
    end: u32,
    max_output: usize,
) -> Result<String, DocumentError> {
    let mut text = String::new();
    for piece in pieces {
        let low = start.max(piece.cp_start);
        let high = end.min(piece.cp_end);
        if low >= high {
            continue;
        }
        let delta = usize::try_from(low - piece.cp_start).map_err(|_| DocumentError::Invalid)?;
        let units = usize::try_from(high - low).map_err(|_| DocumentError::Invalid)?;
        let stride = if piece.compressed { 1 } else { 2 };
        let byte_start = piece
            .file_start
            .checked_add(delta.checked_mul(stride).ok_or(DocumentError::Invalid)?)
            .ok_or(DocumentError::Invalid)?;
        let byte_len = units.checked_mul(stride).ok_or(DocumentError::Invalid)?;
        let bytes = word
            .get(
                byte_start
                    ..byte_start
                        .checked_add(byte_len)
                        .ok_or(DocumentError::Invalid)?,
            )
            .ok_or(DocumentError::Invalid)?;
        if piece.compressed {
            for byte in bytes {
                let character = fc_compressed_character(*byte);
                if text.len().saturating_add(character.len_utf8()) > max_output {
                    return Err(DocumentError::TextTooLarge);
                }
                text.push(character);
            }
        } else {
            if !bytes.len().is_multiple_of(2) {
                return Err(DocumentError::Invalid);
            }
            for decoded in char::decode_utf16(
                bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
            ) {
                let character = decoded.map_err(|_| DocumentError::Invalid)?;
                if text.len().saturating_add(character.len_utf8()) > max_output {
                    return Err(DocumentError::TextTooLarge);
                }
                text.push(character);
            }
        }
        if text.len() > max_output {
            return Err(DocumentError::TextTooLarge);
        }
    }
    Ok(text)
}

fn fc_compressed_character(byte: u8) -> char {
    let codepoint = match byte {
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9B => 0x203A,
        0x9C => 0x0153,
        0x9F => 0x0178,
        _ => u32::from(byte),
    };
    char::from_u32(codepoint).expect("FcCompressed mapping is a Unicode scalar")
}

fn strip_field_instructions(text: &str) -> Result<String, DocumentError> {
    let mut output = String::with_capacity(text.len());
    let mut field_states = Vec::new();
    let mut suppressed_fields = 0usize;
    for character in text.chars() {
        match character {
            '\u{0013}' => {
                if field_states.len() >= MAX_RECURSION {
                    return Err(DocumentError::ExpandedTooLarge);
                }
                field_states.push(false);
                suppressed_fields += 1;
            }
            '\u{0014}' => {
                let Some(result_state) = field_states.last_mut() else {
                    return Err(DocumentError::Invalid);
                };
                if *result_state {
                    return Err(DocumentError::Invalid);
                }
                *result_state = true;
                suppressed_fields -= 1;
            }
            '\u{0015}' => {
                let Some(completed_field) = field_states.pop() else {
                    return Err(DocumentError::Invalid);
                };
                if !completed_field {
                    suppressed_fields = suppressed_fields
                        .checked_sub(1)
                        .ok_or(DocumentError::Invalid)?;
                }
            }
            _ if suppressed_fields == 0 => output.push(character),
            _ => {}
        }
    }
    if !field_states.is_empty() {
        return Err(DocumentError::Invalid);
    }
    Ok(output)
}

fn normalize_legacy_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\r' | '\u{000B}' | '\u{000C}' => normalized.push('\n'),
            '\u{0007}' => normalized.push('\t'),
            '\n' | '\t' => normalized.push(character),
            '\u{FFFC}' => normalized.push(' '),
            character if character.is_control() => normalized.push(' '),
            _ => normalized.push(character),
        }
    }
    normalized
}

// -----------------------------------------------------------------------------

struct BoundedText {
    text: String,
    max_bytes: usize,
}

impl BoundedText {
    fn new(max_bytes: usize) -> Self {
        Self {
            text: String::new(),
            max_bytes,
        }
    }

    fn push(&mut self, value: &str) -> Result<(), DocumentError> {
        let new_len = self
            .text
            .len()
            .checked_add(value.len())
            .ok_or(DocumentError::TextTooLarge)?;
        if new_len > self.max_bytes {
            return Err(DocumentError::TextTooLarge);
        }
        self.text.push_str(value);
        Ok(())
    }

    fn push_char(&mut self, value: char) -> Result<(), DocumentError> {
        if self
            .text
            .len()
            .checked_add(value.len_utf8())
            .ok_or(DocumentError::TextTooLarge)?
            > self.max_bytes
        {
            return Err(DocumentError::TextTooLarge);
        }
        self.text.push(value);
        Ok(())
    }

    fn label(&mut self, label: &str) -> Result<(), DocumentError> {
        let prefix = if self.text.is_empty() { "" } else { "\n" };
        self.push(prefix)?;
        self.push("[")?;
        self.push(label)?;
        self.push("]\n")
    }

    fn finish(self) -> Result<String, DocumentError> {
        if self.text.trim().is_empty() {
            Err(DocumentError::NoText)
        } else {
            Ok(self.text)
        }
    }

    fn finish_or_empty(self) -> Result<String, DocumentError> {
        Ok(self.text)
    }

    fn remaining(&self) -> usize {
        self.max_bytes.saturating_sub(self.text.len())
    }
}

fn u16le(bytes: &[u8], offset: usize) -> Result<u16, DocumentError> {
    let end = offset.checked_add(2).ok_or(DocumentError::Invalid)?;
    let slice = bytes.get(offset..end).ok_or(DocumentError::Invalid)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32le(bytes: &[u8], offset: usize) -> Result<u32, DocumentError> {
    let end = offset.checked_add(4).ok_or(DocumentError::Invalid)?;
    let slice = bytes.get(offset..end).ok_or(DocumentError::Invalid)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
pub(super) fn synthetic_blank_ppt_fixture() -> Vec<u8> {
    tests::synthetic_incremental_ppt(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};

    const POI_DOC: &[u8] = include_bytes!("../../tests/fixtures/office/legacy-poi/simple.doc");
    const POI_PPT: &[u8] = include_bytes!("../../tests/fixtures/office/legacy-poi/text_shapes.ppt");
    const POI_NOTES_PPT: &[u8] =
        include_bytes!("../../tests/fixtures/office/legacy-poi/basic_test_ppt_file.ppt");
    const POI_TABLE_DOC: &[u8] =
        include_bytes!("../../tests/fixtures/office/legacy-poi/simple-table.doc");
    const POI_UNICODE_DOC: &[u8] =
        include_bytes!("../../tests/fixtures/office/legacy-poi/HeaderFooterUnicode.doc");
    const POI_STORIES_DOC: &[u8] =
        include_bytes!("../../tests/fixtures/office/legacy-poi/footnote.doc");
    const POI_EMPTY_DOC: &[u8] = include_bytes!("../../tests/fixtures/office/legacy-poi/empty.doc");
    const POI_ENCRYPTED_DOC: &[u8] =
        include_bytes!("../../tests/fixtures/office/legacy-poi/PasswordProtected.doc");
    const POI_ENCRYPTED_PPT: &[u8] =
        include_bytes!("../../tests/fixtures/office/legacy-poi/Password_Protected-hello.ppt");

    #[test]
    fn extracts_real_poi_word_document() {
        let text = extract_legacy("doc", POI_DOC, 4 * 1024 * 1024).unwrap();
        assert!(text.contains("This is a simple file"), "{text:?}");
    }

    #[test]
    fn extracts_real_poi_powerpoint_document() {
        let text = extract_legacy("ppt", POI_PPT, 4 * 1024 * 1024).unwrap();
        for shape_text in [
            "Title Placeholder",
            "TextBox",
            "Rectangle",
            "Ellipse",
            "Octagon",
        ] {
            assert!(
                text.contains(shape_text),
                "missing {shape_text:?}: {text:?}"
            );
        }
        assert!(
            text.contains("Slide 1"),
            "slide provenance is missing: {text:?}"
        );
    }

    #[test]
    fn extracts_real_poi_notes_by_their_slide_reference() {
        let text = extract_legacy("ppt", POI_NOTES_PPT, 4 * 1024 * 1024).unwrap();
        assert!(text.contains("Notes for slide 1"), "{text:?}");
        assert!(text.contains("These are the notes for page 1"), "{text:?}");
        assert!(text.contains("These are the notes on page two"), "{text:?}");
    }

    #[test]
    fn word_output_discloses_unfiltered_tracked_deletions() {
        let text = extract_legacy("doc", POI_DOC, 4 * 1024 * 1024).unwrap();
        assert!(text.starts_with("[Legacy Word limitation: tracked deletions may appear"));
    }

    #[test]
    fn extracts_producer_tables_unicode_footnotes_and_endnotes_as_plain_text() {
        let table = extract_legacy("doc", POI_TABLE_DOC, 4 * 1024 * 1024).unwrap();
        for cell in [
            "Cell 1,1", "Cell 1,2", "Cell 1,3", "Cell 2,1", "Cell 2,2", "Cell 2,3",
        ] {
            assert!(table.contains(cell), "missing {cell:?}: {table:?}");
        }
        assert!(table.contains("This text is below the table."), "{table:?}");
        assert!(
            table.contains("Cell 1,1\tCell 1,2"),
            "cells need separators: {table:?}"
        );

        let unicode = extract_legacy("doc", POI_UNICODE_DOC, 4 * 1024 * 1024).unwrap();
        assert!(unicode.contains('€'), "{unicode:?}");
        assert!(unicode.contains("Molière"), "{unicode:?}");

        let stories = extract_legacy("doc", POI_STORIES_DOC, 4 * 1024 * 1024).unwrap();
        assert!(stories.contains("TestFootnote"), "{stories:?}");
        assert!(stories.contains("TestEndnote"), "{stories:?}");
    }

    #[test]
    fn blank_word_files_remain_no_text_and_encryption_is_rejected() {
        assert_eq!(
            extract_legacy("doc", POI_EMPTY_DOC, 4096),
            Err(DocumentError::NoText)
        );
        assert_eq!(
            extract_legacy("doc", POI_ENCRYPTED_DOC, 4096),
            Err(DocumentError::Encrypted)
        );
        assert_eq!(
            extract_legacy("ppt", POI_ENCRYPTED_PPT, 4096),
            Err(DocumentError::Encrypted)
        );
    }

    #[test]
    fn normalizes_legacy_line_and_cell_controls_before_paging() {
        let word = extract_legacy("doc", POI_TABLE_DOC, 4 * 1024 * 1024).unwrap();
        let ppt = extract_legacy("ppt", POI_PPT, 4 * 1024 * 1024).unwrap();
        for text in [&word, &ppt] {
            assert!(!text.contains('\r'), "carriage return remained: {text:?}");
            assert!(!text.contains('\u{0007}'), "cell marker remained: {text:?}");
            assert!(!text.contains('\u{000B}'), "soft break remained: {text:?}");
            assert!(!text.contains('\u{000C}'), "page break remained: {text:?}");
            assert!(text
                .chars()
                .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\t')));
        }
        assert!(word.contains("Cell 1,1\tCell 1,2"), "{word:?}");
    }

    #[test]
    fn nested_fields_keep_nested_results_and_reject_unbalanced_markers() {
        let nested = "before \u{0013}OUTER\u{0014}outer result \u{0013}INNER\u{0014}inner result\u{0015} outer tail\u{0015} after";
        assert_eq!(
            strip_field_instructions(nested).unwrap(),
            "before outer result inner result outer tail after"
        );
        assert_eq!(
            strip_field_instructions("\u{0013}instruction"),
            Err(DocumentError::Invalid)
        );
        assert_eq!(
            strip_field_instructions("\u{0015}"),
            Err(DocumentError::Invalid)
        );
        assert_eq!(
            strip_field_instructions("\u{0014}orphan separator"),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn nested_field_results_are_preserved_in_a_real_cfb_word_document() {
        let source = b"\x13OUTER\x14outer result \x13INNER\x14inner result\x15 outer tail\x15\r";
        let bytes = synthetic_doc(source);
        let text = extract_legacy("doc", &bytes, 4096).unwrap();
        assert!(
            text.contains("outer result inner result outer tail"),
            "{text:?}"
        );
        assert!(
            !text.contains("OUTER"),
            "field instruction leaked: {text:?}"
        );
        assert!(
            !text.contains("INNER"),
            "field instruction leaked: {text:?}"
        );
    }

    #[test]
    fn rejects_overdeep_field_nesting_in_a_real_cfb_word_document() {
        let mut source = vec![0x13; MAX_RECURSION + 1];
        source.extend_from_slice(b"INSTRUCTION\x14visible");
        source.extend(vec![0x15; MAX_RECURSION + 1]);
        let bytes = synthetic_doc(&source);
        assert_eq!(
            extract_legacy("doc", &bytes, 4096),
            Err(DocumentError::ExpandedTooLarge)
        );
    }

    #[test]
    fn output_limit_includes_the_word_limitation_marker() {
        let full = extract_legacy("doc", POI_DOC, 4096).unwrap();
        assert_eq!(extract_legacy("doc", POI_DOC, full.len()), Ok(full.clone()));
        assert_eq!(
            extract_legacy("doc", POI_DOC, full.len() - 1),
            Err(DocumentError::TextTooLarge)
        );
        assert_eq!(
            extract_legacy("doc", POI_EMPTY_DOC, 1),
            Err(DocumentError::NoText)
        );
    }

    #[test]
    fn compressed_word_bytes_use_the_ms_doc_mapping_not_windows_1252() {
        assert_eq!(fc_compressed_character(0x80), '\u{0080}');
        assert_eq!(fc_compressed_character(0x8E), '\u{008E}');
        assert_eq!(fc_compressed_character(0x9E), '\u{009E}');
        assert_eq!(fc_compressed_character(0x82), '\u{201A}');
    }

    #[test]
    fn ppt_text_bytes_atoms_decode_as_windows_1252() {
        let atom = ppt_record(0x0FA8, 0, 0, &[0x93, b'h', b'i', 0x94, 0x96, 0x80, 0xE9]);
        let record = record_at(&atom, 0, atom.len()).unwrap();
        let text = decode_ppt_text_atom(record, 64).unwrap();
        let expected = "\u{201C}hi\u{201D}\u{2013}\u{20AC}\u{E9}";
        assert_eq!(text, expected);
        assert_eq!(normalize_legacy_text(&text), expected);
    }

    #[test]
    fn rejects_slides_whose_persist_ids_share_one_record() {
        let distinct = extract_legacy("ppt", &synthetic_two_slide_ppt(false), 4096).unwrap();
        assert!(distinct.contains("First slide"), "{distinct:?}");
        assert!(distinct.contains("Second slide"), "{distinct:?}");
        assert_eq!(
            extract_legacy("ppt", &synthetic_two_slide_ppt(true), 4096),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn current_ppt_graph_excludes_stale_and_orphan_slide_text() {
        let bytes = synthetic_incremental_ppt(true);
        let text = extract_legacy("ppt", &bytes, 4096).unwrap();
        assert!(
            text.contains("Current\ntitle"),
            "paragraph was not normalized: {text:?}"
        );
        assert!(
            text.contains("Current inline\nshape"),
            "soft break was not normalized: {text:?}"
        );
        assert!(!text.contains("Stale old version"), "{text:?}");
        assert!(!text.contains("Orphan slide"), "{text:?}");
    }

    #[test]
    fn rejects_ppt_edit_loops_and_bad_persist_offsets_through_cfb() {
        let valid = synthetic_incremental_ppt(true);
        let current_user = read_cfb_stream(&valid, "Current User");
        let current = record_at(&current_user, 0, current_user.len()).unwrap();
        let current_edit = u32le(current.body, 8).unwrap() as usize;

        let looped = update_cfb_stream(&valid, "PowerPoint Document", |stream| {
            stream[current_edit + 16..current_edit + 20]
                .copy_from_slice(&(current_edit as u32).to_le_bytes());
        });
        assert_eq!(
            extract_legacy("ppt", &looped, 4096),
            Err(DocumentError::Invalid)
        );

        let bad_persist = update_cfb_stream(&valid, "PowerPoint Document", |stream| {
            stream[current_edit + 20..current_edit + 24].copy_from_slice(&(u32::MAX).to_le_bytes());
        });
        assert_eq!(
            extract_legacy("ppt", &bad_persist, 4096),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn rejects_bad_doc_clx_offset_and_malformed_clx_through_cfb() {
        let word = read_cfb_stream(POI_DOC, "WordDocument");
        let fc_clx_field = fib_fc_clx_field(&word);
        let bad_offset = update_cfb_stream(POI_DOC, "WordDocument", |stream| {
            stream[fc_clx_field..fc_clx_field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        });
        assert_eq!(
            extract_legacy("doc", &bad_offset, 4096),
            Err(DocumentError::Invalid)
        );

        let (_, fc_clx, _) = parse_fib(&word).unwrap();
        let flags = u16le(&word, 10).unwrap();
        let table_name = if flags & 0x0200 != 0 {
            "1Table"
        } else {
            "0Table"
        };
        let table = read_cfb_stream(POI_DOC, table_name);
        assert!(fc_clx < table.len());
        let malformed_clx = update_cfb_stream(POI_DOC, table_name, |stream| {
            stream[fc_clx] = 0xFF;
        });
        assert_eq!(
            extract_legacy("doc", &malformed_clx, 4096),
            Err(DocumentError::Unsupported)
        );
    }

    #[test]
    fn rejects_empty_oversized_and_wrong_extension() {
        assert_eq!(extract_legacy("doc", &[], 10), Err(DocumentError::Empty));
        assert_eq!(
            extract_legacy("xlsx", b"data", 10),
            Err(DocumentError::MismatchedFormat)
        );
        assert_eq!(
            extract_legacy("ppt", b"not a file", 10),
            Err(DocumentError::Invalid)
        );
    }

    #[test]
    fn bounded_sink_never_appends_partial_character() {
        let mut sink = BoundedText::new(3);
        sink.push("ab").unwrap();
        assert_eq!(sink.push_char('é'), Err(DocumentError::TextTooLarge));
        assert_eq!(sink.text, "ab");
    }

    pub(super) fn synthetic_incremental_ppt(with_current_text: bool) -> Vec<u8> {
        let mut slide_list = Vec::new();
        slide_list.extend(ppt_record(0x03F3, 0, 0, &slide_persist(2, 0x100, 1)));
        slide_list.extend(ppt_record(0x0F9F, 0, 0, &0u32.to_le_bytes()));
        if with_current_text {
            slide_list.extend(ppt_text_chars("Current\rtitle"));
        }
        let document = ppt_record(0x03E8, 0xF, 0, &ppt_record(0x0FF0, 0xF, 0, &slide_list));

        let mut current_slide = Vec::new();
        let mut slide_atom = [0u8; 24];
        slide_atom[12..16].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        current_slide.extend(ppt_record(0x03EF, 0, 0, &slide_atom));
        current_slide.extend(ppt_record(0x0F9E, 0, 0, &0i32.to_le_bytes()));
        if with_current_text {
            current_slide.extend(ppt_text_chars("Current inline\u{000B}shape"));
        }
        let current_slide = ppt_record(0x03EE, 0xF, 0, &current_slide);

        let mut stale_slide = Vec::new();
        stale_slide.extend(ppt_record(0x03EF, 0, 0, &slide_atom));
        stale_slide.extend(ppt_text_chars("Stale old version"));
        let stale_slide = ppt_record(0x03EE, 0xF, 0, &stale_slide);
        let mut orphan_slide = Vec::new();
        orphan_slide.extend(ppt_record(0x03EF, 0, 0, &slide_atom));
        orphan_slide.extend(ppt_text_chars("Orphan slide"));
        let orphan_slide = ppt_record(0x03EE, 0xF, 0, &orphan_slide);

        let mut presentation = document.clone();
        let current_offset = presentation.len();
        presentation.extend(&current_slide);
        let stale_offset = presentation.len();
        presentation.extend(&stale_slide);
        let orphan_offset = presentation.len();
        presentation.extend(&orphan_slide);

        let old_directory_offset = presentation.len();
        presentation.extend(ppt_record(0x1772, 0, 0, &persist_block(2, &[stale_offset])));
        let old_edit_offset = presentation.len();
        presentation.extend(ppt_record(
            0x0FF5,
            0,
            0,
            &user_edit(0, old_directory_offset, 1),
        ));

        let current_directory_offset = presentation.len();
        let mut current_directory = Vec::new();
        current_directory.extend(persist_block(1, &[0, current_offset, orphan_offset]));
        presentation.extend(ppt_record(0x1772, 0, 0, &current_directory));
        let current_edit_offset = presentation.len();
        presentation.extend(ppt_record(
            0x0FF5,
            0,
            0,
            &user_edit(old_edit_offset, current_directory_offset, 1),
        ));

        let mut current_user_body = Vec::new();
        current_user_body.extend(20u32.to_le_bytes());
        current_user_body.extend(0xE391_C05Fu32.to_le_bytes());
        current_user_body.extend((current_edit_offset as u32).to_le_bytes());
        current_user_body.extend(0u16.to_le_bytes());
        current_user_body.extend(0x03F4u16.to_le_bytes());
        current_user_body.extend([3, 0, 0, 0]);
        let current_user = ppt_record(0x0FF6, 0, 0, &current_user_body);

        let mut cfb = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cfb.create_stream("/PowerPoint Document")
            .unwrap()
            .write_all(&presentation)
            .unwrap();
        cfb.create_stream("/Current User")
            .unwrap()
            .write_all(&current_user)
            .unwrap();
        cfb.into_inner().into_inner()
    }

    /// Two slides. When `aliased`, both persist ids resolve to the first slide's record.
    fn synthetic_two_slide_ppt(aliased: bool) -> Vec<u8> {
        let mut slide_list = Vec::new();
        slide_list.extend(ppt_record(0x03F3, 0, 0, &slide_persist(2, 0x100, 0)));
        slide_list.extend(ppt_record(0x03F3, 0, 0, &slide_persist(3, 0x101, 0)));
        let mut presentation = ppt_record(0x03E8, 0xF, 0, &ppt_record(0x0FF0, 0xF, 0, &slide_list));

        let slide = |text: &str| {
            let mut body = ppt_record(0x03EF, 0, 0, &[0u8; 24]);
            body.extend(ppt_text_chars(text));
            ppt_record(0x03EE, 0xF, 0, &body)
        };
        let first_offset = presentation.len();
        presentation.extend(slide("First slide"));
        let second_offset = if aliased {
            first_offset
        } else {
            let offset = presentation.len();
            presentation.extend(slide("Second slide"));
            offset
        };

        let directory_offset = presentation.len();
        presentation.extend(ppt_record(
            0x1772,
            0,
            0,
            &persist_block(1, &[0, first_offset, second_offset]),
        ));
        let edit_offset = presentation.len();
        presentation.extend(ppt_record(0x0FF5, 0, 0, &user_edit(0, directory_offset, 1)));

        let mut current_user_body = Vec::new();
        current_user_body.extend(20u32.to_le_bytes());
        current_user_body.extend(0xE391_C05Fu32.to_le_bytes());
        current_user_body.extend((edit_offset as u32).to_le_bytes());
        current_user_body.extend(0u16.to_le_bytes());
        current_user_body.extend(0x03F4u16.to_le_bytes());
        current_user_body.extend([3, 0, 0, 0]);
        let current_user = ppt_record(0x0FF6, 0, 0, &current_user_body);

        let mut cfb = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cfb.create_stream("/PowerPoint Document")
            .unwrap()
            .write_all(&presentation)
            .unwrap();
        cfb.create_stream("/Current User")
            .unwrap()
            .write_all(&current_user)
            .unwrap();
        cfb.into_inner().into_inner()
    }

    fn ppt_record(kind: u16, version: u16, instance: u16, body: &[u8]) -> Vec<u8> {
        let mut record = Vec::with_capacity(8 + body.len());
        let options = ((instance & 0x0FFF) << 4) | (version & 0xF);
        record.extend(options.to_le_bytes());
        record.extend(kind.to_le_bytes());
        record.extend((body.len() as u32).to_le_bytes());
        record.extend(body);
        record
    }

    fn ppt_text_chars(text: &str) -> Vec<u8> {
        let body = text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        ppt_record(0x0FA0, 0, 0, &body)
    }

    fn slide_persist(persist_id: u32, slide_id: u32, text_count: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(persist_id.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend(text_count.to_le_bytes());
        body.extend(slide_id.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body
    }

    fn persist_block(start_id: u32, offsets: &[usize]) -> Vec<u8> {
        let mut block = Vec::new();
        let entry = ((offsets.len() as u32) << 20) | start_id;
        block.extend(entry.to_le_bytes());
        for offset in offsets {
            block.extend((*offset as u32).to_le_bytes());
        }
        block
    }

    fn user_edit(previous: usize, directory: usize, document_id: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(0x100u32.to_le_bytes());
        body.extend(0x1FE9u16.to_le_bytes());
        body.extend([0, 3]);
        body.extend((previous as u32).to_le_bytes());
        body.extend((directory as u32).to_le_bytes());
        body.extend(document_id.to_le_bytes());
        body.extend(4u32.to_le_bytes());
        body.extend(1u16.to_le_bytes());
        body.extend(0u16.to_le_bytes());
        body
    }

    fn synthetic_doc(text: &[u8]) -> Vec<u8> {
        let word_text_offset = 512usize;
        let mut word = vec![0u8; word_text_offset + text.len()];
        word[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        word[2..4].copy_from_slice(&0x00C1u16.to_le_bytes());
        word[32..34].copy_from_slice(&0u16.to_le_bytes()); // csw
        word[34..36].copy_from_slice(&11u16.to_le_bytes()); // cslw
        word[36 + 3 * 4..36 + 4 * 4].copy_from_slice(&(text.len() as u32).to_le_bytes());
        word[80..82].copy_from_slice(&34u16.to_le_bytes()); // cbRgFcLcb
        let fc_clx_field = 82 + 33 * 8;

        let mut plc_pcd = Vec::new();
        plc_pcd.extend(0u32.to_le_bytes());
        plc_pcd.extend((text.len() as u32).to_le_bytes());
        plc_pcd.extend([0, 0]);
        let compressed_fc = ((word_text_offset as u32) * 2) | 0x4000_0000;
        plc_pcd.extend(compressed_fc.to_le_bytes());
        plc_pcd.extend([0, 0]);
        let mut clx = vec![0x02];
        clx.extend((plc_pcd.len() as u32).to_le_bytes());
        clx.extend(plc_pcd);
        word[fc_clx_field..fc_clx_field + 4].copy_from_slice(&0u32.to_le_bytes());
        word[fc_clx_field + 4..fc_clx_field + 8].copy_from_slice(&(clx.len() as u32).to_le_bytes());
        word[word_text_offset..].copy_from_slice(text);

        let mut cfb = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cfb.create_stream("/WordDocument")
            .unwrap()
            .write_all(&word)
            .unwrap();
        cfb.create_stream("/0Table")
            .unwrap()
            .write_all(&clx)
            .unwrap();
        cfb.into_inner().into_inner()
    }

    fn read_cfb_stream(bytes: &[u8], name: &str) -> Vec<u8> {
        let mut cfb = cfb::CompoundFile::open(Cursor::new(bytes.to_vec())).unwrap();
        let mut stream = cfb.open_stream(name).unwrap();
        let mut contents = Vec::new();
        stream.read_to_end(&mut contents).unwrap();
        contents
    }

    fn update_cfb_stream(bytes: &[u8], name: &str, update: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut cfb = cfb::CompoundFile::open(Cursor::new(bytes.to_vec())).unwrap();
        let mut stream = cfb.open_stream(name).unwrap();
        let mut contents = Vec::new();
        stream.read_to_end(&mut contents).unwrap();
        update(&mut contents);
        stream.seek(SeekFrom::Start(0)).unwrap();
        stream.write_all(&contents).unwrap();
        drop(stream);
        cfb.into_inner().into_inner()
    }

    fn fib_fc_clx_field(word: &[u8]) -> usize {
        let csw = usize::from(u16le(word, 32).unwrap());
        let rg_w_end = 34 + csw * 2;
        let cslw = usize::from(u16le(word, rg_w_end).unwrap());
        let rg_lw_end = rg_w_end + 2 + cslw * 4;
        rg_lw_end + 2 + 33 * 8
    }
}
