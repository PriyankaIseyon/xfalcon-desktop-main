//! Bounded, policy-gated tool-calling loop for an in-flight agent run.
//!
//! This is Task 7 of `docs/decisions.md`:
//! the first *live caller* of Task 1's tool-call authorization port
//! (`crate::tools::policy::PolicyEngine::authorize`) and invocation
//! lifecycle (`crate::tools::invocation::ToolInvocation`). Full design
//! rationale, numeric-bound justifications, and reference-implementation
//! comparisons live in
//! `docs/decisions.md`;
//! this comment summarizes the load-bearing decisions.
//!
//! # What this replaces
//!
//! Two call sites used to treat *any* tool-call request as an unconditional
//! hard failure: `crate::agent::local_executor::LocalRunExecutor::run`
//! (`tool_requested => Err(ToolsUnavailable)`) and
//! `crate::agent::engine::RunEngine::poll_da_run` (`tool_requested =>
//! RunState::Failed`). [`ToolLoop`] is the real orchestrator that replaces
//! that dead end — for Path A only. See `crate::agent::executor`'s module
//! doc comment for the full Path A/B scoping evidence (a generic DA
//! tool-call update route exists in the pinned contract but is unused by
//! this crate's client wrapper, and whether posting to it resumes a
//! DA-driven run server-side is unverified — not the structural "no such
//! route exists" an earlier draft of that comment claimed; every shipped
//! provider adapter's `ToolCallRequested` marker also carries no structured
//! call to act on yet). [`ToolLoop`] itself is fully real and
//! fully tested against that seam either way: `crate::agent::tests::tool_loop`
//! drives it with structured [`crate::agent::executor::RequestedToolCall`]s
//! from a fake driver, proving the orchestration correct and ready.
//!
//! # The effective per-run tool registry
//!
//! [`effective_registry_for_run`] composes the run-scoped view
//! `PolicyEngine::authorize` is checked against: it prunes a global
//! `crate::tools::registry::ToolRegistry` down to exactly the descriptors
//! the run's allowlist permits, further narrowed by an active skill's
//! `crate::tools::policy::SkillToolScope` when one applies (Task 3's own
//! "narrow, never widen" invariant — `crate::skills::policy::effective_skill_scope`
//! is the same shape one layer up, over a skill manifest). [`ToolLoop`] is
//! constructed once per run with this pruned registry, so "the orchestrator
//! may request tools only from the effective registry published for that
//! run" is enforced structurally: `PolicyEngine::authorize` already denies
//! `ToolNotRegistered`/`ToolNotAllowlisted` for anything outside it.
//!
//! # Two approval systems — investigated, not merged
//!
//! This codebase has two independent approval concepts. This module wires
//! only the first one:
//!
//! 1. `crate::tools::policy::{ApprovalRequest, ApprovalToken, ExecutionGrant}`
//!    (Task 1) — generic: any `xfalcon_contracts::tools::ToolRisk::Mutating`
//!    tool call needs one. **This is what [`ToolLoop`] drives** via
//!    [`ToolLoop::run_round`]/[`ToolLoop::resume`], pausing the owning
//!    [`crate::agent::run::Run`] in `RunState::AwaitingApproval` exactly
//!    when a dispatched invocation enters
//!    `xfalcon_contracts::tools::ToolInvocationStatus::AwaitingApproval`.
//! 2. `crate::approvals::{policy, service}::{ApprovalDecision, ApprovalProof,
//!    MutationPlan}` (Task 6) — specific to `project.propose_patch`/
//!    `crate::projects::writer::execute`. Confirmed by reading
//!    `crate::tools::registry::project_propose_patch_tool_descriptor`'s own
//!    doc comment, `crate::approvals::service::ApprovalsService::resolve`,
//!    and `crate::approvals::commands`: `project.propose_patch` is
//!    advertised as `ToolRisk::ReadOnly` (it only *plans* a mutation, never
//!    writes), so calling it through this module's policy path never
//!    requires an [`crate::tools::policy::ApprovalToken`] at all. The actual
//!    file mutation is gated *exclusively* by
//!    `ApprovalsService::resolve`, reached through `ApprovalsService::apply`
//!    either from an agent patch or from `ApprovalsService::save` (a
//!    person's edit in the file viewer, through the `project_save_file`
//!    command). Neither waits on a review: trusting the folder is the
//!    consent, and every overwritten file is kept in its revision history.
//!    `ApprovalsService::resolve`'s own `tool_call_id: Option<Uuid>`
//!    parameter and its doc comment ("once a live agent run has created a
//!    DA-tracked tool call for the originating `project.propose_patch`
//!    invocation") describe exactly the seam this module's dispatch would
//!    feed *if* it dispatched that tool for real (see the "no live
//!    dispatcher yet" note below) — but the write decision itself
//!    always stays in system 2, never system 1. The two systems are
//!    deliberately not merged: [`ToolLoop`]'s `RequireApproval`/
//!    `AwaitingApproval` machinery exists for *other*, present-or-future
//!    `Mutating` tools (e.g. an MCP-sourced mutating tool), not for project
//!    file mutations.
//!
//! # The live tool-execution dispatcher
//!
//! [`ToolExecutorPort`] is the execution seam. Its production implementation
//! is `crate::tools::dispatch::ProjectToolDispatcher`, composed per run by
//! `crate::agent::tool_session::build_tool_session` with the live
//! `crate::projects::TrustedRoot`/`crate::projects::IndexStore`/account
//! context a Tauri composition root can supply. This module's own tests
//! still use a deterministic in-memory fake, so the orchestrator itself —
//! registry composition, schema validation, policy, journal, approval,
//! idempotency, bounds, audit — is proven without touching a filesystem.
//!
//! # Prompt injection cannot alter policy
//!
//! A tool's *result* is model-visible content the instant it is returned —
//! untrusted, per `docs/threat-model.md`'s governing invariant.
//! Nothing in [`ToolLoop`] ever reads a completed [`crate::tools::invocation::ToolResult`]
//! back into a [`crate::tools::policy::PolicyRequest`]: every field of a
//! fresh `PolicyRequest` in [`ToolLoop::prepare`]/[`ToolLoop::resume`] comes
//! only from run-scoped, Rust-owned state (the pruned registry/allowlist,
//! the fixed active project/skill scope, budgets this struct itself
//! tracks) or from the *current* call's own `tool_id` (never `arguments`,
//! matching `PolicyRequest`'s own doc comment) or an explicitly-presented
//! [`crate::tools::policy::ApprovalToken`] object (never a string parsed out
//! of arguments or a prior result — see [`ToolLoop::resume`]'s signature).
//! `crate::agent::tests::tool_loop::prompt_injected_tool_result_cannot_alter_a_later_policy_decision`
//! proves this directly: a malicious tool result claiming to grant an
//! allowlist entry or a forged approval has zero effect on a later,
//! identical dispatch.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use xfalcon_contracts::tools::{InvocationId, ToolId, ToolInvocationEvent, ToolInvocationStatus};
use xfalcon_contracts::RunId;

use crate::audit::{
    AuditAction, AuditOutcome, AuditPublisher, AuditQueue, AuditResourceType, AuditSink,
};
use crate::tools::invocation::{
    InvocationError, InvocationJournal, JournaledInvocationEntry, ToolInvocation, ToolResult,
    ToolResultError, MAX_FAILURE_MESSAGE_BYTES,
};
use crate::tools::policy::{
    Actor, ApprovalRequest, ApprovalToken, PathAuthority, PolicyDecision, PolicyDenial,
    PolicyEngine, PolicyRequest, ProjectContext, SkillToolScope,
};
use crate::tools::registry::{ToolDescriptor, ToolRegistry};

/// Maximum tool-calling rounds one run may drive.
///
/// Multi-file projects may need many reads, queries, and edits. The context
/// window and the run wall-clock remain
/// the binding limits — this cap is a last-resort tripwire, not a product
/// restriction (`docs/threat-model.md` TM-011).
pub const MAX_TOOL_ROUNDS: u32 = 256;

