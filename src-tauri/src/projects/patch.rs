//! Immutable mutation plans: the only shape a trusted-project file mutation
//! is ever proposed in.
//!
//! # Security contract
//!
//! [`MutationPlan::propose`] is the single entry point that produces a
//! [`MutationPlan`], and it never mutates the filesystem -- it only reads
//! (to capture base hashes and build a human-reviewable diff) and validates.
//! Once constructed, a [`MutationPlan`] is immutable (every field is private
//! and there is no `&mut self` method anywhere on this type): its
//! content-addressed [`PlanId`] ([`MutationPlan::plan_id`]) is a BLAKE3 hash
//! of the plan's own canonical contents (project/root identity, ordered
//! operations, and every base/proposed hash), so two proposals with
//! identical operations against the same root collide onto the same id, and
//! a stale/replayed plan id cannot be silently reused -- not because the id
//! itself changes, but because [`crate::approvals`] independently rejects an
//! expired plan ([`MutationPlan::is_expired`]) and [`super::writer`]
//! independently re-derives every base hash fresh immediately before
//! executing (`docs/decisions.md`,
//! Task 6, Step 4). This module never performs the actual write; see
//! [`super::writer`] for the only code path that does, and for the
//! non-confirmable containment re-check that runs unconditionally before any
//! approval/confirmation state is even consulted.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::time::Duration;

use similar::TextDiff;

use super::domain::{ContainmentError, ProjectId, ResolvedProjectPath, TrustedRoot};
use super::edit::{
    apply_edits, apply_insert, is_prose_path, lazy_placeholder, EditError, EditOperation,
};
use super::path_policy::RelativeProjectPath;
use super::platform::RootIdentity;
use super::read::{classify_bytes, ContentKind};

/// Hard ceiling on operations in one plan -- defense against an unbounded/
/// hostile proposal, mirroring every other `MAX_*` bound in this module tree.
pub const MAX_OPERATIONS_PER_PLAN: usize = 200;
/// Hard ceiling on one plan's total proposed+existing content bytes.
///
/// This bound exists for *memory and durable storage*, not for anything the
/// model sees: a plan holds all of its proposed content in memory and is
/// persisted whole into one encrypted approval row. Raising it past the point
/// where a row is comfortable would mean spilling staged content to disk
/// instead, which is a different design, not a larger constant.
pub const MAX_TOTAL_PLAN_BYTES: u64 = 128 * 1024 * 1024;
/// Hard ceiling on the current content of a file this module will hold in
/// memory to edit, diff, or capture. This bounds *process memory*: anchored
/// content never enters the model's context, so what one read returns to the
/// model is not the relevant limit for editing. A delete or rename of a
/// larger file still works: neither needs content (see [`ExistingContent`]).
pub const MAX_EDITABLE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// More than this many distinct paths in one plan requires a second
/// confirmation beyond the base approval (plan policy).
pub const SECOND_CONFIRMATION_PATH_THRESHOLD: usize = 10;
/// More than this many total bytes in one plan requires a second
/// confirmation beyond the base approval (plan policy).
pub const SECOND_CONFIRMATION_BYTE_THRESHOLD: u64 = 1024 * 1024;
/// How long a proposed plan remains approvable/executable before it expires
/// (plan: "an expiry").
pub const DEFAULT_PLAN_TTL: Duration = Duration::from_secs(15 * 60);
/// Hard ceiling on the rendered unified-diff string's byte length -- a huge
/// diff is truncated (with an explicit marker) rather than growing the
/// persisted encrypted row or a review UI without bound.
pub const MAX_DIFF_BYTES: usize = 256 * 1024;

/// A BLAKE3 digest, used both for per-file base/proposed content hashes and
/// for [`PlanId`] itself. A thin, self-documenting alias over
/// [`blake3::Hash`] (already `Copy`, constant-time `Eq`, and hex-`Display`-
/// capable) rather than a second hand-rolled newtype.
pub type ContentHash = blake3::Hash;

#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> ContentHash {
    blake3::hash(bytes)
}

/// A [`MutationPlan`]'s content-addressed identity: BLAKE3 of the plan's own
/// canonical contents (project/root identity, ordered operation tags/paths,
/// and every base/proposed hash -- see [`MutationPlan::propose`]'s module
/// doc comment). Two proposals with identical operations against the same
/// root always produce the same [`PlanId`].
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct PlanId(ContentHash);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("plan id is not a valid 64-character hex-encoded BLAKE3 digest")]
pub struct PlanIdParseError;

impl PlanId {
    #[must_use]
    pub fn to_hex(self) -> String {
        self.0.to_hex().to_string()
    }

    /// Reconstructs a [`PlanId`] from its hex encoding -- used only by
    /// `crate::approvals::service` when loading a persisted approval
    /// request's own `plan_id` primary-key column back from encrypted
    /// storage. Never derivable from model/skill/MCP/tool output.
    pub fn from_hex(hex: &str) -> Result<Self, PlanIdParseError> {
        ContentHash::from_hex(hex)
            .map(Self)
            .map_err(|_| PlanIdParseError)
    }
}

impl std::fmt::Debug for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PlanId").field(&self.to_hex()).finish()
    }
}

