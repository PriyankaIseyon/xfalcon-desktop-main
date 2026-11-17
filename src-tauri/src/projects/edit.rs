//! Anchored edits: the shape a model-authored file change arrives in, and the
//! pure text transforms that resolve one against a file's current content.
//!
//! # Why this module holds no I/O
//!
//! [`super::patch::MutationPlan::propose`] already reads every file whose
//! current content matters, to capture a base hash and render a reviewable
//! diff. Resolving anchors *there*, against those same bytes, means each file
//! is read exactly once: there is no second read for an out-of-band write to
//! land between, so an anchored edit cannot be computed against bytes that no
//! longer exist. This module therefore contains only the operation shapes and
//! the text functions `propose` applies -- no `TrustedRoot`, no filesystem
//! access, nothing to contain.
//!
//! # Why anchors do not reach the plan
//!
//! [`super::patch::PatchOperation`] stays a closed four-variant,
//! content-addressed, immutable plan carrying final content: `PlanId`, the
//! persisted approval row, `super::writer::plan_already_applied`, and the
//! writer's own base-hash re-derivation are untouched, so none of that
//! security contract has to be re-argued for anchored edits. It also keeps
//! "what was reviewed is what is written" true by construction -- the writer
//! never re-resolves an anchor, so the reviewed diff and the written bytes
//! cannot diverge.

use super::path_policy::RelativeProjectPath;

/// Hard ceiling on edits in one operation. More than this is a rewrite, not a
/// set of anchored edits, and a reviewer cannot follow it either.
pub const MAX_EDITS_PER_OPERATION: usize = 64;
/// Hard ceiling on one anchor's or one inserted block's byte length. An
/// anchor is surrounding context, not a file.
pub const MAX_ANCHOR_BYTES: usize = 64 * 1024;

/// One anchored replacement within an [`EditOperation::Replace`].
///
/// Field names deliberately match the shape current models are trained on for
/// this operation (Anthropic's `str_replace_based_edit_tool` and Claude Code's
/// `Edit`), so the model does not have to learn a private dialect to edit a
/// file.
#[derive(Clone, Debug)]
pub struct TextEdit {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

/// A file mutation as its author expressed it, before
/// [`super::patch::MutationPlan::propose`] resolves it into a plan.
#[derive(Clone, Debug)]
pub enum EditOperation {
    Create {
        path: RelativeProjectPath,
        content: Vec<u8>,
    },
    /// Replaces a file's entire content.
    Modify {
        path: RelativeProjectPath,
        content: Vec<u8>,
    },
    /// Applies anchored replacements to a file's current text.
    Replace {
        path: RelativeProjectPath,
        edits: Vec<TextEdit>,
    },
    Insert {
        path: RelativeProjectPath,
        /// 1-based line to insert *after*; `0` means the top of the file.
        insert_line: u32,
        content: String,
    },
    Rename {
        from: RelativeProjectPath,
        to: RelativeProjectPath,
    },
    Delete {
        path: RelativeProjectPath,
    },
    /// Moves a directory with everything under it. One rename syscall: nothing
    /// is enumerated, copied, or rewritten.
    RenameDirectory {
        from: RelativeProjectPath,
        to: RelativeProjectPath,
    },
    /// Removes an *empty* directory. Deliberately not recursive -- see
    /// `super::domain::WriteTarget::remove_directory`. Removing a populated
    /// folder is expressed as deleting its files and then the folder, in one
    /// plan if wanted, because operations apply in order.
    DeleteDirectory {
        path: RelativeProjectPath,
    },
}

impl EditOperation {
    /// The operation's kind and the path a person would name it by, for a
    /// message about it.
    #[must_use]
    pub fn describe(&self) -> (&'static str, String) {
        match self {
            Self::Create { path, .. } => ("create", path.as_str().to_owned()),
            Self::Modify { path, .. } => ("modify", path.as_str().to_owned()),
            Self::Replace { path, .. } => ("replace", path.as_str().to_owned()),
            Self::Insert { path, .. } => ("insert", path.as_str().to_owned()),
            Self::Delete { path } => ("delete", path.as_str().to_owned()),
            Self::DeleteDirectory { path } => ("delete_directory", path.as_str().to_owned()),
            Self::Rename { from, to } => {
                ("rename", format!("{} -> {}", from.as_str(), to.as_str()))
            }
            Self::RenameDirectory { from, to } => (
                "rename_directory",
                format!("{} -> {}", from.as_str(), to.as_str()),
            ),
        }
    }
}

/// Why an anchored edit could not be resolved.
///
/// Every `Display` string here is read by the author of the failing call --
/// usually a model, which will act on it immediately -- so each one names what
/// was wrong *and* what to do instead. A message that only says "no match"
/// turns a recoverable mistake into a retry loop.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EditError {
    #[error("the edit list for {0} is empty; an edit operation must carry at least one edit")]
    NoEdits(String),
    #[error("the edit list for {0} has more than {1} edits; split the change, or replace the whole file with modify")]
    TooManyEdits(String, usize),
    #[error(
        "edit {1} for {0} has an empty old_string; an anchored edit must name the text it replaces"
    )]
    EmptyAnchor(String, usize),
    #[error("edit {1} for {0} has an old_string or new_string over {2} bytes; anchor on a smaller unique region, or write the larger text with create or modify")]
    AnchorTooLarge(String, usize, usize),
    #[error("edit {1} for {0} found no match. old_string must match the file exactly, including whitespace and indentation. Read the file again and copy the region verbatim{2}")]
    AnchorNotFound(String, usize, String),
    #[error("edit {1} for {0} matched {2} places{3}. Add surrounding lines until old_string is unique, or set replace_all if every occurrence should change")]
    AnchorAmbiguous(String, usize, usize, String),
    #[error("the new text for {0} has a placeholder at line {1}: a comment such as \"... rest unchanged\" standing in for real content. Text is written exactly as sent, so whatever the placeholder stands for would be deleted. Send the complete text")]
    PlaceholderContent(String, usize),
    #[error("edit {1} for {0} changes nothing: new_string is identical to old_string")]
    AnchorNoOp(String, usize),
    #[error("cannot insert after line {1} of {0}: it has {2} lines")]
    InsertLineOutOfRange(String, u32, usize),
    #[error(
        "the text to insert into {0} exceeds {1} bytes; write it as a create or modify instead"
    )]
    InsertTooLarge(String, usize),
}