/// Maximum tool calls dispatched concurrently within one round.
///
/// Raised from TM-011's original "three workers" so a model that emits a
/// batch of independent reads is not serialized. The run's context and
/// wall-clock still bound cost.
pub const MAX_PARALLEL_TOOL_CALLS: usize = 8;

/// Maximum total tool calls one run may make across every round.
///
/// A derived bound, not an independent one: [`MAX_TOOL_ROUNDS`] *
/// [`MAX_PARALLEL_TOOL_CALLS`] — a run can never make more total tool calls
/// than fully saturating every round at maximum parallelism would allow.
pub const MAX_TOOL_CALLS_PER_RUN: u32 = MAX_TOOL_ROUNDS * MAX_PARALLEL_TOOL_CALLS as u32;

/// Maximum calls to any single tool within one run.
///
/// Shares the run's total bound: repeatedly paging project files must not
/// exhaust a separate quota on the read tool.
pub const MAX_CALLS_PER_TOOL: u32 = MAX_TOOL_CALLS_PER_RUN;

/// Wall-clock bound for one tool invocation's execution.
///
/// Allows larger local reads and dashboard queries to finish while keeping
/// one unresponsive invocation bounded. User Stop remains immediate.
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Extra time after a local timeout for the executor to observe its per-call
/// cancellation and finish protocol cleanup. This does not extend the tool's
/// execution budget or change its timeout outcome.
const TOOL_CALL_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

/// Wall-clock bound for one round (all its parallel calls together).
///
/// Longer than [`TOOL_CALL_TIMEOUT`] so [`MAX_PARALLEL_TOOL_CALLS`] calls
/// running genuinely concurrently are never starved by this outer bound —
/// it exists as defense in depth against a scheduling pathology, not as the
/// primary per-call bound.
pub const ROUND_TIMEOUT: Duration = Duration::from_secs(300);

/// One tool call the model requested, ready for [`ToolLoop`] to dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelToolCall {
    pub invocation_id: InvocationId,
    pub tool_id: ToolId,
    pub arguments: Value,
}

/// Errors from driving the loop itself (bound violations, unknown
/// invocations) — distinct from a single tool call's own outcome, which is
/// always a [`ToolCallOutcome`], never an `Err` (per-call failure is a
/// first-class, journaled, reportable result, not an exception).
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ToolLoopError {
    #[error("maximum tool rounds ({0}) exceeded for this run")]
    MaxRoundsExceeded(u32),
    #[error("round exceeded its wall-clock bound")]
    RoundTimeout,
    #[error("an MCP server requires a new sign-in")]
    AuthorizationRefused,
    #[error("invocation {0:?} has no pending approval to resolve")]
    UnknownPendingInvocation(InvocationId),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
}

/// Why a tool call could not even be dispatched (bad arguments) or why its
/// execution did not produce a result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExecutionError {
    /// The tool call exceeded [`TOOL_CALL_TIMEOUT`].
    Timeout,
    /// The transport (e.g. the bundled MCP session) was disconnected.
    Disconnected,
    /// Execution observed run/round cancellation.
    Cancelled,
    /// The tool's server rate limited this account (HTTP 429) and the wait it
    /// asked for, when it named one.
    ///
    /// Distinct from [`Self::Disconnected`]: the connection is healthy, and
    /// calling again at once is refused the same way.
    RateLimited { retry_after: Option<Duration> },
    /// The server refused the credential: its grant expired, was revoked, or
    /// does not cover what the server requires. Carries the label to name it by.
    ///
    /// Distinct from [`Self::Disconnected`] because it must not be retried.
    /// Reconnecting attaches the same refused credential and is refused the same
    /// way, so a retry spends a turn to arrive where it already was and buries
    /// the one thing the user can act on. The user is prompted to sign in
    /// instead — see `crate::mcp::dispatch`.
    AuthorizationRefused { label: String },
    /// The tool returned a structured value too large to retain exactly.
    /// This is terminal: reporting success would lose data, while replaying
    /// the same call cannot make the result smaller.
    ResultTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    /// Any other execution failure, with a bounded, non-retryable message.
    Failed(String),
    /// A failure the caller can fix and retry: its message says what to change
    /// (a bad anchor, a file that changed under it, a partly applied plan).
    /// Reported to the model as `retryable`, so it corrects the call instead of
    /// giving up on it.
    Recoverable(String),
}

impl From<ToolResultError> for ToolExecutionError {
    fn from(error: ToolResultError) -> Self {
        match error {
            ToolResultError::StructuredResultTooLarge {
                actual_bytes,
                max_bytes,
            } => Self::ResultTooLarge {
                actual_bytes,
                max_bytes,
            },
        }
    }
}

/// The tool-execution seam — see the module doc comment's "no live
/// dispatcher yet" section for why no concrete implementation ships with
/// this task.
#[async_trait::async_trait]
pub trait ToolExecutorPort: Send + Sync {
    /// Local writes must finish and report their real outcome after they start.
    /// Their executor still observes cancellation before starting the write.
    /// Reads and remote tools keep their bounded cancellation behavior.
    fn requires_completion(&self, _tool_id: &ToolId) -> bool {
        false
    }

    /// `cancel` belongs to this call. The loop derives it from the owning run
    /// token so run cancellation propagates inward, but an executor must not
    /// cancel it as a way to stop the run or a sibling call.
    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError>;
}

/// Executes one call under its own cancellation scope. A timeout cancels only
/// this call, then polls it for a bounded cleanup window so protocol executors
/// can notify the remote peer and release per-call progress state.
async fn execute_with_timeout(
    executor: &dyn ToolExecutorPort,
    tool_id: &ToolId,
    arguments: &Value,
    call_cancel: CancellationToken,
    timeout: Duration,
) -> Result<ToolResult, ToolExecutionError> {
    let mut execution = Box::pin(executor.execute(tool_id, arguments, call_cancel.clone()));
    let requires_completion = executor.requires_completion(tool_id);
    tokio::select! {
        biased;
        outcome = &mut execution => outcome,
        () = call_cancel.cancelled() => {
            if requires_completion {
                execution.await
            } else {
                let _ = tokio::time::timeout(TOOL_CALL_CLEANUP_TIMEOUT, &mut execution).await;
                Err(ToolExecutionError::Cancelled)
            }
        }
        () = tokio::time::sleep(timeout) => {
            call_cancel.cancel();
            if requires_completion {
                match execution.await {
                    Err(ToolExecutionError::Cancelled) => Err(ToolExecutionError::Timeout),
                    outcome => outcome,
                }
            } else {
                let _ = tokio::time::timeout(TOOL_CALL_CLEANUP_TIMEOUT, &mut execution).await;
                Err(ToolExecutionError::Timeout)
            }
        }
    }
}

/// The denial reason code a human's decline carries to the model.
///
/// A code rather than prose for the same reason every `PolicyDenial::code` is:
/// the model branches on it, and it is written into the durable invocation
/// record where a sentence would drift.
pub const DECLINED_BY_USER: &str = "declined_by_user";

/// One dispatched call's terminal (or paused) outcome.
#[derive(Debug)]
pub enum ToolCallOutcome {
    Completed {
        invocation_id: InvocationId,
        result: ToolResult,
    },
    /// The call's arguments failed the tool's own advertised JSON schema —
    /// a orchestrator-level check distinct from, and performed before,
    /// [`PolicyEngine::authorize`] (which never inspects argument content;
    /// see `crate::tools::policy::PolicyRequest`'s own doc comment).
    InvalidArguments {
        invocation_id: InvocationId,
        tool_id: ToolId,
        reason: InvalidArgumentsReason,
    },
    Denied {
        invocation_id: InvocationId,
        tool_id: ToolId,
        reason_code: &'static str,
    },
    AwaitingApproval {
        invocation_id: InvocationId,
        tool_id: ToolId,
    },
    Failed {
        invocation_id: InvocationId,
        message: String,
        retryable: bool,
    },
    Cancelled {
        invocation_id: InvocationId,
    },
}

