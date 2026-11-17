//! The atomic writer: the only code path that ever mutates a trusted
//! project's filesystem content.
//!
//! # Security contract
//!
//! [`execute`] is the sole function in this crate that writes, renames, or
//! deletes a byte of trusted-project content. There is no other route to it:
//! [`crate::tools::registry`]'s `project.propose_patch` descriptor only
//! *plans* ([`super::patch::MutationPlan::propose`], which never touches the
//! filesystem beyond reading for base hashes/diffing); applying a plan is
//! only ever `crate::approvals::service::ApprovalsService::resolve` with
//! `approve = true`, which is the only production caller of [`execute`].
//!
//! ## The non-confirmable containment check cannot be bypassed by any
//! ## confirmation level
//!
//! [`execute`] revalidates root identity, containment, the approval token,
//! the plan's own digest, and every base hash before staging or applying
//! anything (Task 6, Step 4). Two gates run in [`execute`]'s preamble --
//! the approval-token/expiry/digest checks and the second-confirmation
//! check (`plan.requires_second_confirmation() && !approval.second_confirmation_given`,
//! read *before* the containment re-check below) -- and neither one is an
//! alternative to the other: both are independent, unconditional
//! preconditions that must *both* pass before anything else runs. Passing
//! the confirmation gate never skips, weakens, or influences the
//! containment re-check that follows it; failing the confirmation gate is
//! itself an immediate denial that never reaches containment at all. The
//! containment re-check itself ([`super::domain::TrustedRoot::reopen`] plus
//! a fresh [`super::domain::TrustedRoot::resolve_existing`]/
//! [`super::domain::TrustedRoot::resolve_new`]/[`super::domain::TrustedRoot::write_target`]
//! call for every single path in the plan) is **not** conditioned on
//! [`ApprovalProof`] in any way: it runs using only the plan's own project
//! id, root identity, and paths, and its `Result` is propagated as an
//! immediate `Err`. There is structurally no boolean, enum variant, or code
//! path anywhere in this function (or in [`super::domain::TrustedRoot`],
//! which this function is the only writer-side caller of) that reads
//! `approval.second_confirmation_given` a second time, or that lets a
//! caller "approve past" a containment failure at any confirmation level --
//! a symlink/reparse/mount-escape/outside-root path fails [`execute`]
//! exactly the same way whether `approval.second_confirmation_given` is
//! `true` or `false`. This is what the plan's "non-confirmable denial ...
//! structurally impossible to approve past, at any confirmation level"
//! requirement means in this implementation: it is not a risk tier stored
//! alongside the approval decision, it is a precondition the approval
//! decision's own fields cannot influence.
//!
//! ## Durability
//!
//! Every `Create`/`Modify` writes a same-directory temporary file, flushes
//! its content (`cap_std::fs::File::sync_all`, cross-platform: `fsync` on
//! Unix, `FlushFileBuffers` on Windows), then atomically renames it onto the
//! destination (`super::domain::WriteTarget::atomic_replace`). `Rename`
//! renames the source directly onto the destination with no intervening
//! temp file. `Delete` removes the file outright. After a successful
//! replace/rename/delete, the affected parent directory is fsynced on Unix
//! (`super::platform::fsync_dir_after_replace`) -- the only way to make the
//! rename/unlink itself durable, not just the file content. See
//! `super::domain::WriteTarget::atomic_replace`'s doc comment for the
//! cross-platform atomicity/no-copy-fallback argument (backed by reading
//! `cap-primitives`'s own unchecked implementations directly).
//!
//! A parent-directory flush failure is never swallowed, but it is also
//! never allowed to misreport a mutation that already durably landed as one
//! that "never happened": [`apply`] distinguishes the two by returning
//! [`ApplyOutcome::AppliedDurabilityUnconfirmed`] when the mutating
//! primitive (rename/unlink) itself succeeded but the follow-up parent-fsync
//! failed. [`execute`] still counts that stage in
//! [`WriterExecutionFailure::operations_completed`] (it *did* land) and sets
//! [`WriterExecutionFailure::durability_unconfirmed`], so a caller
//! (`crate::approvals::service::ApprovalsService::resolve`) can persist a
//! status distinct from an ordinary failure rather than asserting a
//! filesystem mutation -- especially an irreversible `Delete` -- never
//! happened when it, in fact, did.
//!
//! ## No cross-file transaction (disclosed limitation)
//!
//! A real filesystem offers no primitive to apply several independent
//! file replaces/renames/deletes as a single cross-file transaction. Every
//! operation in a plan is independently re-validated (containment + base
//! hash) *before* any of them are applied, and every temp file for the whole
//! plan is staged (written + flushed) before the apply phase begins, which
//! bounds -- but does not eliminate -- the window in which a plan could be
//! left partially applied by a failure that appears only during the apply
//! phase itself (e.g. a permission change or sharing violation that starts
//! after staging but before that specific operation's atomic swap). A
//! `Create` operation's "target must not already exist" precondition and every
//! destructive operation's base-hash/directory-entry precondition are
//! re-checked a second time, immediately before that operation mutates disk
//! (see [`apply`]). These checks narrow, but do not eliminate, the final
//! check-to-syscall window: `cap-std` exposes only replacing rename on the
//! platforms this crate supports, not an atomic cross-platform no-replace or
//! compare-and-swap primitive.
//! [`WriterExecutionFailure`] always reports exactly how many operations
//! completed before the failure, and completed operations are never rolled
//! back (each one is itself individually atomic and durable the instant it
//! completes, so "rolling back" would itself be a second, equally
//! non-transactional mutation). This is disclosed here rather than silently
//! assumed away, matching this codebase's existing "honest caveat"
//! convention (see `docs/decisions.md`).

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::domain::{ContainmentError, ResolvedProjectPath, TrustedRoot, WriteTarget};
use super::patch::{hash_bytes, MutationPlan, PatchOperation, PlanId};
use super::path_policy::RelativeProjectPath;
use super::platform::RootIdentity;