/// One file mutation within a plan. Content is always carried verbatim
/// (never normalized -- no CRLF/LF conversion, no forced trailing newline):
/// [`super::writer`] writes exactly these bytes.
#[derive(Clone, Debug)]
pub enum PatchOperation {
    /// `path` must not already exist (verified at propose time via
    /// [`TrustedRoot::resolve_new`] plus an explicit existence check, and
    /// re-verified at execute time).
    Create {
        path: RelativeProjectPath,
        content: Vec<u8>,
    },
    /// `path` must already exist as a plain file.
    Modify {
        path: RelativeProjectPath,
        content: Vec<u8>,
    },
    /// `from` must already exist as a plain file. `to` may or may not exist;
    /// if it does, replacing it is the "rename-existing" second-confirmation
    /// trigger and its own current content is captured as a base hash too.
    /// Content is never re-supplied -- the source's existing bytes move
    /// as-is.
    Rename {
        from: RelativeProjectPath,
        to: RelativeProjectPath,
    },
    /// `path` must already exist as a plain file.
    Delete { path: RelativeProjectPath },
    /// `from` must already exist as a directory; `to` must not exist at all.
    /// Renaming onto an existing directory is refused rather than merged or
    /// clobbered.
    RenameDirectory {
        from: RelativeProjectPath,
        to: RelativeProjectPath,
    },
    /// `path` must already exist as a directory, and must be empty when the
    /// removal runs.
    DeleteDirectory { path: RelativeProjectPath },
}

impl PatchOperation {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Modify { .. } => "modify",
            Self::Rename { .. } => "rename",
            Self::Delete { .. } => "delete",
            Self::RenameDirectory { .. } => "rename_directory",
            Self::DeleteDirectory { .. } => "delete_directory",
        }
    }
}

/// Feeds the approval policy's second-confirmation gate
/// (`crate::approvals::policy`). Computed once, immutably, at propose time
/// from the plan's own operations -- never something a caller supplies or a
/// UI can downgrade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationRiskTier {
    Standard,
    SecondConfirmationRequired,
}

#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error(transparent)]
    Containment(#[from] ContainmentError),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("a mutation plan must contain at least one operation")]
    Empty,
    #[error("a mutation plan may contain at most {0} operations")]
    TooManyOperations(usize),
    #[error("a mutation plan's total content exceeds {0} bytes")]
    TooManyBytes(u64),
    #[error("path {0:?} is targeted by more than one operation in the same plan")]
    DuplicatePathTarget(String),
    #[error("create target {0:?} already exists; use modify instead")]
    CreateTargetExists(String),
    #[error("modify target {0:?} does not exist; use create instead")]
    ModifyTargetMissing(String),
    #[error("delete target {0:?} does not exist")]
    DeleteTargetMissing(String),
    #[error(
        "{0:?} is a directory; use the matching directory operation instead of a file operation"
    )]
    TargetIsDirectory(String),
    #[error("{0:?} is not a directory; use the matching file operation instead")]
    TargetIsNotDirectory(String),
    #[error("directory {0:?} does not exist")]
    DirectoryMissing(String),
    #[error("directory rename destination {0:?} already exists; renaming onto an existing directory is refused")]
    RenameDirectoryDestinationExists(String),
    #[error("this plan both operates on {1:?} and renames or deletes the directory {0:?} that contains it; do the file operations first, then the directory in a second call")]
    DirectoryOperationWithContainedPath(String, String),
    #[error("rename source {0:?} does not exist")]
    RenameSourceMissing(String),
    #[error("rename source and destination must differ: {0:?}")]
    RenameSamePath(String),
    #[error("content for {0:?} exceeds {1} bytes")]
    ContentTooLarge(String, u64),
    #[error("existing content at {0:?} exceeds {1} bytes and cannot be edited")]
    ExistingContentTooLarge(String, u64),
    #[error("{0:?} already contains exactly this content; there is nothing to write")]
    NoChange(String),
    #[error("every file in this plan already contains exactly this content, so there is nothing to write: {0}")]
    AllUnchanged(String),
    #[error("{} of the operations in this plan cannot be applied, and nothing was written. Fix these and send the plan again; the other operations are fine:\n{}", .0.len(), format_issues(.0))]
    Rejected(Vec<OperationIssue>),
    #[error("{0:?} is not UTF-8 text, so it cannot be edited by anchor")]
    NotText(String),
    #[error("failed to read existing content: {0}")]
    Io(#[source] std::io::Error),
}

/// Rewrites each whole-file `Modify` in `edits` to the line-ending convention
/// of the file it replaces, when the file is CRLF throughout and the new text
/// arrived with bare `\n`, and keeps a leading byte order mark the new text
/// dropped.
///
/// For an author that cannot see line endings -- a model -- so that rewriting a
/// Windows file does not turn it into a Unix one and show every line as
/// changed. A file that cannot be read here is left to the plan to report.
/// Content that already carries `\r\n`, non-text content, and mixed files are
/// never touched. Not applied to a person's own save: the editor holds the
/// exact text.
pub fn keep_line_endings(trusted_root: &TrustedRoot, edits: &mut [EditOperation]) {
    for edit in edits {
        let EditOperation::Modify { path, content } = edit else {
            continue;
        };
        let Ok(resolved) = trusted_root.resolve_existing(path.clone()) else {
            continue;
        };
        let Ok(existing) = read_bounded(trusted_root, &resolved, path) else {
            continue;
        };
        let (Ok(old), Ok(new)) = (std::str::from_utf8(&existing), std::str::from_utf8(content))
        else {
            continue;
        };
        let conformed = super::edit::in_files_line_ending(old, new);
        let base = conformed.as_deref().unwrap_or(new);
        // A model cannot see a byte order mark either, and a file that had one
        // (PowerShell, Excel CSV, `.sln`) is misread without it.
        let with_mark = (old.starts_with('\u{feff}') && !base.starts_with('\u{feff}'))
            .then(|| format!("\u{feff}{base}"));
        if let Some(text) = with_mark.or(conformed) {
            *content = text.into_bytes();
        }
    }
}

