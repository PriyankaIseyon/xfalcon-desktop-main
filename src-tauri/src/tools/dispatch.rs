//! The live [`ToolExecutorPort`] for this app's six internal tools.
//!
//! [`crate::tools::registry::project_read_tool_descriptors`] and
//! [`crate::tools::registry::project_propose_patch_tool_descriptor`] shipped
//! as *advertised metadata with no live caller*, each documenting that
//! resolving the run-bound project to a real
//! [`crate::projects::TrustedRoot`] and actually invoking
//! `crate::projects::{read,search}` / `crate::approvals::ApprovalsService::propose`
//! belonged to the agent-loop dispatch wiring. This module is that wiring.
//!
//! # The project a call may touch is fixed by the run, not by the model
//!
//! None of the six schemas accepts a `project_id`. The conversation binding
//! is resolved before the run starts and captured in [`ProjectToolContext`],
//! so model-authored content can never select filesystem authority
//! (`docs/threat-model.md` TM-006: content is never authority).
//!
//! # No sensitive-content override path
//!
//! None of the advertised schemas can express a
//! [`crate::projects::SensitiveOverrideGrant`], and this module never
//! constructs one. A classified-sensitive file therefore always comes back as
//! a request for human override with no content and no hash, and a sensitive
//! candidate is always skipped by content search. Granting an override is a
//! human decision through a UI, never something an agent can ask for.
//!
//! # Blocking work stays off the async core
//!
//! All six tools are synchronous filesystem/SQLite work. Each runs on
//! [`tokio::task::spawn_blocking`], and cancellation resolves the *call*
//! immediately while letting the already-started work finish on its own
//! thread — every one of these operations is bounded by
//! `crate::projects`'s own byte/entry/scan caps, so an abandoned one cannot
//! run away.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::chat::{ToolStepPathV1, ToolStepSummaryV1, TOOL_STEP_PATH_CAP};
use xfalcon_contracts::tools::{ToolId, ToolSource};
use xfalcon_contracts::RunId;

use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort};
use crate::approvals::{ApprovalOutcome, ApprovalsService, ApprovalsServiceError};
use crate::projects::{
    list_directory, read_file, search_files, search_text, ContentKind, DirectoryEntryView,
    EditOperation, EntryKind, FileSearchMode, LineRange, MetadataOnlyReason, MutationPlan,
    PatchOperation, ProjectId, ProjectsService, ReadFileOutcome, ReadFileRequest,
    RelativeProjectPath, SearchMode, SecretClassifier, SensitiveReadScope, TextEdit,
};
use crate::security::account_cleanup::ValidatedAccountId;
use crate::tools::invocation::ToolResult;

pub const TOOL_SEARCH_FILES: &str = "project.search_files";
pub const TOOL_SEARCH_TEXT: &str = "project.search_text";
pub const TOOL_READ_FILE: &str = "project.read_file";
pub const TOOL_LIST_DIRECTORY: &str = "project.list_directory";
pub const TOOL_PROPOSE_PATCH: &str = "project.propose_patch";
pub const TOOL_UPDATE_PLAN: &str = "project.update_plan";

/// Everything one run's internal tool calls need, fixed for the run's whole
/// lifetime. Cloned into each [`tokio::task::spawn_blocking`] closure, so
/// every field is owned or an [`Arc`].
#[derive(Clone)]
pub(crate) struct ProjectToolContext {
    pub projects: Arc<ProjectsService>,
    pub approvals: Arc<ApprovalsService>,
    pub classifier: Arc<SecretClassifier>,
    pub account: ValidatedAccountId,
    /// The run's project. Not negotiable by the model — see the module doc
    /// comment.
    pub project_id: ProjectId,
    pub run_id: RunId,
    /// The run's tool-result budget. Pages and listings fill it and never
    /// exceed it, so what the model reads is exactly what a page says it is.
    pub budget: crate::tools::ToolResultBudget,
}

/// Dispatches the six internal tools against live project state.
///
/// Deliberately does **not** handle MCP-sourced tools: those need a live
/// session and a catalog with a different lifetime, and are composed
/// alongside this one rather than folded into it.
pub(crate) struct ProjectToolDispatcher {
    context: ProjectToolContext,
}

impl ProjectToolDispatcher {
    #[must_use]
    pub(crate) fn new(context: ProjectToolContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl ToolExecutorPort for ProjectToolDispatcher {
    fn requires_completion(&self, tool_id: &ToolId) -> bool {
        tool_id.as_str() == TOOL_PROPOSE_PATCH
    }

    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        if cancel.is_cancelled() {
            return Err(ToolExecutionError::Cancelled);
        }

        let context = self.context.clone();
        let tool_id = tool_id.clone();
        let arguments = arguments.clone();
        let blocking_context = context.clone();
        let handle =
            tokio::task::spawn_blocking(move || run_tool(&blocking_context, &tool_id, &arguments));

        let run = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ToolExecutionError::Cancelled),
            joined = handle => match joined {
                Ok(result) => result,
                // A panic inside a bounded read is a bug, not a condition the
                // model can fix by retrying with different arguments — but it
                // must still surface as this call's own failure rather than
                // taking the run down.
                Err(_) => Err(ToolExecutionError::Failed(
                    "the tool worker stopped unexpectedly".to_owned(),
                )),
            },
        }?;

        match run {
            ToolRun::Done(result) => Ok(result),
            // Trusting the folder is the consent to write in it, so an agent
            // patch applies directly. The plan is still journaled, audited,
            // backed up, and re-validated for containment before any byte
            // lands. A run cancelled while the plan was being built stops
            // here; once applying starts it finishes, since a half-written
            // plan is worse than a whole one.
            ToolRun::Proposed { plan, unchanged } => {
                if cancel.is_cancelled() {
                    return Err(ToolExecutionError::Cancelled);
                }
                let outcome = context
                    .approvals
                    .apply_cancellable(&context.account, &plan, &cancel)
                    .await
                    .map_err(|error| apply_failure(&plan, error))?;
                internal_result(plan_json(&plan, &outcome, &unchanged))
            }
        }
    }
}

/// What one internal tool call produced. A proposal is the one outcome the
/// blocking pool cannot finish on its own: applying it is async.
enum ToolRun {
    Done(ToolResult),
    Proposed {
        plan: MutationPlan,
        /// Paths whose content already matched, so the plan left them out.
        unchanged: Vec<String>,
    },
}

fn run_tool(
    context: &ProjectToolContext,
    tool_id: &ToolId,
    arguments: &Value,
) -> Result<ToolRun, ToolExecutionError> {
    match tool_id.as_str() {
        TOOL_SEARCH_FILES => run_search_files(context, arguments).map(ToolRun::Done),
        TOOL_SEARCH_TEXT => run_search_text(context, arguments).map(ToolRun::Done),
        TOOL_READ_FILE => run_read_file(context, arguments).map(ToolRun::Done),
        TOOL_LIST_DIRECTORY => run_list_directory(context, arguments).map(ToolRun::Done),
        TOOL_PROPOSE_PATCH => run_propose_patch(context, arguments),
        TOOL_UPDATE_PLAN => run_update_plan(arguments).map(ToolRun::Done),
        other => Err(ToolExecutionError::Failed(format!(
            "{other} is not an internal tool"
        ))),
    }
}