#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("the approval token does not match this exact plan")]
    ApprovalTokenMismatch,
    #[error("this plan has expired and can no longer be executed")]
    Expired,
    #[error("this plan requires a second confirmation that was not given")]
    SecondConfirmationRequired,
    #[error("the plan's own content-addressed digest is no longer self-consistent")]
    PlanDigestMismatch,
    #[error(transparent)]
    Containment(ContainmentError),
    #[error("the file is locked by another process and cannot be mutated: {0}")]
    Locked(#[source] io::Error),
    #[error("create target {0:?} now exists (it did not when this plan was proposed)")]
    CreateTargetNowExists(String),
    #[error("the current content of {0:?} no longer matches the base hash captured when this plan was proposed")]
    StaleBaseHash(String),
    #[error("failed to write staged content for {0:?}: {1}")]
    StageWrite(String, #[source] io::Error),
    #[error("{0:?} is not a directory")]
    NotADirectory(String),
    #[error("directory rename destination {0:?} already exists; renaming onto an existing directory is refused")]
    RenameDirectoryDestinationExists(String),
    #[error("rename destination {0:?} now exists (it did not when this plan was proposed)")]
    RenameTargetNowExists(String),
    #[error(
        "directory {0:?} is no longer the directory entry captured while this plan was staged"
    )]
    DirectoryEntryChanged(String),
    #[error("directory {0:?} is not empty; delete its contents first: {1}")]
    DirectoryNotEmpty(String, #[source] std::io::Error),
    #[error("{0:?} and its destination are on different filesystems/volumes; atomic rename cannot cross filesystems")]
    CrossVolume(String),
    #[error("the atomic replace for {0:?} failed: {1}")]
    Replace(String, #[source] io::Error),
}

/// Proof that `crate::approvals`'s policy already ran and approved *exactly*
/// this plan. The only thing that authorizes [`execute`] to proceed past its
/// token/expiry/second-confirmation preamble. The type itself is
/// `pub(crate)` (crate-internal only; no external crate linking this lib can
/// name it), and `new` is also `pub(crate)` rather than the tighter
/// `pub(in crate::approvals)` because Rust's `pub(in path)` visibility
/// requires `path` to be an ancestor of `new`'s own defining module
/// (`crate::projects::writer`) -- see the doc comment on `new` itself for
/// why a cross-module, non-ancestor call site cannot be restricted any
/// further. As a matter of this codebase's own convention (not a
/// compiler-enforced guarantee beyond that crate boundary), the only
/// production call site is inside
/// `crate::approvals::policy::require_sufficient_confirmation`, which
/// constructs and returns this value itself only after its own
/// second-confirmation check has already passed -- callers never supply the
/// `second_confirmation_given` field freely. Carrying `plan_id` (never a bare
/// `bool` "approved" flag alone) means an approval obtained for one plan can
/// never be presented against a different one: [`execute`]'s very first
/// check compares this against [`MutationPlan::plan_id`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct ApprovalProof {
    plan_id: PlanId,
    second_confirmation_given: bool,
}

impl ApprovalProof {
    /// `pub(crate)` rather than narrower because Rust's `pub(in path)`
    /// visibility requires `path` to be an ancestor of this item's own
    /// defining module (`crate::projects::writer`); `crate::approvals` is a
    /// sibling, not an ancestor, so `pub(crate)` is the strictest visibility
    /// that still lets `crate::approvals::policy::require_sufficient_confirmation`
    /// call this. See that function's doc comment for why it -- not an
    /// arbitrary caller -- is the only place in this codebase that does.
    #[must_use]
    pub(crate) fn new(plan_id: PlanId, second_confirmation_given: bool) -> Self {
        Self {
            plan_id,
            second_confirmation_given,
        }
    }
}

/// A plan's execution result: how many of its operations actually landed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionOutcome {
    pub plan_id: PlanId,
    pub operations_completed: usize,
    pub operations_total: usize,
}

/// Splits an ordinary file-lock condition out of [`ContainmentError`] before
/// it reaches a caller.
///
/// `ContainmentError` deliberately carries security verdicts —
/// `LinkOrReparse`, `RootIdentityChanged`, `MismatchedRoot` — alongside a
/// generic `Io`. Forwarding the whole enum through `WriterError::Containment`
/// meant a mundane "another process has this file open" surfaced to the
/// approvals layer, the audit trail, and the user as a *containment*
/// failure: indistinguishable from a path escaping the trusted root. The two
/// demand opposite responses. A containment failure is a security event and
/// is never retryable; a lock clears the moment the other process closes the
/// handle, and the right advice is "close the file and try again".
impl From<ContainmentError> for WriterError {
    fn from(error: ContainmentError) -> Self {
        match error {
            ContainmentError::Io(io) if is_lock_violation(&io) => Self::Locked(io),
            other => Self::Containment(other),
        }
    }
}

/// Whether an I/O error means "another process holds this file", as opposed
/// to a genuine access-control or containment problem.
///
/// Matched on raw OS codes because `io::ErrorKind` has no stable variant for
/// either condition — both surface as `Uncategorized`, which is not
/// matchable.
fn is_lock_violation(error: &io::Error) -> bool {
    match error.raw_os_error() {
        // ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33): the
        // canonical Windows result of opening a file another process holds
        // without FILE_SHARE_* rights.
        #[cfg(windows)]
        Some(32 | 33) => true,
        // ETXTBSY: the closest Unix equivalent — the file is being executed.
        #[cfg(unix)]
        Some(libc_etxtbsy) => libc_etxtbsy == 26,
        _ => false,
    }
}