/// One operation of a proposed plan that could not be planned, with enough
/// identity for its author to fix exactly that operation.
#[derive(Debug)]
pub struct OperationIssue {
    /// Zero-based position in the submitted list.
    pub index: usize,
    pub kind: &'static str,
    pub path: String,
    pub error: PatchError,
}

fn format_issues(issues: &[OperationIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            format!(
                "- operation {} ({} {}): {}",
                issue.index, issue.kind, issue.path, issue.error
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// An immutable, content-addressed plan for mutating a trusted project's
/// files. See the module doc comment for the full security contract. Every
/// field is private; there is no mutator anywhere on this type.
#[derive(Clone, Debug)]
pub struct MutationPlan {
    plan_id: PlanId,
    project_id: ProjectId,
    root_identity: RootIdentity,
    operations: Vec<PatchOperation>,
    base_hashes: BTreeMap<String, ContentHash>,
    proposed_hashes: BTreeMap<String, ContentHash>,
    unified_diff: String,
    total_paths: usize,
    total_bytes: u64,
    risk: MutationRiskTier,
    created_at_ms: i64,
    expires_at_ms: i64,
}

impl MutationPlan {
    /// Validates `operations` against `trusted_root` (every path must
    /// resolve through the same containment chokepoint every other project
    /// accessor uses -- no raw path join), captures a BLAKE3 base hash of
    /// every file whose *current* content matters, computes the BLAKE3
    /// proposed hash of every file's post-write content, renders a bounded
    /// unified diff for human review, classifies the plan's risk tier, and
    /// derives its content-addressed [`PlanId`]. Performs no mutation.
    pub fn propose(
        trusted_root: &TrustedRoot,
        edits: Vec<EditOperation>,
    ) -> Result<Self, PatchError> {
        Self::propose_with(trusted_root, edits, false).map(|(plan, _unchanged)| plan)
    }

    /// [`Self::propose`] for a plan a model authored, also returning the paths
    /// whose content already matched what was asked for.
    ///
    /// Both entry points drop such a modification from the plan instead of
    /// failing it, and both check every operation rather than stopping at the
    /// first bad one ([`PatchError::Rejected`]). A plan in which *every*
    /// operation is a no-op is still an error.
    ///
    /// What is specific to a model's plan: its text is checked for placeholder
    /// comments standing in for content, and re-sending a `create` that already
    /// landed is a no-op, not an error. A person's own plan ([`Self::propose`])
    /// gets neither, so a save or a restore is never refused for its wording.
    pub fn propose_reporting(
        trusted_root: &TrustedRoot,
        edits: Vec<EditOperation>,
    ) -> Result<(Self, Vec<String>), PatchError> {
        Self::propose_with(trusted_root, edits, true)
    }

    fn propose_with(
        trusted_root: &TrustedRoot,
        edits: Vec<EditOperation>,
        model_authored: bool,
    ) -> Result<(Self, Vec<String>), PatchError> {
        if edits.is_empty() {
            return Err(PatchError::Empty);
        }
        if edits.len() > MAX_OPERATIONS_PER_PLAN {
            return Err(PatchError::TooManyOperations(MAX_OPERATIONS_PER_PLAN));
        }

        let mut build = PlanBuild {
            model_authored,
            ..PlanBuild::default()
        };
        let mut operations: Vec<PatchOperation> = Vec::with_capacity(edits.len());
        let mut issues: Vec<OperationIssue> = Vec::new();

        for (index, edit) in edits.into_iter().enumerate() {
            let (kind, path) = edit.describe();
            match plan_operation(trusted_root, edit, &mut build, &mut operations) {
                Ok(()) => {}
                Err(PatchError::NoChange(unchanged)) => build.unchanged.push(unchanged),
                Err(error) => issues.push(OperationIssue {
                    index,
                    kind,
                    path,
                    error,
                }),
            }
            if build.total_bytes > MAX_TOTAL_PLAN_BYTES {
                return Err(PatchError::TooManyBytes(MAX_TOTAL_PLAN_BYTES));
            }
        }
        if issues.len() == 1 {
            return Err(issues.remove(0).error);
        }
        if !issues.is_empty() {
            return Err(PatchError::Rejected(issues));
        }
        if operations.is_empty() {
            return Err(match build.unchanged.as_slice() {
                [only] => PatchError::NoChange(only.clone()),
                all => PatchError::AllUnchanged(all.join(", ")),
            });
        }

        reject_directory_operation_over_contained_path(&operations)?;

        let PlanBuild {
            touched_paths,
            base_hashes,
            proposed_hashes,
            diff_sections,
            total_bytes,
            has_binary_content,
            has_delete,
            has_rename_existing,
            unchanged,
            model_authored: _,
        } = build;
        // Paths left out as already matching are not part of the change.
        let total_paths = touched_paths
            .iter()
            .filter(|path| !unchanged.contains(path))
            .count();
        let risk = if has_delete
            || has_rename_existing
            || has_binary_content
            || total_paths > SECOND_CONFIRMATION_PATH_THRESHOLD
            || total_bytes > SECOND_CONFIRMATION_BYTE_THRESHOLD
        {
            MutationRiskTier::SecondConfirmationRequired
        } else {
            MutationRiskTier::Standard
        };

        let unified_diff = bound_diff(diff_sections);
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let ttl_ms = i64::try_from(DEFAULT_PLAN_TTL.as_millis()).unwrap_or(i64::MAX);
        let expires_at_ms = created_at_ms.saturating_add(ttl_ms);

        let plan_id = compute_plan_id(
            *trusted_root.project_id(),
            trusted_root.identity(),
            &operations,
            &base_hashes,
            &proposed_hashes,
        );

        Ok((
            Self {
                plan_id,
                project_id: *trusted_root.project_id(),
                root_identity: trusted_root.identity(),
                operations,
                base_hashes,
                proposed_hashes,
                unified_diff,
                total_paths,
                total_bytes,
                risk,
                created_at_ms,
                expires_at_ms,
            },
            unchanged,
        ))
    }

    /// Raw reconstruction from already-validated, previously-proposed parts
    /// -- used only by `crate::approvals::service` when loading a persisted
    /// approval request back from encrypted storage. Performs no I/O and no
    /// re-validation: `super::writer`'s pre-execute revalidation (fresh
    /// containment, fresh base-hash comparison) is what actually re-proves
    /// this loaded plan is still safe to execute, exactly as it would for a
    /// freshly-proposed one.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        plan_id: PlanId,
        project_id: ProjectId,
        root_identity: RootIdentity,
        operations: Vec<PatchOperation>,
        base_hashes: BTreeMap<String, ContentHash>,
        proposed_hashes: BTreeMap<String, ContentHash>,
        unified_diff: String,
        total_paths: usize,
        total_bytes: u64,
        risk: MutationRiskTier,
        created_at_ms: i64,
        expires_at_ms: i64,
    ) -> Self {
        Self {
            plan_id,
            project_id,
            root_identity,
            operations,
            base_hashes,
            proposed_hashes,
            unified_diff,
            total_paths,
            total_bytes,
            risk,
            created_at_ms,
            expires_at_ms,
        }
    }

    #[must_use]
    pub fn plan_id(&self) -> PlanId {
        self.plan_id
    }

    #[must_use]
    pub fn project_id(&self) -> ProjectId {
        self.project_id
    }

    #[must_use]
    pub fn root_identity(&self) -> RootIdentity {
        self.root_identity
    }

    #[must_use]
    pub fn operations(&self) -> &[PatchOperation] {
        &self.operations
    }

    #[must_use]
    pub fn base_hashes(&self) -> &BTreeMap<String, ContentHash> {
        &self.base_hashes
    }

    #[must_use]
    pub fn proposed_hashes(&self) -> &BTreeMap<String, ContentHash> {
        &self.proposed_hashes
    }

    #[must_use]
    pub fn unified_diff(&self) -> &str {
        &self.unified_diff
    }

    #[must_use]
    pub fn total_paths(&self) -> usize {
        self.total_paths
    }

    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn risk(&self) -> MutationRiskTier {
        self.risk
    }

    #[must_use]
    pub fn requires_second_confirmation(&self) -> bool {
        matches!(self.risk, MutationRiskTier::SecondConfirmationRequired)
    }

    #[must_use]
    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }

    #[must_use]
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    #[must_use]
    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms >= self.expires_at_ms
    }

    /// Recomputes [`PlanId`] from this value's own current fields and
    /// compares it against the id it already carries. Used by
    /// [`super::writer::execute`] as an extra, cheap, defense-in-depth check
    /// immediately before executing (plan: "revalidate ... the plan's own
    /// content-addressed digest") -- distinct from the AEAD authentication
    /// `crate::approvals::service` already relies on for the persisted row's
    /// ciphertext integrity, this instead guards against a decode-side bug
    /// producing a self-inconsistent in-memory value from otherwise
    /// correctly-decrypted bytes.
    #[must_use]
    pub fn digest_is_self_consistent(&self) -> bool {
        self.plan_id
            == compute_plan_id(
                self.project_id,
                self.root_identity,
                &self.operations,
                &self.base_hashes,
                &self.proposed_hashes,
            )
    }
}