fn required_str(arguments: &Value, key: &str) -> Result<String, ToolExecutionError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ToolExecutionError::Failed(format!("{key} is required")))
}

fn relative_path(raw: &str) -> Result<RelativeProjectPath, ToolExecutionError> {
    RelativeProjectPath::new(raw)
        .map_err(|_| ToolExecutionError::Failed("path is not a valid project path".to_owned()))
}

fn run_search_files(
    context: &ProjectToolContext,
    arguments: &Value,
) -> Result<ToolResult, ToolExecutionError> {
    let query = required_str(arguments, "query")?;
    let mode = match arguments.get("mode").and_then(Value::as_str) {
        None | Some("substring") => FileSearchMode::Substring,
        Some("glob") => FileSearchMode::Glob,
        Some(other) => {
            return Err(ToolExecutionError::Failed(format!(
                "{other} is not a path search mode; use \"substring\" or \"glob\""
            )))
        }
    };
    let trusted_root = context
        .projects
        .trusted_root(&context.account, context.project_id)
        .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;
    let store = context
        .projects
        .index_store(&context.account)
        .map_err(|_| ToolExecutionError::Failed("the project index is unavailable".to_owned()))?;
    let outcome = search_files(
        &trusted_root,
        &store,
        &context.classifier,
        &context.account,
        context.project_id,
        &query,
        mode,
    )
    .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;

    let results: Vec<Value> = outcome
        .results
        .iter()
        .map(|result| {
            json!({
                "path": result.relative_path.as_str(),
                "is_sensitive": result.is_sensitive,
                "content_kind": content_kind_code(result.content_kind),
                "size_bytes": result.size_bytes,
            })
        })
        .collect();

    let mut document = json!({
        "results": results,
        "truncated": outcome.truncated,
        "index_truncated": outcome.index_truncated,
        "live_scanned": outcome.live_scanned,
    });
    // Present only when the search was cut short: a complete result keeps the
    // shape every stored conversation was written in.
    if let Some(detail) = search_detail(
        outcome.truncated,
        outcome.index_truncated,
        results.len(),
        "files",
    ) {
        document["detail"] = Value::String(detail);
    }
    internal_result(document)
}

fn run_search_text(
    context: &ProjectToolContext,
    arguments: &Value,
) -> Result<ToolResult, ToolExecutionError> {
    let query = required_str(arguments, "query")?;
    let mode = match arguments.get("mode").and_then(Value::as_str) {
        None | Some("literal") => SearchMode::Literal,
        Some("regex") => SearchMode::Regex,
        Some(other) => {
            return Err(ToolExecutionError::Failed(format!(
                "{other} is not a content search mode; use \"literal\" or \"regex\""
            )))
        }
    };
    let case_sensitive = arguments
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let trusted_root = context
        .projects
        .trusted_root(&context.account, context.project_id)
        .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;
    let store = context
        .projects
        .index_store(&context.account)
        .map_err(|_| ToolExecutionError::Failed("the project index is unavailable".to_owned()))?;

    // No overrides, ever — see the module doc comment.
    let outcome = search_text(
        &trusted_root,
        &store,
        &context.classifier,
        &context.account,
        context.project_id,
        &query,
        mode,
        case_sensitive,
        &[],
        &context.run_id,
    )
    .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;

    let matches: Vec<Value> = outcome
        .matches
        .iter()
        .map(|hit| {
            json!({
                "path": hit.relative_path.as_str(),
                "line": hit.line_number,
                "preview": hit.line_preview,
            })
        })
        .collect();

    let mut document = json!({
        "matches": matches,
        "truncated": outcome.truncated,
        "index_truncated": outcome.index_truncated,
        "files_scanned": outcome.files_scanned,
        "files_stale": outcome.files_stale,
        "live_scanned": outcome.live_scanned,
    });
    if let Some(detail) = search_detail(
        outcome.truncated,
        outcome.index_truncated,
        matches.len(),
        "matches",
    ) {
        document["detail"] = Value::String(detail);
    }
    internal_result(document)
}

fn run_read_file(
    context: &ProjectToolContext,
    arguments: &Value,
) -> Result<ToolResult, ToolExecutionError> {
    let path = relative_path(&required_str(arguments, "path")?)?;
    let line_range = match (
        arguments.get("start_line").and_then(Value::as_u64),
        arguments.get("end_line").and_then(Value::as_u64),
    ) {
        (Some(start), Some(end)) => {
            let start = u32::try_from(start)
                .map_err(|_| ToolExecutionError::Failed("start_line is out of range".to_owned()))?;
            let end = u32::try_from(end)
                .map_err(|_| ToolExecutionError::Failed("end_line is out of range".to_owned()))?;
            Some(
                LineRange::new(start, end)
                    .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?,
            )
        }
        // "From line 40" means one page starting there -- the idiom for
        // walking a file too long to return whole.
        (Some(start), None) => {
            let start = u32::try_from(start)
                .map_err(|_| ToolExecutionError::Failed("start_line is out of range".to_owned()))?;
            Some(
                LineRange::page_from(start)
                    .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?,
            )
        }
        // An end without a start is genuinely ambiguous: it could mean "the
        // first N lines" or "the N lines before here".
        (None, Some(_)) => {
            return Err(ToolExecutionError::Failed(
                "end_line needs a start_line; give start_line alone to read one page from there"
                    .to_owned(),
            ))
        }
        // No range means "read this file", which is served as its first page.
        // A model-facing read is always paged: a page is sized from the run's
        // budget, so it always arrives whole and always carries a cursor, and
        // a file of any size is readable page by page.
        (None, None) => Some(
            LineRange::page_from(1)
                .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?,
        ),
    };

    let trusted_root = context
        .projects
        .trusted_root(&context.account, context.project_id)
        .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;
    let scope = SensitiveReadScope::new(context.project_id, path.clone(), context.run_id.clone());
    let outcome = read_file(
        &trusted_root,
        &context.classifier,
        scope,
        ReadFileRequest {
            relative_path: path.clone(),
            line_range,
            sensitive_override: None,
            page_bytes: context
                .budget
                .page_bytes(crate::tools::budget::json_string_len(path.as_str())),
        },
    )
    .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;

    let structured = match outcome {
        ReadFileOutcome::Text(content) => json!({
            "kind": "text",
            "path": path.as_str(),
            "content": content.content,
            "first_line": content.first_line,
            "last_line": content.last_line,
            "truncated": content.truncated,
            // Present only when content remains: its absence is what says
            // "this is the whole file", so the reader never has to guess.
            "next_line": content.next_line,
            "detail": read_detail(content.next_line, content.truncated),
        }),
        ReadFileOutcome::MetadataOnly {
            size_bytes,
            content_kind,
            sha256,
            reason,
        } => json!({
            "kind": "metadata_only",
            "path": path.as_str(),
            "size_bytes": size_bytes,
            "content_kind": content_kind_code(content_kind),
            "sha256": sha256,
            "reason": metadata_only_reason_code(reason),
        }),
        ReadFileOutcome::SensitiveOverrideRequired(request) => json!({
            "kind": "sensitive_override_required",
            "path": path.as_str(),
            "secret_class": request.class().code(),
            "detail": "This file is classified sensitive. A person must approve \
                       reading it; an agent cannot grant that itself.",
        }),
    };

    internal_result(structured)
}