/// A failed [`execute`] call, carrying how far it got before failing (see
/// the module doc comment's "no cross-file transaction" disclosure).
///
/// `durability_unconfirmed` is `true` only when every one of
/// `operations_completed` operations' mutating primitive (rename/unlink)
/// actually succeeded, and the failure itself is a post-mutation
/// parent-directory fsync that could not be confirmed -- never a case where
/// the mutation itself failed to land. Callers (`crate::approvals::service`)
/// use this to avoid persisting a status that claims a mutation never
/// happened when it, in fact, did.
#[derive(Debug, thiserror::Error)]
#[error("{error} ({operations_completed} of {operations_total} operations completed; durability_unconfirmed={durability_unconfirmed})")]
pub struct WriterExecutionFailure {
    #[source]
    pub error: WriterError,
    pub operations_completed: usize,
    pub operations_total: usize,
    pub durability_unconfirmed: bool,
}

fn fail(
    error: WriterError,
    operations_completed: usize,
    operations_total: usize,
) -> WriterExecutionFailure {
    WriterExecutionFailure {
        error,
        operations_completed,
        operations_total,
        durability_unconfirmed: false,
    }
}

fn fail_durability_unconfirmed(
    error: WriterError,
    operations_completed: usize,
    operations_total: usize,
) -> WriterExecutionFailure {
    WriterExecutionFailure {
        error,
        operations_completed,
        operations_total,
        durability_unconfirmed: true,
    }
}

/// Which existence precondition a [`Staged::Replace`] stage must satisfy
/// immediately before its own atomic swap (see [`apply`]'s `Create` branch
/// and the module doc comment's "no cross-file transaction" section).
/// `Create`'s target must still not exist; `Modify`'s base hash is rechecked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplaceKind {
    Create,
    Modify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenameSourceKind {
    File,
    Directory(RootIdentity),
}

/// One operation's already-staged (but not yet applied) filesystem state.
/// `Create`/`Modify` have already had their new content written and flushed
/// to a same-directory temp file; `Delete`/`Rename` need no staging step at
/// all (there is no content to write ahead of time).
enum Staged {
    Replace {
        path: RelativeProjectPath,
        target: WriteTarget,
        temp_name: OsString,
        kind: ReplaceKind,
        /// Parent directories this operation created, in creation order.
        created_dirs: Vec<PathBuf>,
    },
    Delete {
        path: RelativeProjectPath,
        target: WriteTarget,
    },
    /// A directory removal. Separate from [`Staged::Delete`] because the
    /// primitive is `remove_dir`, whose refusal to touch a non-empty directory
    /// is this operation's whole safety argument.
    DeleteDirectory {
        path: RelativeProjectPath,
        target: WriteTarget,
        identity: RootIdentity,
    },
    Rename {
        from_path: RelativeProjectPath,
        to_path: RelativeProjectPath,
        from_target: WriteTarget,
        to_target: WriteTarget,
        source_kind: RenameSourceKind,
        destination_existed: bool,
        created_dirs: Vec<PathBuf>,
    },
}

/// Executes an already-approved [`MutationPlan`]. `canonical_path` is the
/// project's *current* trusted canonical path (the caller -- `ApprovalsService`
/// -- looks this up fresh from the trust store; it is deliberately not
/// carried on [`MutationPlan`] itself, which only carries the identity that
/// path must still resolve to). See the module doc comment for the full
/// non-confirmable-containment argument and the durability/no-cross-file-
/// transaction contract.
///
/// `pub(crate)`, not externally `pub`: the only legitimate caller is
/// `crate::approvals::service::ApprovalsService::resolve`, a same-crate,
/// non-ancestor module, so `pub(crate)` is the strictest visibility Rust's
/// module system allows here (mirroring [`ApprovalProof`]'s own visibility
/// immediately above).
pub(crate) fn execute(
    canonical_path: &Path,
    plan: &MutationPlan,
    approval: &ApprovalProof,
    now_ms: i64,
) -> Result<ExecutionOutcome, WriterExecutionFailure> {
    let total = plan.operations().len();

    if approval.plan_id != plan.plan_id() {
        return Err(fail(WriterError::ApprovalTokenMismatch, 0, total));
    }
    if plan.is_expired(now_ms) {
        return Err(fail(WriterError::Expired, 0, total));
    }
    if !plan.digest_is_self_consistent() {
        return Err(fail(WriterError::PlanDigestMismatch, 0, total));
    }
    if plan.requires_second_confirmation() && !approval.second_confirmation_given {
        return Err(fail(WriterError::SecondConfirmationRequired, 0, total));
    }

    // Non-confirmable from here on: nothing below ever reads
    // `approval.second_confirmation_given` again. A containment or
    // stale-base-hash failure in the loop below is returned unconditionally.
    let trusted_root = TrustedRoot::reopen(
        plan.project_id(),
        canonical_path.to_path_buf(),
        plan.root_identity(),
    )
    .map_err(|error| fail(WriterError::Containment(error), 0, total))?;

    let mut staged: Vec<Staged> = Vec::with_capacity(total);
    for operation in plan.operations() {
        match stage_operation(&trusted_root, plan, operation) {
            Ok(stage) => staged.push(stage),
            Err(error) => {
                let orphans = cleanup(&staged);
                // Dropping the staged targets closes their directory handles,
                // which Windows requires before those directories can go.
                drop(staged);
                remove_created_dirs(&trusted_root, &orphans);
                return Err(fail(error, 0, total));
            }
        }
    }

    #[cfg(test)]
    test_hooks::run_after_staging_before_apply_hook();

    let mut completed = 0usize;
    let mut durability_unconfirmed_error: Option<WriterError> = None;
    for stage in &staged {
        match apply(&trusted_root, plan, stage) {
            Ok(ApplyOutcome::Applied) => completed += 1,
            Ok(ApplyOutcome::AppliedDurabilityUnconfirmed(error)) => {
                // The mutation itself already landed -- count it -- but a
                // required post-mutation durability step could not be
                // confirmed. Stop attempting further operations (later,
                // not-yet-applied stages are simply left not-applied, which
                // is safe), and surface this distinctly from an ordinary
                // failure (see the module doc comment's "Durability"
                // section and `WriterExecutionFailure::durability_unconfirmed`).
                completed += 1;
                durability_unconfirmed_error = Some(error);
                break;
            }
            Err(error) => {
                // Only the not-yet-applied tail rolls back: an operation that
                // already landed keeps the directories it needed.
                let orphans = cleanup(&staged[completed..]);
                drop(staged);
                remove_created_dirs(&trusted_root, &orphans);
                return Err(fail(error, completed, total));
            }
        }
    }

    if let Some(error) = durability_unconfirmed_error {
        let orphans = cleanup(&staged[completed..]);
        drop(staged);
        remove_created_dirs(&trusted_root, &orphans);
        return Err(fail_durability_unconfirmed(error, completed, total));
    }

    Ok(ExecutionOutcome {
        plan_id: plan.plan_id(),
        operations_completed: completed,
        operations_total: total,
    })
}