impl ToolCallOutcome {
    #[must_use]
    pub fn invocation_id(&self) -> &InvocationId {
        match self {
            Self::Completed { invocation_id, .. }
            | Self::InvalidArguments { invocation_id, .. }
            | Self::Denied { invocation_id, .. }
            | Self::AwaitingApproval { invocation_id, .. }
            | Self::Failed { invocation_id, .. }
            | Self::Cancelled { invocation_id } => invocation_id,
        }
    }
}

/// Outcome of one [`ToolLoop::run_round`] call.
#[derive(Debug)]
pub struct RoundOutcome {
    pub outcomes: Vec<ToolCallOutcome>,
    /// `true` iff at least one outcome is [`ToolCallOutcome::AwaitingApproval`]
    /// — the caller must transition the owning `Run` to
    /// `RunState::AwaitingApproval` when this is set.
    pub paused: bool,
}

/// Configurable bounds, mirroring `crate::agent::run::RunBudgets`'s own
/// "plain struct, production constant vs. test override" shape.
#[derive(Clone, Copy, Debug)]
pub struct ToolLoopBudgets {
    pub max_rounds: u32,
    pub max_parallel_calls: usize,
    pub max_calls_per_run: u32,
    pub max_calls_per_tool: u32,
    pub tool_call_timeout: Duration,
    pub round_timeout: Duration,
}

impl ToolLoopBudgets {
    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_rounds: MAX_TOOL_ROUNDS,
            max_parallel_calls: MAX_PARALLEL_TOOL_CALLS,
            max_calls_per_run: MAX_TOOL_CALLS_PER_RUN,
            max_calls_per_tool: MAX_CALLS_PER_TOOL,
            tool_call_timeout: TOOL_CALL_TIMEOUT,
            round_timeout: ROUND_TIMEOUT,
        }
    }
}

/// Live identity/tenancy context for the first real `crate::audit::AuditSink`
/// caller. Optional: a [`ToolLoop`] with no [`ToolLoopAudit`] simply never
/// publishes (audit is an observability concern, never a gate on tool
/// execution — matching `crate::approvals::service::ApprovalsService::sync_da`'s
/// own "must never block or reverse a decision" precedent). Thin wrapper
/// around the shared `crate::audit::AuditPublisher` (identical "publish or
/// spool, sanitize identifiers, never reuse a sequence across instances"
/// logic every other real `AuditSink` caller in this codebase now shares —
/// see that type's own module doc comment) plus this module's own fixed
/// `AuditResourceType::ToolCall` / `"tools.policy.v1"` policy id convention.
pub struct ToolLoopAudit {
    publisher: AuditPublisher,
}

impl ToolLoopAudit {
    #[must_use]
    pub fn new(
        actor_id: impl Into<String>,
        tenant_id: impl Into<String>,
        device_id: impl Into<String>,
        sink: Arc<dyn AuditSink>,
        queue: Arc<AuditQueue>,
    ) -> Self {
        Self {
            publisher: AuditPublisher::new(actor_id, tenant_id, device_id, sink, queue),
        }
    }

    /// Audits through an existing publisher, such as the local log's
    /// [`AuditPublisher`] shared with the other audited services.
    #[must_use]
    pub fn from_publisher(publisher: AuditPublisher) -> Self {
        Self { publisher }
    }
}

/// Composes the run-scoped view of `global` — see the module doc comment's
/// "effective per-run tool registry" section. A descriptor absent from
/// `allowlist`, or (when `skill_scope` is set) outside that skill's
/// narrowed set, is simply omitted — this can only ever narrow what `global`
/// contains, never add to it (every descriptor in the result came from
/// `global` itself, unmodified).
#[must_use]
pub fn effective_registry_for_run(
    global: &ToolRegistry,
    allowlist: &std::collections::HashSet<ToolId>,
    skill_scope: Option<&SkillToolScope>,
) -> ToolRegistry {
    let mut effective = ToolRegistry::new();
    for descriptor in global.iter() {
        if !allowlist.contains(descriptor.id()) {
            continue;
        }
        if let Some(scope) = skill_scope {
            if !scope.allows(descriptor.id()) {
                continue;
            }
        }
        // `global`'s ids are already unique (it is itself a `ToolRegistry`),
        // so registering each surviving descriptor into a fresh registry
        // can never collide.
        let _ = effective.register(descriptor.clone());
    }
    effective
}

/// Reconstructs an invocation's last-journaled status without re-executing
/// anything — mirrors `crate::agent::journal_recovery::replay_policy_for_journal`'s
/// "inspect the tail of a persisted journal" idiom one level down
/// (per-invocation instead of per-run). This is the durable evidence Task 7
/// Step 4's "approval state survives restart" requirement rests on: the
/// journal entry for `AwaitingApproval` is written by
/// [`ToolInvocation::require_approval`] *before* [`ToolLoop`]'s own
/// in-memory pending-approval map is updated, so the status this function
/// reconstructs is never behind what [`ToolLoop`]'s live state claims.
#[must_use]
pub fn last_journaled_status(
    entries: &[JournaledInvocationEntry],
    invocation_id: &InvocationId,
) -> Option<ToolInvocationStatus> {
    entries
        .iter()
        .rfind(|entry| &entry.invocation_id == invocation_id)
        .map(|entry| status_for_event(&entry.event))
}

fn status_for_event(event: &ToolInvocationEvent) -> ToolInvocationStatus {
    match event {
        ToolInvocationEvent::Requested { .. } => ToolInvocationStatus::Requested,
        ToolInvocationEvent::ApprovalRequired { .. } => ToolInvocationStatus::AwaitingApproval,
        ToolInvocationEvent::Approved { .. } => ToolInvocationStatus::Approved,
        ToolInvocationEvent::Denied { .. } => ToolInvocationStatus::Denied,
        ToolInvocationEvent::Started { .. } | ToolInvocationEvent::Progress { .. } => {
            ToolInvocationStatus::Running
        }
        ToolInvocationEvent::Completed { .. } => ToolInvocationStatus::Completed,
        ToolInvocationEvent::Failed { .. } => ToolInvocationStatus::Failed,
        ToolInvocationEvent::Cancelled { .. } => ToolInvocationStatus::Cancelled,
    }
}

/// Why a call's arguments were refused before it ran. Both are answers the
/// model can act on, so both reach it rather than ending the run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidArgumentsReason {
    /// They did not match the tool's advertised schema.
    Schema,
    /// They serialized to `bytes`, over the per-call bound of `limit`.
    TooLarge { bytes: usize, limit: usize },
}

/// A completed dispatch's cacheable outcome — see [`ToolLoop::prepare`]'s
/// idempotency handling. Reconstructing an equivalent [`ToolResult`] from
/// its own accessors (rather than requiring `ToolResult: Clone`, which
/// `crate::tools::invocation` does not derive) keeps this module from
/// needing to touch Task 1's files at all.
#[derive(Debug)]
enum TerminalOutcome {
    Completed(ToolResult),
    InvalidArguments(InvalidArgumentsReason),
    Denied(&'static str),
    Failed { message: String, retryable: bool },
    Cancelled,
}

struct PendingApproval {
    invocation: ToolInvocation,
    request: ApprovalRequest,
}

enum Prepared {
    Ready {
        tool_id: ToolId,
        arguments: Value,
        invocation: ToolInvocation,
    },
    Done(ToolCallOutcome),
}

/// The bounded, policy-gated tool-calling orchestrator for one run. See the
/// module doc comment for the full design.
pub struct ToolLoop {
    registry: ToolRegistry,
    allowlist: std::collections::HashSet<ToolId>,
    actor: Actor,
    run_id: RunId,
    active_project: Option<ProjectContext>,
    skill_scope: Option<SkillToolScope>,
    budgets: ToolLoopBudgets,
    audit: Option<ToolLoopAudit>,
    /// Standing "stop asking about this tool" grants. `None` (or a miss)
    /// keeps the invocation-by-invocation parking behavior.
    standing_approvals: Option<std::sync::Arc<crate::tools::standing::StandingToolApprovals>>,