fn run_list_directory(
    context: &ProjectToolContext,
    arguments: &Value,
) -> Result<ToolResult, ToolExecutionError> {
    let relative = match arguments.get("path").and_then(Value::as_str) {
        // The schema allows an empty `path`, and the empty relative path is
        // how `TrustedRoot` spells "the root itself" — treat both the missing
        // key and the empty string as the root rather than rejecting one.
        None | Some("") => None,
        Some(raw) => Some(relative_path(raw)?),
    };
    let trusted_root = context
        .projects
        .trusted_root(&context.account, context.project_id)
        .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;
    let mut entries = list_directory(&trusted_root, &context.classifier, relative)
        .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;
    // Filesystem order is not stable between calls; a page cursor needs one.
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    let offset = optional_count(arguments, "offset")?.unwrap_or(0);
    let limit = optional_count(arguments, "limit")?;
    if limit == Some(0) {
        return Err(ToolExecutionError::Failed(
            "limit must be at least 1".to_owned(),
        ));
    }
    let total = entries.len();
    if offset > total || (offset == total && total > 0) {
        return Err(ToolExecutionError::Failed(format!(
            "offset {offset} is past the last entry; this directory has {total}"
        )));
    }

    // As many entries as the requested limit allows and the run's budget
    // holds. What does not fit is reached with `next_offset`, never dropped.
    const FIELDS_RESERVE: usize = 512;
    let mut room = context.budget.result_bytes().saturating_sub(FIELDS_RESERVE);
    let mut page = Vec::new();
    for entry in entries.iter().skip(offset) {
        if limit.is_some_and(|limit| page.len() >= limit) {
            break;
        }
        let rendered = directory_entry_json(entry);
        let size = rendered.to_string().len() + 1;
        if size > room && !page.is_empty() {
            break;
        }
        room = room.saturating_sub(size);
        page.push(rendered);
    }
    let end = offset + page.len();
    let next_offset = (end < total).then_some(end);

    internal_result(json!({
        "entries": page,
        "offset": offset,
        "total": total,
        "next_offset": next_offset,
        "detail": next_offset.map(|next| format!(
            "{} of {total} entries shown. Call project.list_directory again with \
             offset {next} for the rest.",
            end - offset
        )),
    }))
}

/// What the model needs to know about a search that did not cover everything.
///
/// The flags alone say a search was cut; this says what that means and what to
/// do, so a model never reports "these are all the files" over a partial list
/// and then edits only those.
pub(crate) fn search_detail(
    truncated: bool,
    index_truncated: bool,
    shown: usize,
    noun: &str,
) -> Option<String> {
    let mut notes = Vec::new();
    if truncated {
        notes.push(format!(
            "Only the first {shown} {noun} are shown and more exist, so this list is NOT complete. \
             Narrow the query or use a more specific pattern before acting on it \
             as if it were everything."
        ));
    }
    if index_truncated {
        notes.push(
            "The project has more files than the search index holds, so some files may not have \
             been searched. Do not treat a missing result as proof that nothing matches."
                .to_owned(),
        );
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// What the model needs to know about a page it did not get whole.
fn read_detail(next_line: Option<u32>, truncated: bool) -> Option<String> {
    let cut = format!(
        "Some lines were cut: a line over {} bytes, over this page's budget, or \
         holding bytes that are not UTF-8 text is sent only up to that point, and \
         the rest of that line is not shown. The file itself is unchanged.",
        crate::projects::MAX_LINE_BYTES
    );
    match (next_line, truncated) {
        (Some(next), true) => Some(format!(
            "{cut} More of this file follows. Call project.read_file again with \
             start_line {next} to continue."
        )),
        (Some(next), false) => Some(format!(
            "More of this file follows. Call project.read_file again with start_line \
             {next} to continue."
        )),
        (None, true) => Some(cut),
        (None, false) => None,
    }
}

/// A non-negative integer argument, absent when not given.
fn optional_count(arguments: &Value, key: &str) -> Result<Option<usize>, ToolExecutionError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .map(Some)
            .ok_or_else(|| {
                ToolExecutionError::Failed(format!("{key} must be a non-negative integer"))
            }),
    }
}

/// The checklist items still open in a `project.update_plan` call's
/// arguments, or `None` when the arguments are not a plan.
///
/// Read by the turn driver as well as by this tool, so the two can never
/// disagree about what counts as unfinished.
pub(crate) fn unfinished_plan_items(arguments: &Value) -> Option<Vec<String>> {
    let items = arguments.get("plan")?.as_array()?;
    let mut open = Vec::new();
    for item in items {
        let content = item.get("content")?.as_str()?;
        match item.get("status")?.as_str()? {
            "pending" | "in_progress" => open.push(content.to_owned()),
            "completed" => {}
            _ => return None,
        }
    }
    Some(open)
}

fn run_update_plan(arguments: &Value) -> Result<ToolResult, ToolExecutionError> {
    let open = unfinished_plan_items(arguments).ok_or_else(|| {
        ToolExecutionError::Recoverable(
            "plan must be a list of items, each with `content` and a `status` of pending, \
             in_progress, or completed"
                .to_owned(),
        )
    })?;
    let total = arguments
        .get("plan")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    internal_result(json!({
        "plan": arguments.get("plan"),
        "total": total,
        "completed": total - open.len(),
        "open": open.len(),
        "detail": if open.is_empty() {
            "Every item is completed."
        } else {
            "Carry on with the open items. Do not end your reply until they are completed."
        },
    }))
}

fn run_propose_patch(
    context: &ProjectToolContext,
    arguments: &Value,
) -> Result<ToolRun, ToolExecutionError> {
    let raw_operations = arguments
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolExecutionError::Failed("operations is required".to_owned()))?;
    let mut operations = Vec::with_capacity(raw_operations.len());
    for (index, raw) in raw_operations.iter().enumerate() {
        // Named by position: a model that sent thirty operations needs to know
        // which one is malformed, and the bare message would not say.
        operations.push(edit_operation(raw).map_err(|error| match error {
            ToolExecutionError::Failed(message) => ToolExecutionError::Recoverable(format!(
                "operation {index} is not valid: {message}"
            )),
            other => other,
        })?);
    }
    refuse_protected_writes(context, &operations)?;
    refuse_blind_whole_file_replace(context, &operations)?;
    keep_files_line_endings(context, &mut operations)?;

    let (plan, unchanged) = context
        .approvals
        .propose_reporting_for_account(&context.account, context.project_id, operations)
        .map_err(|error| match error {
            // A plan the model can correct: an anchor that did not match, a
            // path that does not exist, two operations on one file.
            ApprovalsServiceError::Patch(error) => {
                ToolExecutionError::Recoverable(format!("{error}"))
            }
            other => ToolExecutionError::Failed(format!("{other}")),
        })?;
    Ok(ToolRun::Proposed { plan, unchanged })
}