/// The accumulating state of one [`MutationPlan::propose`] call. A struct
/// rather than eight locals so the shared tail of every content-replacing
/// operation ([`Self::record_modify`]) can be written once.
#[derive(Default)]
struct PlanBuild {
    touched_paths: BTreeSet<String>,
    base_hashes: BTreeMap<String, ContentHash>,
    proposed_hashes: BTreeMap<String, ContentHash>,
    diff_sections: Vec<String>,
    total_bytes: u64,
    has_binary_content: bool,
    has_delete: bool,
    has_rename_existing: bool,
    /// Modifications whose content already matched, in the order given.
    unchanged: Vec<String>,
    /// The plan comes from a model rather than a person.
    model_authored: bool,
}

/// Plans one operation into `build` and `operations`, or says why it cannot be.
///
/// Reads only. A failure may leave `build` holding part of what the operation
/// recorded, which is fine because the whole plan is refused in that case.
fn plan_operation(
    trusted_root: &TrustedRoot,
    edit: EditOperation,
    build: &mut PlanBuild,
    operations: &mut Vec<PatchOperation>,
) -> Result<(), PatchError> {
    match edit {
        EditOperation::Create { path, content } => {
            insert_unique(&mut build.touched_paths, path.as_str())?;
            check_content_bound(&path, &content)?;
            if build.model_authored {
                refuse_placeholder(&path, &content, &[])?;
            }
            let resolved = trusted_root.resolve_new(path.clone())?;
            if trusted_root.file_metadata(&resolved).is_ok() {
                // A create sent again after it landed is the goal already met.
                if build.model_authored && holds_exactly(trusted_root, &path, &content) {
                    return Err(PatchError::NoChange(path.as_str().to_owned()));
                }
                return Err(PatchError::CreateTargetExists(path.as_str().to_owned()));
            }
            let kind = classify_bytes(&content);
            if !matches!(kind, ContentKind::Text) {
                build.has_binary_content = true;
            }
            build
                .proposed_hashes
                .insert(path.as_str().to_owned(), hash_bytes(&content));
            build.total_bytes += content.len() as u64;
            build
                .diff_sections
                .push(diff_for_create(path.as_str(), &content, kind));
            operations.push(PatchOperation::Create { path, content });
        }
        EditOperation::Modify { path, content } => {
            check_content_bound(&path, &content)?;
            // A person saving their own edit has the whole file in the
            // editor; the model has only what its reads showed, and
            // `crate::tools::dispatch` refuses a blind whole-file
            // replace before a model's plan gets here.
            let existing = existing_content_for_edit(trusted_root, &path, build)?;
            if build.model_authored {
                refuse_placeholder(&path, &content, &existing)?;
            }
            build.record_modify(&path, &existing, &content)?;
            operations.push(PatchOperation::Modify { path, content });
        }
        EditOperation::Replace {
            path,
            edits: text_edits,
        } => {
            let existing = existing_content_for_edit(trusted_root, &path, build)?;
            let text = require_text(&existing, &path)?;
            let updated = apply_edits(text, &text_edits, path.as_str())?;
            let content = updated.into_bytes();
            check_content_bound(&path, &content)?;
            build.record_modify(&path, &existing, &content)?;
            operations.push(PatchOperation::Modify { path, content });
        }
        EditOperation::Insert {
            path,
            insert_line,
            content: block,
        } => {
            let existing = existing_content_for_edit(trusted_root, &path, build)?;
            let text = require_text(&existing, &path)?;
            if build.model_authored {
                refuse_placeholder(&path, block.as_bytes(), &existing)?;
            }
            let updated = apply_insert(text, insert_line, &block, path.as_str())?;
            let content = updated.into_bytes();
            check_content_bound(&path, &content)?;
            build.record_modify(&path, &existing, &content)?;
            operations.push(PatchOperation::Modify { path, content });
        }
        EditOperation::Delete { path } => {
            insert_unique(&mut build.touched_paths, path.as_str())?;
            build.has_delete = true;
            let resolved = resolve_existing_or(
                &path,
                || PatchError::DeleteTargetMissing(path.as_str().to_owned()),
                trusted_root,
            )?;
            reject_directory(trusted_root, &resolved, &path)?;
            // A delete needs no content -- only the identity of what
            // is being removed -- so a file too large to hold in
            // memory is still deletable, with a diff that names it
            // instead of reproducing it.
            let existing = read_or_digest(trusted_root, &resolved, &path)?;
            build
                .base_hashes
                .insert(path.as_str().to_owned(), existing.hash());
            build.total_bytes += existing.size();
            build.diff_sections.push(match &existing {
                ExistingContent::Full(bytes) => {
                    diff_for_delete(path.as_str(), bytes, classify_bytes(bytes))
                }
                ExistingContent::Digest { size, .. } => {
                    format!("Deleted: {} ({size} bytes)\n", path.as_str())
                }
            });
            operations.push(PatchOperation::Delete { path });
        }
        EditOperation::Rename { from, to } => {
            if from == to {
                return Err(PatchError::RenameSamePath(from.as_str().to_owned()));
            }
            insert_rename_targets(&mut build.touched_paths, from.as_str(), to.as_str())?;
            let resolved_from = resolve_existing_or(
                &from,
                || PatchError::RenameSourceMissing(from.as_str().to_owned()),
                trusted_root,
            )?;
            reject_directory(trusted_root, &resolved_from, &from)?;
            // A rename moves bytes without reading them: its diff is
            // a header, so hashing the source by streaming is all the
            // plan needs and no size bound applies.
            let (from_hash, from_size) = hash_stream(trusted_root, &resolved_from)?;
            build
                .base_hashes
                .insert(from.as_str().to_owned(), from_hash);
            build.total_bytes += from_size;

            let destination_exists = match trusted_root.resolve_existing(to.clone()) {
                Ok(resolved_to) => {
                    reject_directory(trusted_root, &resolved_to, &to)?;
                    let (dest_hash, _) = hash_stream(trusted_root, &resolved_to)?;
                    build.base_hashes.insert(to.as_str().to_owned(), dest_hash);
                    build.has_rename_existing = true;
                    true
                }
                Err(ContainmentError::NotFound) => {
                    // Still must name a legally creatable location.
                    trusted_root.resolve_new(to.clone())?;
                    false
                }
                Err(other) => return Err(other.into()),
            };
            build
                .proposed_hashes
                .insert(to.as_str().to_owned(), from_hash);
            build.diff_sections.push(diff_for_rename(
                from.as_str(),
                to.as_str(),
                destination_exists,
            ));
            operations.push(PatchOperation::Rename { from, to });
        }
        EditOperation::RenameDirectory { from, to } => {
            if from == to {
                return Err(PatchError::RenameSamePath(from.as_str().to_owned()));
            }
            insert_rename_targets(&mut build.touched_paths, from.as_str(), to.as_str())?;
            let resolved_from = match trusted_root.resolve_existing(from.clone()) {
                Ok(resolved) => resolved,
                Err(ContainmentError::NotFound) => {
                    return Err(PatchError::DirectoryMissing(from.as_str().to_owned()))
                }
                Err(other) => return Err(other.into()),
            };
            require_directory(trusted_root, &resolved_from, &from)?;
            // Renaming onto an existing directory would either clobber or
            // merge, and neither is something a reviewer reading "rename"
            // would expect.
            match trusted_root.resolve_existing(to.clone()) {
                Ok(_) => {
                    return Err(PatchError::RenameDirectoryDestinationExists(
                        to.as_str().to_owned(),
                    ))
                }
                Err(ContainmentError::NotFound) => {
                    trusted_root.resolve_new(to.clone())?;
                }
                Err(other) => return Err(other.into()),
            }
            // A directory rename moves whatever is under it in one syscall:
            // there is no content to hash and nothing to enumerate, so the
            // plan records the move itself.
            build.diff_sections.push(format!(
                "Directory renamed: {} -> {}\n",
                from.as_str(),
                to.as_str()
            ));
            operations.push(PatchOperation::RenameDirectory { from, to });
        }
        EditOperation::DeleteDirectory { path } => {
            insert_unique(&mut build.touched_paths, path.as_str())?;
            // A directory delete is as unrecoverable as a file delete.
            build.has_delete = true;
            let resolved = match trusted_root.resolve_existing(path.clone()) {
                Ok(resolved) => resolved,
                Err(ContainmentError::NotFound) => {
                    return Err(PatchError::DirectoryMissing(path.as_str().to_owned()))
                }
                Err(other) => return Err(other.into()),
            };
            require_directory(trusted_root, &resolved, &path)?;
            // Emptiness is deliberately *not* checked here: the removal
            // itself enforces it atomically, so a directory that gains an
            // entry between now and then fails rather than being destroyed.
            build
                .diff_sections
                .push(format!("Directory deleted: {}\n", path.as_str()));
            operations.push(PatchOperation::DeleteDirectory { path });
        }
    }
    Ok(())
}