    rounds_used: u32,
    run_calls_used: u32,
    per_tool_calls_used: HashMap<ToolId, u32>,
    /// Keyed by `invocation_id` alone, but the stored `ToolId` is compared
    /// against every replay's *newly requested* `tool_id` before the cached
    /// outcome is ever returned (see [`Self::prepare`]) — an invocation id
    /// reused for a different tool must never silently answer with a prior,
    /// unrelated tool's cached result (a review finding caught this: the
    /// cache used to key on `invocation_id` alone).
    terminal: HashMap<InvocationId, (ToolId, TerminalOutcome)>,
    pending_approval: HashMap<InvocationId, PendingApproval>,
    /// The tool each invocation named, recorded the first time the loop sees
    /// it. Audit events carry it as their detail.
    invocation_tools: HashMap<InvocationId, ToolId>,
    /// Set when an MCP executor reports that its grant expired or was revoked.
    /// The owning run reads this and stops before asking the model for another
    /// turn against an invalidated tool session.
    authorization_refused: bool,
    round_timed_out: bool,
}

/// Stable, tool-loop-local reason code for [`ToolLoop::prepare`]'s
/// cross-tool-id collision guard. Not a `crate::tools::policy::PolicyDenial`
/// variant: `PolicyDenial` is Task 1's closed set of structural facts
/// `PolicyEngine::authorize` itself computes (see that enum's own doc
/// comment), and this is a `ToolLoop`-level idempotency-cache invariant
/// `PolicyEngine::authorize` is never even consulted for.
const INVOCATION_ID_TOOL_MISMATCH_REASON_CODE: &str = "invocation_id_tool_mismatch";

/// Refusal code for the calls a round could not take because the model asked
/// for more parallelism than [`MAX_PARALLEL_TOOL_CALLS`] allows.
///
/// A refusal, not a failure: the call was never opened, authorized, or
/// journaled, and re-issuing it on the next round is the expected response.
/// It reaches the model as `{"status":"denied","reason":"parallel_call_limit"}`
/// alongside the results of the calls that did run.
const PARALLEL_CALL_LIMIT_REASON_CODE: &str = "parallel_call_limit";

impl ToolLoop {
    #[must_use]
    pub fn new(
        registry: ToolRegistry,
        allowlist: std::collections::HashSet<ToolId>,
        actor: Actor,
        run_id: RunId,
        active_project: Option<ProjectContext>,
        skill_scope: Option<SkillToolScope>,
        budgets: ToolLoopBudgets,
    ) -> Self {
        Self {
            registry,
            allowlist,
            actor,
            run_id,
            active_project,
            skill_scope,
            budgets,
            audit: None,
            standing_approvals: None,
            rounds_used: 0,
            run_calls_used: 0,
            per_tool_calls_used: HashMap::new(),
            terminal: HashMap::new(),
            pending_approval: HashMap::new(),
            invocation_tools: HashMap::new(),
            authorization_refused: false,
            round_timed_out: false,
        }
    }

    #[must_use]
    pub(crate) fn authorization_refused(&self) -> bool {
        self.authorization_refused
    }

    #[must_use]
    pub(crate) fn round_timed_out(&self) -> bool {
        self.round_timed_out
    }

    #[must_use]
    pub fn with_audit(mut self, audit: ToolLoopAudit) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Wires the user's standing "allow for session"/"always allow" grants.
    /// Without it every mutating call parks — the pre-existing behavior.
    #[must_use]
    pub fn with_standing_approvals(
        mut self,
        standing: std::sync::Arc<crate::tools::standing::StandingToolApprovals>,
    ) -> Self {
        self.standing_approvals = Some(standing);
        self
    }

    #[must_use]
    pub fn rounds_used(&self) -> u32 {
        self.rounds_used
    }

    #[must_use]
    pub fn run_calls_used(&self) -> u32 {
        self.run_calls_used
    }

    /// The [`ApprovalRequest`] for a currently-paused invocation, if any —
    /// the only way a caller (the future approval UI; this task's own
    /// tests standing in for it) can mint a matching
    /// [`ApprovalToken`] via [`ApprovalRequest::approve`].
    #[must_use]
    pub fn pending_approval_request(
        &self,
        invocation_id: &InvocationId,
    ) -> Option<&ApprovalRequest> {
        self.pending_approval
            .get(invocation_id)
            .map(|pending| &pending.request)
    }

    #[must_use]
    pub fn has_pending_approval(&self, invocation_id: &InvocationId) -> bool {
        self.pending_approval.contains_key(invocation_id)
    }

    /// Every invocation currently paused awaiting approval. Lets a caller
    /// composing a [`crate::agent::run::Run`] with this `ToolLoop` (e.g.
    /// `crate::agent::engine::run_with_tool_loop`) resolve every pause when
    /// the owning run is cancelled out from under it, rather than leaving
    /// any of them durably stuck at
    /// `xfalcon_contracts::tools::ToolInvocationStatus::AwaitingApproval`
    /// with nothing left able to decline/resume them.
    #[must_use]
    pub fn pending_invocation_ids(&self) -> Vec<InvocationId> {
        self.pending_approval.keys().cloned().collect()
    }