/// Writes a whole-file replacement in the line endings the file already uses.
/// See [`crate::projects::keep_line_endings`].
fn keep_files_line_endings(
    context: &ProjectToolContext,
    operations: &mut [EditOperation],
) -> Result<(), ToolExecutionError> {
    if !operations
        .iter()
        .any(|operation| matches!(operation, EditOperation::Modify { .. }))
    {
        return Ok(());
    }
    let root = context
        .projects
        .trusted_root(&context.account, context.project_id)
        .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?;
    crate::projects::keep_line_endings(&root, operations);
    Ok(())
}

/// Directories the agent never writes into. Their contents are how the
/// repository and this app keep their own state, not project content.
const PROTECTED_WRITE_ROOTS: [&str; 2] = [".git", ".xfalcon"];

/// Refuses a write the model must not make on its own: credentials and key
/// material, and the repository's and the app's own state.
///
/// The read tool already withholds these files. Without the same rule here, a
/// write is a way around it: a diff of an edit or delete echoes the old
/// content, and a rename can move a secret to a name the read rule allows.
/// A person saving in the file viewer is unaffected; this runs only on the
/// model's tool call.
fn refuse_protected_writes(
    context: &ProjectToolContext,
    operations: &[EditOperation],
) -> Result<(), ToolExecutionError> {
    for operation in operations {
        let paths: Vec<&RelativeProjectPath> = match operation {
            EditOperation::Create { path, .. }
            | EditOperation::Modify { path, .. }
            | EditOperation::Replace { path, .. }
            | EditOperation::Insert { path, .. }
            | EditOperation::Delete { path }
            | EditOperation::DeleteDirectory { path } => vec![path],
            EditOperation::Rename { from, to } | EditOperation::RenameDirectory { from, to } => {
                vec![from, to]
            }
        };
        let directory_operation = matches!(
            operation,
            EditOperation::RenameDirectory { .. } | EditOperation::DeleteDirectory { .. }
        );
        for path in paths {
            // Any component, not only the first: a nested repository's `.git`
            // holds hooks that run on the person's next commit.
            let protected_root = path.as_str().split('/').any(|component| {
                PROTECTED_WRITE_ROOTS
                    .iter()
                    .any(|root| component.eq_ignore_ascii_case(root))
            });
            // A directory is classified as one: a bare `.ssh` or `.kube` has no
            // trailing slash for the file rules to match, and moving it to a
            // plain name would hand its contents to the read rules.
            let classified = if directory_operation {
                context.classifier.classify_directory(path)
            } else {
                context.classifier.classify(path)
            };
            if protected_root || classified.is_some() {
                return Err(ToolExecutionError::Failed(format!(
                    "{:?} holds credentials or the project's own state, so you cannot change it. \
                     Leave it as it is and ask the person to edit it themselves if it needs to change.",
                    path.as_str()
                )));
            }
        }
    }
    Ok(())
}

/// The largest file the model may replace whole is one a single read returns
/// in full. Past it the model has seen the file only in pages, so "replace the
/// whole content" would discard bytes it may never have read; anchored edits
/// carry no such risk. A person saving their own edit is never refused this way
/// — the editor holds the whole file.
fn refuse_blind_whole_file_replace(
    context: &ProjectToolContext,
    operations: &[EditOperation],
) -> Result<(), ToolExecutionError> {
    let modified = operations.iter().filter_map(|operation| match operation {
        EditOperation::Modify { path, .. } => Some(path),
        _ => None,
    });
    let mut trusted_root = None;
    for path in modified {
        let root = match &trusted_root {
            Some(root) => root,
            None => trusted_root.insert(
                context
                    .projects
                    .trusted_root(&context.account, context.project_id)
                    .map_err(|error| ToolExecutionError::Failed(format!("{error}")))?,
            ),
        };
        // Ask the read path itself whether one read returns this file whole: its
        // budget counts the escaped JSON form, which raw bytes undercount.
        let page_bytes = context
            .budget
            .page_bytes(crate::tools::budget::json_string_len(path.as_str()));
        let scope =
            SensitiveReadScope::new(context.project_id, path.clone(), context.run_id.clone());
        // The first page, exactly as a model's own read of this file is served:
        // an unranged read would hand back the whole file, which no model-facing
        // read ever does.
        let Ok(first_page) = LineRange::page_from(1) else {
            continue;
        };
        let outcome = read_file(
            root,
            &context.classifier,
            scope,
            ReadFileRequest {
                relative_path: path.clone(),
                line_range: Some(first_page),
                sensitive_override: None,
                page_bytes,
            },
        );
        let Ok(ReadFileOutcome::Text(content)) = outcome else {
            // Binary, sensitive or unreadable: the plan reports those.
            continue;
        };
        if content.next_line.is_some() || content.truncated {
            return Err(ToolExecutionError::Recoverable(format!(
                "{:?} is larger than one read returns ({} bytes a page), so replacing its \
                 whole content would discard bytes you may never have read. Use \
                 `replace` or `insert` edits on it instead.",
                path.as_str(),
                page_bytes
            )));
        }
    }
    Ok(())
}

fn edit_operation(raw: &Value) -> Result<EditOperation, ToolExecutionError> {
    let kind = raw
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolExecutionError::Failed("each operation needs a kind".to_owned()))?;
    match kind {
        "create" => Ok(EditOperation::Create {
            path: relative_path(&required_str(raw, "path")?)?,
            content: operation_content(raw)?,
        }),
        "modify" => Ok(EditOperation::Modify {
            path: relative_path(&required_str(raw, "path")?)?,
            content: operation_content(raw)?,
        }),
        "replace" => Ok(EditOperation::Replace {
            path: relative_path(&required_str(raw, "path")?)?,
            edits: text_edits(raw)?,
        }),
        "insert" => Ok(EditOperation::Insert {
            path: relative_path(&required_str(raw, "path")?)?,
            insert_line: insert_line(raw)?,
            content: required_str(raw, "content")?,
        }),
        "rename" => Ok(EditOperation::Rename {
            from: relative_path(&required_str(raw, "from")?)?,
            to: relative_path(&required_str(raw, "to")?)?,
        }),
        "delete" => Ok(EditOperation::Delete {
            path: relative_path(&required_str(raw, "path")?)?,
        }),
        "rename_directory" => Ok(EditOperation::RenameDirectory {
            from: relative_path(&required_str(raw, "from")?)?,
            to: relative_path(&required_str(raw, "to")?)?,
        }),
        "delete_directory" => Ok(EditOperation::DeleteDirectory {
            path: relative_path(&required_str(raw, "path")?)?,
        }),
        other => Err(ToolExecutionError::Failed(format!(
            "{other} is not a write operation kind"
        ))),
    }
}

/// Converts provider-authored UTF-8 text to the patch engine's byte shape.
/// Binary writes are intentionally outside the model-facing tool contract;
/// they remain possible through the explicit UI approval command.
fn operation_content(raw: &Value) -> Result<Vec<u8>, ToolExecutionError> {
    Ok(required_str(raw, "content")?.into_bytes())
}

