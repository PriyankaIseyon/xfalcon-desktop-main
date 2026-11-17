//! Resolving "the nth HTML document of this message" to bytes.
//!
//! The renderer sends a source identity — a conversation, a message, and an
//! ordinal — and never markup. This module loads that message from the
//! encrypted chat projection, re-scans its text for fenced HTML documents, and
//! takes the one at that ordinal. Re-reading from what was persisted is what
//! makes the save-to-project path unusable for turning renderer-supplied markup
//! into a stored file: there is no input here a caller could substitute.
//!
//! [`find_html_documents`] is one half of a contract with `findHtmlDocuments`
//! in `src/features/chat/richContent/detect.ts`. The renderer supplies the
//! ordinal and this side resolves it, so index `n` must mean the same document
//! on both sides. Both are pinned to
//! `src/features/chat/richContent/htmlDocumentFixtures.json`, and the two are
//! the SAME algorithm written twice rather than two algorithms that happen to
//! agree on the cases anyone thought to write down.

use std::sync::OnceLock;

use regex::Regex;
use uuid::Uuid;
use xfalcon_contracts::ConversationId;

use crate::chat::domain::{ChatError, MessageContent, MessageRole};
use crate::chat::repository::LocalChatProjection;

/// Web parity, and the frontend's own `MIN_HTML_DOC_LENGTH`: a document span
/// shorter than this is noise, not a document.
const MIN_HTML_DOC_LENGTH: usize = 100;
const HTML_DOC_END: &str = "</html>";

/// What looking for the document found. Nothing here is an error — the caller
/// asked what is at an ordinal, and these are the answers.
#[derive(Debug, Eq, PartialEq)]
pub enum ResolvedDocument {
    Found(String),
    /// The message does not exist, does not belong to the named conversation,
    /// or is not an assistant turn. One arm for all three: the difference is an
    /// existence oracle.
    NotFound,
    /// The message holds no HTML document at that ordinal.
    NotHtml,
}

/// Loads the named message and takes its `ordinal`-th HTML document.
pub async fn resolve_document(
    projection: &dyn LocalChatProjection,
    conversation_id: ConversationId,
    message_id: &str,
    ordinal: u32,
) -> Result<ResolvedDocument, ChatError> {
    let Some(snapshot) = projection.load_snapshot(conversation_id.clone()).await? else {
        return Ok(ResolvedDocument::NotFound);
    };
    // A malformed id is a miss, not a validation failure: a caller must not be
    // able to tell "no such message" from "that is not even an id".
    let Ok(target) = Uuid::parse_str(message_id) else {
        return Ok(ResolvedDocument::NotFound);
    };
    // The snapshot is loaded BY conversation, so a message id belonging to a
    // different conversation simply is not in it — the cross-conversation check
    // is structural here rather than a predicate that could be forgotten.
    let Some(message) = snapshot
        .messages()
        .iter()
        .find(|message| message.id().as_uuid() == &target)
    else {
        return Ok(ResolvedDocument::NotFound);
    };
    if message.role() != MessageRole::Assistant {
        return Ok(ResolvedDocument::NotFound);
    }

    let text = message_text(message.content());
    let documents = find_html_documents(&text);
    let Some(html) = documents.into_iter().nth(ordinal as usize) else {
        return Ok(ResolvedDocument::NotHtml);
    };
    Ok(ResolvedDocument::Found(html))
}

/// A message's prose, concatenated in order. Only text parts carry a fenced
/// document; the reference parts name attachments and tool results.
fn message_text(content: &[MessageContent]) -> String {
    let mut text = String::new();
    for part in content {
        if let MessageContent::Text { text: part } = part {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(part);
        }
    }
    text
}

/// Every HTML document in one message's raw text, in the order the renderer
/// draws them. See the module doc for why this is line-based and where its twin
/// lives.
pub fn find_html_documents(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut documents = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let Some(open) = parse_open_fence(lines[index]) else {
            index += 1;
            continue;
        };
        let mut cursor = index + 1;
        let mut closed = false;
        let mut body: Vec<&str> = Vec::new();
        while cursor < lines.len() {
            if trim_indent(lines[cursor]) == open.fence {
                closed = true;
                break;
            }
            body.push(lines[cursor]);
            cursor += 1;
        }
        // An unclosed fence is not a block: the renderer has no settled fence
        // to draw a card for either. Nothing after it can be a block either,
        // since everything left is inside it.
        if !closed {
            break;
        }
        let code = body.join("\n");
        if detect_html_document(&open.language, &code) {
            documents.push(extract_html_document(&code).unwrap_or(code));
        }
        index = cursor + 1;
    }
    documents
}

