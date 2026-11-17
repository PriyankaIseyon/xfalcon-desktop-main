//! The second-confirmation approval policy over an already-valid,
//! already-proposed [`MutationPlan`].
//!
//! # Security contract
//!
//! Every mutation requires an approval decision -- there is no
//! path in this crate that reaches [`crate::projects::execute`] other than
//! [`super::service::ApprovalsService::resolve`], and that function only
//! ever obtains a [`crate::projects::ApprovalProof`] from
//! [`require_sufficient_confirmation`] below, which is called only after a
//! caller has explicitly presented an [`ApprovalDecision`] with
//! `approve: true` (`service.rs`; a `deny` decision never reaches this
//! module at all). Delete, rename-onto-an-existing-path, binary content,
//! more than ten distinct paths, or more than 1 MiB total in one plan
//! ([`crate::projects::MutationRiskTier::SecondConfirmationRequired`],
//! computed once and immutably by [`crate::projects::MutationPlan::propose`]
//! from the plan's own operations) additionally require
//! [`ApprovalDecision::second_confirmation`] to be `true` -- checked here.
//!
//! [`require_sufficient_confirmation`] is this crate's sole production
//! constructor of [`crate::projects::ApprovalProof`]. It, and
//! [`ApprovalDecision::approve`]/[`ApprovalDecision::deny`], are
//! `pub(in crate::approvals)` -- compiler-enforced, not merely conventional:
//! no module outside this one's own subtree (`service.rs`, `commands.rs`,
//! `policy.rs`, `tests.rs`) can construct an [`ApprovalDecision`] or call
//! this function at all, so nothing elsewhere in the crate (e.g. a future
//! `crate::agent`/`crate::tools` caller) can mint a decision or a proof
//! through this route. The one piece of this chain that necessarily remains
//! reachable crate-wide is [`crate::projects::writer::ApprovalProof::new`]
//! itself (`pub(crate)`, not `pub(in crate::approvals)`, because
//! `crate::approvals` is a sibling of, not an ancestor of,
//! `crate::projects::writer` -- see that function's own doc comment) and
//! [`crate::projects::execute`] (also `pub(crate)` for the same reason,
//! never externally `pub`); both are that doc comment's own disclosed,
//! structurally-unavoidable residual. A caller never supplies
//! `second_confirmation_given` freely: the proof this function returns is
//! only ever produced after its own sufficiency check has already passed,
//! carrying the decision's real value through rather than an arbitrary one.
//!
//! ## This module cannot authorize a containment failure at any confirmation
//! ## level
//!
//! [`require_sufficient_confirmation`] takes an already-constructed
//! [`MutationPlan`] and only ever reads its `is_expired`/
//! `requires_second_confirmation` methods -- it has no path/root/link
//! information to decide over at all, because a plan whose paths failed
//! containment was never constructed in the first place
//! ([`crate::projects::MutationPlan::propose`] itself resolves every
//! operation through [`crate::projects::TrustedRoot`]). There is
//! structurally no confirmation-level input this function could ever use to
//! "approve past" a containment failure, because containment is not a
//! question this function is ever asked. The actual, independent,
//! unconditional re-check of containment happens a second time, at execute
//! time, in [`crate::projects::execute`] itself -- see that function's
//! module doc comment for the full argument, including why it is
//! structurally impossible to bypass at any confirmation level there either.

use crate::projects::{ApprovalProof, MutationPlan};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ApprovalPolicyError {
    #[error("this plan has expired and can no longer be approved")]
    Expired,
    #[error("this plan requires a second confirmation beyond the base approval")]
    SecondConfirmationRequired,
}

/// A caller's explicit decision for one pending plan. Only ever constructed
/// via [`Self::approve`]/[`Self::deny`] -- there is no `Default` and no
/// field is `pub` outside this crate's own reach, so a decision is always
/// unambiguous about which of the two it represents. The type itself stays
/// `pub` (it appears in [`super::service::ApprovalsService::resolve`]'s
/// signature, which must remain externally reachable), but both
/// constructors are `pub(in crate::approvals)`: nothing outside this
/// module's own subtree can ever produce a value of this type, which is
/// what actually keeps `resolve` un-callable from anywhere else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalDecision {
    approve: bool,
    second_confirmation: bool,
}

impl ApprovalDecision {
    #[must_use]
    pub(in crate::approvals) fn approve(second_confirmation: bool) -> Self {
        Self {
            approve: true,
            second_confirmation,
        }
    }

    /// No production caller since agent patches stopped waiting on a person;
    /// the store's denial path stays covered by its tests.
    #[cfg(test)]
    #[must_use]
    pub(in crate::approvals) fn deny() -> Self {
        Self {
            approve: false,
            second_confirmation: false,
        }
    }

    #[must_use]
    pub fn is_approve(&self) -> bool {
        self.approve
    }

    #[must_use]
    pub fn second_confirmation_given(&self) -> bool {
        self.second_confirmation
    }
}

/// Checks `decision` against `plan`'s own risk classification and expiry,
/// and -- only once that check passes -- constructs and returns the
/// [`ApprovalProof`] that authorizes [`crate::projects::execute`] to run
/// (see the module doc comment's security contract). Callers MUST only
/// invoke this for an `approve` decision (`service.rs` never calls this for
/// a `deny` decision, which always succeeds trivially and never reaches
/// [`crate::projects::execute`] at all). `pub(in crate::approvals)`: this is
/// this crate's sole production route to minting an [`ApprovalProof`], and
/// no module outside this one's own subtree may call it.
pub(in crate::approvals) fn require_sufficient_confirmation(
    plan: &MutationPlan,
    decision: &ApprovalDecision,
    now_ms: i64,
) -> Result<ApprovalProof, ApprovalPolicyError> {
    if plan.is_expired(now_ms) {
        return Err(ApprovalPolicyError::Expired);
    }
    if plan.requires_second_confirmation() && !decision.second_confirmation_given() {
        return Err(ApprovalPolicyError::SecondConfirmationRequired);
    }
    Ok(ApprovalProof::new(
        plan.plan_id(),
        decision.second_confirmation_given(),
    ))
}