impl PlanBuild {
    /// Records one file whose whole content is being replaced -- whether the
    /// author supplied that content directly or it was resolved from anchored
    /// edits. Both arrive here as final bytes, so both are hashed, risk-rated,
    /// and diffed identically.
    fn record_modify(
        &mut self,
        path: &RelativeProjectPath,
        existing: &[u8],
        content: &[u8],
    ) -> Result<(), PatchError> {
        if existing == content {
            return Err(PatchError::NoChange(path.as_str().to_owned()));
        }
        self.base_hashes
            .insert(path.as_str().to_owned(), hash_bytes(existing));
        let old_kind = classify_bytes(existing);
        let new_kind = classify_bytes(content);
        // Either side being non-text makes this modification unreviewable in
        // the rendered diff (`diff_for_modify` falls back to the opaque
        // "Binary files ... differ" placeholder under the identical
        // condition), so both sides -- not just the proposed content -- must
        // trigger the second-confirmation risk tier.
        if !matches!(old_kind, ContentKind::Text) || !matches!(new_kind, ContentKind::Text) {
            self.has_binary_content = true;
        }
        self.proposed_hashes
            .insert(path.as_str().to_owned(), hash_bytes(content));
        self.total_bytes += content.len() as u64;
        self.diff_sections.push(diff_for_modify(
            path.as_str(),
            existing,
            old_kind,
            content,
            new_kind,
        ));
        Ok(())
    }
}

