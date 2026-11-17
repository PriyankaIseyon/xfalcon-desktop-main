//! Tauri command surface for user-initiated directory listings and file reads
//! inside a trusted project.
//!
//! # One set of gates, sized by who reads
//!
//! Every command resolves a [`super::domain::TrustedRoot`] from
//! [`super::trust::ProjectsService`] and reads through the same gates
//! `crate::tools::dispatch` uses for the agent: the path policy, containment
//! and root identity revalidated on every open, and the sensitivity decision
//! made from the path before the file is touched. Nothing here re-derives any
//! of them, so a person and an agent cannot diverge on *what* may be read —
//! the plan's decision D7 (`docs/decisions.md`).
//!
//! What differs is *how much*, because the reader differs. The agent's reads
//! go to a model and are paged to its run's budget (`super::read`). A person's
//! go to this device's own webview (`super::viewer`, `super::table`): raw bytes
//! in ranges of up to 8 MiB, files of up to 10 GiB, tables paged by the
//! backend (spec: project-file-editing).
//!
//! # Shape of the wrappers
//!
//! Thin `Result<_, String>` wrappers over `projects`-private functions that take
//! `&ProjectsService`, matching `crate::approvals::commands`'s idiom (all logic
//! in the service, none in the command) and `crate::app::commands`'s
//! `project_create`/`project_list`/`project_activate`/`project_revoke` -- the
//! immediate siblings of these two, which reach the service through
//! [`AppRuntimeState`] exactly like this. Reusing that route rather than
//! managing `Arc<ProjectsService>` as its own Tauri state means registering
//! these commands in `src-tauri/src/lib.rs`'s `tauri::generate_handler!` is
//! the *only* composition-root change they need; a forgotten `app.manage(...)`
//! cannot turn into a runtime state-not-found panic.
//!
//! A listing runs inline on the command's own task, bounded by
//! [`super::domain::MAX_DIRECTORY_ENTRIES`]. The file view's reads — a stat, a
//! range, a preview load, a table window — move to
//! [`tokio::task::spawn_blocking`]: each opens and reads a file, and all but
//! the stat can move megabytes or scan a large one.
//!
//! # A sensitive file is withheld here, never overridden
//!
//! Neither command can express a [`super::sensitive::SensitiveOverrideGrant`],
//! and this module never constructs one -- same stance as
//! `crate::tools::dispatch`. A classified-sensitive path comes back as
//! [`ProjectFileStat::SensitiveWithheld`], which structurally carries no
//! content, no hash, and no size: the read genuinely did not happen, so there
//! is nothing to report about the file beyond which class refused it. Granting
//! an override is a separate, deliberate human action and will be its own
//! command when that UI exists.
//!
//! Neither command publishes an audit event either.
//! [`super::read::read_file_with_audit`] is the sensitive-read decision log,
//! but nothing constructs the `crate::audit::AuditPublisher` it needs yet --
//! that identity only exists after the interactive login, so
//! `src-tauri/src/lib.rs` builds `ProjectsService` without
//! [`ProjectsService::with_audit`] and `crate::tools::dispatch` calls plain
//! [`super::read::read_file`] for the same reason. Closing that seam is one
//! change for both callers; closing it here alone would split the user path
//! and the agent path across two functions, which is precisely what D7
//! forbids.
//!
//! # Failures are loud
//!
//! A `..` component, an absolute path, a Windows-reserved or
//! alternate-data-stream component, a symlink or reparse point on the resolved
//! path, a root whose identity no longer matches what was trusted, and an
//! unknown or other-account project id all return `Err`. None of them
//! degrades to an empty listing or empty content, and every message is one of
//! `super::`'s own already-redacted `Display` strings -- no absolute path, no
//! filesystem layout, and no file content ever reaches the frontend through an
//! error.

use std::str::FromStr;

use tauri::Manager;
use xfalcon_contracts::projects::{
    ProjectDirectoryEntry, ProjectDirectoryListing, ProjectDocumentRead, ProjectEntryKind,
    ProjectFileMetadataReason, ProjectFileStat, ProjectSchemaSummary, ProjectSensitiveClass,
    ProjectTableRows,
};
use xfalcon_contracts::{RunId, CONTRACT_VERSION};

use crate::app::commands::AppRuntimeState;

use super::domain::ProjectId;
use super::path_policy::{PathPolicyError, RelativeProjectPath};
use super::read::{list_directory, DirectoryEntryView, EntryKind, MetadataOnlyReason, ReadError};
use super::sensitive::{SecretClass, SecretClassifier};
use super::trust::{ProjectsService, ProjectsServiceError};
use super::viewer::{ViewerError, ViewerStat};