    /// Dispatches one round of independent tool calls. Calls are prepared
    /// (idempotency check, schema validation, policy authorization,
    /// journaling) strictly sequentially — deterministic and directly
    /// testable — then any that reached `Approved`/`Running` execute
    /// genuinely concurrently (bounded by `budgets.max_parallel_calls`)
    /// before results are journaled back sequentially. See the reference
    /// evidence doc for why this split (rather than a fully sequential
    /// round) was chosen.
    pub async fn run_round(
        &mut self,
        calls: Vec<ModelToolCall>,
        executor: &dyn ToolExecutorPort,
        path_authority: Option<&dyn PathAuthority>,
        journal: &mut dyn InvocationJournal,
        cancel: &CancellationToken,
    ) -> Result<RoundOutcome, ToolLoopError> {
        self.round_timed_out = false;
        if self.rounds_used >= self.budgets.max_rounds {
            return Err(ToolLoopError::MaxRoundsExceeded(self.budgets.max_rounds));
        }
        self.rounds_used += 1;

        // A model asking for more parallelism than TM-011 allows is ordinary
        // behaviour, not grounds to kill the run — which is exactly what this
        // used to do (`ToolLoopError::TooManyParallelCalls` bubbled to
        // `RunEngineError`, the run reached `Failed`, and the user got a
        // finished-looking tool trail with no answer under it). Dispatch what
        // fits the bound and refuse the surplus with a reason the model can
        // act on, so it re-issues the rest on the next round.
        //
        // Every call the model made still gets an outcome: an assistant turn
        // left with unanswered tool calls is rejected outright by OpenAI and
        // Anthropic on the following request.
        let mut calls = calls;
        let refused = if calls.len() > self.budgets.max_parallel_calls {
            calls.split_off(self.budgets.max_parallel_calls)
        } else {
            Vec::new()
        };
        for call in &refused {
            self.remember_tool(&call.invocation_id, &call.tool_id);
        }

        let mut prepared = Vec::with_capacity(calls.len());
        for call in calls {
            prepared.push(self.prepare(call, path_authority, journal, cancel).await?);
        }

        let tool_call_timeout = self.budgets.tool_call_timeout;
        let round_timeout = self.budgets.round_timeout;

        let mut futures = Vec::new();
        let mut call_cancels = Vec::new();
        for (index, item) in prepared.iter().enumerate() {
            if let Prepared::Ready {
                tool_id, arguments, ..
            } = item
            {
                let tool_id = tool_id.clone();
                let arguments = arguments.clone();
                let call_cancel = cancel.child_token();
                call_cancels.push(call_cancel.clone());
                futures.push(async move {
                    let outcome = execute_with_timeout(
                        executor,
                        &tool_id,
                        &arguments,
                        call_cancel,
                        tool_call_timeout,
                    )
                    .await;
                    (index, outcome)
                });
            }
        }

        let mut joined = Box::pin(futures_util::future::join_all(futures));
        let requires_completion = prepared.iter().any(|item| {
            matches!(item, Prepared::Ready { tool_id, .. } if executor.requires_completion(tool_id))
        });
        let joined = match tokio::time::timeout(round_timeout, &mut joined).await {
            Ok(joined) => joined,
            Err(_) => {
                for call_cancel in &call_cancels {
                    call_cancel.cancel();
                }
                if requires_completion {
                    // The local writer owns its terminal status and may already
                    // have changed files. Drain it and journal the real result;
                    // cancellation bounds all read/remote siblings to cleanup.
                    self.round_timed_out = true;
                    joined.await
                } else {
                    // Keep the executor futures alive briefly after cancellation.
                    // MCP uses this poll to send `notifications/cancelled` and to
                    // remove the call's progress token.
                    let _ = tokio::time::timeout(TOOL_CALL_CLEANUP_TIMEOUT, &mut joined).await;
                    // The round itself timed out. Any `Prepared::Ready` invocation
                    // was already opened, approved, and started (journaled) before
                    // dispatch — with no execution result available, it must still
                    // be finalized now (as a retryable failure) rather than left
                    // dangling in neither `self.terminal` nor `self.pending_approval`.
                    // Left dangling, a later same-invocation-id round would fall
                    // through `Self::prepare`'s idempotency checks and silently
                    // re-open/re-authorize/re-dispatch it from scratch — a review
                    // finding caught exactly this gap.
                    for item in prepared {
                        if let Prepared::Ready { mut invocation, .. } = item {
                            self.finalize_failure(
                                &mut invocation,
                                "round exceeded its wall-clock bound",
                                true,
                                journal,
                            )
                            .await;
                        }
                    }
                    return Err(ToolLoopError::RoundTimeout);
                }
            }
        };

        let mut execution_results: HashMap<usize, Result<ToolResult, ToolExecutionError>> =
            HashMap::new();
        for (index, outcome) in joined {
            let mapped = outcome;
            execution_results.insert(index, mapped);
        }

        let mut outcomes = Vec::with_capacity(prepared.len());
        let mut paused = false;
        for (index, item) in prepared.into_iter().enumerate() {
            let outcome = match item {
                Prepared::Done(outcome) => outcome,
                Prepared::Ready { invocation, .. } => {
                    let result = execution_results.remove(&index).unwrap_or(Err(
                        ToolExecutionError::Failed("no execution result was produced".to_owned()),
                    ));
                    self.finalize(invocation, result, journal).await
                }
            };
            if matches!(outcome, ToolCallOutcome::AwaitingApproval { .. }) {
                paused = true;
            }
            outcomes.push(outcome);
        }

        for call in refused {
            self.publish_tool_audit(
                AuditAction::ToolDeny,
                AuditOutcome::Rejected,
                &call.invocation_id,
            )
            .await;
            outcomes.push(ToolCallOutcome::Denied {
                invocation_id: call.invocation_id,
                tool_id: call.tool_id,
                reason_code: PARALLEL_CALL_LIMIT_REASON_CODE,
            });
        }

        Ok(RoundOutcome { outcomes, paused })
    }

    /// Resolves exactly one paused invocation with `token`. `token` must be
    /// the exact [`ApprovalToken`] minted for this invocation/tool pair
    /// (`ApprovalToken::matches`, re-checked fresh by [`PolicyEngine::authorize`]
    /// below) — never a bare boolean, never a value derived from tool
    /// output.
    pub async fn resume(
        &mut self,
        invocation_id: &InvocationId,
        token: ApprovalToken,
        executor: &dyn ToolExecutorPort,
        path_authority: Option<&dyn PathAuthority>,
        journal: &mut dyn InvocationJournal,
        cancel: &CancellationToken,
    ) -> Result<ToolCallOutcome, ToolLoopError> {
        let Some(pending) = self.pending_approval.remove(invocation_id) else {
            return Err(ToolLoopError::UnknownPendingInvocation(
                invocation_id.clone(),
            ));
        };
        let PendingApproval {
            mut invocation,
            request,
        } = pending;
        let tool_id = invocation.tool_id().clone();
        let arguments = invocation.arguments().clone();

        let run_remaining = self
            .budgets
            .max_calls_per_run
            .saturating_sub(self.run_calls_used);
        let tool_remaining = self
            .budgets
            .max_calls_per_tool
            .saturating_sub(*self.per_tool_calls_used.get(&tool_id).unwrap_or(&0));

        let requested_path = requested_path_for(&self.registry, &tool_id, &arguments);
        let decision = {
            let engine = PolicyEngine::new(&self.registry, &self.allowlist);
            engine.authorize(&PolicyRequest {
                actor: Some(&self.actor),
                invocation_id,
                tool_id: &tool_id,
                arguments: &arguments,
                run_budget_remaining: run_remaining,
                tool_budget_remaining: tool_remaining,
                active_project: self.active_project.as_ref(),
                requested_path: requested_path.as_deref(),
                path_authority,
                skill_scope: self.skill_scope.as_ref(),
                approval: Some(&token),
            })
        };

        let outcome = match decision {
            PolicyDecision::Allow(_grant) => {
                // Both fallible transitions run *before* `invocation` is
                // consumed by `self.finalize` below; on either error, the
                // already-`remove`d `PendingApproval` is re-inserted first —
                // a review finding caught that an earlier version of this
                // arm used `?` directly here, which would drop the
                // invocation without re-inserting it into
                // `self.pending_approval` or recording it into
                // `self.terminal`, permanently orphaning it (no subsequent
                // resume/decline/query could ever reach it again).
                if let Err(error) = invocation.approve(journal) {
                    self.pending_approval.insert(
                        invocation_id.clone(),
                        PendingApproval {
                            invocation,
                            request,
                        },
                    );
                    return Err(error.into());
                }
                if let Err(error) = invocation.start(journal) {
                    self.pending_approval.insert(
                        invocation_id.clone(),
                        PendingApproval {
                            invocation,
                            request,
                        },
                    );
                    return Err(error.into());
                }
                self.publish_tool_audit(
                    AuditAction::ToolApprove,
                    AuditOutcome::Approved,
                    invocation_id,
                )
                .await;
                let execution = execute_with_timeout(
                    executor,
                    &tool_id,
                    &arguments,
                    cancel.child_token(),
                    self.budgets.tool_call_timeout,
                )
                .await;
                self.finalize(invocation, execution, journal).await
            }
            PolicyDecision::RequireApproval(new_request) => {
                // Re-presenting an approval that authorize() still does not
                // consider sufficient (e.g. it mismatched) leaves the
                // invocation genuinely still pending, never silently
                // dropped.
                let invocation_id = invocation.invocation_id().clone();
                let tool_id = invocation.tool_id().clone();
                self.pending_approval.insert(
                    invocation_id.clone(),
                    PendingApproval {
                        invocation,
                        request: new_request,
                    },
                );
                ToolCallOutcome::AwaitingApproval {
                    invocation_id,
                    tool_id,
                }
            }
            PolicyDecision::Deny(denial) => {
                if let Err(error) = invocation.deny(&denial, journal) {
                    self.pending_approval.insert(
                        invocation_id.clone(),
                        PendingApproval {
                            invocation,
                            request,
                        },
                    );
                    return Err(error.into());
                }
                let invocation_id = invocation.invocation_id().clone();
                let tool_id = invocation.tool_id().clone();
                self.terminal.insert(
                    invocation_id.clone(),
                    (tool_id.clone(), TerminalOutcome::Denied(denial.code())),
                );
                self.publish_tool_audit(
                    AuditAction::ToolDeny,
                    AuditOutcome::Denied,
                    &invocation_id,
                )
                .await;
                ToolCallOutcome::Denied {
                    invocation_id,
                    tool_id,
                    reason_code: denial.code(),
                }
            }
        };
        Ok(outcome)
    }