/// Checks whether every one of `plan`'s operations already matches the
/// current on-disk state, as though [`execute`] had already fully applied
/// it. Used by `crate::approvals::service::ApprovalsService::resolve` to
/// distinguish "this plan never ran" from "this plan already ran to
/// completion, but a crash occurred before its approval row's status could
/// be persisted" -- a crash between [`execute`] returning `Ok` and that
/// caller's own follow-up status write must never cause a retry to
/// misclassify an already-successful plan as newly `Failed`.
///
/// Only ever returns `true` when *every* operation's expected post-state is
/// positively confirmed; any ambiguity (an unreadable path, a containment
/// error, oversized content) returns `false` so the caller falls back to
/// its normal `execute` attempt -- this function never causes a plan to be
/// marked executed on anything less than certainty.
#[must_use]
pub(crate) fn plan_already_applied(trusted_root: &TrustedRoot, plan: &MutationPlan) -> bool {
    for operation in plan.operations() {
        let satisfied = match operation {
            PatchOperation::Create { path, .. } | PatchOperation::Modify { path, .. } => plan
                .proposed_hashes()
                .get(path.as_str())
                .is_some_and(|expected| current_content_matches(trusted_root, path, *expected)),
            PatchOperation::Delete { path } | PatchOperation::DeleteDirectory { path } => {
                path_absent(trusted_root, path)
            }
            // A directory rename leaves no content hash to compare, so
            // "already applied" is exactly: the source is gone and the
            // destination is a directory.
            PatchOperation::RenameDirectory { from, to } => {
                path_absent(trusted_root, from)
                    && trusted_root
                        .resolve_existing(to.clone())
                        .and_then(|resolved| trusted_root.file_metadata(&resolved))
                        .is_ok_and(|metadata| metadata.is_dir())
            }
            PatchOperation::Rename { from, to } => {
                path_absent(trusted_root, from)
                    && plan
                        .proposed_hashes()
                        .get(to.as_str())
                        .is_some_and(|expected| {
                            current_content_matches(trusted_root, to, *expected)
                        })
            }
        };
        if !satisfied {
            return false;
        }
    }
    true
}

/// Whether `path` currently resolves to a plain file whose full content
/// hashes to exactly `expected`. Any error (missing, containment failure,
/// oversized) is treated as "does not match" -- never silently assumed.
fn current_content_matches(
    trusted_root: &TrustedRoot,
    path: &super::path_policy::RelativeProjectPath,
    expected: super::patch::ContentHash,
) -> bool {
    let Ok(resolved) = trusted_root.resolve_existing(path.clone()) else {
        return false;
    };
    let Ok(metadata) = trusted_root.file_metadata(&resolved) else {
        return false;
    };
    if metadata.is_dir() {
        return false;
    }
    let Ok(file) = trusted_root.open_file(&resolved) else {
        return false;
    };
    let mut buffer = Vec::new();
    {
        use std::io::Read;
        if file
            .take(super::read::MAX_HASHED_FILE_BYTES + 1)
            .read_to_end(&mut buffer)
            .is_err()
        {
            return false;
        }
    }
    if buffer.len() as u64 > super::read::MAX_HASHED_FILE_BYTES {
        return false;
    }
    hash_bytes(&buffer) == expected
}

/// Whether `path` currently does not exist (containment-wise: resolves to
/// exactly [`ContainmentError::NotFound`]). Any other error is treated as
/// "cannot confirm absence" (`false`), never silently assumed gone.
fn path_absent(trusted_root: &TrustedRoot, path: &super::path_policy::RelativeProjectPath) -> bool {
    matches!(
        trusted_root.resolve_existing(path.clone()),
        Err(ContainmentError::NotFound)
    )
}