fn text_edits(raw: &Value) -> Result<Vec<TextEdit>, ToolExecutionError> {
    let raw_edits = raw
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolExecutionError::Failed("a replace operation needs edits".to_owned()))?;
    raw_edits
        .iter()
        .map(|edit| {
            Ok(TextEdit {
                old_string: required_str(edit, "old_string")?,
                new_string: required_str(edit, "new_string")?,
                replace_all: edit
                    .get("replace_all")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// `0` means the top of the file, so this is the one numeric field here that
/// is legitimately zero -- and the only one whose absence is not defaulted:
/// silently inserting at the top of a file the model meant to append to would
/// be a wrong write, not a harmless default.
fn insert_line(raw: &Value) -> Result<u32, ToolExecutionError> {
    let line = raw
        .get("insert_line")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ToolExecutionError::Failed(
                "an insert operation needs insert_line (0 for the top of the file)".to_owned(),
            )
        })?;
    u32::try_from(line)
        .map_err(|_| ToolExecutionError::Failed("insert_line is out of range".to_owned()))
}

/// The model's account of what its change did. The plan is always applied by
/// the time this renders, so `applied` reports the execution's own result
/// rather than whether anyone consented -- a `Denied` outcome here would mean
/// the policy layer rejected the plan, not that a person did.
fn plan_json(plan: &MutationPlan, outcome: &ApprovalOutcome, unchanged: &[String]) -> Value {
    let applied = matches!(outcome, ApprovalOutcome::Executed(_));
    let mut result = json!({
        "applied": applied,
        "plan_id": plan.plan_id().to_hex(),
        "total_paths": plan.total_paths(),
        "total_bytes": plan.total_bytes(),
        // Named by path so the model can check its own work against what
        // landed. The rendered diff is left out: the model wrote the content,
        // and echoing it back doubled the cost of every file it changed.
        "operations": plan
            .operations()
            .iter()
            .enumerate()
            .map(|(index, operation)| operation_json(index, operation, applied))
            .collect::<Vec<_>>(),
        "detail": match outcome {
            ApprovalOutcome::Executed(_) => "Applied. The change is written to the \
                project on disk.",
            ApprovalOutcome::Denied => "Rejected by the local write policy. Nothing \
                was written.",
        },
    });
    if !unchanged.is_empty() {
        result["unchanged"] = json!(unchanged);
        result["unchanged_note"] = json!(
            "These files already contained exactly the content you sent, so they were left \
             alone. They need no further change."
        );
    }
    result
}

fn operation_json(index: usize, operation: &PatchOperation, applied: bool) -> Value {
    let mut row = match operation {
        PatchOperation::Create { path, content } | PatchOperation::Modify { path, content } => {
            json!({ "path": path.as_str(), "bytes": content.len() })
        }
        PatchOperation::Delete { path } | PatchOperation::DeleteDirectory { path } => {
            json!({ "path": path.as_str() })
        }
        PatchOperation::Rename { from, to } | PatchOperation::RenameDirectory { from, to } => {
            json!({ "from": from.as_str(), "to": to.as_str() })
        }
    };
    row["index"] = json!(index);
    row["kind"] = json!(operation.kind());
    row["applied"] = json!(applied);
    row
}

/// Turns a failed apply into what the model needs to carry on: which
/// operations already landed, which did not, and why.
///
/// Operations apply in the order given and stop at the first failure, so the
/// first `operations_completed` of them are on disk. A bare "2 of 5
/// completed" left the model unable to tell which two, and resending the whole
/// plan then failed on the files that had already changed.
pub(crate) fn apply_failure(
    plan: &MutationPlan,
    error: ApprovalsServiceError,
) -> ToolExecutionError {
    if matches!(error, ApprovalsServiceError::Cancelled) {
        return ToolExecutionError::Cancelled;
    }
    let ApprovalsServiceError::Execution(failure) = error else {
        return ToolExecutionError::Failed(format!("{error}"));
    };
    let describe = |operation: &PatchOperation| match operation {
        PatchOperation::Rename { from, to } | PatchOperation::RenameDirectory { from, to } => {
            format!(
                "{} ({} {} -> {})",
                from.as_str(),
                operation.kind(),
                from.as_str(),
                to.as_str()
            )
        }
        PatchOperation::Create { path, .. }
        | PatchOperation::Modify { path, .. }
        | PatchOperation::Delete { path }
        | PatchOperation::DeleteDirectory { path } => {
            format!("{} ({})", path.as_str(), operation.kind())
        }
    };
    let operations = plan.operations();
    let done = failure.operations_completed.min(operations.len());
    let (applied, pending) = operations.split_at(done);
    let list = |operations: &[PatchOperation]| {
        if operations.is_empty() {
            "none".to_owned()
        } else {
            operations
                .iter()
                .map(describe)
                .collect::<Vec<_>>()
                .join("; ")
        }
    };
    if failure.durability_unconfirmed {
        if pending.is_empty() {
            // Every operation ran; only the final flush to disk could not be
            // confirmed. Sending anything again would repeat finished work.
            return ToolExecutionError::Failed(format!(
                "Every operation was written, but the final flush to disk could not be confirmed: {}. \
                 Do not resend them. Read the files back to check they hold what you wrote.",
                failure.error
            ));
        }
        // The writer stops at the first flush it cannot confirm, so the
        // operations after it never ran.
        return ToolExecutionError::Recoverable(format!(
            "The write stopped after an operation whose flush to disk could not be confirmed: {}. \
             Already applied: {}. Not applied: {}. Do not resend the applied operations. Read the \
             last applied file back, then send only the ones that were not applied.",
            failure.error,
            list(applied),
            list(pending),
        ));
    }
    if failure.operations_completed == 0 {
        return ToolExecutionError::Recoverable(format!(
            "The write failed and nothing was changed: {}. Fix the cause and send the plan again.",
            failure.error
        ));
    }
    ToolExecutionError::Recoverable(format!(
        "The write stopped part way: {}. Already applied and on disk: {}. Not applied: {}. \
         Do not resend the applied operations. Fix the cause and send only the ones that were \
         not applied.",
        failure.error,
        list(applied),
        list(pending),
    ))
}

fn directory_entry_json(entry: &DirectoryEntryView) -> Value {
    json!({
        "name": entry.name,
        "kind": match entry.kind {
            EntryKind::File => "file",
            EntryKind::Directory => "directory",
            EntryKind::Symlink => "symlink",
            EntryKind::Other => "other",
        },
        "size_bytes": entry.size_bytes,
        "is_sensitive": entry.is_sensitive,
    })
}

const fn content_kind_code(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => "text",
        ContentKind::Binary => "binary",
        ContentKind::NonUtf8Text => "non_utf8_text",
        ContentKind::Oversized => "oversized",
    }
}

const fn metadata_only_reason_code(reason: MetadataOnlyReason) -> &'static str {
    match reason {
        MetadataOnlyReason::Binary => "binary",
        MetadataOnlyReason::NonUtf8Text => "non_utf8_text",
        MetadataOnlyReason::Oversized => "oversized",
    }
}

fn internal_result(structured: Value) -> Result<ToolResult, ToolExecutionError> {
    ToolResult::new(Vec::new(), Some(structured), ToolSource::Internal).map_err(Into::into)
}