    /// Declines a pending approval without ever executing the tool.
    ///
    /// Two layers record it differently, on purpose:
    ///
    /// - the **invocation journal** records `AwaitingApproval -> Cancelled`.
    ///   Task 1's `crate::tools::policy` module deliberately has no "denied by
    ///   a human" `PolicyDenial` variant — every variant is a *structural
    ///   policy fact* `PolicyEngine::authorize` computes, never a decision a
    ///   caller injects — and cancelling is already a legal transition there.
    /// - the **model** is told [`ToolCallOutcome::Denied`] with
    ///   [`DECLINED_BY_USER`]. `Cancelled` is what a run-wide cancel produces,
    ///   and a model that cannot tell the two apart re-requests the call it was
    ///   just refused, which puts the same prompt back in front of the person
    ///   who said no.
    pub async fn decline_pending_approval(
        &mut self,
        invocation_id: &InvocationId,
        journal: &mut dyn InvocationJournal,
    ) -> Result<ToolCallOutcome, ToolLoopError> {
        let Some(mut pending) = self.pending_approval.remove(invocation_id) else {
            return Err(ToolLoopError::UnknownPendingInvocation(
                invocation_id.clone(),
            ));
        };
        let tool_id = pending.invocation.tool_id().clone();
        if let Err(error) = pending.invocation.cancel(journal) {
            // Re-insert before propagating — see `Self::resume`'s identical
            // "never drop an already-removed pending invocation on a
            // fallible call's error" discipline (same review finding).
            self.pending_approval.insert(invocation_id.clone(), pending);
            return Err(error.into());
        }
        self.terminal.insert(
            invocation_id.clone(),
            (tool_id.clone(), TerminalOutcome::Denied(DECLINED_BY_USER)),
        );
        self.publish_tool_audit(AuditAction::ToolDeny, AuditOutcome::Rejected, invocation_id)
            .await;
        Ok(ToolCallOutcome::Denied {
            invocation_id: invocation_id.clone(),
            tool_id,
            reason_code: DECLINED_BY_USER,
        })
    }

    /// `async` (rather than the plain sync fn an earlier version of this
    /// method was) specifically so every freshly-decided outcome — not just
    /// the ones reached through [`Self::finalize`]/[`Self::resume`] — can be
    /// audited uniformly here, at the single point each is actually decided
    /// (never on a *replayed* outcome from `self.terminal`/
    /// `self.pending_approval`, which was already audited the first time).
    /// A review finding caught that the previous, sync version never
    /// audited a first-attempt policy denial or a schema-invalid call at
    /// all — only a *second* denial reached via [`Self::resume`] was.
    async fn prepare(
        &mut self,
        call: ModelToolCall,
        path_authority: Option<&dyn PathAuthority>,
        journal: &mut dyn InvocationJournal,
        cancel: &CancellationToken,
    ) -> Result<Prepared, ToolLoopError> {
        let ModelToolCall {
            invocation_id,
            tool_id,
            arguments,
        } = call;
        self.remember_tool(&invocation_id, &tool_id);

        // Idempotency: a repeated invocation id for an already-terminal
        // invocation is never re-executed — its recorded outcome is
        // returned as-is (Task 7 Step 4). The stored `tool_id` is compared
        // against this call's own `tool_id` first: a reused invocation id
        // naming a *different* tool than the one that actually produced the
        // cached outcome must never silently replay that unrelated result
        // mislabeled as an answer for this call (a review finding caught
        // this: the cache used to key on `invocation_id` alone).
        if let Some((cached_tool_id, terminal)) = self.terminal.get(&invocation_id) {
            if cached_tool_id != &tool_id {
                self.publish_tool_audit(
                    AuditAction::ToolDeny,
                    AuditOutcome::Rejected,
                    &invocation_id,
                )
                .await;
                return Ok(Prepared::Done(ToolCallOutcome::Denied {
                    invocation_id,
                    tool_id,
                    reason_code: INVOCATION_ID_TOOL_MISMATCH_REASON_CODE,
                }));
            }
            return Ok(Prepared::Done(outcome_from_terminal(
                invocation_id,
                tool_id,
                terminal,
            )));
        }
        // A repeat of an id still awaiting approval is a no-op re-report,
        // not a fresh dispatch — resuming only ever happens through
        // `Self::resume`. Same cross-tool-id guard as above.
        if let Some(pending) = self.pending_approval.get(&invocation_id) {
            if pending.invocation.tool_id() != &tool_id {
                self.publish_tool_audit(
                    AuditAction::ToolDeny,
                    AuditOutcome::Rejected,
                    &invocation_id,
                )
                .await;
                return Ok(Prepared::Done(ToolCallOutcome::Denied {
                    invocation_id,
                    tool_id,
                    reason_code: INVOCATION_ID_TOOL_MISMATCH_REASON_CODE,
                }));
            }
            return Ok(Prepared::Done(ToolCallOutcome::AwaitingApproval {
                invocation_id,
                tool_id,
            }));
        }

        if cancel.is_cancelled() {
            // Never opened, so nothing to journal — a cancelled run must
            // not fabricate invocation history for calls it never started.
            self.publish_tool_audit(
                AuditAction::ToolExecute,
                AuditOutcome::Failure,
                &invocation_id,
            )
            .await;
            return Ok(Prepared::Done(ToolCallOutcome::Cancelled { invocation_id }));
        }

        // A `tool_id` absent from the effective per-run registry can never
        // be authorized — `PolicyEngine::authorize` would deny it
        // `ToolNotRegistered` the moment it got there — so reject it here,
        // before `ToolInvocation::open`, mirroring the cancelled and
        // invalid-arguments cases: never fabricate invocation history for a
        // call that was never a real attempt (a review finding caught an
        // earlier version of this method falling through to `open` for
        // exactly this case).
        let Some(descriptor) = self.registry.get(&tool_id) else {
            let reason_code = PolicyDenial::ToolNotRegistered(tool_id.clone()).code();
            self.terminal.insert(
                invocation_id.clone(),
                (tool_id.clone(), TerminalOutcome::Denied(reason_code)),
            );
            self.publish_tool_audit(AuditAction::ToolDeny, AuditOutcome::Denied, &invocation_id)
                .await;
            return Ok(Prepared::Done(ToolCallOutcome::Denied {
                invocation_id,
                tool_id,
                reason_code,
            }));
        };

        // Schema validation is a distinct, pre-policy step — see the module
        // doc comment. Deliberately checked *before* `ToolInvocation::open`:
        // `crate::tools::invocation`'s own transition table has no
        // `(Requested, Failed)` edge (only `Running -> Failed`, reached only
        // after policy already allowed execution to start), so arguments
        // that fail the tool's own schema are rejected before this call is
        // ever considered "Requested" at all — mirroring the cancelled-
        // before-dispatch case just above, never fabricating invocation
        // history or budget consumption for a call that was never a real
        // attempt.
        //
        // Size first: `ToolInvocation::open` refuses oversized arguments as an
        // error, which would end the run over a call the model can simply
        // split. Answering it here keeps it a correctable mistake.
        let argument_bytes = crate::tools::invocation::argument_bytes(&arguments);
        let invalid = if argument_bytes > crate::tools::invocation::MAX_ARGUMENTS_BYTES {
            Some(InvalidArgumentsReason::TooLarge {
                bytes: argument_bytes,
                limit: crate::tools::invocation::MAX_ARGUMENTS_BYTES,
            })
        } else if !arguments_match_schema(descriptor, &arguments) {
            Some(InvalidArgumentsReason::Schema)
        } else {
            None
        };
        if let Some(reason) = invalid {
            // Ids and the kind of mismatch only, never the arguments.
            tracing::info!(tool = %tool_id.as_str(), ?reason, "tool call refused: arguments did not fit the tool");
            self.terminal.insert(
                invocation_id.clone(),
                (tool_id.clone(), TerminalOutcome::InvalidArguments(reason)),
            );
            self.publish_tool_audit(
                AuditAction::ToolDeny,
                AuditOutcome::Rejected,
                &invocation_id,
            )
            .await;
            return Ok(Prepared::Done(ToolCallOutcome::InvalidArguments {
                invocation_id,
                tool_id,
                reason,
            }));
        }

        let run_remaining = self
            .budgets
            .max_calls_per_run
            .saturating_sub(self.run_calls_used);
        let tool_remaining = self
            .budgets
            .max_calls_per_tool
            .saturating_sub(*self.per_tool_calls_used.get(&tool_id).unwrap_or(&0));
        self.run_calls_used += 1;
        *self.per_tool_calls_used.entry(tool_id.clone()).or_insert(0) += 1;

        let deadline = std::time::Instant::now() + self.budgets.tool_call_timeout;
        let (mut invocation, _opened) = ToolInvocation::open(
            invocation_id.clone(),
            self.run_id.clone(),
            tool_id.clone(),
            arguments.clone(),
            self.actor.clone(),
            deadline,
            journal,
        )?;

        let requested_path = requested_path_for(&self.registry, &tool_id, &arguments);
        let decision = {
            let engine = PolicyEngine::new(&self.registry, &self.allowlist);
            engine.authorize(&PolicyRequest {
                actor: Some(&self.actor),
                invocation_id: &invocation_id,
                tool_id: &tool_id,
                arguments: &arguments,
                run_budget_remaining: run_remaining,
                tool_budget_remaining: tool_remaining,
                active_project: self.active_project.as_ref(),
                requested_path: requested_path.as_deref(),
                path_authority,
                skill_scope: self.skill_scope.as_ref(),
                approval: None,
            })
        };

        match decision {
            PolicyDecision::Allow(_grant) => {
                invocation.approve(journal)?;
                invocation.start(journal)?;
                Ok(Prepared::Ready {
                    tool_id,
                    arguments,
                    invocation,
                })
            }
            PolicyDecision::RequireApproval(request) => {
                // Standing consent: the user has already said "stop asking
                // about this tool" (session or always scope). Mint the same
                // invocation-scoped token a click would have — the journal
                // and audit record a normal approval — and proceed instead
                // of parking the whole run.
                let standing_allow = self.standing_approvals.as_ref().is_some_and(|standing| {
                    standing.allows(self.actor.subject(), tool_id.as_str())
                });
                if standing_allow {
                    let _token = request.approve();
                    invocation.approve(journal)?;
                    invocation.start(journal)?;
                    self.publish_tool_audit(
                        AuditAction::ToolApprove,
                        AuditOutcome::Approved,
                        &invocation_id,
                    )
                    .await;
                    return Ok(Prepared::Ready {
                        tool_id,
                        arguments,
                        invocation,
                    });
                }
                invocation.require_approval(journal)?;
                self.pending_approval.insert(
                    invocation_id.clone(),
                    PendingApproval {
                        invocation,
                        request,
                    },
                );
                self.publish_tool_audit(
                    AuditAction::ToolApprove,
                    AuditOutcome::ApprovalRequired,
                    &invocation_id,
                )
                .await;
                Ok(Prepared::Done(ToolCallOutcome::AwaitingApproval {
                    invocation_id,
                    tool_id,
                }))
            }
            PolicyDecision::Deny(denial) => {
                invocation.deny(&denial, journal)?;
                self.terminal.insert(
                    invocation_id.clone(),
                    (tool_id.clone(), TerminalOutcome::Denied(denial.code())),
                );
                self.publish_tool_audit(
                    AuditAction::ToolDeny,
                    AuditOutcome::Denied,
                    &invocation_id,
                )
                .await;
                Ok(Prepared::Done(ToolCallOutcome::Denied {
                    invocation_id,
                    tool_id,
                    reason_code: denial.code(),
                }))
            }
        }
    }