fn stage_operation(
    trusted_root: &TrustedRoot,
    plan: &MutationPlan,
    operation: &PatchOperation,
) -> Result<Staged, WriterError> {
    match operation {
        PatchOperation::Create { path, content } => {
            let resolved = trusted_root.resolve_new(path.clone())?;
            if trusted_root.file_metadata(&resolved).is_ok() {
                return Err(WriterError::CreateTargetNowExists(path.as_str().to_owned()));
            }
            // Creating a file in a directory that does not exist yet is an
            // ordinary create, so the chain is materialized here -- the first
            // mutation this operation makes, and the one `cleanup` undoes if
            // anything after it fails.
            let created_dirs = trusted_root.create_missing_parents(&resolved)?;
            let staged = trusted_root
                .write_target(&resolved)
                .map_err(WriterError::from)
                .and_then(|target| {
                    let temp_name = stage_content(&target, content, path.as_str())?;
                    Ok(Staged::Replace {
                        path: path.clone(),
                        target,
                        temp_name,
                        kind: ReplaceKind::Create,
                        created_dirs: created_dirs.clone(),
                    })
                });
            if staged.is_err() {
                let deepest_first: Vec<PathBuf> = created_dirs.iter().rev().cloned().collect();
                remove_created_dirs(trusted_root, &deepest_first);
            }
            staged
        }
        PatchOperation::Modify { path, content } => {
            let resolved = trusted_root.resolve_existing(path.clone())?;
            require_base_hash_match(trusted_root, plan, path.as_str(), &resolved)?;
            let target = trusted_root.write_target(&resolved)?;
            let temp_name = stage_content(&target, content, path.as_str())?;
            Ok(Staged::Replace {
                path: path.clone(),
                target,
                temp_name,
                kind: ReplaceKind::Modify,
                created_dirs: Vec::new(),
            })
        }
        PatchOperation::Delete { path } => {
            let resolved = trusted_root.resolve_existing(path.clone())?;
            require_base_hash_match(trusted_root, plan, path.as_str(), &resolved)?;
            let target = trusted_root.write_target(&resolved)?;
            Ok(Staged::Delete {
                path: path.clone(),
                target,
            })
        }
        PatchOperation::DeleteDirectory { path } => {
            let resolved = trusted_root.resolve_existing(path.clone())?;
            let target = trusted_root.write_target(&resolved)?;
            // Re-proven here, not trusted from propose time: a directory
            // replaced by a file since then must not be removed by this
            // operation. Emptiness is *not* checked -- `remove_directory`
            // enforces that atomically at apply time.
            let metadata = trusted_root.file_metadata(&resolved)?;
            if !metadata.is_dir() || !target.is_directory() {
                return Err(WriterError::NotADirectory(path.as_str().to_owned()));
            }
            Ok(Staged::DeleteDirectory {
                path: path.clone(),
                target,
                identity: trusted_root.directory_identity(&resolved)?,
            })
        }
        PatchOperation::RenameDirectory { from, to } => {
            let resolved_from = trusted_root.resolve_existing(from.clone())?;
            let from_target = trusted_root.write_target(&resolved_from)?;
            let source_metadata = trusted_root.file_metadata(&resolved_from)?;
            if !source_metadata.is_dir() || !from_target.is_directory() {
                return Err(WriterError::NotADirectory(from.as_str().to_owned()));
            }
            let identity = trusted_root.directory_identity(&resolved_from)?;
            let resolved_to = trusted_root.resolve_new(to.clone())?;
            let created_dirs = trusted_root.create_missing_parents(&resolved_to)?;
            let to_target = match trusted_root.write_target(&resolved_to) {
                Ok(target) => target,
                Err(error) => {
                    let deepest_first: Vec<PathBuf> = created_dirs.iter().rev().cloned().collect();
                    remove_created_dirs(trusted_root, &deepest_first);
                    return Err(error.into());
                }
            };
            // Refused rather than merged or clobbered: "rename" must not
            // quietly absorb an existing tree.
            if to_target.exists() {
                let deepest_first: Vec<PathBuf> = created_dirs.iter().rev().cloned().collect();
                remove_created_dirs(trusted_root, &deepest_first);
                return Err(WriterError::RenameDirectoryDestinationExists(
                    to.as_str().to_owned(),
                ));
            }
            Ok(Staged::Rename {
                from_path: from.clone(),
                to_path: to.clone(),
                from_target,
                to_target,
                source_kind: RenameSourceKind::Directory(identity),
                destination_existed: false,
                created_dirs,
            })
        }
        PatchOperation::Rename { from, to } => {
            let resolved_from = trusted_root.resolve_existing(from.clone())?;
            require_base_hash_match(trusted_root, plan, from.as_str(), &resolved_from)?;
            let from_target = trusted_root.write_target(&resolved_from)?;

            let mut created_dirs = Vec::new();
            let (to_target, destination_existed) = match trusted_root.resolve_existing(to.clone()) {
                Ok(resolved_to) => {
                    require_base_hash_match(trusted_root, plan, to.as_str(), &resolved_to)?;
                    (trusted_root.write_target(&resolved_to)?, true)
                }
                Err(ContainmentError::NotFound) => {
                    let resolved_to = trusted_root.resolve_new(to.clone())?;
                    created_dirs = trusted_root.create_missing_parents(&resolved_to)?;
                    match trusted_root.write_target(&resolved_to) {
                        Ok(target) => (target, false),
                        Err(error) => {
                            let deepest_first: Vec<PathBuf> =
                                created_dirs.iter().rev().cloned().collect();
                            remove_created_dirs(trusted_root, &deepest_first);
                            return Err(error.into());
                        }
                    }
                }
                Err(other) => return Err(other.into()),
            };

            Ok(Staged::Rename {
                from_path: from.clone(),
                to_path: to.clone(),
                from_target,
                to_target,
                source_kind: RenameSourceKind::File,
                destination_existed,
                created_dirs,
            })
        }
    }
}