/// Resolves the existing file a modification targets and reads its whole
/// current content. This is the *single* read every anchored or whole-file
/// modification is resolved, hashed, and diffed from -- see
/// `super::edit`'s module doc comment for why one read matters.
fn existing_content_for_edit(
    trusted_root: &TrustedRoot,
    path: &RelativeProjectPath,
    build: &mut PlanBuild,
) -> Result<Vec<u8>, PatchError> {
    insert_unique(&mut build.touched_paths, path.as_str())?;
    let resolved = resolve_existing_or(
        path,
        || PatchError::ModifyTargetMissing(path.as_str().to_owned()),
        trusted_root,
    )?;
    reject_directory(trusted_root, &resolved, path)?;
    read_bounded(trusted_root, &resolved, path)
}

/// Whether the file at `path` already holds exactly `content`.
fn holds_exactly(trusted_root: &TrustedRoot, path: &RelativeProjectPath, content: &[u8]) -> bool {
    trusted_root
        .resolve_existing(path.clone())
        .ok()
        .and_then(|resolved| read_bounded(trusted_root, &resolved, path).ok())
        .is_some_and(|existing| existing == content)
}

/// Refuses whole-file content that contains a placeholder comment standing in
/// for code, unless the file already held that very line. Binary content has
/// no comments to read, so it passes.
fn refuse_placeholder(
    path: &RelativeProjectPath,
    content: &[u8],
    existing: &[u8],
) -> Result<(), PatchError> {
    let Ok(text) = std::str::from_utf8(content) else {
        return Ok(());
    };
    if is_prose_path(path.as_str()) {
        return Ok(());
    }
    let reference = std::str::from_utf8(existing).unwrap_or("");
    match lazy_placeholder(text, reference) {
        Some(line) => Err(EditError::PlaceholderContent(path.as_str().to_owned(), line).into()),
        None => Ok(()),
    }
}

fn require_text<'a>(existing: &'a [u8], path: &RelativeProjectPath) -> Result<&'a str, PatchError> {
    if !matches!(classify_bytes(existing), ContentKind::Text) {
        return Err(PatchError::NotText(path.as_str().to_owned()));
    }
    Ok(std::str::from_utf8(existing)
        .expect("classify_bytes only reports Text for a fully valid UTF-8 buffer"))
}

/// What a plan knows about a file it is not replacing. A delete needs the
/// identity of the bytes it removes, not the bytes themselves, so a file too
/// large to hold in memory still has a usable base hash.
enum ExistingContent {
    Full(Vec<u8>),
    Digest { hash: ContentHash, size: u64 },
}

impl ExistingContent {
    fn hash(&self) -> ContentHash {
        match self {
            Self::Full(bytes) => hash_bytes(bytes),
            Self::Digest { hash, .. } => *hash,
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::Full(bytes) => bytes.len() as u64,
            Self::Digest { size, .. } => *size,
        }
    }
}

fn read_or_digest(
    trusted_root: &TrustedRoot,
    resolved: &ResolvedProjectPath,
    path: &RelativeProjectPath,
) -> Result<ExistingContent, PatchError> {
    if trusted_root.file_metadata(resolved)?.len() > MAX_EDITABLE_FILE_BYTES {
        let (hash, size) = hash_stream(trusted_root, resolved)?;
        return Ok(ExistingContent::Digest { hash, size });
    }
    read_bounded(trusted_root, resolved, path).map(ExistingContent::Full)
}

