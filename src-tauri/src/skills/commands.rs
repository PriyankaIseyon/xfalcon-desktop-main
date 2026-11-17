//! The four commands the Skills settings surface uses, and the picker that
//! feeds the import.
//!
//! Paths never cross the boundary, and neither do bytes. A pick (`skill_pick`)
//! or a drop (`crate::skills::register_skill_drop_handler`) is recorded in
//! `crate::path_grants` and the renderer is given only an id; the import takes
//! that id. The renderer may not read a file (`src/security/frontend-boundary`),
//! so this side does the reading, the parsing, and the storing.
//!
//! Every failure crosses as a [`SkillOperationError`] code. A *file's* failure
//! is different and does not use that channel — it rides back inside
//! [`SkillImportResult::failed`] with its own sentence, because "this one file
//! is malformed" is an outcome of a successful import, not a failed one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::Manager as _;
use tauri_plugin_dialog::{DialogExt as _, FilePath};
use xfalcon_contracts::skills::{
    SkillErrorCode, SkillImportFailure, SkillImportResult, SkillListResult, SkillOperationError,
    SkillPickKind, SkillSourceGrantView, SkillView,
};
use xfalcon_contracts::ContractVersion;

use super::store::{SkillStore, SkillStoreError, StoredSkill};
use crate::app::commands::AppRuntimeState;
use crate::audit::{emit, AuditAction, AuditOutcome, AuditResourceType};
use crate::path_grants::{GrantKind, PathGrants};
use crate::security::account_cleanup::ValidatedAccountId;

/// The file types the file picker offers: a skill file or an archive of them.
const SKILL_FILTER_NAME: &str = "Skill or archive";
const SKILL_EXTENSIONS: [&str; 3] = ["md", "skill", "zip"];

/// Source names a grant view carries. Enough to show what was chosen; a drop of
/// a folder full of files does not put a hundred names on the wire.
const MAX_LABELS: usize = 5;

impl From<SkillStoreError> for SkillOperationError {
    fn from(error: SkillStoreError) -> Self {
        Self::new(match error {
            SkillStoreError::UnknownSkill(_) | SkillStoreError::DisabledSkill(_) => {
                SkillErrorCode::NotFound
            }
            SkillStoreError::Discovery(_) | SkillStoreError::Storage { .. } => {
                SkillErrorCode::StorageUnavailable
            }
        })
    }
}

/// A skill as Settings shows it.
///
/// Infallible on purpose: the count of bundled files is a detail, and a
/// folder that cannot be listed must neither hide the library nor turn an
/// import that already stored its skills into a reported failure. It is
/// logged and shown as zero.
fn view(store: &SkillStore, skill: &StoredSkill) -> SkillView {
    let bundled_files = match store.bundled_files(skill) {
        Ok(files) => files.len(),
        Err(error) => {
            // The kind, not the error: its text carries this device's
            // app-data path, and with it the user's account name.
            tracing::warn!(
                skill = %skill.name(),
                kind = error.kind_label(),
                "skills: bundled files could not be listed"
            );
            0
        }
    };
    SkillView {
        version: ContractVersion::V1,
        name: skill.name().to_owned(),
        description: skill.description().to_owned(),
        declared_version: skill.discovered.manifest().version().as_str().to_owned(),
        enabled: skill.enabled,
        content_hash: skill.discovered.content_hash().to_owned(),
        // Bounded by `MAX_BUNDLED_FILES`, far below `u32::MAX`.
        bundled_files: u32::try_from(bundled_files).unwrap_or(u32::MAX),
    }
}

/// Where the signed-in account's library lives. Resolved once per command,
/// so an import that is running when the account changes finishes in the
/// library it started in rather than splitting across two.
fn library(
    app: &tauri::AppHandle,
) -> Result<(std::path::PathBuf, ValidatedAccountId), SkillOperationError> {
    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|_| SkillOperationError::new(SkillErrorCode::StorageUnavailable))?;
    let account = app
        .state::<AppRuntimeState>()
        .accounts()
        .current()
        .map_err(|_| SkillOperationError::new(SkillErrorCode::StorageUnavailable))?
        .ok_or_else(|| SkillOperationError::new(SkillErrorCode::NoActiveAccount))?;
    Ok((app_local_data, account))
}