/// Re-derives `path`'s *current* content hash (fresh, right now) and
/// rejects the whole plan if it no longer matches the base hash captured at
/// propose time -- the stale-base-hash check the plan requires ("never
/// partially apply"). A path with no recorded base hash (should not occur
/// for any operation kind that reaches this function) is treated as a
/// mismatch, failing closed rather than skipping the check.
fn require_base_hash_match(
    trusted_root: &TrustedRoot,
    plan: &MutationPlan,
    path: &str,
    resolved: &ResolvedProjectPath,
) -> Result<(), WriterError> {
    let metadata = trusted_root.file_metadata(resolved)?;
    if metadata.is_dir() {
        return Err(ContainmentError::TargetIsDirectory.into());
    }
    let mut file = trusted_root.open_file(resolved)?;
    // Hashed as it is read, in fixed chunks. Holding the whole file to hash it
    // put a file of up to `MAX_HASHED_FILE_BYTES` in memory at once, for a
    // digest that is identical either way.
    let mut hasher = blake3::Hasher::new();
    let mut chunk = vec![0u8; 256 * 1024];
    let mut size: u64 = 0;
    loop {
        use std::io::Read;
        let read = match file.read(&mut chunk) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(WriterError::StageWrite(path.to_owned(), error)),
        };
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        size += read as u64;
        // A file that has grown past the bound its base hash was captured under
        // is treated the same as any other stale-base-hash mismatch, never
        // silently hashed over a truncated read (a truncated read's hash would
        // not actually re-prove anything about the file's real current content).
        if size > super::read::MAX_HASHED_FILE_BYTES {
            return Err(WriterError::StaleBaseHash(path.to_owned()));
        }
    }
    let current_hash = hasher.finalize();
    match plan.base_hashes().get(path) {
        Some(expected) if *expected == current_hash => Ok(()),
        _ => Err(WriterError::StaleBaseHash(path.to_owned())),
    }
}

fn require_current_base_hash_match(
    trusted_root: &TrustedRoot,
    plan: &MutationPlan,
    path: &RelativeProjectPath,
) -> Result<(), WriterError> {
    let resolved = match trusted_root.resolve_existing(path.clone()) {
        Ok(resolved) => resolved,
        Err(ContainmentError::NotFound) => {
            return Err(WriterError::StaleBaseHash(path.as_str().to_owned()))
        }
        Err(error) => return Err(error.into()),
    };
    require_base_hash_match(trusted_root, plan, path.as_str(), &resolved)
}

fn require_current_directory_identity(
    trusted_root: &TrustedRoot,
    path: &RelativeProjectPath,
    expected: RootIdentity,
) -> Result<(), WriterError> {
    let resolved = match trusted_root.resolve_existing(path.clone()) {
        Ok(resolved) => resolved,
        Err(ContainmentError::NotFound) => {
            return Err(WriterError::DirectoryEntryChanged(path.as_str().to_owned()))
        }
        Err(error) => return Err(error.into()),
    };
    let metadata = trusted_root.file_metadata(&resolved)?;
    if !metadata.is_dir() {
        return Err(WriterError::NotADirectory(path.as_str().to_owned()));
    }
    if trusted_root.directory_identity(&resolved)? != expected {
        return Err(WriterError::DirectoryEntryChanged(path.as_str().to_owned()));
    }
    Ok(())
}

/// Writes `content` into a fresh same-directory temp sibling of `target`,
/// flushing it to disk before returning. On any failure, the temp file (if
/// created at all) is removed before the error propagates -- no orphaned
/// temp file survives a staging failure.
fn stage_content(
    target: &WriteTarget,
    content: &[u8],
    path: &str,
) -> Result<OsString, WriterError> {
    let (mut file, temp_name) = target
        .create_temp_sibling()
        .map_err(WriterError::Containment)?;
    // Read before writing: the target is about to be replaced, and after the
    // rename there is no original left to ask.
    let inherited = target.existing_permissions();
    let result: io::Result<()> = (|| {
        file.write_all(content)?;
        // After the content, because a mode without write permission would
        // reject the write itself; before the sync and the rename, so a
        // failure here leaves the original file untouched.
        if let Some(permissions) = &inherited {
            super::platform::apply_replacement_permissions(&file, permissions)?;
        }
        #[cfg(test)]
        if test_hooks::force_stage_sync_failure() {
            return Err(io::Error::other(
                "forced stage-content sync failure for test",
            ));
        }
        file.sync_all()
    })();
    let result = result.map_err(|error| WriterError::StageWrite(path.to_owned(), error));
    match result {
        Ok(()) => Ok(temp_name),
        Err(error) => {
            target.remove_temp_sibling(&temp_name);
            Err(error)
        }
    }
}

/// The result of applying one already-staged operation: either it fully
/// landed (mutation plus its post-hoc durability confirmation both
/// succeeded), or the mutation itself landed but the durability step could
/// not be confirmed (see the module doc comment's "Durability" section).
/// [`WriterError`] (a plain `Err`) is reserved for the mutation itself
/// failing to land at all.
enum ApplyOutcome {
    Applied,
    AppliedDurabilityUnconfirmed(WriterError),
}