/// Chunk size for [`hash_stream`]. BLAKE3 is fastest on large slices; 64 KiB
/// keeps the read syscall count low without holding a meaningful buffer.
const STREAM_HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Hashes a file's whole current content without holding it in memory, and
/// returns its size in the same pass. This is what lets a plan capture the
/// identity of a file of any size.
fn hash_stream(
    trusted_root: &TrustedRoot,
    resolved: &ResolvedProjectPath,
) -> Result<(ContentHash, u64), PatchError> {
    let mut file = trusted_root.open_file(resolved)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; STREAM_HASH_CHUNK_BYTES];
    let mut size: u64 = 0;
    loop {
        let read = file.read(&mut buffer).map_err(PatchError::Io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hasher.finalize(), size))
}

/// A path as the filesystem compares it: folded on the filesystems that fold,
/// where `Page.html` and `page.html` are one file.
fn folded(path: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

/// Records both ends of a rename. A rename that only changes case names one
/// file twice, which is the rename itself and not a clash.
fn insert_rename_targets(
    touched: &mut BTreeSet<String>,
    from: &str,
    to: &str,
) -> Result<(), PatchError> {
    insert_unique(touched, from)?;
    if folded(from) == folded(to) {
        return Ok(());
    }
    insert_unique(touched, to)
}

fn insert_unique(touched: &mut BTreeSet<String>, path: &str) -> Result<(), PatchError> {
    let key = folded(path);
    if touched.insert(key) {
        Ok(())
    } else {
        Err(PatchError::DuplicatePathTarget(path.to_owned()))
    }
}

fn check_content_bound(path: &RelativeProjectPath, content: &[u8]) -> Result<(), PatchError> {
    if content.len() as u64 > MAX_EDITABLE_FILE_BYTES {
        return Err(PatchError::ContentTooLarge(
            path.as_str().to_owned(),
            MAX_EDITABLE_FILE_BYTES,
        ));
    }
    Ok(())
}

fn resolve_existing_or(
    path: &RelativeProjectPath,
    missing: impl FnOnce() -> PatchError,
    trusted_root: &TrustedRoot,
) -> Result<ResolvedProjectPath, PatchError> {
    match trusted_root.resolve_existing(path.clone()) {
        Ok(resolved) => Ok(resolved),
        Err(ContainmentError::NotFound) => Err(missing()),
        Err(other) => Err(other.into()),
    }
}

/// A directory operation cannot share a plan with an operation *inside* that
/// directory.
///
/// Not a policy choice: `super::writer` stages every operation before applying
/// any of them (its "no cross-file transaction" bound), so a staged file
/// operation holds an open handle to its parent directory for the whole apply
/// phase -- and Windows refuses to rename or remove a directory while any
/// handle to it is open. Rather than weaken the stage-then-apply property that
/// bounds partial application, this is refused at propose time with the
/// sequence that does work: the file operations in one call, the directory in
/// the next.
fn reject_directory_operation_over_contained_path(
    operations: &[PatchOperation],
) -> Result<(), PatchError> {
    let directories: Vec<&str> = operations
        .iter()
        .filter_map(|operation| match operation {
            PatchOperation::DeleteDirectory { path } => Some(path.as_str()),
            PatchOperation::RenameDirectory { from, .. } => Some(from.as_str()),
            _ => None,
        })
        .collect();
    if directories.is_empty() {
        return Ok(());
    }

    for operation in operations {
        let paths: Vec<&str> = match operation {
            PatchOperation::Create { path, .. }
            | PatchOperation::Modify { path, .. }
            | PatchOperation::Delete { path } => vec![path.as_str()],
            PatchOperation::Rename { from, to } => vec![from.as_str(), to.as_str()],
            // A directory operation nested under another one has the same
            // problem, and is caught by the same comparison.
            PatchOperation::RenameDirectory { from, to } => vec![from.as_str(), to.as_str()],
            PatchOperation::DeleteDirectory { path } => vec![path.as_str()],
        };
        for path in paths {
            for directory in &directories {
                if path == *directory {
                    continue;
                }
                // Compared on the separator so `srcfile` is not treated as
                // living inside `src`.
                if path.starts_with(&format!("{directory}/")) {
                    return Err(PatchError::DirectoryOperationWithContainedPath(
                        (*directory).to_owned(),
                        path.to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The mirror of [`reject_directory`]: a directory operation must actually
/// name a directory, so a file caught here is named along with the operation
/// that would have worked.
fn require_directory(
    trusted_root: &TrustedRoot,
    resolved: &ResolvedProjectPath,
    path: &RelativeProjectPath,
) -> Result<(), PatchError> {
    if !trusted_root.file_metadata(resolved)?.is_dir() {
        return Err(PatchError::TargetIsNotDirectory(path.as_str().to_owned()));
    }
    Ok(())
}

fn reject_directory(
    trusted_root: &TrustedRoot,
    resolved: &ResolvedProjectPath,
    path: &RelativeProjectPath,
) -> Result<(), PatchError> {
    let metadata = trusted_root.file_metadata(resolved)?;
    if metadata.is_dir() {
        return Err(PatchError::TargetIsDirectory(path.as_str().to_owned()));
    }
    Ok(())
}

/// Reads `resolved`'s entire current content, bounded by
/// [`MAX_EDITABLE_FILE_BYTES`] -- never truncated silently: a file over the
/// bound fails the whole proposal outright (an edit resolved against a
/// truncated buffer would drop everything past the bound, and a base hash
/// over it would not cover the whole file, defeating the stale-base-hash
/// check's own purpose).
fn read_bounded(
    trusted_root: &TrustedRoot,
    resolved: &ResolvedProjectPath,
    path: &RelativeProjectPath,
) -> Result<Vec<u8>, PatchError> {
    let file = trusted_root.open_file(resolved)?;
    let mut limited = file.take(MAX_EDITABLE_FILE_BYTES + 1);
    let mut buffer = Vec::new();
    limited.read_to_end(&mut buffer).map_err(PatchError::Io)?;
    if buffer.len() as u64 > MAX_EDITABLE_FILE_BYTES {
        return Err(PatchError::ExistingContentTooLarge(
            path.as_str().to_owned(),
            MAX_EDITABLE_FILE_BYTES,
        ));
    }
    Ok(buffer)
}

fn diff_for_create(path: &str, content: &[u8], kind: ContentKind) -> String {
    if !matches!(kind, ContentKind::Text) {
        return format!("Binary file created: {path} ({} bytes)\n", content.len());
    }
    let Ok(text) = std::str::from_utf8(content) else {
        return format!("Binary file created: {path} ({} bytes)\n", content.len());
    };
    let diff = TextDiff::from_lines("", text);
    let rendered = diff
        .unified_diff()
        .context_radius(3)
        .header("/dev/null", &format!("b/{path}"))
        .to_string();
    if rendered.is_empty() {
        format!("Created empty file: {path}\n")
    } else {
        rendered
    }
}

fn diff_for_delete(path: &str, content: &[u8], kind: ContentKind) -> String {
    if !matches!(kind, ContentKind::Text) {
        return format!("Binary file deleted: {path} ({} bytes)\n", content.len());
    }
    let Ok(text) = std::str::from_utf8(content) else {
        return format!("Binary file deleted: {path} ({} bytes)\n", content.len());
    };
    let diff = TextDiff::from_lines(text, "");
    let rendered = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), "/dev/null")
        .to_string();
    if rendered.is_empty() {
        format!("Deleted empty file: {path}\n")
    } else {
        rendered
    }
}

fn diff_for_modify(
    path: &str,
    old: &[u8],
    old_kind: ContentKind,
    new: &[u8],
    new_kind: ContentKind,
) -> String {
    if !matches!(old_kind, ContentKind::Text) || !matches!(new_kind, ContentKind::Text) {
        return format!("Binary files a/{path} and b/{path} differ\n");
    }
    let (Ok(old_text), Ok(new_text)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return format!("Binary files a/{path} and b/{path} differ\n");
    };
    if old_text == new_text {
        return format!("(no content change): {path}\n");
    }
    let diff = TextDiff::from_lines(old_text, new_text);
    diff.unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

fn diff_for_rename(from: &str, to: &str, destination_exists: bool) -> String {
    if destination_exists {
        format!("renamed: {from} -> {to} (replaces existing file)\n")
    } else {
        format!("renamed: {from} -> {to}\n")
    }
}

fn bound_diff(sections: Vec<String>) -> String {
    let mut combined = String::new();
    for section in sections {
        combined.push_str(&section);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    if combined.len() > MAX_DIFF_BYTES {
        let mut end = MAX_DIFF_BYTES;
        while end > 0 && !combined.is_char_boundary(end) {
            end -= 1;
        }
        combined.truncate(end);
        combined.push_str("\n... (diff truncated)\n");
    }
    combined
}

fn update_field(hasher: &mut blake3::Hasher, field: &[u8]) {
    hasher.update(&(field.len() as u64).to_be_bytes());
    hasher.update(field);
}

/// BLAKE3 of the plan's own canonical contents: project/root identity, every
/// operation's kind tag and path(s) in order, and every base/proposed hash
/// (iterated in `BTreeMap` sorted-key order, so this is independent of
/// insertion order). Operation *content* is not hashed a second time here --
/// it is already captured via `proposed_hashes` (Create/Modify) or carried
/// through `base_hashes` (Rename's unchanged source content), so this never
/// re-reads or re-hashes potentially large byte buffers.
fn compute_plan_id(
    project_id: ProjectId,
    root_identity: RootIdentity,
    operations: &[PatchOperation],
    base_hashes: &BTreeMap<String, ContentHash>,
    proposed_hashes: &BTreeMap<String, ContentHash>,
) -> PlanId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"xfalcon-mutation-plan-v1");
    update_field(&mut hasher, project_id.as_uuid().as_bytes());
    let (primary, secondary) = root_identity.into_parts();
    hasher.update(&primary.to_be_bytes());
    hasher.update(&secondary.to_be_bytes());
    hasher.update(&(operations.len() as u64).to_be_bytes());
    for operation in operations {
        update_field(&mut hasher, operation.kind().as_bytes());
        match operation {
            PatchOperation::Create { path, .. }
            | PatchOperation::Modify { path, .. }
            | PatchOperation::Delete { path }
            | PatchOperation::DeleteDirectory { path } => {
                update_field(&mut hasher, path.as_str().as_bytes());
            }
            PatchOperation::Rename { from, to } | PatchOperation::RenameDirectory { from, to } => {
                update_field(&mut hasher, from.as_str().as_bytes());
                update_field(&mut hasher, to.as_str().as_bytes());
            }
        }
    }
    hasher.update(&(base_hashes.len() as u64).to_be_bytes());
    for (path, hash) in base_hashes {
        update_field(&mut hasher, path.as_bytes());
        hasher.update(hash.as_bytes());
    }
    hasher.update(&(proposed_hashes.len() as u64).to_be_bytes());
    for (path, hash) in proposed_hashes {
        update_field(&mut hasher, path.as_bytes());
        hasher.update(hash.as_bytes());
    }
    PlanId(hasher.finalize())
}