#[derive(Debug, thiserror::Error)]
pub enum ProjectFileError {
    #[error("project id is not a valid identifier")]
    InvalidProjectId,
    #[error(transparent)]
    Project(#[from] ProjectsServiceError),
    #[error(transparent)]
    Path(#[from] PathPolicyError),
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Viewer(#[from] ViewerError),
    #[error("{0:?} is not a delimiter this view reads; use \",\" or a tab")]
    UnsupportedDelimiter(String),
    #[error("{0:?} is not a preview kind; use \"html\" or \"svg\"")]
    UnsupportedPreviewKind(String),
    #[error("the file view's worker stopped unexpectedly")]
    WorkerStopped,
}

/// Extracts tabular/dataset schemas across CSV, JSON, and SQLite files in `project_id`'s trusted root.
#[tauri::command]
pub async fn project_schema_summary(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<ProjectSchemaSummary, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    get_project_schema_summary(projects, &project_id).map_err(|error| error.to_string())
}

/// Lists one directory's immediate children inside `project_id`'s trusted
/// root. `path` of `None` lists the root itself.
#[tauri::command]
pub async fn project_list_directory(
    app: tauri::AppHandle,
    project_id: String,
    path: Option<String>,
) -> Result<ProjectDirectoryListing, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    list_project_directory(projects, &project_id, path.as_deref())
        .map_err(|error| error.to_string())
}

/// A file's size, modified time, and kind, for the file view (spec:
/// project-file-editing). Reads at most a sniff-sized prefix.
#[tauri::command]
pub async fn project_file_stat(
    app: tauri::AppHandle,
    project_id: String,
    path: String,
) -> Result<ProjectFileStat, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let (trusted_root, relative_path) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let stat = tokio::task::spawn_blocking(move || {
        let stat = super::viewer::stat(
            &trusted_root,
            &SecretClassifier::built_in(),
            &relative_path,
            read_scope_id(),
        );
        stat.map(|stat| stat_projection(relative_path.as_str(), stat))
    })
    .await
    .map_err(|_| ProjectFileError::WorkerStopped.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(stat)
}

/// Up to [`super::MAX_READ_RANGE_BYTES`] raw bytes of a file from `offset`,
/// returned as an `ArrayBuffer` rather than a JSON string: the view decodes and
/// aligns them itself, and an 8 MiB range costs no escaping either way.
#[tauri::command]
pub async fn project_file_read_bytes(
    app: tauri::AppHandle,
    project_id: String,
    path: String,
    offset: u64,
    length: u32,
) -> Result<tauri::ipc::Response, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let (trusted_root, path) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let bytes = tokio::task::spawn_blocking(move || {
        super::viewer::read_range(
            &trusted_root,
            &SecretClassifier::built_in(),
            &path,
            read_scope_id(),
            offset,
            length,
        )
    })
    .await
    .map_err(|_| ProjectFileError::WorkerStopped.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Reads bounded text from an Office file for a read-only project preview.
/// Raw-byte APIs remain unchanged and do not become editable through this path.
#[tauri::command]
pub async fn project_document_read(
    app: tauri::AppHandle,
    project_id: String,
    path: String,
) -> Result<ProjectDocumentRead, String> {
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let (trusted_root, relative_path) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let result = tokio::task::spawn_blocking(move || {
        super::viewer::read_office_document(
            &trusted_root,
            &SecretClassifier::built_in(),
            &relative_path,
            read_scope_id(),
        )
    })
    .await
    .map_err(|_| ProjectFileError::WorkerStopped.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(ProjectDocumentRead {
        version: CONTRACT_VERSION,
        path,
        text: result.0,
        size_bytes: result.1,
        modified_ms: result.2,
    })
}

/// The trusted root and validated path a viewer command reads, resolved for
/// the account signed in now. Owned, so the read can move to the blocking pool.
pub(super) fn viewer_target(
    projects: &ProjectsService,
    project_id: &str,
    path: &str,
) -> Result<(super::domain::TrustedRoot, RelativeProjectPath), ProjectFileError> {
    let project_id = parse_project_id(project_id)?;
    let path = RelativeProjectPath::new(path)?;
    Ok((projects.trusted_root_for_current_account(project_id)?, path))
}

/// [`project_file_read_bytes`] without the IPC wrapper, for tests.
#[cfg(test)]
pub(super) fn read_project_file_bytes(
    projects: &ProjectsService,
    project_id: &str,
    path: &str,
    offset: u64,
    length: u32,
) -> Result<Vec<u8>, ProjectFileError> {
    let (trusted_root, path) = viewer_target(projects, project_id, path)?;
    Ok(super::viewer::read_range(
        &trusted_root,
        &SecretClassifier::built_in(),
        &path,
        read_scope_id(),
        offset,
        length,
    )?)
}

#[cfg(test)]
pub(super) fn read_project_document(
    projects: &ProjectsService,
    project_id: &str,
    path: &str,
) -> Result<ProjectDocumentRead, ProjectFileError> {
    let (trusted_root, relative_path) = viewer_target(projects, project_id, path)?;
    let (text, size_bytes, modified_ms) = super::viewer::read_office_document(
        &trusted_root,
        &SecretClassifier::built_in(),
        &relative_path,
        read_scope_id(),
    )?;
    Ok(ProjectDocumentRead {
        version: CONTRACT_VERSION,
        path: path.to_owned(),
        text,
        size_bytes,
        modified_ms,
    })
}

/// Registers an HTML or SVG project file with the preview store, reading it
/// here so the document never crosses IPC (spec: preview-protocol, "A project
/// file previews from disk"). Returns the preview document id.
#[tauri::command]
pub async fn project_preview_register(
    app: tauri::AppHandle,
    project_id: String,
    path: String,
    kind: String,
) -> Result<String, String> {
    let preview_kind = crate::preview::PreviewKind::parse(&kind)
        .ok_or_else(|| ProjectFileError::UnsupportedPreviewKind(kind).to_string())?;
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let (trusted_root, path) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let store = app
        .state::<std::sync::Arc<crate::preview::PreviewDocumentStore>>()
        .inner()
        .clone();
    // Hashing a 32 MiB document is CPU work too, so it stays off the async
    // runtime with the read.
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let text = super::viewer::read_preview_text(
            &trusted_root,
            &SecretClassifier::built_in(),
            &path,
            read_scope_id(),
        )
        .map_err(|error| error.to_string())?;
        store
            .register(preview_kind, text)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| ProjectFileError::WorkerStopped.to_string())?
}

/// One window of a delimited file's rows, parsed here (spec:
/// project-file-editing, "Large tables page from the backend").
#[tauri::command]
pub async fn project_table_rows(
    app: tauri::AppHandle,
    project_id: String,
    path: String,
    delimiter: String,
    start_row: u64,
    limit: u32,
) -> Result<ProjectTableRows, String> {
    let delimiter_byte = match delimiter.as_str() {
        "," => b',',
        "\t" => b'\t',
        _ => return Err(ProjectFileError::UnsupportedDelimiter(delimiter).to_string()),
    };
    let state = app.state::<AppRuntimeState>();
    let projects = state.projects().map_err(|error| error.to_string())?;
    let (trusted_root, relative_path) =
        viewer_target(projects, &project_id, &path).map_err(|error| error.to_string())?;
    let window = tokio::task::spawn_blocking(move || {
        super::table::table_rows(
            &trusted_root,
            &SecretClassifier::built_in(),
            &relative_path,
            read_scope_id(),
            delimiter_byte,
            start_row,
            limit,
        )
    })
    .await
    .map_err(|_| ProjectFileError::WorkerStopped.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(ProjectTableRows {
        version: CONTRACT_VERSION,
        path,
        headers: window.headers,
        rows: window.rows,
        start_row: window.start_row,
        next_row: window.next_row,
        total_rows: window.total_rows,
        truncated: window.truncated,
    })
}

/// [`project_file_stat`] without the IPC wrapper, for tests.
#[cfg(test)]
pub(super) fn stat_project_file(
    projects: &ProjectsService,
    project_id: &str,
    path: &str,
) -> Result<ProjectFileStat, ProjectFileError> {
    let (trusted_root, relative_path) = viewer_target(projects, project_id, path)?;
    let stat = super::viewer::stat(
        &trusted_root,
        &SecretClassifier::built_in(),
        &relative_path,
        read_scope_id(),
    )?;
    Ok(stat_projection(relative_path.as_str(), stat))
}

fn stat_projection(path: &str, stat: ViewerStat) -> ProjectFileStat {
    let path = path.to_owned();
    match stat {
        ViewerStat::Text {
            size_bytes,
            modified_ms,
        } => ProjectFileStat::Text {
            version: CONTRACT_VERSION,
            path,
            size_bytes,
            modified_ms,
        },
        ViewerStat::NotText {
            size_bytes,
            modified_ms,
            reason,
        } => ProjectFileStat::NotText {
            version: CONTRACT_VERSION,
            path,
            size_bytes,
            modified_ms,
            reason: metadata_reason_projection(reason),
        },
        ViewerStat::TooLarge { size_bytes } => ProjectFileStat::TooLarge {
            version: CONTRACT_VERSION,
            path,
            size_bytes,
            limit_bytes: super::viewer::MAX_VIEWABLE_FILE_BYTES,
        },
        ViewerStat::Sensitive(class) => ProjectFileStat::SensitiveWithheld {
            version: CONTRACT_VERSION,
            path,
            sensitive_class: sensitive_class_projection(class),
        },
    }
}

pub(super) fn get_project_schema_summary(
    projects: &ProjectsService,
    project_id: &str,
) -> Result<ProjectSchemaSummary, ProjectFileError> {
    use super::read::MAX_FILE_READ_BYTES;
    use std::io::Read;

    let project_id_typed = parse_project_id(project_id)?;
    let trusted_root = projects.trusted_root_for_current_account(project_id_typed)?;
    let classifier = SecretClassifier::built_in();

    let mut tables = Vec::new();
    let mut dir_queue = std::collections::VecDeque::new();
    dir_queue.push_back(None::<RelativeProjectPath>);

    let mut dirs_visited = 0;
    let mut files_scanned = 0;
    const MAX_DIRS_TO_VISIT: usize = 100;
    const MAX_FILES_TO_SCAN: usize = 1_000;

    while let Some(current_dir) = dir_queue.pop_front() {
        dirs_visited += 1;
        if dirs_visited > MAX_DIRS_TO_VISIT {
            break;
        }

        let Ok(entries) = list_directory(&trusted_root, &classifier, current_dir.clone()) else {
            continue;
        };

        for entry in entries {
            if entry.is_sensitive {
                continue;
            }

            let child_path_str = match &current_dir {
                None => entry.name.clone(),
                Some(parent) => format!("{}/{}", parent.as_str(), entry.name),
            };

            let Ok(relative) = RelativeProjectPath::new(&child_path_str) else {
                continue;
            };

            match entry.kind {
                EntryKind::Directory => {
                    if dirs_visited + dir_queue.len() < MAX_DIRS_TO_VISIT {
                        dir_queue.push_back(Some(relative));
                    }
                }
                EntryKind::File => {
                    files_scanned += 1;
                    if files_scanned > MAX_FILES_TO_SCAN {
                        break;
                    }

                    let path_str = relative.as_str().to_string();
                    let filename = relative
                        .as_path()
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&path_str)
                        .to_string();

                    if path_str.ends_with(".csv") {
                        let table_name = filename.strip_suffix(".csv").unwrap_or(&filename);
                        if let Ok(resolved) = trusted_root.resolve_existing(relative) {
                            if let Ok(metadata) = trusted_root.file_metadata(&resolved) {
                                if metadata.len() <= MAX_FILE_READ_BYTES {
                                    if let Ok(file) = trusted_root.open_file(&resolved) {
                                        let mut content = String::new();
                                        if file
                                            .take(MAX_FILE_READ_BYTES)
                                            .read_to_string(&mut content)
                                            .is_ok()
                                        {
                                            if let Some(table) = super::schema::extract_csv_schema(
                                                table_name, &path_str, &content,
                                            ) {
                                                tables.push(table);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if path_str.ends_with(".json") {
                        let table_name = filename.strip_suffix(".json").unwrap_or(&filename);
                        if let Ok(resolved) = trusted_root.resolve_existing(relative) {
                            if let Ok(metadata) = trusted_root.file_metadata(&resolved) {
                                if metadata.len() <= MAX_FILE_READ_BYTES {
                                    if let Ok(file) = trusted_root.open_file(&resolved) {
                                        let mut content = String::new();
                                        if file
                                            .take(MAX_FILE_READ_BYTES)
                                            .read_to_string(&mut content)
                                            .is_ok()
                                        {
                                            if let Some(table) = super::schema::extract_json_schema(
                                                table_name, &path_str, &content,
                                            ) {
                                                tables.push(table);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if path_str.ends_with(".sqlite")
                        || path_str.ends_with(".sqlite3")
                        || path_str.ends_with(".db")
                    {
                        if let Ok(resolved) = trusted_root.resolve_existing(relative) {
                            if let Ok(metadata) = trusted_root.file_metadata(&resolved) {
                                const MAX_SQLITE_SCHEMA_BYTES: u64 = 10 * 1024 * 1024;
                                if metadata.len() <= MAX_SQLITE_SCHEMA_BYTES {
                                    let db_path = trusted_root
                                        .canonical_path()
                                        .join(resolved.relative().as_path());
                                    if let Ok(sqlite_tables) =
                                        super::schema::extract_sqlite_schemas(&db_path, &path_str)
                                    {
                                        tables.extend(sqlite_tables);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(ProjectSchemaSummary::new(
        project_id_typed.to_string(),
        tables,
    ))
}

pub(super) fn list_project_directory(
    projects: &ProjectsService,
    project_id: &str,
    path: Option<&str>,
) -> Result<ProjectDirectoryListing, ProjectFileError> {
    let project_id = parse_project_id(project_id)?;
    // `Some("")` is treated as the root alongside `None`, exactly as
    // `crate::tools::dispatch`'s `run_list_directory` treats it: a caller that
    // spells "the current directory is the root" as an empty string must get
    // the same answer the agent gets for the same input, not an error on one
    // path and a listing on the other.
    let relative = match path {
        None | Some("") => None,
        Some(raw) => Some(RelativeProjectPath::new(raw)?),
    };

    let trusted_root = projects.trusted_root_for_current_account(project_id)?;
    let entries = list_directory(
        &trusted_root,
        &SecretClassifier::built_in(),
        relative.clone(),
    )?;

    Ok(ProjectDirectoryListing::new(
        project_id.to_string(),
        relative.map(|relative| relative.as_str().to_owned()),
        entries.iter().map(entry_projection).collect(),
    ))
}

fn parse_project_id(raw: &str) -> Result<ProjectId, ProjectFileError> {
    ProjectId::from_str(raw).map_err(|_| ProjectFileError::InvalidProjectId)
}

/// A fresh, single-use run id for one command's sensitive-read scope.
///
/// [`super::sensitive::SensitiveReadScope`] exists to bind an override grant to one exact
/// `(project, path, run)` triple, and a user-initiated read has no run to
/// name. Minting a fresh id per call keeps that scoping honest: the scope this
/// call classified under is never nameable again, so no grant derived from a
/// returned [`super::sensitive::SensitiveOverrideRequest`] could be replayed
/// against a later call. A fixed synthetic id (`"desktop-ui"`) would hand out
/// exactly that replayable scope for free.
pub(crate) fn read_scope_id() -> RunId {
    RunId::from(uuid::Uuid::new_v4().to_string())
}

fn entry_projection(entry: &DirectoryEntryView) -> ProjectDirectoryEntry {
    ProjectDirectoryEntry {
        name: entry.name.clone(),
        kind: entry_kind_projection(entry.kind),
        size_bytes: entry.size_bytes,
        is_sensitive: entry.is_sensitive,
    }
}

const fn entry_kind_projection(kind: EntryKind) -> ProjectEntryKind {
    match kind {
        EntryKind::File => ProjectEntryKind::File,
        EntryKind::Directory => ProjectEntryKind::Directory,
        EntryKind::Symlink => ProjectEntryKind::Symlink,
        EntryKind::Other => ProjectEntryKind::Other,
    }
}

const fn metadata_reason_projection(reason: MetadataOnlyReason) -> ProjectFileMetadataReason {
    match reason {
        MetadataOnlyReason::Binary => ProjectFileMetadataReason::Binary,
        MetadataOnlyReason::NonUtf8Text => ProjectFileMetadataReason::NonUtf8Text,
        MetadataOnlyReason::Oversized => ProjectFileMetadataReason::Oversized,
    }
}

/// One arm per [`SecretClass`], renamed only for `BrowserProfileSecret` --
/// `xfalcon_contracts::projects` spells it `BrowserProfileStore` to stay clear
/// of the credential-shaped vocabulary its own snapshot tripwire scans for
/// (`crates/contracts/tests/contract_snapshots.rs`).
pub(crate) const fn sensitive_class_projection(class: SecretClass) -> ProjectSensitiveClass {
    match class {
        SecretClass::DotEnv => ProjectSensitiveClass::DotEnv,
        SecretClass::PrivateKeyMaterial => ProjectSensitiveClass::PrivateKeyMaterial,
        SecretClass::CloudOrKubernetesCredentials => {
            ProjectSensitiveClass::CloudOrKubernetesCredentials
        }
        SecretClass::PasswordOrVaultExport => ProjectSensitiveClass::PasswordOrVaultExport,
        SecretClass::BrowserProfileSecret => ProjectSensitiveClass::BrowserProfileStore,
        SecretClass::SshOrGpgMaterial => ProjectSensitiveClass::SshOrGpgMaterial,
        SecretClass::TokenOrConfigStore => ProjectSensitiveClass::TokenOrConfigStore,
        SecretClass::Configured => ProjectSensitiveClass::Configured,
    }
}