/// Runs a store operation off the main thread, opening the store there too.
/// Tauri runs a synchronous command on the main thread, and an import that
/// unpacks a 30 MB package there freezes the window — the progress spinner
/// included. Opening can move the device-wide library into the account, so
/// it is file work as well and belongs on the same blocking thread.
async fn blocking<T: Send + 'static>(
    app: &tauri::AppHandle,
    operation: impl FnOnce(SkillStore) -> Result<T, SkillOperationError> + Send + 'static,
) -> Result<T, SkillOperationError> {
    let (app_local_data, account) = library(app)?;
    tokio::task::spawn_blocking(move || operation(SkillStore::open(&app_local_data, &account)))
        .await
        .map_err(|_| SkillOperationError::new(SkillErrorCode::StorageUnavailable))?
}

#[tauri::command]
pub async fn skill_list(app: tauri::AppHandle) -> Result<SkillListResult, SkillOperationError> {
    blocking(&app, |store| {
        let skills = store
            .list()?
            .iter()
            .map(|skill| view(&store, skill))
            .collect();
        Ok(SkillListResult::new(skills))
    })
    .await
}

/// Opens the native picker Settings asked for and records the choice as a
/// grant. Cancelling is `None`, not an error; a selection too large to grant is
/// an error, so the person is told rather than seeing nothing happen.
#[tauri::command]
pub async fn skill_pick(
    app: tauri::AppHandle,
    kind: SkillPickKind,
) -> Result<Option<SkillSourceGrantView>, SkillOperationError> {
    let picked = match kind {
        SkillPickKind::Files => app
            .dialog()
            .file()
            .add_filter(SKILL_FILTER_NAME, &SKILL_EXTENSIONS)
            .blocking_pick_files(),
        SkillPickKind::Folder => app
            .dialog()
            .file()
            .blocking_pick_folder()
            .map(|folder| vec![folder]),
    };
    let Some(picked) = picked else {
        return Ok(None);
    };
    let paths = picked
        .into_iter()
        .map(FilePath::into_path)
        .collect::<Result<Vec<PathBuf>, _>>()
        // A selection this desktop cannot name as a file has nothing to import.
        .map_err(|_| SkillOperationError::new(SkillErrorCode::StorageUnavailable))?;
    grant_sources(&app.state::<Arc<PathGrants>>(), paths)
}

#[tauri::command]
pub async fn skill_import(
    app: tauri::AppHandle,
    grant_id: String,
) -> Result<SkillImportResult, SkillOperationError> {
    let chosen = resolve_sources(&app.state::<Arc<PathGrants>>(), &grant_id);
    let result = match chosen {
        Ok(chosen) => import_chosen(&app, chosen).await,
        Err(error) => Err(error),
    };
    emit::record(
        &app,
        AuditAction::SkillImport,
        AuditResourceType::Skill,
        "import",
        skill_import_outcome(&result),
    );
    result
}

/// Imports the chosen sources into the signed-in account's library.
///
/// A selection where every file was malformed is still a successful import
/// call: the per-file reasons ARE the answer the user needs, and collapsing
/// them into one error code would throw them away.
async fn import_chosen(
    app: &tauri::AppHandle,
    chosen: Vec<PathBuf>,
) -> Result<SkillImportResult, SkillOperationError> {
    blocking(app, move |store| {
        let outcome = store.import(&chosen);
        Ok(SkillImportResult::new(
            outcome
                .imported
                .iter()
                .map(|skill| view(&store, skill))
                .collect(),
            outcome
                .failed
                .into_iter()
                .map(|failure| SkillImportFailure {
                    path: failure_name(&failure.path),
                    reason: failure.reason,
                })
                .collect(),
        ))
    })
    .await
}

/// The sources a skill grant names, or the refusal. An unknown, expired, or
/// wrong-kind id is the same miss as a skill that is not there, from the
/// caller's side.
pub(crate) fn resolve_sources(
    grants: &PathGrants,
    grant_id: &str,
) -> Result<Vec<PathBuf>, SkillOperationError> {
    grants
        .resolve(GrantKind::SkillSources, grant_id)
        .ok_or_else(|| SkillOperationError::new(SkillErrorCode::InvalidChoice))
}

/// Records `paths` as a skill-source grant and describes it for the renderer.
/// `Ok(None)` when there is nothing to grant; `SelectionTooLarge` when the
/// selection is over the grant limit. Shared by the picker and the drop, so
/// both give the renderer the same view.
pub(crate) fn grant_sources(
    grants: &PathGrants,
    paths: Vec<PathBuf>,
) -> Result<Option<SkillSourceGrantView>, SkillOperationError> {
    if paths.is_empty() {
        return Ok(None);
    }
    let count = paths.len();
    let labels = paths
        .iter()
        .take(MAX_LABELS)
        .map(|path| label(path))
        .collect();
    let Some(grant_id) = grants.issue(GrantKind::SkillSources, paths) else {
        // Nothing is granted, so nothing is imported: a partial import of a
        // selection this large would not be the selection the person made.
        tracing::warn!("a skill selection exceeded the grant limit");
        return Err(SkillOperationError::new(SkillErrorCode::SelectionTooLarge));
    };
    Ok(Some(SkillSourceGrantView::new(
        grant_id,
        u32::try_from(count).unwrap_or(u32::MAX),
        labels,
    )))
}

