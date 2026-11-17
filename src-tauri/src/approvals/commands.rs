//! Tauri command surface for saving a file a person edited in the viewer.
//! Shaped after `crate::app::commands`'s `project_create`/`project_list`/
//! `project_activate`/`project_revoke` (thin wrapper, `Result<_, String>`,
//! all logic lives in the service).
//!
//! Registered in `src-tauri/src/lib.rs`'s `tauri::generate_handler!`. It takes
//! `tauri::State<'_, Arc<ApprovalsService>>` directly rather than reaching
//! through `crate::app::commands::AppRuntimeState`, because approvals are a
//! self-contained surface with no dependency on the rest of that runtime. The
//! one thing read from that runtime is whether the account is signed in: every
//! command here writes or reads project files on this device, and no server
//! checks the bearer on the way (see `AppRuntimeState::is_signed_in`).
//!
//! A person editing a local file saves the change directly because the editor
//! already shows the exact bytes they authored. Agent patches apply the same
//! way, from the project tool dispatcher: trusting the folder is the consent.
//! Both paths still go through `crate::projects::execute`, which re-validates
//! containment on its own, and both keep every overwritten or deleted file as
//! a revision this module can list, diff, and restore.

use std::str::FromStr;
use std::sync::Arc;

use tauri::State;

use crate::app::commands::AppRuntimeState;
use crate::projects::{sensitive_class_projection, RevisionDiff, RevisionDiffLimit};
use crate::projects::{EditOperation, ProjectId, RelativeProjectPath};
use xfalcon_contracts::projects::{
    ProjectFileRestoreResult, ProjectFileRevisionDiff, ProjectFileRevisionList,
    ProjectFileRevisionView, ProjectId as ProjectIdView, ProjectRevisionDiffLimit,
};
use xfalcon_contracts::CONTRACT_VERSION;

use super::service::{ApprovalOutcome, ApprovalsService, ApprovalsServiceError};

/// Refuses a command unless the bound account is signed in, with the same
/// `NoActiveAccount` a signed-out device already answers with.
fn require_signed_in(runtime: &AppRuntimeState) -> Result<(), String> {
    if runtime.is_signed_in() {
        Ok(())
    } else {
        Err(ApprovalsServiceError::NoActiveAccount.to_string())
    }
}

/// Writes `content_base64` over `path` inside `project_id`'s trusted root.
///
/// Content is base64 rather than a JSON string for the reason the persisted
/// `ApprovalPayload` inside `service.rs` uses it: one encoding represents both
/// text and binary bytes without the caller declaring which it has.
#[tauri::command]
pub async fn project_save_file(
    runtime: State<'_, AppRuntimeState>,
    service: State<'_, Arc<ApprovalsService>>,
    project_id: String,
    path: String,
    content_base64: String,
) -> Result<(), String> {
    use base64::Engine as _;
    require_signed_in(&runtime)?;
    let project_id = ProjectId::from_str(&project_id).map_err(|error| error.to_string())?;
    let path = RelativeProjectPath::new(&path)
        .map_err(|error| format!("invalid path {path:?}: {error}"))?;
    let content = base64::engine::general_purpose::STANDARD
        .decode(content_base64)
        .map_err(|error| format!("invalid base64 content: {error}"))?;

    let outcome = service
        .save(project_id, vec![EditOperation::Modify { path, content }])
        .await
        .map_err(|error| error.to_string())?;

    // `save` approves the plan it just built, so a denial is unreachable rather
    // than merely unlikely -- and a partial execution of a one-operation plan
    // is a contradiction. Both are reported instead of being reduced to
    // success: a save that says nothing must mean the bytes are on disk.
    match outcome {
        ApprovalOutcome::Executed(execution)
            if execution.operations_completed == execution.operations_total =>
        {
            Ok(())
        }
        ApprovalOutcome::Executed(execution) => Err(format!(
            "saved {completed} of {total} changes",
            completed = execution.operations_completed,
            total = execution.operations_total
        )),
        ApprovalOutcome::Denied => Err("this save was refused before anything was written".into()),
    }
}