/// Applies one staged operation. A parent-directory flush failure
/// ([`super::domain::WriteTarget::fsync_parent`]) after an already-landed
/// rename/unlink is reported via [`ApplyOutcome::AppliedDurabilityUnconfirmed`]
/// rather than a plain `Err`, so [`execute`] can count the mutation as
/// completed while still surfacing that its durability could not be
/// confirmed -- never silently claiming success, and never claiming the
/// mutation did not happen when it did (see the module doc comment).
fn apply(
    trusted_root: &TrustedRoot,
    plan: &MutationPlan,
    stage: &Staged,
) -> Result<ApplyOutcome, WriterError> {
    match stage {
        Staged::Replace {
            path,
            target,
            temp_name,
            kind,
            // Only `cleanup` acts on the created directories; an applying
            // operation keeps them.
            created_dirs: _,
        } => {
            match kind {
                ReplaceKind::Create if target.exists() => {
                    target.remove_temp_sibling(temp_name);
                    return Err(WriterError::CreateTargetNowExists(path.as_str().to_owned()));
                }
                ReplaceKind::Modify => {
                    require_current_base_hash_match(trusted_root, plan, path)?;
                }
                ReplaceKind::Create => {}
            }
            if let Err(error) = target.atomic_replace(temp_name) {
                target.remove_temp_sibling(temp_name);
                return Err(classify_replace_error(path.as_str(), error));
            }
            match fsync_parent_confirmed(target) {
                Ok(()) => Ok(ApplyOutcome::Applied),
                Err(error) => Ok(ApplyOutcome::AppliedDurabilityUnconfirmed(
                    classify_replace_error(path.as_str(), error),
                )),
            }
        }
        Staged::DeleteDirectory {
            path,
            target,
            identity,
        } => {
            require_current_directory_identity(trusted_root, path, *identity)?;
            // A non-empty directory fails here, atomically, which is what makes
            // this operation safe without a tree walk.
            target.remove_directory().map_err(|error| match error {
                ContainmentError::Io(io) => {
                    WriterError::DirectoryNotEmpty(path.as_str().to_owned(), io)
                }
                other => classify_replace_error(path.as_str(), other),
            })?;
            match fsync_parent_confirmed(target) {
                Ok(()) => Ok(ApplyOutcome::Applied),
                Err(error) => Ok(ApplyOutcome::AppliedDurabilityUnconfirmed(
                    classify_replace_error(path.as_str(), error),
                )),
            }
        }
        Staged::Delete { path, target } => {
            require_current_base_hash_match(trusted_root, plan, path)?;
            target
                .remove()
                .map_err(|error| classify_replace_error(path.as_str(), error))?;
            match fsync_parent_confirmed(target) {
                Ok(()) => Ok(ApplyOutcome::Applied),
                Err(error) => Ok(ApplyOutcome::AppliedDurabilityUnconfirmed(
                    classify_replace_error(path.as_str(), error),
                )),
            }
        }
        Staged::Rename {
            from_path,
            to_path,
            from_target,
            to_target,
            source_kind,
            destination_existed,
            created_dirs: _,
        } => {
            match source_kind {
                RenameSourceKind::File => {
                    require_current_base_hash_match(trusted_root, plan, from_path)?;
                }
                RenameSourceKind::Directory(identity) => {
                    require_current_directory_identity(trusted_root, from_path, *identity)?;
                }
            }
            if *destination_existed {
                require_current_base_hash_match(trusted_root, plan, to_path)?;
            } else if to_target.exists() {
                return Err(match source_kind {
                    RenameSourceKind::File => {
                        WriterError::RenameTargetNowExists(to_path.as_str().to_owned())
                    }
                    RenameSourceKind::Directory(_) => {
                        WriterError::RenameDirectoryDestinationExists(to_path.as_str().to_owned())
                    }
                });
            }
            let label = format!("{} -> {}", from_path.as_str(), to_path.as_str());
            from_target
                .atomic_rename_onto(to_target)
                .map_err(|error| classify_replace_error(&label, error))?;
            if let Err(error) = fsync_parent_confirmed(from_target) {
                return Ok(ApplyOutcome::AppliedDurabilityUnconfirmed(
                    classify_replace_error(&label, error),
                ));
            }
            match fsync_parent_confirmed(to_target) {
                Ok(()) => Ok(ApplyOutcome::Applied),
                Err(error) => Ok(ApplyOutcome::AppliedDurabilityUnconfirmed(
                    classify_replace_error(&label, error),
                )),
            }
        }
    }
}

/// `target.fsync_parent()`, with a `#[cfg(test)]`-only fault-injection seam
/// (see the `test_hooks` module below) so this codebase's "a
/// parent-directory flush failure is not swallowed, and a completed
/// mutation is never miscounted as incomplete" guarantee is proven by a
/// real, deterministic test rather than only argued -- production builds
/// call straight through to [`WriteTarget::fsync_parent`] with zero
/// behavioral change.
fn fsync_parent_confirmed(target: &WriteTarget) -> Result<(), ContainmentError> {
    #[cfg(test)]
    if test_hooks::force_fsync_parent_failure() {
        return Err(ContainmentError::Io(io::Error::other(
            "forced fsync_parent failure for test",
        )));
    }
    target.fsync_parent()
}

fn classify_replace_error(path: &str, error: ContainmentError) -> WriterError {
    if let ContainmentError::Io(io_error) = &error {
        if is_cross_volume_error(io_error) {
            return WriterError::CrossVolume(path.to_owned());
        }
    }
    match error {
        ContainmentError::Io(io_error) => WriterError::Replace(path.to_owned(), io_error),
        other => WriterError::Containment(other),
    }
}

/// Whether `error` represents the OS reporting "these two paths are on
/// different filesystems/volumes" -- Unix `EXDEV` (18), Windows
/// `ERROR_NOT_SAME_DEVICE` (17). Checked against both the modern, stable
/// `std::io::ErrorKind::CrossesDevices` classification and the raw OS error
/// code directly (belt-and-suspenders: the raw-code check is
/// platform-specific defense in depth in case `ErrorKind` classification
/// behavior ever differs from expectation on a given target, not a
/// substitute for it).
fn is_cross_volume_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::CrossesDevices {
        return true;
    }
    match error.raw_os_error() {
        Some(18) if cfg!(unix) => true,
        Some(17) if cfg!(windows) => true,
        _ => false,
    }
}