/// The line ending a file actually uses. `Mixed` exists so a file with both
/// conventions is matched exactly only -- guessing there would rewrite
/// whichever convention the anchor did not use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineEnding {
    Lf,
    Crlf,
    Mixed,
}

impl LineEnding {
    fn terminator(self) -> &'static str {
        match self {
            Self::Crlf => "\r\n",
            Self::Lf | Self::Mixed => "\n",
        }
    }
}

fn line_ending_of(text: &str) -> LineEnding {
    let crlf = text.matches("\r\n").count();
    if crlf == 0 {
        return LineEnding::Lf;
    }
    // `matches('\n')` counts the `\n` inside every `\r\n` too, so equality
    // means every newline in the file is part of a CRLF pair.
    if text.matches('\n').count() == crlf {
        LineEnding::Crlf
    } else {
        LineEnding::Mixed
    }
}

/// Rewrites `text`'s line endings to CRLF, via LF so an already-CRLF input
/// does not become `\r\r\n`.
fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn to_file_ending(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Crlf => to_crlf(text),
        LineEnding::Lf => text.replace("\r\n", "\n"),
        // A mixed file has no convention to normalize toward.
        LineEnding::Mixed => text.to_owned(),
    }
}

/// `new` written in `existing`'s line-ending convention, when that is not how
/// it arrived.
///
/// A model rewriting a whole file sends `\n` whatever the file uses, so taking
/// its bytes as they are would turn a CRLF file into an LF one and show the
/// whole file as changed. Only a file that is CRLF throughout and new text that
/// has no CRLF at all are touched: a mixed file has no convention to follow,
/// and text that already carries `\r\n` was written deliberately.
#[must_use]
pub(crate) fn in_files_line_ending(existing: &str, new: &str) -> Option<String> {
    (line_ending_of(existing) == LineEnding::Crlf && new.contains('\n') && !new.contains("\r\n"))
        .then(|| to_crlf(new))
}