/// The last component of a source, for display. A path with no last component
/// (a drive or a root) is shown whole.
fn label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The name a failed source is shown by: its last component and nothing above
/// it, so no absolute path reaches the renderer. A root has no name, so it is
/// shown as nothing.
fn failure_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// A selection fails only when every source failed. An import that stored at
/// least one skill, or a selection with nothing to import, is a success.
fn skill_import_outcome(result: &Result<SkillImportResult, SkillOperationError>) -> AuditOutcome {
    match result {
        Ok(imported) if imported.imported.is_empty() && !imported.failed.is_empty() => {
            AuditOutcome::Failure
        }
        Ok(_) => AuditOutcome::Success,
        Err(_) => AuditOutcome::Failure,
    }
}

#[tauri::command]
pub async fn skill_set_enabled(
    app: tauri::AppHandle,
    name: String,
    enabled: bool,
) -> Result<SkillView, SkillOperationError> {
    blocking(&app, move |store| {
        let skill = store.set_enabled(&name, enabled)?;
        Ok(view(&store, &skill))
    })
    .await
}

#[tauri::command]
pub async fn skill_delete(app: tauri::AppHandle, name: String) -> Result<(), SkillOperationError> {
    let audited_id = audited_skill_id(&name);
    let result = blocking(&app, move |store| Ok(store.delete(&name)?)).await;
    emit::record(
        &app,
        AuditAction::SkillDelete,
        AuditResourceType::Skill,
        &audited_id,
        emit::outcome_for(&result),
    );
    result
}