    async fn finalize(
        &mut self,
        mut invocation: ToolInvocation,
        result: Result<ToolResult, ToolExecutionError>,
        journal: &mut dyn InvocationJournal,
    ) -> ToolCallOutcome {
        let invocation_id = invocation.invocation_id().clone();
        let tool_id = invocation.tool_id().clone();
        match result {
            Ok(tool_result) => {
                let truncated = tool_result.truncated();
                if invocation.complete(truncated, journal).is_err() {
                    return ToolCallOutcome::Failed {
                        invocation_id,
                        message: "failed to journal tool completion".to_owned(),
                        retryable: true,
                    };
                }
                let cached = tool_result.clone();
                self.terminal.insert(
                    invocation_id.clone(),
                    (tool_id, TerminalOutcome::Completed(cached)),
                );
                self.publish_tool_audit(
                    AuditAction::ToolExecute,
                    AuditOutcome::Success,
                    &invocation_id,
                )
                .await;
                ToolCallOutcome::Completed {
                    invocation_id,
                    result: tool_result,
                }
            }
            // Both messages reach the model as the tool result and the reader
            // as the failed tool row, so each says what happened and what to
            // do next. A timed-out query run again unchanged times out again.
            Err(ToolExecutionError::Timeout) => {
                self.finalize_failure(
                    &mut invocation,
                    "The tool call did not finish within its time budget and was stopped. \
                     Do not repeat it unchanged: narrow it (fewer rows, a shorter date range, \
                     fewer columns) or tell the user it timed out.",
                    true,
                    journal,
                )
                .await
            }
            Err(ToolExecutionError::Disconnected) => {
                self.finalize_failure(
                    &mut invocation,
                    "The connection to the tool's server dropped before it answered. \
                     The connection is reopened on the next call, so it can be tried once more.",
                    true,
                    journal,
                )
                .await
            }
            Err(ToolExecutionError::RateLimited { retry_after }) => {
                let wait = match retry_after {
                    Some(wait) => format!("about {} seconds", wait.as_secs().max(1)),
                    None => "a minute".to_owned(),
                };
                self.finalize_failure(
                    &mut invocation,
                    &format!(
                        "The tool's server is limiting how many requests this account can make. \
                         Wait {wait} before calling it again, and make fewer calls to it at once. \
                         Tell the user if the answer cannot wait."
                    ),
                    true,
                    journal,
                )
                .await
            }
            Err(ToolExecutionError::Cancelled) => {
                // Mirrors the `Ok(tool_result)` branch's own "check the
                // journal write's Result, don't just assume it landed"
                // discipline above (a review finding caught this arm still
                // using `let _ =` and unconditionally recording an
                // optimistic `Cancelled` terminal outcome even when the
                // journal write itself failed).
                if invocation.cancel(journal).is_err() {
                    return ToolCallOutcome::Failed {
                        invocation_id,
                        message: "failed to journal tool cancellation".to_owned(),
                        retryable: true,
                    };
                }
                self.terminal
                    .insert(invocation_id.clone(), (tool_id, TerminalOutcome::Cancelled));
                self.publish_tool_audit(
                    AuditAction::ToolExecute,
                    AuditOutcome::Failure,
                    &invocation_id,
                )
                .await;
                ToolCallOutcome::Cancelled { invocation_id }
            }
            // Not retryable: the same credential would be refused identically,
            // and the user has been prompted to replace it. Telling the model
            // *why* the tool is unavailable is what lets it say so instead of
            // reporting that it has no such capability at all — the failure
            // this whole path exists to end. The label is ours (a row's display
            // name), not server-supplied text, and `finalize_failure` bounds it.
            Err(ToolExecutionError::AuthorizationRefused { label }) => {
                self.authorization_refused = true;
                self.finalize_failure(
                    &mut invocation,
                    &format!(
                        "{label} refused its saved sign-in, so its tools cannot run until it is reconnected"
                    ),
                    false,
                    journal,
                )
                .await
            }
            Err(ToolExecutionError::ResultTooLarge {
                actual_bytes,
                max_bytes,
            }) => {
                self.finalize_failure(
                    &mut invocation,
                    &format!(
                        "tool result is {actual_bytes} bytes; maximum exact result size is {max_bytes} bytes"
                    ),
                    false,
                    journal,
                )
                .await
            }
            Err(ToolExecutionError::Failed(message)) => {
                self.finalize_failure(&mut invocation, &message, false, journal)
                    .await
            }
            Err(ToolExecutionError::Recoverable(message)) => {
                self.finalize_failure(&mut invocation, &message, true, journal)
                    .await
            }
        }
    }