// ---------------------------------------------------------------------
// Reading one stored outcome back as a step
// ---------------------------------------------------------------------

/// The six wire aliases `crate::agent::tool_session::wire_tool_name` produces
/// for the tools above — the only names a durable row ever carries for them.
/// Pinned to that function by test rather than derived, because sanitization is
/// not invertible.
const WIRE_SEARCH_FILES: &str = "project_search_files";
const WIRE_SEARCH_TEXT: &str = "project_search_text";
const WIRE_READ_FILE: &str = "project_read_file";
const WIRE_LIST_DIRECTORY: &str = "project_list_directory";
const WIRE_PROPOSE_PATCH: &str = "project_propose_patch";
const WIRE_UPDATE_PLAN: &str = "project_update_plan";

/// Reads one durable call's stored answer back as a task-shaped step
/// (spec: chat-tool-call-detail).
///
/// It lives beside the arms that WRITE these documents on purpose. The reader
/// has to bind to the JSON keys those arms emit and not to the Rust fields
/// behind them — `RelativeProjectPath` serializes as `path`, so a reader
/// looking for `relative_path` matches nothing — and matching nothing is this
/// function's contracted answer for a tool it does not recognize. A key rename
/// on one side of the app and a reader on the other would therefore empty every
/// step summary while every test stayed green; one file makes the two sides one
/// edit apart.
///
/// `answer_text` is the stored answer row: the wrapper
/// `crate::agent::turn_driver::render_tool_outcome` composes AROUND a tool's
/// document (`status`, `result`, …), never the document itself. `argument_text`
/// is the call's own durable argument row, needed only by `list_directory`,
/// whose entries are bare names.
///
/// Returns `None` for every name that is not one of the six above — every
/// `mcp_`-prefixed name included. An MCP result is text a server chose under a
/// shape the server chose and may name any path at all; a chip drawn from one
/// would put a server's claim about the filesystem into this app's own voice
/// (`docs/threat-model.md` TM-006).
pub(crate) fn tool_step_summary(
    wire_name: &str,
    answer_text: &str,
    argument_text: Option<&str>,
) -> Option<ToolStepSummaryV1> {
    // First, because every tool travels through the same wrapper: an MCP call
    // is refused with the same `denied` status as one of ours, and a row whose
    // tool this app cannot name is one it cannot describe either.
    let tool = recognized(wire_name)?;
    let answer: Value = serde_json::from_str(answer_text).ok()?;
    let status = answer.get("status").and_then(Value::as_str)?;
    if status != "completed" {
        return unfinished_outcome_label(status)
            .map(|label| ToolStepSummaryV1::new(label, Vec::new(), 0));
    }
    // A structural preview is useful to the model but cannot support exact UI
    // counts. Historical rows used `truncated`; current rows separate preview
    // completeness from source truncation. Fail closed for either generation.
    if answer.get("preview_complete").and_then(Value::as_bool) == Some(false)
        || answer.get("source_truncated").and_then(Value::as_bool) == Some(true)
        || answer.get("truncated").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let outcome = answer.get("result").filter(|value| !value.is_null())?;
    let draft = match tool {
        StepTool::SearchFiles => search_files_step(outcome),
        StepTool::SearchText => search_text_step(outcome),
        StepTool::ReadFile => read_file_step(outcome)?,
        StepTool::ListDirectory => list_directory_step(outcome, argument_text),
        StepTool::ProposePatch => propose_patch_step(outcome),
        StepTool::UpdatePlan => update_plan_step(outcome),
    };
    draft.finish()
}

#[derive(Clone, Copy)]
enum StepTool {
    SearchFiles,
    SearchText,
    ReadFile,
    ListDirectory,
    ProposePatch,
    UpdatePlan,
}

fn recognized(wire_name: &str) -> Option<StepTool> {
    match wire_name {
        WIRE_SEARCH_FILES => Some(StepTool::SearchFiles),
        WIRE_SEARCH_TEXT => Some(StepTool::SearchText),
        WIRE_READ_FILE => Some(StepTool::ReadFile),
        WIRE_LIST_DIRECTORY => Some(StepTool::ListDirectory),
        WIRE_PROPOSE_PATCH => Some(StepTool::ProposePatch),
        WIRE_UPDATE_PLAN => Some(StepTool::UpdatePlan),
        _ => None,
    }
}

/// Names one of the five statuses that are answered rows with no outcome
/// behind them, from this app's own fixed vocabulary.
///
/// These rows are today indistinguishable from successful ones — an answer
/// landed, so the trail draws the success glyph and a refused patch proposal
/// reads exactly like an applied one. The wrapper's `error` and `reason`
/// members stay out of it: the rule that a label carries no text this app
/// authored for a reader gets no exception for the arm where that text happens
/// to be ours today.
fn unfinished_outcome_label(status: &str) -> Option<&'static str> {
    match status {
        "denied" => Some("Blocked — this call was not permitted"),
        "failed" => Some("Failed — the tool did not finish"),
        "invalid_arguments" => Some("Rejected — the arguments did not match this tool"),
        "cancelled" => Some("Cancelled"),
        "awaiting_approval" => Some("Waiting for a person to approve this"),
        // A status this app never writes. Silence, not a guess.
        _ => None,
    }
}

/// One step under construction: its label, the project-relative candidates the
/// outcome named in the order it named them, and the count the outcome itself
/// held.
struct StepDraft {
    label: String,
    candidates: Vec<(String, bool)>,
    path_total: u32,
    /// Set when the step's whole meaning IS its one path. Losing that path then
    /// has to lose the step, rather than render a verb with no object.
    path_is_the_step: bool,
}

impl StepDraft {
    /// A step that reports a count and names no file.
    fn counted(label: String, path_total: u32) -> Self {
        Self {
            label,
            candidates: Vec::new(),
            path_total,
            path_is_the_step: false,
        }
    }

    fn finish(self) -> Option<ToolStepSummaryV1> {
        let mut paths = Vec::new();
        for (raw, is_sensitive) in self.candidates {
            if paths.len() == TOOL_STEP_PATH_CAP {
                break;
            }
            // Re-validated here rather than trusted for having been ours when
            // it was written: these values round-tripped the DA data plane, and
            // a chip is this app asserting "the run touched this file". A
            // failure drops the path and leaves `path_total` alone — it lands
            // in the same shown-versus-held difference the display budget does.
            if let Ok(valid) = RelativeProjectPath::new(&raw) {
                paths.push(ToolStepPathV1 {
                    path: valid.as_str().to_owned(),
                    is_sensitive,
                });
            }
        }
        if self.path_is_the_step && paths.is_empty() {
            return None;
        }
        Some(ToolStepSummaryV1::new(self.label, paths, self.path_total))
    }
}

fn search_files_step(outcome: &Value) -> StepDraft {
    let results = outcome.get("results").and_then(Value::as_array);
    let total = bounded_count(results.map_or(0, Vec::len));
    StepDraft {
        label: format!(
            "Found {found}{cut}",
            found = file_count(total),
            cut = stopped_early(outcome)
        ),
        candidates: results
            .map(|rows| rows.iter().filter_map(classified_path).collect())
            .unwrap_or_default(),
        path_total: total,
        path_is_the_step: false,
    }
}