/// Applies `edits` in order to one working buffer, so a later edit may anchor
/// on text an earlier edit wrote. `path` appears only in error messages.
pub(super) fn apply_edits(text: &str, edits: &[TextEdit], path: &str) -> Result<String, EditError> {
    if edits.is_empty() {
        return Err(EditError::NoEdits(path.to_owned()));
    }
    if edits.len() > MAX_EDITS_PER_OPERATION {
        return Err(EditError::TooManyEdits(
            path.to_owned(),
            MAX_EDITS_PER_OPERATION,
        ));
    }

    let mut buffer = text.to_owned();
    for (index, edit) in edits.iter().enumerate() {
        // 1-based in every message: the author wrote a list, not an array.
        let position = index + 1;
        if edit.old_string.is_empty() {
            return Err(EditError::EmptyAnchor(path.to_owned(), position));
        }
        if edit.old_string.len() > MAX_ANCHOR_BYTES || edit.new_string.len() > MAX_ANCHOR_BYTES {
            return Err(EditError::AnchorTooLarge(
                path.to_owned(),
                position,
                MAX_ANCHOR_BYTES,
            ));
        }
        if edit.old_string == edit.new_string {
            return Err(EditError::AnchorNoOp(path.to_owned(), position));
        }

        if !is_prose_path(path) {
            if let Some(line) = lazy_placeholder(&edit.new_string, &buffer) {
                return Err(EditError::PlaceholderContent(path.to_owned(), line));
            }
        }

        let crlf_file = line_ending_of(&buffer) == LineEnding::Crlf;
        let exact = if buffer.contains(edit.old_string.as_str()) {
            // A one-line anchor matches as written, but a multi-line
            // replacement arrives with `\n` and would leave the file with both
            // endings. The file's own convention wins.
            let replacement = if crlf_file {
                to_crlf(&edit.new_string)
            } else {
                edit.new_string.clone()
            };
            Some((edit.old_string.clone(), replacement))
        } else if crlf_file {
            // The author read this file as text and wrote the anchor back; a
            // CRLF file's invisible `\r` must not be what fails the edit.
            let needle = to_crlf(&edit.old_string);
            buffer
                .contains(needle.as_str())
                .then(|| (needle, to_crlf(&edit.new_string)))
        } else {
            None
        };
        let Some((needle, replacement)) = exact else {
            buffer = apply_tolerant(&buffer, edit, path, position)?;
            continue;
        };

        let matches = buffer.matches(needle.as_str()).count();
        if matches > 1 && !edit.replace_all {
            let places = buffer
                .match_indices(needle.as_str())
                .map(|(offset, _)| buffer[..offset].matches('\n').count() + 1);
            return Err(EditError::AnchorAmbiguous(
                path.to_owned(),
                position,
                matches,
                where_text(places),
            ));
        }

        buffer = if edit.replace_all {
            buffer.replace(needle.as_str(), &replacement)
        } else {
            buffer.replacen(needle.as_str(), &replacement, 1)
        };
    }

    Ok(buffer)
}

/// How far two lines may differ and still be the same line. Tried in this
/// order, so the closest reading of the anchor wins.
#[derive(Clone, Copy)]
enum Tier {
    /// Trailing spaces and tabs ignored.
    TrailingSpace,
    /// Leading and trailing whitespace ignored, so an anchor copied at the
    /// wrong indentation still finds its lines.
    Trimmed,
    /// As `Trimmed`, and typographic quotes, dashes, ellipses and no-break
    /// spaces read as the plain characters a model types in their place.
    Folded,
}

const TIERS: [Tier; 3] = [Tier::TrailingSpace, Tier::Trimmed, Tier::Folded];

/// Largest buffer the line-by-line match is attempted on. It holds one string
/// per line per tolerance, so a file with millions of short lines must not be
/// read this way.
const MAX_TOLERANT_BYTES: usize = 4 * 1024 * 1024;

/// Longest matching-line list an error carries.
const MAX_REPORTED_PLACES: usize = 5;
/// Fewest characters a line needs before it is worth suggesting as the closest.
const MIN_HINT_CHARS: usize = 4;
/// Similarity (0 to 1) a line needs to be offered as the closest text.
const MIN_HINT_RATIO: f32 = 0.6;
/// Lines searched for the closest text, and the longest one worth comparing.
const MAX_HINT_LINES: usize = 20_000;
const MAX_HINT_LINE_CHARS: usize = 400;
/// How long the search for the closest line may take before it gives up with
/// whatever it has found.
const MAX_HINT_TIME: std::time::Duration = std::time::Duration::from_millis(75);
/// Longest quoted line in an error.
const MAX_HINT_SNIPPET_CHARS: usize = 120;

fn fold_typography(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{00A0}' | '\u{2007}' | '\u{202F}' => ' ',
            other => other,
        })
        .collect::<String>()
        .replace('\u{2026}', "...")
}

fn normalize_line(line: &str, tier: Tier) -> String {
    match tier {
        Tier::TrailingSpace => line.trim_end().to_owned(),
        Tier::Trimmed => line.trim().to_owned(),
        Tier::Folded => fold_typography(line.trim()),
    }
}