    async fn finalize_failure(
        &mut self,
        invocation: &mut ToolInvocation,
        message: &str,
        retryable: bool,
        journal: &mut dyn InvocationJournal,
    ) -> ToolCallOutcome {
        let invocation_id = invocation.invocation_id().clone();
        let tool_id = invocation.tool_id().clone();
        // Truncate *before* calling `invocation.fail` — an untrusted,
        // unbounded tool/MCP error string could otherwise exceed
        // `MAX_FAILURE_MESSAGE_BYTES` and make `fail` return
        // `Err(FailureMessageTooLarge)` *without journaling anything at
        // all* (a review finding caught this), which the code below used to
        // ignore via `let _ =` and record an optimistic terminal outcome
        // the durable journal never actually recorded.
        let bounded_message = truncate_to_byte_boundary(message, MAX_FAILURE_MESSAGE_BYTES);
        // The tool and whether it can be retried; the message may carry text
        // from a server or a file, so it stays out of the log.
        tracing::info!(tool = %tool_id.as_str(), retryable, "tool call failed");
        if invocation
            .fail(&bounded_message, retryable, journal)
            .is_err()
        {
            return ToolCallOutcome::Failed {
                invocation_id,
                message: "failed to journal tool failure".to_owned(),
                retryable: true,
            };
        }
        self.terminal.insert(
            invocation_id.clone(),
            (
                tool_id,
                TerminalOutcome::Failed {
                    message: bounded_message.clone(),
                    retryable,
                },
            ),
        );
        self.publish_tool_audit(
            AuditAction::ToolExecute,
            AuditOutcome::Failure,
            &invocation_id,
        )
        .await;
        ToolCallOutcome::Failed {
            invocation_id,
            message: bounded_message,
            retryable,
        }
    }

    /// Keeps the first tool id seen for an invocation. A reused id naming a
    /// different tool is refused in `prepare`, and its audit detail stays the
    /// tool that first owned the invocation.
    fn remember_tool(&mut self, invocation_id: &InvocationId, tool_id: &ToolId) {
        self.invocation_tools
            .entry(invocation_id.clone())
            .or_insert_with(|| tool_id.clone());
    }

    /// Best-effort, content-free audit publication. Never blocks or alters
    /// a tool call's own outcome — matching
    /// `crate::approvals::service::ApprovalsService::sync_da`'s precedent.
    /// Delegates to the shared `crate::audit::AuditPublisher` (identifier
    /// sanitization, publish-or-spool, permanent-rejection handling — see
    /// that type's own module doc comment for the full rationale). The tool
    /// id rides along as the event detail when the loop has seen it.
    async fn publish_tool_audit(
        &mut self,
        action: AuditAction,
        outcome: AuditOutcome,
        invocation_id: &InvocationId,
    ) {
        let detail = self
            .invocation_tools
            .get(invocation_id)
            .map(|tool_id| tool_id.as_str().to_owned());
        let Some(audit) = self.audit.as_mut() else {
            return;
        };
        match detail {
            Some(detail) => {
                audit
                    .publisher
                    .publish_with_detail(
                        action,
                        AuditResourceType::ToolCall,
                        invocation_id.as_str(),
                        outcome,
                        "tools.policy.v1",
                        self.run_id.as_str(),
                        &detail,
                    )
                    .await;
            }
            None => {
                audit
                    .publisher
                    .publish(
                        action,
                        AuditResourceType::ToolCall,
                        invocation_id.as_str(),
                        outcome,
                        "tools.policy.v1",
                        self.run_id.as_str(),
                    )
                    .await;
            }
        }
    }
}

fn outcome_from_terminal(
    invocation_id: InvocationId,
    tool_id: ToolId,
    terminal: &TerminalOutcome,
) -> ToolCallOutcome {
    match terminal {
        TerminalOutcome::Completed(result) => ToolCallOutcome::Completed {
            invocation_id,
            result: result.clone(),
        },
        TerminalOutcome::InvalidArguments(reason) => ToolCallOutcome::InvalidArguments {
            invocation_id,
            tool_id,
            reason: *reason,
        },
        TerminalOutcome::Denied(reason_code) => ToolCallOutcome::Denied {
            invocation_id,
            tool_id,
            reason_code,
        },
        TerminalOutcome::Failed { message, retryable } => ToolCallOutcome::Failed {
            invocation_id,
            message: message.clone(),
            retryable: *retryable,
        },
        TerminalOutcome::Cancelled => ToolCallOutcome::Cancelled { invocation_id },
    }
}

/// Truncates `message` to at most `max_bytes` bytes at a valid UTF-8
/// character boundary — mirrors `agent::context::ContextMessage::tool_result`'s
/// own "truncate at a char boundary, never split a multi-byte character"
/// discipline one layer over.
fn truncate_to_byte_boundary(message: &str, max_bytes: usize) -> String {
    if message.len() <= max_bytes {
        return message.to_owned();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !message.is_char_boundary(boundary) {
        boundary -= 1;
    }
    message[..boundary].to_owned()
}

/// Validates `arguments` against `descriptor`'s own advertised JSON schema
/// using the same pinned `jsonschema` validator this codebase already uses
/// for the bundled MCP catalog (`crate::mcp::catalog::McpToolCatalog::validate_arguments`).
/// A descriptor whose own schema is not a valid compilable shape is treated
/// as a validation failure here too — `PolicyEngine::authorize` denies that
/// case independently via `ToolDescriptor::has_valid_schema`
/// (`PolicyDenial::InvalidAdvertisedSchema`), so this is defense in depth,
/// not the only place it is caught.
fn arguments_match_schema(descriptor: &ToolDescriptor, arguments: &Value) -> bool {
    match jsonschema::validator_for(descriptor.input_schema()) {
        Ok(validator) => validator.is_valid(arguments),
        Err(_) => false,
    }
}

/// Derives the path a trusted-path-requiring tool call names, from its own
/// validated arguments' top-level `"path"` string field
/// (`crate::tools::registry::project_read_tool_descriptors`' own convention
/// for `project.read_file`/`project.list_directory`). `project.search_files`/
/// `project.search_text` carry the same
/// `crate::tools::registry::CAPABILITY_REQUIRES_TRUSTED_PATH` tag but have
/// no `path` argument at all (they search the whole project) — for those,
/// this resolves to the project's own root (an empty relative path) rather
/// than `None`, so `PolicyEngine::authorize`'s `PathRequired` check does not
/// spuriously deny a whole-project search. This relies on
/// `crate::projects::domain::TrustedRoot::contains` treating an empty
/// relative path as "the trusted root itself" (see that method's own doc
/// comment) — a review finding caught an earlier version of `contains`
/// routing the empty path through `RelativeProjectPath`/
/// `resolve_existing_str`, which always denies an empty string, silently
/// turning this fix into an equally spurious `PathNotTrusted` denial for
/// every whole-project search once wired to a real `TrustedRoot`. Covered
/// against the real descriptors from `project_read_tool_descriptors` in
/// `agent::tests::tool_loop::a_whole_project_search_tool_is_allowed_via_the_trusted_roots_own_empty_path_case`,
/// not just the test-local `project_path_descriptor` fixture.
fn requested_path_for(
    registry: &ToolRegistry,
    tool_id: &ToolId,
    arguments: &Value,
) -> Option<PathBuf> {
    let descriptor = registry.get(tool_id)?;
    if !descriptor.requires_trusted_path() {
        return None;
    }
    Some(match arguments.get("path").and_then(Value::as_str) {
        Some(path) => PathBuf::from(path),
        None => PathBuf::new(),
    })
}
