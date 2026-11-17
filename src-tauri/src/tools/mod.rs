//! Policy-first unified tool domain.
//!
//! [`registry`], [`policy`], and [`invocation`] are the pure vocabulary — a
//! tool registry, authorization decisions, and invocation journaling — with
//! no MCP client, filesystem access, skill loading, or Tauri command in them
//! (see `docs/decisions.md` Task 1 and
//! `docs/decisions.md`).
//!
//! [`dispatch`] is the one module here that does reach outward: it is the
//! live executor that turns an authorized call into real work against
//! `crate::projects`/`crate::approvals`. It sits behind
//! `crate::agent::tool_loop::ToolExecutorPort`, so the pure layers above
//! still never depend on it.

pub mod budget;
pub mod dispatch;
pub(crate) mod exact_result;
pub mod invocation;
pub mod policy;
pub mod registry;
pub mod standing;

pub use budget::ToolResultBudget;
pub use invocation::{
    ContentRef, InMemoryInvocationJournal, InvocationError, InvocationJournal,
    InvocationJournalError, InvocationTransitionOutcome, JournaledInvocationEntry, ToolInvocation,
    ToolResult, ToolResultError, ToolResultPreview, ToolResultPreviewStrategy,
    MAX_EXACT_TOOL_RESULT_BYTES,
};
pub use policy::{
    Actor, ActorError, ApprovalRequest, ApprovalToken, ExecutionGrant, PathAuthority,
    PolicyDecision, PolicyDenial, PolicyEngine, PolicyRequest, ProjectContext, SkillToolScope,
};
pub use registry::{
    ToolCapability, ToolCapabilityError, ToolDescriptor, ToolDescriptorError, ToolRegistry,
    ToolRegistryError, CAPABILITY_REQUIRES_PROJECT, CAPABILITY_REQUIRES_TRUSTED_PATH,
};

#[cfg(test)]
mod tests;