fn search_text_step(outcome: &Value) -> StepDraft {
    let hits = outcome.get("matches").and_then(Value::as_array);
    let hit_count = bounded_count(hits.map_or(0, Vec::len));
    // One chip per FILE, not per hit: twenty matches in one file is one file
    // the run read, and twenty identical chips would say otherwise.
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for path in hits
        .into_iter()
        .flatten()
        .filter_map(|hit| hit.get("path").and_then(Value::as_str))
    {
        if seen.insert(path) {
            // Never sensitive by construction: content search skips a
            // classified-sensitive candidate outright rather than matching in
            // it (see this module's own doc comment).
            candidates.push((path.to_owned(), false));
        }
    }
    let scanned = outcome
        .get("files_scanned")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    StepDraft {
        label: format!(
            "Searched {scanned} · {found}{cut}",
            scanned = file_count(bounded_count_u64(scanned)),
            found = match_count(hit_count),
            cut = stopped_early(outcome)
        ),
        // Its meaning is the two counts, so a scan whose hits all failed
        // validation — or that matched nothing at all — still says what it did.
        path_total: bounded_count(candidates.len()),
        candidates,
        path_is_the_step: false,
    }
}

fn read_file_step(outcome: &Value) -> Option<StepDraft> {
    let path = outcome.get("path").and_then(Value::as_str)?;
    let (label, is_sensitive) = match outcome.get("kind").and_then(Value::as_str)? {
        "text" if flag(outcome, "truncated") => ("Read · content cut short", false),
        "text" => ("Read", false),
        "metadata_only" => ("Read metadata only", false),
        // The classifier refused this one and no agent can grant itself the
        // override. Showing it as a plain read is the single thing this data
        // must never do.
        "sensitive_override_required" => {
            ("Blocked — a person must approve reading this file", true)
        }
        _ => return None,
    };
    Some(StepDraft {
        label: label.to_owned(),
        candidates: vec![(path.to_owned(), is_sensitive)],
        path_total: 1,
        path_is_the_step: true,
    })
}

