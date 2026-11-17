//! Executes `__internal__get_skill` against the imported skill library.
//!
//! One tool serves every level of a skill's progressive disclosure: called
//! with a name it returns the instructions and lists the files the skill
//! bundles; called with a name and a `file` it returns that bundled file.
//! A `SKILL.md` that says "see `references/guide.md`" therefore resolves
//! against the same folder the skill was imported with, and nothing bundled
//! is ever placed in a prompt unasked.
//!
//! Its own dispatcher rather than an arm of `crate::tools::dispatch`: that
//! one resolves a project id to a live trusted root for every call, and a
//! skill read has no project. Keeping them apart is what lets a run with
//! skills and nothing else declare a tool at all.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::tools::ToolId;

use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort};
use crate::tools::budget::{json_prefix_end, json_string_len};
use crate::tools::invocation::ToolResult;
use crate::tools::registry::GET_SKILL_TOOL_ID;
use xfalcon_contracts::tools::ToolSource;

use super::store::{SkillFileError, SkillStore, SkillStoreError};

/// How many bundled files one response lists. A skill may bundle more (fonts,
/// images); the rest are counted, and the instructions name the ones that
/// matter.
const MAX_LISTED_FILES: usize = 200;

/// Said beside the files listed with a skill's first page.
const FILES_NOTE: &str = "These files are bundled with the skill. Paths are relative to the \
     skill's folder, as its instructions refer to them. Read one with get_skill(name, file).";

#[cfg(test)]
pub(crate) fn prompt_scan_files_note() -> &'static str {
    FILES_NOTE
}

pub(crate) struct SkillToolDispatcher {
    store: std::sync::Arc<SkillStore>,
    /// The run's budget. One call returns a page of the instructions or of a
    /// bundled file, measured as it is sent and sized so the whole response
    /// is always a complete model preview; longer text continues at the
    /// reported `next_offset`.
    budget: crate::tools::ToolResultBudget,
}

impl SkillToolDispatcher {
    #[must_use]
    pub(crate) fn new(
        store: std::sync::Arc<SkillStore>,
        budget: crate::tools::ToolResultBudget,
    ) -> Self {
        Self { store, budget }
    }
}

/// One page of `text` starting at byte `offset`: at most `page_bytes` once
/// escaped, cut on character boundaries, with the offset the next page starts
/// at when text remains. An offset inside a character moves back to that
/// character's start, so no page begins mid-character.
fn page(text: &str, offset: usize, page_bytes: usize) -> Result<(String, Option<usize>), String> {
    if offset > text.len() {
        return Err(format!(
            "offset {offset} is past the end of the text ({} bytes).",
            text.len()
        ));
    }
    let mut start = offset;
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let end = start + json_prefix_end(&text[start..], page_bytes);
    if end == text.len() {
        return Ok((text[start..].to_owned(), None));
    }
    Ok((text[start..end].to_owned(), Some(end)))
}

/// What a request asked for, read from arguments the model wrote.
struct Request {
    name: String,
    file: Option<String>,
    offset: usize,
}

fn parse_request(arguments: &Value) -> Result<Request, ToolExecutionError> {
    let failed = |message: &str| Err(ToolExecutionError::Failed(message.to_owned()));
    let Some(name) = arguments.get("name").and_then(Value::as_str) else {
        return failed("get_skill needs the skill's name.");
    };
    let file = match arguments.get("file") {
        None | Some(Value::Null) => None,
        // An empty string is "no file", the same as leaving it out.
        Some(Value::String(file)) if file.trim().is_empty() => None,
        Some(Value::String(file)) => Some(file.clone()),
        Some(_) => return failed("file must be a path from the skill's files list."),
    };
    let offset = match arguments.get("offset") {
        None | Some(Value::Null) => 0,
        Some(value) => match value
            .as_u64()
            .and_then(|offset| usize::try_from(offset).ok())
        {
            Some(offset) => offset,
            None => return failed("offset must be a whole number of bytes, 0 or more."),
        },
    };
    Ok(Request {
        name: name.to_owned(),
        file,
        offset,
    })
}

