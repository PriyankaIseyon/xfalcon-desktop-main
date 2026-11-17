//! Wire vocabulary for trusted local projects (Task 4: project identity,
//! trust, and path containment).
//!
//! A project is a user-picked directory the account has explicitly chosen to
//! trust. Only a bounded, versioned projection ever crosses the Tauri IPC
//! boundary. `ProjectView::root_path` does carry the full canonical path —
//! the UI must display the trusted directory to the user, and the path
//! itself is not the security concern TM-007 cares about. What never
//! crosses this boundary is platform root-*identity* (device/inode, volume
//! serial/file index) or any raw filesystem handle: those stay
//! authoritative in `src-tauri/src/projects` (see
//! `docs/threat-model.md`, TM-007).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct ProjectId(String);

impl ProjectId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ProjectId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ProjectId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Whether a trusted project's root still resolves to the exact identity
/// recorded at trust time. `Unavailable` covers every revalidation failure
/// (root missing, identity changed because the root was deleted/recreated or
/// a different volume was mounted at the same path, permission denied, or a
/// link/reparse point now sits where a plain directory was trusted) without
/// distinguishing the cause on this wire boundary — the cause is an internal
/// diagnostic, not something a compromised/confused root should get to
/// describe back to the UI.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectTrustStatus {
    Trusted,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ProjectId,
    pub display_name: String,
    pub root_path: String,
    pub status: ProjectTrustStatus,
    pub active: bool,
    #[ts(type = "number")]
    pub trusted_at_ms: i64,
}

impl ProjectView {
    pub fn new(
        id: impl Into<ProjectId>,
        display_name: impl Into<String>,
        root_path: impl Into<String>,
        status: ProjectTrustStatus,
        active: bool,
        trusted_at_ms: i64,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            display_name: display_name.into(),
            root_path: root_path.into(),
            status,
            active,
            trusted_at_ms,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectListResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub projects: Vec<ProjectView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectFileRevisionView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub backup_id: String,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    #[ts(type = "number")]
    pub original_bytes: u64,
    #[ts(type = "number")]
    pub compressed_bytes: u64,
    pub content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectFileRevisionList {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub project_id: ProjectId,
    pub relative_path: String,
    pub revisions: Vec<ProjectFileRevisionView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectFileRestoreResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub project_id: ProjectId,
    pub relative_path: String,
}

/// Why a revision's changes are not shown line by line.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectRevisionDiffLimit {
    /// The revision or the current file is too large to compare.
    FileSize,
    /// The edit changes too many lines to read as a diff.
    ChangeSize,
}

/// What changed in a file from one of its revisions: a unified diff from the
/// revision (`a/`) to either the file as it is now or a newer revision of the
/// same file (`b/`), or why there is none to show. Nothing past the limits is
/// returned in part.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum ProjectFileRevisionDiff {
    Changes {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        backup_id: String,
        unified_diff: String,
        added_lines: u32,
        removed_lines: u32,
    },
    Unchanged {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        backup_id: String,
    },
    /// The revision or the current file is binary, or text in another encoding.
    NotText {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        backup_id: String,
    },
    TooLarge {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        backup_id: String,
        limit: ProjectRevisionDiffLimit,
    },
    /// Classified as sensitive from the path alone; neither side was read.
    SensitiveWithheld {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        backup_id: String,
        sensitive_class: ProjectSensitiveClass,
    },
}

impl ProjectListResult {
    pub fn new(projects: Vec<ProjectView>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            projects,
        }
    }
}

/// A folder the person chose in the native picker.
///
/// `grant_id` is what the renderer sends back to act on the folder. The path
/// stays in the backend: `display_path` is text to show, not something the
/// renderer can act on, and `display_name` is its last component.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FolderGrantView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub grant_id: String,
    pub display_name: String,
    pub display_path: String,
}

impl FolderGrantView {
    pub fn new(
        grant_id: impl Into<String>,
        display_name: impl Into<String>,
        display_path: impl Into<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            grant_id: grant_id.into(),
            display_name: display_name.into(),
            display_path: display_path.into(),
        }
    }
}