fn list_directory_step(outcome: &Value, argument_text: Option<&str>) -> StepDraft {
    let entries = outcome.get("entries").and_then(Value::as_array);
    let total = bounded_count(entries.map_or(0, Vec::len));
    // A paged listing names the directory's size, not just this page's.
    let directory_total = outcome
        .get("total")
        .and_then(Value::as_u64)
        .map(bounded_count_u64)
        .filter(|all| *all > total);
    let label = match directory_total {
        Some(all) => format!("Listed {total} of {}", entry_count(all)),
        None => format!("Listed {listed}", listed = entry_count(total)),
    };
    // `run_list_directory` records bare `name` values and never the directory
    // they came from, so a chip drawn straight from one would name a file at
    // the project root that the run never touched — drawn identically to the
    // chip a read step draws. An unusable argument costs the chips, not the
    // step, and never falls back to bare names.
    let Some(directory) = listed_directory(argument_text) else {
        return StepDraft::counted(label, total);
    };
    StepDraft {
        label,
        candidates: entries
            .map(|rows| {
                rows.iter()
                    .filter_map(|entry| {
                        let (name, is_sensitive) = classified_name(entry)?;
                        Some((join_listed(directory.as_ref(), &name), is_sensitive))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        path_total: total,
        path_is_the_step: false,
    }
}

/// "Plan · 3 of 7 done": the checklist's progress, from the counts
/// [`run_update_plan`] publishes. Names no file.
fn update_plan_step(outcome: &Value) -> StepDraft {
    let count =
        |key: &str| bounded_count_u64(outcome.get(key).and_then(Value::as_u64).unwrap_or(0));
    StepDraft::counted(
        format!("Plan · {} of {} done", count("completed"), count("total")),
        0,
    )
}

/// One chip per file the write touched, from the `operations` rows
/// [`plan_json`] publishes. A rename names its destination: that is the path a
/// reader can open afterwards.
fn propose_patch_step(outcome: &Value) -> StepDraft {
    let total = bounded_count_u64(
        outcome
            .get("total_paths")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    let candidates = outcome
        .get("operations")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let path = row
                        .get("to")
                        .or_else(|| row.get("path"))
                        .and_then(Value::as_str)?;
                    // Sensitive paths are refused before any write, so none
                    // reaches here marked.
                    Some((path.to_owned(), false))
                })
                .collect()
        })
        .unwrap_or_default();
    let applied = outcome
        .get("applied")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Answers stored before rows carried a path or an `index` read "proposed",
    // whether or not they were applied. They stay in old conversations, so the
    // reader still names them the way it always did.
    let has_indexed_rows = outcome
        .get("operations")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .is_some_and(|row| row.get("index").is_some());
    StepDraft {
        label: if applied {
            format!("Changed {touched}", touched = file_count(total))
        } else if has_indexed_rows {
            format!("Could not change {touched}", touched = file_count(total))
        } else {
            format!("Proposed changes to {touched}", touched = file_count(total))
        },
        candidates,
        path_total: total,
        path_is_the_step: false,
    }
}

/// The directory a listing named, from the call's own durable argument row.
///
/// `Some(None)` is the project root — how [`run_list_directory`] spells it —
/// where an entry name already IS the project-relative path. `None` is an
/// argument this reader could not validate, which must cost the chips rather
/// than root them at a guess.
fn listed_directory(argument_text: Option<&str>) -> Option<Option<RelativeProjectPath>> {
    let Some(text) = argument_text else {
        return Some(None);
    };
    let arguments: Value = serde_json::from_str(text).ok()?;
    match arguments.get("path").and_then(Value::as_str) {
        None | Some("") => Some(None),
        Some(raw) => RelativeProjectPath::new(raw).ok().map(Some),
    }
}

/// Joins one entry name onto the listed directory. The result is validated as a
/// path in its own right by [`StepDraft::finish`] — a name is a name until then.
fn join_listed(directory: Option<&RelativeProjectPath>, name: &str) -> String {
    match directory {
        None => name.to_owned(),
        Some(directory) => format!("{parent}/{name}", parent = directory.as_str()),
    }
}

/// A search hit's path and the classifier's verdict on it.
fn classified_path(row: &Value) -> Option<(String, bool)> {
    Some((
        row.get("path").and_then(Value::as_str)?.to_owned(),
        sensitive_unless_proven_otherwise(row),
    ))
}

/// A directory entry's name and the classifier's verdict on it.
fn classified_name(entry: &Value) -> Option<(String, bool)> {
    Some((
        entry.get("name").and_then(Value::as_str)?.to_owned(),
        sensitive_unless_proven_otherwise(entry),
    ))
}

/// Reads the classifier's verdict, defaulting to *marked* when the flag cannot
/// be read at all.
///
/// Both arms that produce these rows always write the flag, so an unreadable
/// one means the shape moved. The two errors are not symmetrical: an extra cue
/// on an ordinary file is noise a reader can see and question, while a missing
/// cue on a withheld candidate presents it as an ordinary hit — the one
/// confusion this data must never create.
fn sensitive_unless_proven_otherwise(row: &Value) -> bool {
    row.get("is_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// The TOOL's own caps stopped the scan early — a different fact from the
/// display budget, which means this card is showing fewer chips than it holds.
fn stopped_early(outcome: &Value) -> &'static str {
    if flag(outcome, "truncated") || flag(outcome, "index_truncated") {
        " · stopped early"
    } else {
        ""
    }
}

fn flag(outcome: &Value, key: &str) -> bool {
    outcome.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn bounded_count(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn bounded_count_u64(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn file_count(count: u32) -> String {
    if count == 1 {
        "1 file".to_owned()
    } else {
        format!("{count} files")
    }
}

fn entry_count(count: u32) -> String {
    if count == 1 {
        "1 entry".to_owned()
    } else {
        format!("{count} entries")
    }
}

fn match_count(count: u32) -> String {
    if count == 1 {
        "1 match".to_owned()
    } else {
        format!("{count} matches")
    }
}

/// Recorded ANSWER TEXT: the string a run stores in the pairing `role="tool"`
/// row, `crate::agent::turn_driver::render_tool_outcome`'s wrapper included.
///
/// Recorded at THIS boundary rather than at [`run_tool`]'s return value on
/// purpose. A fixture taken one layer deeper would agree with the reader about a
/// document production never hands it, and would pass while the app showed
/// nothing: a reader pointed at the wrapper's top level finds none of the keys
/// it wants, for every tool, on every call — and returns `None`, which is also
/// its correct answer for a tool it does not recognize. Silence is the symptom
/// of both, so only a fixture recorded here can tell them apart.
///
/// Pinned from two directions: `crate::tools::tests::dispatch` asserts a live
/// dispatch still writes these shapes, and `crate::tools::tests::step_summary`
/// asserts the reader still reads them.
#[cfg(test)]
pub(crate) mod recorded_answers {
    pub(crate) const SEARCH_FILES: &str = r#"{"status":"completed","result":{"results":[{"path":"src/features/cards.tsx","is_sensitive":false,"content_kind":"text","size_bytes":1024},{"path":".env","is_sensitive":true,"content_kind":"text","size_bytes":48}],"truncated":false,"index_truncated":false,"live_scanned":0},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    pub(crate) const SEARCH_FILES_CUT_SHORT: &str = r#"{"status":"completed","result":{"results":[{"path":"src/features/cards.tsx","is_sensitive":false,"content_kind":"text","size_bytes":1024}],"truncated":true,"index_truncated":false},"truncated":false,"content_refs":[]}"#;

    pub(crate) const SEARCH_TEXT: &str = r#"{"status":"completed","result":{"matches":[{"path":"src/app.ts","line":12,"preview":"let value = 1"},{"path":"src/app.ts","line":40,"preview":"let value = 2"},{"path":"src/main.ts","line":7,"preview":"let value = 3"}],"truncated":false,"index_truncated":false,"files_scanned":128,"files_stale":0,"live_scanned":0},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    /// The model authored this query; the reader must keep it out of the label.
    pub(crate) const SEARCH_TEXT_ARGUMENTS: &str = r#"{"query":"; DROP TABLE users; --"}"#;

    pub(crate) const READ_FILE: &str = r#"{"status":"completed","result":{"kind":"text","path":"README.md","content":"hello world\n","first_line":1,"last_line":1,"truncated":false,"next_line":null,"detail":null},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    pub(crate) const READ_FILE_SENSITIVE: &str = r#"{"status":"completed","result":{"kind":"sensitive_override_required","path":".env","secret_class":"dot_env","detail":"This file is classified sensitive. A person must approve reading it; an agent cannot grant that itself."},"truncated":false,"content_refs":[]}"#;

    pub(crate) const READ_FILE_TRAVERSAL: &str = r#"{"status":"completed","result":{"kind":"text","path":"../../etc/passwd","content":"root:x:0:0","first_line":1,"last_line":1,"truncated":false,"next_line":null,"detail":null},"truncated":false,"content_refs":[]}"#;

    pub(crate) const LIST_DIRECTORY: &str = r#"{"status":"completed","result":{"entries":[{"name":"cards.tsx","kind":"file","size_bytes":1024,"is_sensitive":false},{"name":"id_ed25519","kind":"file","size_bytes":464,"is_sensitive":true}],"offset":0,"total":2,"next_offset":null,"detail":null},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    /// One page of a larger directory: two of 5,000 entries shown.
    pub(crate) const LIST_DIRECTORY_PAGED: &str = r#"{"status":"completed","result":{"entries":[{"name":"a.csv","kind":"file","size_bytes":10,"is_sensitive":false},{"name":"b.csv","kind":"file","size_bytes":10,"is_sensitive":false}],"offset":0,"total":5000,"next_offset":2,"detail":"2 of 5000 entries shown. Call project.list_directory again with offset 2 for the rest."},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    /// Historical shape written before listings paged: entries only.
    pub(crate) const LIST_DIRECTORY_UNPAGED: &str = r#"{"status":"completed","result":{"entries":[{"name":"cards.tsx","kind":"file","size_bytes":1024,"is_sensitive":false},{"name":"id_ed25519","kind":"file","size_bytes":464,"is_sensitive":true}]},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    pub(crate) const LIST_DIRECTORY_ARGUMENTS: &str = r#"{"path":"src/features"}"#;

    /// How `run_list_directory` spells the project root: the key is absent.
    pub(crate) const LIST_DIRECTORY_ROOT_ARGUMENTS: &str = r#"{}"#;

    pub(crate) const PROPOSE_PATCH: &str = r#"{"status":"completed","result":{"applied":true,"plan_id":"9f2b","total_paths":3,"total_bytes":2048,"operations":[{"index":0,"kind":"create","path":"src/app.ts","bytes":512,"applied":true},{"index":1,"kind":"modify","path":"src/util.ts","bytes":1024,"applied":true},{"index":2,"kind":"delete","path":"src/old.ts","applied":true}],"detail":"Applied. The change is written to the project on disk."},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;
    /// What a run stored before write results named their paths.
    pub(crate) const LEGACY_PROPOSE_PATCH: &str = r#"{"status":"completed","result":{"plan_id":"9f2b","risk":"standard","requires_second_confirmation":false,"total_paths":3,"total_bytes":2048,"expires_at_ms":1783500000000,"unified_diff":"--- a/src/app.ts\n+++ b/src/app.ts\n","operations":[{"kind":"create"},{"kind":"modify"},{"kind":"delete"}],"applied":false,"detail":"Proposed only. Nothing is written until a person approves this exact plan id."},"preview_complete":true,"preview_strategy":"complete","exact_byte_length":0,"sha256":"fixture","source_truncated":false,"content_refs":[]}"#;

    pub(crate) const DENIED: &str = r#"{"status":"denied","reason":"project.not_active"}"#;

    pub(crate) const FAILED: &str =
        r#"{"status":"failed","error":"the project index is unavailable","retryable":false}"#;

    /// Historical shape written before exact payload persistence existed.
    pub(crate) const RESULT_DROPPED_FOR_SIZE: &str =
        r#"{"status":"completed","result":null,"truncated":true,"content_refs":[]}"#;
}