/// The instructions, and on the first page the files beside them.
fn skill_response(
    store: &SkillStore,
    request: &Request,
    page_bytes: usize,
) -> Result<Value, ToolExecutionError> {
    let (instructions, files) = store
        .instructions_and_files(&request.name)
        .map_err(missed)?;
    // Listed once, with the first page: later pages continue the same text.
    // The listing shares that page with the instructions, so it is charged
    // against the page, and it may take at most half of it — the instructions
    // always make progress.
    let listing = (request.offset == 0 && !files.is_empty()).then(|| {
        let mut room = page_bytes / 2;
        let mut listed = Vec::new();
        for file in files.iter().take(MAX_LISTED_FILES) {
            let entry = json!({ "path": file.path, "bytes": file.bytes });
            let size = entry.to_string().len() + 1;
            if size > room {
                break;
            }
            room -= size;
            listed.push(entry);
        }
        let mut listing = json!({ "files": listed, "files_note": FILES_NOTE });
        if files.len() > listed.len() {
            listing["files_not_listed"] = json!(files.len() - listed.len());
        }
        listing
    });
    let listing_bytes = listing
        .as_ref()
        .map_or(0, |listing| listing.to_string().len());
    let (instructions, next_offset) = page(
        &instructions,
        request.offset,
        page_bytes.saturating_sub(listing_bytes),
    )
    .map_err(ToolExecutionError::Failed)?;
    let mut response = json!({
        "name": request.name,
        "instructions": instructions,
        "truncated": next_offset.is_some(),
    });
    if let Some(next_offset) = next_offset {
        response["next_offset"] = json!(next_offset);
    }
    if let Some(Value::Object(listing)) = listing {
        for (key, value) in listing {
            response[key] = value;
        }
    }
    Ok(response)
}

fn file_response(
    store: &SkillStore,
    request: &Request,
    file: &str,
    page_bytes: usize,
) -> Result<Value, ToolExecutionError> {
    let mut page = store
        .read_file(&request.name, file, request.offset as u64, page_bytes)
        .map_err(|err| match err {
            SkillFileError::Store(err) => missed(err),
            SkillFileError::UnknownFile { skill, path } => ToolExecutionError::Failed(format!(
                "The skill {skill:?} bundles no file {path:?}. Use a path from its files list."
            )),
            SkillFileError::Binary { path, bytes } => ToolExecutionError::Failed(format!(
                "{path:?} is a binary file ({bytes} bytes) and cannot be read as text."
            )),
            SkillFileError::OffsetPastEnd {
                path,
                offset,
                bytes,
            } => ToolExecutionError::Failed(format!(
                "offset {offset} is past the end of {path:?} ({bytes} bytes)."
            )),
        })?;
    // The store pages raw bytes; the budget counts them as sent. A page whose
    // escaped form overflows is cut back, and its cursor moves back with it.
    let fits = json_prefix_end(&page.content, page_bytes);
    if fits < page.content.len() {
        let cut = (page.content.len() - fits) as u64;
        page.next_offset = Some(page.next_offset.unwrap_or(page.bytes) - cut);
        page.content.truncate(fits);
    }
    let mut response = json!({
        "name": request.name,
        "file": file,
        "content": page.content,
        "bytes": page.bytes,
        "truncated": page.next_offset.is_some(),
    });
    if let Some(next_offset) = page.next_offset {
        response["next_offset"] = json!(next_offset);
    }
    Ok(response)
}

/// Both misses are reported, never answered with an empty body: an empty
/// result reads to the model as a skill that says nothing, and it would act
/// on that silence.
///
/// A storage failure is logged by kind: the model gets a sentence it can act
/// on, and the log keeps the reason a skill went unread.
fn missed(error: SkillStoreError) -> ToolExecutionError {
    ToolExecutionError::Failed(match error {
        SkillStoreError::UnknownSkill(requested) => format!(
            "No skill named {requested:?} is available. Use one of the names in <active_skills>."
        ),
        SkillStoreError::DisabledSkill(requested) => {
            format!("The skill {requested:?} is turned off.")
        }
        other => {
            tracing::warn!(
                kind = other.kind_label(),
                "skills: a skill could not be read for a run"
            );
            "That skill could not be read from this device.".to_owned()
        }
    })
}

#[async_trait]
impl ToolExecutorPort for SkillToolDispatcher {
    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        if cancel.is_cancelled() {
            return Err(ToolExecutionError::Cancelled);
        }
        if tool_id.as_str() != GET_SKILL_TOOL_ID {
            return Err(ToolExecutionError::Failed(format!(
                "{} is not a skill tool",
                tool_id.as_str()
            )));
        }
        let request = parse_request(arguments)?;

        // A blocking read of small local files. `spawn_blocking` mirrors
        // `ProjectToolDispatcher`, so a slow disk cannot stall the runtime.
        let store = self.store.clone();
        // The page's own fields are reserved by the budget; the names it
        // echoes are charged at their escaped length.
        let named =
            json_string_len(&request.name) + request.file.as_deref().map_or(0, json_string_len);
        let page_bytes = self.budget.page_bytes(named);
        let handle = tokio::task::spawn_blocking(move || match request.file.as_deref() {
            Some(file) => file_response(&store, &request, file, page_bytes),
            None => skill_response(&store, &request, page_bytes),
        });

        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ToolExecutionError::Cancelled),
            joined = handle => match joined {
                Ok(result) => result?,
                Err(_) => return Err(ToolExecutionError::Failed(
                    "the skill reader stopped unexpectedly".to_owned(),
                )),
            },
        };

        ToolResult::new(Vec::new(), Some(response), ToolSource::Internal).map_err(Into::into)
    }
}