/// Whether a document references a script or stylesheet it cannot load. The
/// preview scheme's per-response policy is `default-src 'none'` with no host
/// sources, so such a reference is dead on every open — and a saved report
/// freezes whatever it renders, so this is worth saying out loud. Advisory
/// only: it never blocks a save, because a false positive refusing a working
/// dashboard would be worse than the warning.
///
/// The two patterns are the same ones `hasExternalReference` uses in
/// `detect.ts`, which is what lets the naming dialog warn BEFORE the save while
/// this stays the authoritative answer after it.
pub fn has_external_reference(html: &str) -> bool {
    static SCRIPT_SRC: OnceLock<Regex> = OnceLock::new();
    static STYLESHEET_LINK: OnceLock<Regex> = OnceLock::new();
    let script_src = SCRIPT_SRC.get_or_init(|| {
        Regex::new(r"(?i)<script\b[^>]*\bsrc\s*=").expect("static pattern compiles")
    });
    let stylesheet_link = STYLESHEET_LINK.get_or_init(|| {
        Regex::new(r#"(?i)<link\b[^>]*\brel\s*=\s*["']?stylesheet"#)
            .expect("static pattern compiles")
    });
    script_src.is_match(html) || stylesheet_link.is_match(html)
}

struct OpenFence {
    fence: String,
    language: String,
}

/// Spaces and tabs only, matching `trimIndent` in `detect.ts`. `str::trim`
/// would also eat vertical and Unicode whitespace that JavaScript's `trim`
/// treats differently, and the two sides have to agree exactly.
fn trim_indent(line: &str) -> &str {
    line.trim_matches(|character| character == ' ' || character == '\t')
}

fn parse_open_fence(line: &str) -> Option<OpenFence> {
    let trimmed = trim_indent(line);
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = trimmed
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if length < 3 {
        return None;
    }
    let info = trim_indent(&trimmed[length..]);
    if info.contains('`') || info.contains('~') {
        return None;
    }
    Some(OpenFence {
        fence: trimmed[..length].to_owned(),
        language: info
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase(),
    })
}

/// The frontend's `detectHtmlDocument`, verbatim gate: full-document content in
/// an unlabelled or html-ish fence.
fn detect_html_document(language: &str, code: &str) -> bool {
    let normalized = code.trim_matches(is_js_whitespace).to_ascii_lowercase();
    let looks_like_html =
        normalized.starts_with("<!doctype html") || normalized.starts_with("<html");
    looks_like_html && matches!(language, "" | "html" | "htm" | "xml")
}

/// What JavaScript's `\s` and `String.prototype.trim` treat as whitespace. It
/// is Unicode's White_Space plus the byte-order mark, less U+0085, which Rust
/// counts and JavaScript does not. The renderer decides what is a document with
/// the JavaScript set, so this side must use the same one.
fn is_js_whitespace(character: char) -> bool {
    (character.is_whitespace() && character != '\u{85}') || character == '\u{FEFF}'
}

/// The frontend's `extractHtmlDocument`: the document SPAN, first start marker
/// to FIRST `</html>`, so stray prose inside the fence never leaks into what is
/// stored.
fn extract_html_document(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let start = html_doc_start(&lower)?;
    // Offsets are taken from the ASCII-lowered copy on purpose:
    // `to_ascii_lowercase` is length-preserving byte for byte, so an index into
    // it indexes the original. A full Unicode `to_lowercase` is NOT (U+0130
    // expands), which is the bug the frontend's own comment records.
    let close = lower[start..].find(HTML_DOC_END)?;
    let end = start + close + HTML_DOC_END.len();
    let span = &text[start..end];
    // UTF-16 code units, as the frontend's `String.length` counts them: a byte
    // count would let a short non-ASCII span through that the renderer refuses.
    if span.encode_utf16().count() < MIN_HTML_DOC_LENGTH {
        return None;
    }
    Some(span.to_owned())
}

/// `<!doctype\s+html` or a bare `<html` followed by whitespace or `>`.
fn html_doc_start(lower: &str) -> Option<usize> {
    let doctype = find_doctype_html(lower);
    let bare = find_bare_html_tag(lower);
    match (doctype, bare) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (found, None) | (None, found) => found,
    }
}

fn find_doctype_html(lower: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(offset) = lower[from..].find("<!doctype") {
        let start = from + offset;
        let rest = &lower[start + "<!doctype".len()..];
        let after_spaces = rest.trim_start_matches(is_js_whitespace);
        if after_spaces.len() < rest.len() && after_spaces.starts_with("html") {
            return Some(start);
        }
        from = start + "<!doctype".len();
    }
    None
}

fn find_bare_html_tag(lower: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(offset) = lower[from..].find("<html") {
        let start = from + offset;
        let next = lower[start + "<html".len()..].chars().next();
        if next.is_some_and(|character| is_js_whitespace(character) || character == '>') {
            return Some(start);
        }
        from = start + "<html".len();
    }
    None
}