/// Every saved revision of one file. Off the main thread: each revision is
/// decrypted in full to read its header, so a file with many large revisions
/// would otherwise hold the window for the whole listing.
#[tauri::command]
pub async fn project_file_revisions(
    runtime: State<'_, AppRuntimeState>,
    service: State<'_, Arc<ApprovalsService>>,
    project_id: String,
    relative_path: String,
) -> Result<ProjectFileRevisionList, String> {
    require_signed_in(&runtime)?;
    let project_id = ProjectId::from_str(&project_id).map_err(|error| error.to_string())?;
    let path = RelativeProjectPath::new(&relative_path)
        .map_err(|error| format!("invalid path {relative_path:?}: {error}"))?;
    let service = service.inner().clone();
    let revisions = tokio::task::spawn_blocking(move || service.file_revisions(project_id, &path))
        .await
        .map_err(|_| "the revision list worker stopped".to_owned())?
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|revision| ProjectFileRevisionView {
            version: CONTRACT_VERSION,
            backup_id: revision.backup_id.to_string(),
            created_at_ms: revision.created_at_ms,
            original_bytes: revision.original_bytes,
            compressed_bytes: revision.compressed_bytes,
            content_hash: revision.content_hash,
        })
        .collect();
    Ok(ProjectFileRevisionList {
        version: CONTRACT_VERSION,
        project_id: ProjectIdView::from(project_id.to_string()),
        relative_path,
        revisions,
    })
}

/// What changed in a file since one of its revisions, bounded (spec:
/// project-file-patch-inspector). Off the main thread: decrypting a revision
/// and diffing it can take a noticeable moment, and a sync command would hold
/// the window while it did.
#[tauri::command]
pub async fn project_file_revision_diff(
    runtime: State<'_, AppRuntimeState>,
    service: State<'_, Arc<ApprovalsService>>,
    project_id: String,
    relative_path: String,
    backup_id: String,
    against_backup_id: Option<String>,
) -> Result<ProjectFileRevisionDiff, String> {
    require_signed_in(&runtime)?;
    let project_id = ProjectId::from_str(&project_id).map_err(|error| error.to_string())?;
    let path = RelativeProjectPath::new(&relative_path)
        .map_err(|error| format!("invalid path {relative_path:?}: {error}"))?;
    let backup_id = uuid::Uuid::parse_str(&backup_id)
        .map_err(|_| "backup id is not a valid UUID".to_owned())?;
    let against = against_backup_id
        .map(|id| uuid::Uuid::parse_str(&id))
        .transpose()
        .map_err(|_| "backup id is not a valid UUID".to_owned())?;
    let service = service.inner().clone();
    let diff = tokio::task::spawn_blocking(move || {
        service.revision_diff(project_id, &path, backup_id, against)
    })
    .await
    .map_err(|_| "the revision diff worker stopped".to_owned())?
    .map_err(|error| error.to_string())?;
    Ok(revision_diff_projection(backup_id.to_string(), diff))
}

fn revision_diff_projection(backup_id: String, diff: RevisionDiff) -> ProjectFileRevisionDiff {
    let version = CONTRACT_VERSION;
    match diff {
        RevisionDiff::Changes {
            unified_diff,
            added_lines,
            removed_lines,
        } => ProjectFileRevisionDiff::Changes {
            version,
            backup_id,
            unified_diff,
            // Bounded by `MAX_REVISION_DIFF_CHANGED_LINES`, far inside `u32`.
            added_lines: u32::try_from(added_lines).unwrap_or(u32::MAX),
            removed_lines: u32::try_from(removed_lines).unwrap_or(u32::MAX),
        },
        RevisionDiff::Unchanged => ProjectFileRevisionDiff::Unchanged { version, backup_id },
        RevisionDiff::NotText => ProjectFileRevisionDiff::NotText { version, backup_id },
        RevisionDiff::TooLarge(limit) => ProjectFileRevisionDiff::TooLarge {
            version,
            backup_id,
            limit: match limit {
                RevisionDiffLimit::FileSize => ProjectRevisionDiffLimit::FileSize,
                RevisionDiffLimit::ChangeSize => ProjectRevisionDiffLimit::ChangeSize,
            },
        },
        RevisionDiff::Sensitive(class) => ProjectFileRevisionDiff::SensitiveWithheld {
            version,
            backup_id,
            sensitive_class: sensitive_class_projection(class),
        },
    }
}

#[tauri::command]
pub async fn project_file_restore(
    runtime: State<'_, AppRuntimeState>,
    service: State<'_, Arc<ApprovalsService>>,
    backup_id: String,
) -> Result<ProjectFileRestoreResult, String> {
    require_signed_in(&runtime)?;
    let backup_id = uuid::Uuid::parse_str(&backup_id)
        .map_err(|_| "backup id is not a valid UUID".to_owned())?;
    let restored = service
        .restore_revision(backup_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ProjectFileRestoreResult {
        version: CONTRACT_VERSION,
        project_id: ProjectIdView::from(restored.project_id.to_string()),
        relative_path: restored.relative_path.as_str().to_owned(),
    })
}