/// What one listed child of a project directory is. `Symlink` and `Other` are
/// reported rather than filtered out: the containment layer refuses to follow
/// them, and a listing that silently omitted entries the filesystem reports
/// would be a silent failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectDirectoryEntry {
    pub name: String,
    pub kind: ProjectEntryKind,
    #[ts(type = "number")]
    pub size_bytes: u64,
    /// Classified as sensitive by the same classifier that gates reads.
    /// Flagged so the UI can mark it without exposing any content.
    pub is_sensitive: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectDirectoryListing {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub project_id: ProjectId,
    /// The listed directory, project-relative. `None` is the trusted root.
    pub path: Option<String>,
    pub entries: Vec<ProjectDirectoryEntry>,
}

impl ProjectDirectoryListing {
    pub fn new(
        project_id: impl Into<ProjectId>,
        path: Option<String>,
        entries: Vec<ProjectDirectoryEntry>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            project_id: project_id.into(),
            path,
            entries,
        }
    }
}

/// Why a file's bytes were classified but not returned as text.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectFileMetadataReason {
    Binary,
    NonUtf8Text,
    Oversized,
}

/// Why a read was refused outright. One arm per
/// `src-tauri/src/projects::SecretClass` variant, renamed only where that
/// vocabulary would trip the boundary tripwire in
/// `crates/contracts/tests/contract_snapshots.rs`, so the UI can explain the
/// refusal instead of showing an opaque error.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectSensitiveClass {
    DotEnv,
    PrivateKeyMaterial,
    CloudOrKubernetesCredentials,
    PasswordOrVaultExport,
    /// `SecretClass::BrowserProfileSecret`.
    BrowserProfileStore,
    SshOrGpgMaterial,
    TokenOrConfigStore,
    Configured,
}

/// What the file view needs to know before it reads a byte: the file's size and
/// when it last changed, and whether it is text at all (from a prefix sniff).
///
/// Sizes and times are `number` on the wire: a file up to the view's 10 GiB cap
/// is far inside `Number.MAX_SAFE_INTEGER`, and the view does arithmetic on them.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum ProjectFileStat {
    /// Text, as far as its first bytes show. A later invalid byte is found
    /// when that part is read, never guessed here.
    Text {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        path: String,
        #[ts(type = "number")]
        size_bytes: u64,
        #[ts(type = "number | null")]
        modified_ms: Option<i64>,
    },
    /// Not text this view renders: binary, or text in another encoding.
    NotText {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        path: String,
        #[ts(type = "number")]
        size_bytes: u64,
        #[ts(type = "number | null")]
        modified_ms: Option<i64>,
        reason: ProjectFileMetadataReason,
    },
    /// Past the view's cap; nothing is read.
    TooLarge {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        path: String,
        #[ts(type = "number")]
        size_bytes: u64,
        #[ts(type = "number")]
        limit_bytes: u64,
    },
    /// Classified as sensitive from the path alone. No size, no time — the file
    /// was not touched.
    SensitiveWithheld {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        path: String,
        sensitive_class: ProjectSensitiveClass,
    },
}

/// Identifies a registered preview before its bounded worker starts. A caller
/// that has already closed the view can cancel this id without a start/cancel race.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectOfficePreviewStarted {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub render_id: String,
}

/// A bounded text-only projection of a local Office document. The source stays
/// a read-only binary file; this response carries no edit or byte-write path.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectDocumentRead {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub path: String,
    pub text: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    #[ts(type = "number | null")]
    pub modified_ms: Option<i64>,
}

/// One window of a delimited file's rows, read by the backend.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectTableRows {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub path: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// The first data row in `rows`, counting from zero after the header.
    #[ts(type = "number")]
    pub start_row: u64,
    /// Where the next window starts; absent at the end of the file.
    #[ts(type = "number | null")]
    pub next_row: Option<u64>,
    /// The file's data-row count, once a read has reached its end.
    #[ts(type = "number | null")]
    pub total_rows: Option<u64>,
    /// A cell or a row was cut to the display bound.
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectColumnSchema {
    pub name: String,
    pub inferred_type: String,
    pub sample_values: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectTableSchema {
    pub name: String,
    pub relative_path: String,
    pub format: String,
    pub columns: Vec<ProjectColumnSchema>,
    pub sample_rows: Vec<Vec<String>>,
    #[ts(type = "number | null")]
    pub row_count_estimate: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectSchemaSummary {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub project_id: ProjectId,
    pub tables: Vec<ProjectTableSchema>,
}

impl ProjectSchemaSummary {
    pub fn new(project_id: impl Into<ProjectId>, tables: Vec<ProjectTableSchema>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            project_id: project_id.into(),
            tables,
        }
    }
}
