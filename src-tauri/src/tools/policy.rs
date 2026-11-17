//! Policy-first authorization: every tool invocation is decided by
//! [`PolicyEngine::authorize`], never by the model's own text or arguments.
//!
//! The evaluation order below is deliberate and load-bearing: cheaper,
//! structural checks (principal, allowlist, schema, budgets) run before
//! deeper, contextual ones (project, path, skill scope, approval), and a
//! skill or project can only ever narrow what the allowlist already permits
//! — never expand it. See
//! `docs/decisions.md`
//! for the reference-implementation comparison behind this shape.

use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use xfalcon_contracts::tools::{InvocationId, ToolId, ToolRisk};

use super::registry::ToolRegistry;

const MAX_ACTOR_SUBJECT_BYTES: usize = 256;

/// The authenticated identity a tool invocation runs as. Production wiring
/// (a later task) constructs this from the real session identity; Task 1
/// only needs a non-empty, bounded subject so "no authenticated principal"
/// is representable and deniable without importing the auth module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Actor(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ActorError {
    #[error("actor subject must not be empty")]
    Empty,
    #[error("actor subject exceeds {0} bytes")]
    TooLong(usize),
}

impl Actor {
    pub fn new(subject: impl Into<String>) -> Result<Self, ActorError> {
        let subject = subject.into();
        if subject.is_empty() {
            return Err(ActorError::Empty);
        }
        if subject.len() > MAX_ACTOR_SUBJECT_BYTES {
            return Err(ActorError::TooLong(MAX_ACTOR_SUBJECT_BYTES));
        }
        Ok(Self(subject))
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.0
    }
}

/// Opaque marker for "a trusted project is active". Task 4 replaces this
/// with a real trusted-root/capability handle; Task 1 only needs to know
/// whether one is present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectContext {
    project_id: String,
}

impl ProjectContext {
    pub fn new(project_id: impl Into<String>) -> Self {
        Self {
            project_id: project_id.into(),
        }
    }

    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }
}

/// Trait-based extension point for "is this path inside a trusted root".
/// Task 4's cap-std/`TrustedRoot` handle implements this; Task 1's tests use
/// a minimal fake so the policy shape can be proven without filesystem code.
///
/// # Security contract
///
/// A conforming implementation MUST canonicalize both the trusted root and
/// the candidate path — resolving `.`/`..` segments, symlinks, Windows
/// junctions/reparse points, and mount/bind-mount boundaries — before
/// comparing them, and MUST revalidate that identity immediately before use
/// rather than trusting a check performed earlier. A purely lexical/textual
/// containment check (e.g. `Path::starts_with` on unresolved paths) is NOT
/// sufficient and MUST NOT be used in production: it accepts traversal
/// payloads such as `root/../../etc/passwd` and symlinks planted inside the
/// root that resolve outside it. This mirrors this repository's threat
/// model (`docs/threat-model.md`, TM-007: canonical and stable
/// root identity, link/reparse/mount escape rejection, and immediate
/// pre-use identity revalidation).
/// `Send + Sync` because an authority is consulted from inside a run's async
/// task, which is spawned onto the multi-threaded runtime and holds
/// `&dyn PathAuthority` across await points
/// (`crate::agent::engine::run_with_tool_loop`). A non-`Sync` implementation
/// could not be shared there at all.
pub trait PathAuthority: Send + Sync {
    fn contains(&self, path: &Path) -> bool;
}

/// The set of tools an active skill permits. A skill can only ever narrow
/// the global allowlist, never expand it — [`PolicyEngine`] checks the
/// allowlist before it ever consults this scope, and this scope has no
/// influence over the path-trust decision at all, so a skill cannot grant
/// itself a tool or a path absent from global/project policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillToolScope {
    allowed_tools: HashSet<ToolId>,
}

impl SkillToolScope {
    #[must_use]
    pub fn new(allowed_tools: impl IntoIterator<Item = ToolId>) -> Self {
        Self {
            allowed_tools: allowed_tools.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn allows(&self, tool_id: &ToolId) -> bool {
        self.allowed_tools.contains(tool_id)
    }
}

/// A token proving a specific mutating invocation was approved. Only ever
/// constructed by [`ApprovalRequest::approve`] (the stand-in for the real,
/// later approval-UI flow), and only matches the exact invocation/tool pair
/// it was issued for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalToken {
    invocation_id: InvocationId,
    tool_id: ToolId,
}

impl ApprovalToken {
    #[must_use]
    pub fn matches(&self, invocation_id: &InvocationId, tool_id: &ToolId) -> bool {
        &self.invocation_id == invocation_id && &self.tool_id == tool_id
    }
}

/// Returned by [`PolicyDecision::RequireApproval`] when a mutating tool has
/// no approval token yet. The (future) approval UI calls [`Self::approve`]
/// once a human authorizes it, producing a token scoped to this exact
/// invocation and tool — it cannot be replayed against a different one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRequest {
    invocation_id: InvocationId,
    tool_id: ToolId,
}