/// The audit resource id for a skill: a short digest of its name. A skill name
/// is user content (it can be a title), and the audit trail is exported with
/// diagnostics, so the name itself must never be written there. Twelve hex
/// characters keep one skill's events together without naming it.
fn audited_skill_id(name: &str) -> String {
    blake3::hash(name.as_bytes()).to_hex()[..12].to_owned()
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use crate::audit::{emit::test_hub, AuditEvent};
    use tempfile::tempdir;
    use xfalcon_contracts::skills::SkillImportFailure;

    fn failed_source() -> SkillImportFailure {
        SkillImportFailure {
            path: "/private/folder/bad-skill.zip".to_owned(),
            reason: "malformed manifest".to_owned(),
        }
    }

    #[test]
    fn an_import_where_every_source_failed_is_a_failure() {
        let all_failed = Ok(SkillImportResult::new(vec![], vec![failed_source()]));
        assert_eq!(skill_import_outcome(&all_failed), AuditOutcome::Failure);
        let nothing_to_import = Ok(SkillImportResult::new(vec![], vec![]));
        assert_eq!(
            skill_import_outcome(&nothing_to_import),
            AuditOutcome::Success
        );
        let partial = Ok(SkillImportResult::new(
            vec![SkillView {
                version: xfalcon_contracts::ContractVersion::V1,
                name: "ok".to_owned(),
                description: String::new(),
                declared_version: "1.0.0".to_owned(),
                enabled: true,
                content_hash: String::new(),
                bundled_files: 0,
            }],
            vec![failed_source()],
        ));
        assert_eq!(skill_import_outcome(&partial), AuditOutcome::Success);
        let unavailable = Err(SkillOperationError::new(SkillErrorCode::NoActiveAccount));
        assert_eq!(skill_import_outcome(&unavailable), AuditOutcome::Failure);
    }

    #[test]
    fn skill_import_and_delete_record_ids_and_outcomes_without_paths() {
        let root = tempdir().expect("temp dir");
        let app = tauri::test::mock_app();
        let hub = test_hub(app.handle(), root.path());

        let failing_import: Result<SkillImportResult, SkillOperationError> =
            Ok(SkillImportResult::new(vec![], vec![failed_source()]));
        emit::record(
            app.handle(),
            AuditAction::SkillImport,
            AuditResourceType::Skill,
            "import",
            skill_import_outcome(&failing_import),
        );
        emit::record(
            app.handle(),
            AuditAction::SkillDelete,
            AuditResourceType::Skill,
            &audited_skill_id("release-notes"),
            AuditOutcome::Success,
        );

        let mut events: Vec<AuditEvent> = hub
            .store()
            .recent(10)
            .expect("read audit log")
            .iter()
            .map(|record| record.event().clone())
            .collect();
        events.sort_by_key(AuditEvent::sequence);
        let summary: Vec<_> = events
            .iter()
            .map(|event| {
                (
                    event.action(),
                    event.resource_id().to_owned(),
                    event.outcome(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    AuditAction::SkillImport,
                    "import".to_owned(),
                    AuditOutcome::Failure,
                ),
                (
                    AuditAction::SkillDelete,
                    audited_skill_id("release-notes"),
                    AuditOutcome::Success,
                ),
            ]
        );
        let serialized = serde_json::to_string(&events).expect("serialize events");
        assert!(!serialized.contains("bad-skill"));
        assert!(!serialized.contains("/private/folder"));
        assert!(!serialized.contains("release-notes"));
    }

    #[test]
    fn a_deleted_skill_is_audited_by_a_digest_that_never_contains_its_name() {
        let title = "Q3 acquisition plan";
        let id = audited_skill_id(title);

        assert_eq!(id, audited_skill_id(title), "one name keeps one id");
        assert_ne!(id, audited_skill_id("Q3 acquisition plans"));
        assert_eq!(id.len(), 12);
        assert!(id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert!(!id.contains(title));
        assert!(!id.contains("acquisition"));
    }

    #[test]
    fn a_failed_source_is_named_by_its_file_name_only() {
        assert_eq!(failure_name(Path::new("/a/b/c/bad.zip")), "bad.zip");
        assert_eq!(failure_name(Path::new("/")), "");
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;
    use crate::path_grants::MAX_PATHS_PER_GRANT;

    #[test]
    fn a_skill_grant_resolves_to_the_sources_it_names() {
        let grants = PathGrants::default();
        let id = grants
            .issue(GrantKind::SkillSources, vec![PathBuf::from("/chosen/a.md")])
            .expect("within the grant limit");

        assert_eq!(
            resolve_sources(&grants, &id),
            Ok(vec![PathBuf::from("/chosen/a.md")])
        );
    }

    #[test]
    fn an_unknown_or_wrong_kind_skill_grant_is_refused_as_an_invalid_choice() {
        let grants = PathGrants::default();
        let folder = grants
            .issue(GrantKind::Folder, vec![PathBuf::from("/chosen/project")])
            .expect("within the grant limit");

        for id in ["00000000000000000000000000000000", folder.as_str()] {
            assert_eq!(
                resolve_sources(&grants, id),
                Err(SkillOperationError::new(SkillErrorCode::InvalidChoice)),
                "{id} must be refused"
            );
        }
    }

    #[test]
    fn a_selection_with_nothing_in_it_grants_nothing() {
        let grants = PathGrants::default();
        assert_eq!(grant_sources(&grants, Vec::new()), Ok(None));
    }

    #[test]
    fn a_grant_view_counts_every_source_but_names_only_the_first_few() {
        let grants = PathGrants::default();
        let paths: Vec<PathBuf> = (0..7)
            .map(|index| PathBuf::from(format!("/chosen/skill-{index}.md")))
            .collect();

        let view = grant_sources(&grants, paths)
            .expect("a selection within the limit is not refused")
            .expect("a selection is granted");

        assert_eq!(view.count, 7);
        assert_eq!(
            view.labels,
            vec![
                "skill-0.md",
                "skill-1.md",
                "skill-2.md",
                "skill-3.md",
                "skill-4.md"
            ]
        );
    }

    #[test]
    fn a_selection_past_the_grant_limit_is_refused_whole_and_imports_nothing() {
        let grants = PathGrants::default();
        let paths: Vec<PathBuf> = (0..MAX_PATHS_PER_GRANT + 1)
            .map(|index| PathBuf::from(format!("/chosen/{index}.md")))
            .collect();

        assert_eq!(
            grant_sources(&grants, paths),
            Err(SkillOperationError::new(SkillErrorCode::SelectionTooLarge))
        );
        // Nothing was recorded, so no id from it can import a prefix.
        assert_eq!(grants.resolve(GrantKind::SkillSources, "anything"), None);
    }

    #[test]
    fn a_selection_at_the_grant_limit_is_granted_in_full() {
        let grants = PathGrants::default();
        let paths: Vec<PathBuf> = (0..MAX_PATHS_PER_GRANT)
            .map(|index| PathBuf::from(format!("/chosen/{index}.md")))
            .collect();

        let view = grant_sources(&grants, paths)
            .expect("a selection at the limit is not refused")
            .expect("a selection at the limit is granted");

        assert_eq!(view.count as usize, MAX_PATHS_PER_GRANT);
        assert_eq!(
            resolve_sources(&grants, &view.grant_id).map(|paths| paths.len()),
            Ok(MAX_PATHS_PER_GRANT)
        );
    }
}
