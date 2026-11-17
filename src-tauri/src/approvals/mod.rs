//! The one decision path every trusted-project file mutation takes.
//!
//! # Security contract
//!
//! Every mutation is planned, decided, journaled, audited, and backed up here;
//! no path bypasses this (`docs/decisions.md`, Task 6). No mutation waits on a
//! person any more: trusting the folder is the consent, and the revision
//! history is how a change is undone.
//! [`service::ApprovalsService::resolve`] is the exclusive production route
//! to [`crate::projects::execute`] (see that function's own module doc
//! comment for the full non-confirmable-containment argument, and
//! `service.rs`/`policy.rs`'s own doc comments for how the second-
//! confirmation policy layered on top of it composes with, but never
//! substitutes for, that unconditional re-check). This is compiler-enforced,
//! not merely conventional: [`policy::require_sufficient_confirmation`] and
//! `policy::ApprovalDecision::approve`/`deny` -- the only way to mint an
//! [`crate::projects::ApprovalProof`] or construct a decision at all -- are
//! `pub(in crate::approvals)` and are deliberately not re-exported below, so
//! no module outside this one's own subtree can reach them.
//!
//! - [`patch`]/[`writer`] (in `crate::projects`) own the immutable plan
//!   shape and the atomic filesystem mutation itself.
//! - [`policy`] owns the second-confirmation decision over an
//!   already-proposed, already-valid plan.
//! - [`service`] owns encrypted local persistence of pending/resolved
//!   approval requests and is the Tauri-facing runtime.
//! - [`commands`] is the (currently unwired -- see its own doc comment)
//!   Tauri command surface over [`service::ApprovalsService`].

mod commands;
mod policy;
mod service;

#[cfg(test)]
mod tests;

pub use commands::{
    project_file_restore, project_file_revision_diff, project_file_revisions, project_save_file,
};
pub use policy::ApprovalPolicyError;
pub use service::{ApprovalOutcome, ApprovalStore, ApprovalsService, ApprovalsServiceError};