/// One line of a buffer: where its text starts and ends, and where the line
/// including its terminator ends.
struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}

fn line_table(text: &str) -> Vec<Line> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let content_end = if index > start && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            lines.push(Line {
                start,
                content_end,
                end: index + 1,
            });
            start = index + 1;
        }
    }
    if start < text.len() {
        lines.push(Line {
            start,
            content_end: text.len(),
            end: text.len(),
        });
    }
    lines
}

/// The lines of an anchor, without the newline that ends its last line.
fn anchor_lines(text: &str) -> Vec<&str> {
    let body = text
        .strip_suffix('\n')
        .map_or(text, |rest| rest.strip_suffix('\r').unwrap_or(rest));
    body.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn first_indent<'a>(lines: impl IntoIterator<Item = &'a str>) -> &'a str {
    lines
        .into_iter()
        .find(|line| !line.trim().is_empty())
        .map_or("", leading_whitespace)
}

/// `new` moved to the indentation the file really has, when the anchor that
/// found the place was written at a different one. Lines that do not share the
/// anchor's indentation are left as written.
fn reindent(new: &str, file_indent: &str, anchor_indent: &str) -> String {
    if file_indent == anchor_indent {
        return new.to_owned();
    }
    let lines = new.split('\n');
    if let Some(extra) = file_indent.strip_prefix(anchor_indent) {
        lines
            .map(|line| {
                if line.trim().is_empty() {
                    line.to_owned()
                } else {
                    format!("{extra}{line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else if let Some(surplus) = anchor_indent.strip_prefix(file_indent) {
        lines
            .map(|line| line.strip_prefix(surplus).unwrap_or(line).to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        new.to_owned()
    }
}

/// Whether each line of `file` is indented from the file block's first line by
/// what the matching line of `anchor` is indented from the anchor's.
fn same_relative_indentation<'a>(
    file: impl IntoIterator<Item = &'a str> + Clone,
    anchor: impl IntoIterator<Item = &'a str>,
    anchor_indent: &str,
) -> bool {
    let file_indent = first_indent(file.clone());
    file.into_iter()
        .zip(anchor)
        .all(|(file_line, anchor_line)| {
            if file_line.trim().is_empty() && anchor_line.trim().is_empty() {
                return true;
            }
            let file_rest = leading_whitespace(file_line).strip_prefix(file_indent);
            let anchor_rest = leading_whitespace(anchor_line).strip_prefix(anchor_indent);
            file_rest.is_some() && file_rest == anchor_rest
        })
}

fn where_text(lines: impl Iterator<Item = usize>) -> String {
    let shown: Vec<String> = lines
        .take(MAX_REPORTED_PLACES + 1)
        .map(|line| line.to_string())
        .collect();
    if shown.is_empty() {
        return String::new();
    }
    if shown.len() > MAX_REPORTED_PLACES {
        format!(
            " (first at lines {}, and more)",
            shown[..MAX_REPORTED_PLACES].join(", ")
        )
    } else {
        format!(" (at lines {})", shown.join(", "))
    }
}

/// Resolves an anchor that matches nowhere as written by reading it line by
/// line, ignoring the differences a model cannot see when it copies text back:
/// trailing spaces, indentation, typographic punctuation. The first tier that
/// finds the lines decides, and it must find them in one place unless
/// `replace_all` says otherwise -- being tolerant never means guessing.
///
/// A file that mixes line endings gets no tolerance, for the same reason it
/// gets no normalization: there is no convention to write toward.
fn apply_tolerant(
    buffer: &str,
    edit: &TextEdit,
    path: &str,
    position: usize,
) -> Result<String, EditError> {
    let not_found = || {
        EditError::AnchorNotFound(
            path.to_owned(),
            position,
            closest_hint(buffer, &edit.old_string),
        )
    };
    let ending = line_ending_of(buffer);
    let old_lines = anchor_lines(&edit.old_string);
    if ending == LineEnding::Mixed
        || buffer.len() > MAX_TOLERANT_BYTES
        || old_lines.iter().all(|line| line.trim().is_empty())
    {
        return Err(not_found());
    }

    let table = line_table(buffer);
    let span = old_lines.len();
    for tier in TIERS {
        let wanted: Vec<String> = old_lines
            .iter()
            .map(|line| normalize_line(line, tier))
            .collect();
        let have: Vec<String> = table
            .iter()
            .map(|line| normalize_line(&buffer[line.start..line.content_end], tier))
            .collect();
        let anchor_indent = first_indent(old_lines.iter().copied());
        let mut hits = Vec::new();
        let mut index = 0;
        while index + span <= table.len() {
            let same_lines = have[index..index + span] == wanted[..];
            // Where indentation was ignored, the lines must still nest the way
            // the anchor's do: a flattened `if x:` / `return 1` is not the
            // file's `if x:` / `    return 1`, and writing it over them would
            // change what the code means.
            let same_nesting = matches!(tier, Tier::TrailingSpace) || {
                let file = table[index..index + span]
                    .iter()
                    .map(|line| &buffer[line.start..line.content_end]);
                // One indent must extend the other. A tab-indented file read
                // back with spaces (or the reverse) cannot be moved between
                // the two without guessing the width, so it is not matched.
                let file_indent = first_indent(file.clone());
                (file_indent.starts_with(anchor_indent) || anchor_indent.starts_with(file_indent))
                    && same_relative_indentation(file, old_lines.iter().copied(), anchor_indent)
            };
            if same_lines && same_nesting {
                hits.push(index);
                index += span;
            } else {
                index += 1;
            }
        }
        if hits.is_empty() {
            continue;
        }
        if hits.len() > 1 && !edit.replace_all {
            return Err(EditError::AnchorAmbiguous(
                path.to_owned(),
                position,
                hits.len(),
                where_text(hits.iter().map(|first| first + 1)),
            ));
        }

        let ends_with_newline = edit.old_string.ends_with('\n');
        let mut out = buffer.to_owned();
        for &first in hits.iter().rev() {
            let last = &table[first + span - 1];
            let end = if ends_with_newline {
                last.end
            } else {
                last.content_end
            };
            let mut replacement = match tier {
                Tier::TrailingSpace => edit.new_string.clone(),
                Tier::Trimmed | Tier::Folded => reindent(
                    &edit.new_string,
                    first_indent(
                        table[first..first + span]
                            .iter()
                            .map(|line| &buffer[line.start..line.content_end]),
                    ),
                    anchor_indent,
                ),
            };
            replacement = to_file_ending(&replacement, ending);
            if ends_with_newline
                && !replacement.is_empty()
                && !replacement.ends_with('\n')
                && last.end > last.content_end
            {
                replacement.push_str(ending.terminator());
            }
            out.replace_range(table[first].start..end, &replacement);
        }
        return Ok(out);
    }
    Err(not_found())
}

/// Whether every non-blank line starts with a line number the way a numbered
/// listing prints it (`12: text`, `12| text`, `12\ttext`).
fn looks_numbered(lines: &[&str]) -> bool {
    let mut counted = 0;
    for line in lines.iter().filter(|line| !line.trim().is_empty()) {
        let rest = line.trim_start();
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || !matches!(rest[digits..].chars().next(), Some(':' | '|' | '\t' | '→')) {
            return false;
        }
        counted += 1;
    }
    counted >= 2
}

/// What to add to a "no match" error so the next attempt can be right: the
/// line of the file the anchor most resembles, or the cause when it is plain.
fn closest_hint(buffer: &str, old_string: &str) -> String {
    let old_lines = anchor_lines(old_string);
    if looks_numbered(&old_lines) {
        return ". old_string looks like it carries line numbers; an anchor is the file's own text, without them".to_owned();
    }
    let Some(first) = old_lines.iter().find(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let target = normalize_line(first, Tier::Folded);
    if target.chars().count() < MIN_HINT_CHARS {
        return String::new();
    }
    let deadline = std::time::Instant::now() + MAX_HINT_TIME;
    let target_chars = target.chars().count();
    let mut best: Option<(f32, usize, &str)> = None;
    for (index, line) in buffer.lines().take(MAX_HINT_LINES).enumerate() {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let candidate = normalize_line(line, Tier::Folded);
        let length = candidate.chars().count();
        // A line half or double the anchor's length is not its near-miss, and
        // skipping it is what keeps a long file from costing a diff per line.
        if length == 0
            || length > MAX_HINT_LINE_CHARS
            || length * 2 < target_chars
            || length > target_chars * 2
        {
            continue;
        }
        let ratio = similar::TextDiff::from_chars(target.as_str(), candidate.as_str()).ratio();
        if best.is_none_or(|(top, _, _)| ratio > top) {
            best = Some((ratio, index + 1, line));
        }
    }
    match best {
        Some((ratio, line, text)) if ratio >= MIN_HINT_RATIO => {
            let shown: String = text.trim().chars().take(MAX_HINT_SNIPPET_CHARS).collect();
            format!(". The closest text is at line {line}: {shown}")
        }
        _ => String::new(),
    }
}

/// Wording that only a placeholder uses, wherever the ellipsis sits.
const PLACEHOLDER_STRONG_WORDS: [&str; 6] = [
    "unchanged",
    "omitted",
    "existing code",
    "existing content",
    "previous code",
    "same as before",
];
/// Wording that is also ordinary prose ("handle the remaining cases..."), so it
/// only counts when the comment opens with the ellipsis.
const PLACEHOLDER_WEAK_WORDS: [&str; 3] = ["rest of", "remaining", "as before"];

/// Whether `path` names a prose file, which holds no code to leave out: a
/// bulleted list that trails off with an ellipsis there is writing, not a
/// placeholder.
pub(super) fn is_prose_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        ["md", "markdown", "rst", "adoc"]
            .iter()
            .any(|prose| extension.eq_ignore_ascii_case(prose))
    })
}

/// The 1-based line of the first placeholder comment in `new` that stands in
/// for content, unless the same line is already in `reference`.
///
/// A model that shortens a long file writes `// ... rest unchanged` and means
/// "keep the rest". Text is written exactly as sent, so that line would be the
/// rest. Only comment lines that say so are caught: prose that merely contains
/// an ellipsis is left alone, and so is a prose file (see [`is_prose_path`]).
pub(super) fn lazy_placeholder(new: &str, reference: &str) -> Option<usize> {
    new.lines().enumerate().find_map(|(index, line)| {
        let text = line.trim();
        let body = ["<!--", "//", "/*", "--", "#", ";"]
            .iter()
            .find_map(|marker| text.strip_prefix(marker))?
            .trim_end_matches("-->")
            .trim_end_matches("*/")
            .trim();
        let ellipsis = body.contains("...") || body.contains('\u{2026}');
        let opens_with_ellipsis = body.starts_with("...") || body.starts_with('\u{2026}');
        let lowered = body.to_lowercase();
        let says_so = PLACEHOLDER_STRONG_WORDS
            .iter()
            .any(|word| lowered.contains(word))
            || (opens_with_ellipsis
                && PLACEHOLDER_WEAK_WORDS
                    .iter()
                    .any(|word| lowered.contains(word)));
        let bare = body == "..." || body == "\u{2026}";
        ((bare || (ellipsis && says_so)) && !reference.contains(text)).then_some(index + 1)
    })
}

/// Inserts `content` as whole lines after 1-based line `insert_line` (`0` for
/// the top of the file), using the file's own line ending.
pub(super) fn apply_insert(
    text: &str,
    insert_line: u32,
    content: &str,
    path: &str,
) -> Result<String, EditError> {
    if content.len() > MAX_ANCHOR_BYTES {
        return Err(EditError::InsertTooLarge(path.to_owned(), MAX_ANCHOR_BYTES));
    }

    let ending = line_ending_of(text);
    let terminator = ending.terminator();
    // The inserted text becomes whole lines, so one trailing newline of its
    // own would double the terminator added below.
    let trimmed = content
        .strip_suffix('\n')
        .map_or(content, |rest| rest.strip_suffix('\r').unwrap_or(rest));
    let block = to_file_ending(trimmed, ending);

    let newlines: Vec<usize> = text
        .bytes()
        .enumerate()
        .filter(|(_, byte)| *byte == b'\n')
        .map(|(index, _)| index)
        .collect();
    let line_count = if text.is_empty() {
        0
    } else if text.ends_with('\n') {
        newlines.len()
    } else {
        newlines.len() + 1
    };
    let target = insert_line as usize;
    if target > line_count {
        return Err(EditError::InsertLineOutOfRange(
            path.to_owned(),
            insert_line,
            line_count,
        ));
    }

    let mut out = String::with_capacity(text.len() + block.len() + terminator.len());
    if target <= newlines.len() {
        // `\n` is one byte, so the byte after it is always a char boundary.
        let offset = if target == 0 {
            0
        } else {
            newlines[target - 1] + 1
        };
        out.push_str(&text[..offset]);
        out.push_str(&block);
        out.push_str(terminator);
        out.push_str(&text[offset..]);
    } else {
        // Inserting after the final line of a file that does not end in a
        // newline: terminate that line, then append, leaving the file's own
        // "no trailing newline" shape intact.
        out.push_str(text);
        out.push_str(terminator);
        out.push_str(&block);
    }
    Ok(out)
}