impl ApprovalRequest {
    fn new(invocation_id: InvocationId, tool_id: ToolId) -> Self {
        Self {
            invocation_id,
            tool_id,
        }
    }

    #[must_use]
    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    #[must_use]
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    #[must_use]
    pub fn approve(&self) -> ApprovalToken {
        ApprovalToken {
            invocation_id: self.invocation_id.clone(),
            tool_id: self.tool_id.clone(),
        }
    }
}

/// Proof that every policy condition passed. Only [`PolicyEngine::authorize`]
/// can construct one.
///
/// # Single-use contract
///
/// A grant is valid for exactly the invocation attempt that produced it.
/// Callers MUST call [`PolicyEngine::authorize`] fresh for every execution
/// attempt, including retries, and MUST NOT cache, persist, or replay a
/// grant across attempts. This repository's threat model requires that Rust
/// "validates and authorizes every privileged operation at time of use"
/// (`docs/threat-model.md`, Security Requirement 2) — a cached
/// grant would silently skip that re-check (for example, a budget that was
/// exhausted, or a project/path/skill scope that changed, between the
/// original grant and a later replay). `Clone` exists so one decision can be
/// threaded through a single execution's call stack, not so it can be
/// persisted or reused across separate authorization attempts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionGrant {
    invocation_id: InvocationId,
    tool_id: ToolId,
    approval: Option<ApprovalToken>,
}

impl ExecutionGrant {
    #[must_use]
    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    #[must_use]
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    #[must_use]
    pub fn approval(&self) -> Option<&ApprovalToken> {
        self.approval.as_ref()
    }
}

/// Every reason [`PolicyEngine::authorize`] can deny an invocation. One
/// variant per authorization condition; each is a policy fact, never
/// model-authored text (see the reference evidence's "error taxonomy split
/// by audience" note).
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PolicyDenial {
    #[error("no authenticated principal for this invocation")]
    PrincipalNotAuthenticated,
    #[error("tool {0:?} is not allowlisted")]
    ToolNotAllowlisted(ToolId),
    #[error("tool {0:?} has no registered descriptor")]
    ToolNotRegistered(ToolId),
    #[error("tool {0:?} advertises an invalid input schema")]
    InvalidAdvertisedSchema(ToolId),
    #[error("run budget exhausted for this invocation")]
    RunBudgetExhausted,
    #[error("tool budget exhausted for this invocation")]
    ToolBudgetExhausted,
    #[error("tool {0:?} requires an active trusted project")]
    ProjectRequired(ToolId),
    #[error("tool {0:?} requires a path but none was provided")]
    PathRequired(ToolId),
    #[error("requested path is outside the trusted project root")]
    PathNotTrusted,
    #[error("tool {0:?} is outside the active skill's restricted scope")]
    OutsideSkillScope(ToolId),
    #[error("approval token does not match this invocation")]
    ApprovalTokenMismatch,
}

impl PolicyDenial {
    /// Stable machine code for journaling/wire use (see
    /// `ToolInvocationEvent::Denied.reason_code`); never the `Display` text,
    /// which may embed a debug-formatted [`ToolId`].
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PrincipalNotAuthenticated => "principal_not_authenticated",
            Self::ToolNotAllowlisted(_) => "tool_not_allowlisted",
            Self::ToolNotRegistered(_) => "tool_not_registered",
            Self::InvalidAdvertisedSchema(_) => "invalid_advertised_schema",
            Self::RunBudgetExhausted => "run_budget_exhausted",
            Self::ToolBudgetExhausted => "tool_budget_exhausted",
            Self::ProjectRequired(_) => "project_required",
            Self::PathRequired(_) => "path_required",
            Self::PathNotTrusted => "path_not_trusted",
            Self::OutsideSkillScope(_) => "outside_skill_scope",
            Self::ApprovalTokenMismatch => "approval_token_mismatch",
        }
    }
}