/// Best-effort cleanup of every not-yet-applied `Replace` stage's temp
/// sibling. `Delete`/`Rename` stages created no temp file, so there is
/// nothing to clean up for them.
/// Removes every staged temp file and reports the directories this batch
/// created, deepest-first.
///
/// The directories are *reported* rather than removed here because a staged
/// [`WriteTarget`] holds an open handle to its own parent directory, and
/// Windows refuses to remove a directory while any handle to it is open. The
/// caller therefore drops `staged` before passing this list to
/// [`remove_created_dirs`].
#[must_use]
fn cleanup(staged: &[Staged]) -> Vec<PathBuf> {
    let mut created: Vec<PathBuf> = Vec::new();
    for stage in staged {
        if let Staged::Replace {
            target, temp_name, ..
        } = stage
        {
            target.remove_temp_sibling(temp_name);
        }
    }
    // Reverse creation order across the whole batch puts children before
    // parents, so a parent is only attempted once its child is gone.
    for stage in staged.iter().rev() {
        match stage {
            Staged::Replace { created_dirs, .. } | Staged::Rename { created_dirs, .. } => {
                created.extend(created_dirs.iter().rev().cloned());
            }
            Staged::Delete { .. } | Staged::DeleteDirectory { .. } => {}
        }
    }
    created
}

/// Removes `created_dirs` in the order given, best effort: a directory another
/// process has since filled is left alone (see
/// `TrustedRoot::remove_created_dir`). Callers pass children before parents.
fn remove_created_dirs(trusted_root: &TrustedRoot, created_dirs: &[PathBuf]) {
    for relative in created_dirs {
        trusted_root.remove_created_dir(relative);
    }
}

/// Test-only fault injection, entirely absent from production builds
/// (`#[cfg(test)]`). This codebase's stated preference is real,
/// reproducible OS-level failures over a generic fault-injection trait
/// (see `docs/decisions.md`'s
/// "Rejected patterns" section, which rejected a broad `WritableFile`
/// abstraction covering the entire write path); these three narrow,
/// single-purpose hooks are not that -- each simulates exactly one specific
/// failure/race this sandbox has no deterministic real-OS way to reproduce
/// (a post-mutation parent-directory fsync failure, a staged-content fsync
/// failure, and a file planted in the narrow window between this
/// operation's own staging check and its apply), and none of them changes
/// production behavior in any way.
#[cfg(test)]
pub(crate) mod test_hooks {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static FORCE_STAGE_SYNC_FAILURE: Cell<bool> = const { Cell::new(false) };
        static FORCE_FSYNC_PARENT_FAILURE: Cell<bool> = const { Cell::new(false) };
        static AFTER_STAGING_BEFORE_APPLY_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
            const { RefCell::new(None) };
    }

    pub(crate) fn force_stage_sync_failure() -> bool {
        FORCE_STAGE_SYNC_FAILURE.with(Cell::get)
    }

    pub(crate) fn force_fsync_parent_failure() -> bool {
        FORCE_FSYNC_PARENT_FAILURE.with(Cell::get)
    }

    pub(crate) fn run_after_staging_before_apply_hook() {
        let hook = AFTER_STAGING_BEFORE_APPLY_HOOK.with(|cell| cell.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }

    /// While alive, forces [`super::stage_content`]'s content-flush
    /// (`sync_all`) call to fail -- proving a staged-content flush failure
    /// is cleaned up (no orphaned temp file) and rejects the whole plan
    /// (Task 6 plan Step 1's "file ... flush failures" test category).
    /// Resets itself on drop, so a leaked guard can never affect a later
    /// test even if the test thread is reused.
    pub(crate) struct ForceStageContentSyncFailure(());

    impl ForceStageContentSyncFailure {
        #[must_use]
        pub(crate) fn enable() -> Self {
            FORCE_STAGE_SYNC_FAILURE.with(|cell| cell.set(true));
            Self(())
        }
    }

    impl Drop for ForceStageContentSyncFailure {
        fn drop(&mut self) {
            FORCE_STAGE_SYNC_FAILURE.with(|cell| cell.set(false));
        }
    }

    /// While alive, forces [`super::fsync_parent_confirmed`] to fail --
    /// proving a parent-directory flush failure after an already-successful
    /// rename/unlink/create is counted as completed and flagged
    /// `durability_unconfirmed`, never miscounted as "0 of N operations
    /// completed" (Task 6 review finding on `writer.rs`'s `apply`). Resets
    /// itself on drop, so a leaked guard can never affect a later test even
    /// if the test thread is reused.
    pub(crate) struct ForceFsyncParentFailure(());

    impl ForceFsyncParentFailure {
        #[must_use]
        pub(crate) fn enable() -> Self {
            FORCE_FSYNC_PARENT_FAILURE.with(|cell| cell.set(true));
            Self(())
        }
    }

    impl Drop for ForceFsyncParentFailure {
        fn drop(&mut self) {
            FORCE_FSYNC_PARENT_FAILURE.with(|cell| cell.set(false));
        }
    }

    /// Registers a one-shot hook that [`super::execute`] runs immediately
    /// after its staging loop completes and before its apply loop begins --
    /// the exact point in a single `execute` call where a `Create`
    /// operation's staging-time existence check has already passed but its
    /// own apply-time swap has not yet run, proving the apply-time re-check
    /// (not the staging-time one) is what rejects a file planted in that
    /// window (Task 6 review finding on `writer.rs`'s `execute`/`apply`).
    pub(crate) fn set_after_staging_before_apply_hook(hook: impl FnOnce() + 'static) {
        AFTER_STAGING_BEFORE_APPLY_HOOK.with(|cell| *cell.borrow_mut() = Some(Box::new(hook)));
    }
}