/// Outcome of [`PolicyEngine::authorize`]. There is no fourth option: every
/// invocation is exactly allowed, pending approval, or denied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyDecision {
    Allow(ExecutionGrant),
    RequireApproval(ApprovalRequest),
    Deny(PolicyDenial),
}

/// Everything [`PolicyEngine::authorize`] needs to decide one invocation.
/// `arguments` is carried only so adversarial tests can prove its content
/// never influences the decision — the engine reads `tool_id` and
/// `approval`, never fields inside `arguments`.
pub struct PolicyRequest<'a> {
    pub actor: Option<&'a Actor>,
    pub invocation_id: &'a InvocationId,
    pub tool_id: &'a ToolId,
    pub arguments: &'a Value,
    pub run_budget_remaining: u32,
    pub tool_budget_remaining: u32,
    pub active_project: Option<&'a ProjectContext>,
    pub requested_path: Option<&'a Path>,
    pub path_authority: Option<&'a dyn PathAuthority>,
    pub skill_scope: Option<&'a SkillToolScope>,
    pub approval: Option<&'a ApprovalToken>,
}

/// Owns the global allowlist and the tool registry it is checked against.
/// Neither is mutated by `authorize`; both are supplied by the (future)
/// caller that owns global policy configuration.
pub struct PolicyEngine<'a> {
    registry: &'a ToolRegistry,
    allowlist: &'a HashSet<ToolId>,
}

impl<'a> PolicyEngine<'a> {
    #[must_use]
    pub fn new(registry: &'a ToolRegistry, allowlist: &'a HashSet<ToolId>) -> Self {
        Self {
            registry,
            allowlist,
        }
    }

    #[must_use]
    pub fn authorize(&self, request: &PolicyRequest<'_>) -> PolicyDecision {
        // `request.arguments` is intentionally never read below: see the
        // `PolicyRequest` docs and the adversarial tests in
        // `tests/policy.rs` proving arguments content cannot influence this
        // decision.
        if request.actor.is_none() {
            return PolicyDecision::Deny(PolicyDenial::PrincipalNotAuthenticated);
        }

        if !self.allowlist.contains(request.tool_id) {
            return PolicyDecision::Deny(PolicyDenial::ToolNotAllowlisted(request.tool_id.clone()));
        }

        let Some(descriptor) = self.registry.get(request.tool_id) else {
            return PolicyDecision::Deny(PolicyDenial::ToolNotRegistered(request.tool_id.clone()));
        };

        if !descriptor.has_valid_schema() {
            return PolicyDecision::Deny(PolicyDenial::InvalidAdvertisedSchema(
                request.tool_id.clone(),
            ));
        }

        if request.run_budget_remaining == 0 {
            return PolicyDecision::Deny(PolicyDenial::RunBudgetExhausted);
        }
        if request.tool_budget_remaining == 0 {
            return PolicyDecision::Deny(PolicyDenial::ToolBudgetExhausted);
        }

        if descriptor.requires_active_project() && request.active_project.is_none() {
            return PolicyDecision::Deny(PolicyDenial::ProjectRequired(request.tool_id.clone()));
        }

        if descriptor.requires_trusted_path() {
            match (request.requested_path, request.path_authority) {
                (Some(path), Some(authority)) => {
                    if !authority.contains(path) {
                        return PolicyDecision::Deny(PolicyDenial::PathNotTrusted);
                    }
                }
                _ => {
                    return PolicyDecision::Deny(PolicyDenial::PathRequired(
                        request.tool_id.clone(),
                    ));
                }
            }
        }

        if let Some(scope) = request.skill_scope {
            if !scope.allows(request.tool_id) {
                return PolicyDecision::Deny(PolicyDenial::OutsideSkillScope(
                    request.tool_id.clone(),
                ));
            }
        }

        let approval = if matches!(descriptor.risk(), ToolRisk::Mutating) {
            match request.approval {
                None => {
                    return PolicyDecision::RequireApproval(ApprovalRequest::new(
                        request.invocation_id.clone(),
                        request.tool_id.clone(),
                    ));
                }
                Some(token) => {
                    if !token.matches(request.invocation_id, request.tool_id) {
                        return PolicyDecision::Deny(PolicyDenial::ApprovalTokenMismatch);
                    }
                    Some(token.clone())
                }
            }
        } else {
            None
        };

        PolicyDecision::Allow(ExecutionGrant {
            invocation_id: request.invocation_id.clone(),
            tool_id: request.tool_id.clone(),
            approval,
        })
    }
}
