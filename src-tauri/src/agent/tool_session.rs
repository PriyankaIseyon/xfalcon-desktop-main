//! Everything one tool-capable run needs, assembled in one place.
//!
//! The pieces this joins were each built with a documented "no live caller
//! yet" seam: `crate::tools::registry`'s descriptors, `crate::tools::policy`'s
//! authorization, `crate::agent::tool_loop::ToolLoop`'s orchestration,
//! `crate::tools::dispatch`'s execution. This module is the composition root
//! that makes them one running system, and it is deliberately the *only*
//! place that knows how they fit together — `crate::chat::service` asks for a
//! session and drives it, without assembling anything itself.
//!
//! # When a run is tool-capable
//!
//! Whenever *something* is registrable: the conversation's bound trusted
//! project contributes the `project.*` tools, and every reachable MCP server contributes whatever
//! it publishes. Either alone is enough. With neither, the effective registry
//! is empty. The run still uses the same turn driver with an empty declaration
//! set, so context compaction and durable recovery have one state machine.
//!
//! # Four sources, one executor
//!
//! Project tools run locally against a trusted project; MCP tools run
//! server-side; `__internal__get_skill` reads this device's imported skill
//! library; `__internal__read_tool_result` reads only exact payloads admitted
//! by the run's authoritative DA context. [`CompositeToolExecutor`] routes
//! each call to the right one by tool id, so `ToolLoop` — which owns policy,
//! budgets, and journaling for all four — never has to care where a tool came
//! from.
//!
//! An MCP server that is unreachable degrades to the tools that *are*
//! reachable rather than failing the run: the user asked a question, not for an
//! MCP session, and a chat that refuses to answer because a background
//! connection is down is worse than one that answers with fewer tools.
//! `crate::mcp::runtime::McpRuntimeSet::register_tools` isolates that per
//! server, so one dead registration cannot take the tenant workspace's tools
//! with it.
//!
//! # Wire names are not tool ids
//!
//! OpenAI and Anthropic both constrain a function name to
//! `^[a-zA-Z0-9_-]{1,64}$`. This app's ids are dotted and namespaced
//! (`project.read_file`, `mcp.<server>.<tool>`), and an MCP server may publish
//! something neither accepts. [`wire_tool_name`] produces the alias and
//! [`ToolSession::tool_ids`] is the authoritative way back — never string
//! surgery on the alias, which two different ids can collide into.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::tools::ToolId;
use xfalcon_contracts::{ConversationId, RunId};

use crate::approvals::ApprovalsService;
use crate::audit::AuditPublisher;
use crate::mcp::dispatch::{McpToolDispatcher, ReauthNotifier};
use crate::mcp::runtime::{McpRuntimeSet, ServerContribution, MCP_TOOL_ID_PREFIX};
use crate::projects::{ProjectsService, SecretClassifier};
use crate::providers::ProviderToolSchema;
use crate::reports::origin::ReportOriginRecorder;
use crate::security::account_cleanup::ValidatedAccountId;
use crate::skills::dispatch::SkillToolDispatcher;
use crate::skills::store::SkillStore;
use crate::tools::dispatch::{
    ProjectToolContext, ProjectToolDispatcher, TOOL_SEARCH_FILES, TOOL_SEARCH_TEXT,
};
use crate::tools::exact_result::ExactResultToolExecutor;
use crate::tools::invocation::ToolResult;
use crate::tools::policy::{Actor, PathAuthority, ProjectContext, SkillToolScope};
use crate::tools::registry::{
    get_skill_tool_descriptor, project_propose_patch_tool_descriptor,
    project_read_tool_descriptors, project_update_plan_tool_descriptor,
    read_tool_result_tool_descriptor, ToolRegistry, GET_SKILL_TOOL_ID, READ_TOOL_RESULT_TOOL_ID,
};

use super::tool_loop::{
    effective_registry_for_run, ToolExecutionError, ToolExecutorPort, ToolLoop, ToolLoopAudit,
    ToolLoopBudgets,
};

/// Longest function name OpenAI and Anthropic accept.
const MAX_WIRE_TOOL_NAME_BYTES: usize = 64;

/// One run's assembled tool subsystem.
pub(crate) struct ToolSession {
    pub tool_loop: ToolLoop,
    /// Executes an authorized call. A trait object because the run that owns
    /// it does not care which tool sources are composed behind it.
    pub executor: Arc<dyn ToolExecutorPort>,
    /// The trusted root, as the `PathAuthority` every path-taking policy
    /// decision consults. Kept alive for the run: holding it is also proof the
    /// project's root identity was still valid when the run started.
    ///
    /// `None` for an MCP-only session. No path-taking tool is registered
    /// without a project, and `PolicyEngine::authorize` independently denies
    /// any `requires_trusted_path` tool whose authority is absent
    /// (`PolicyDenial::PathRequired`) — so the absence fails closed on its own
    /// rather than needing a stand-in that claims to contain nothing.
    pub path_authority: Option<Arc<dyn PathAuthority>>,
    /// Whether the effective provider registry contains project tools. This
    /// also gates project-specific prompt guidance so an unavailable root
    /// cannot leave the model instructed to call tools that were withheld.
    pub has_project_tools: bool,
    /// Declarations to send the provider.
    pub tools: Vec<ProviderToolSchema>,
    /// Wire name back to [`ToolId`] — see the module doc comment.
    pub tool_ids: HashMap<String, ToolId>,
    /// Servers this run holds no tools for because they could not be reached.
    ///
    /// Named, not counted: the model is told so it can say *which* server is
    /// down. Without this the tools simply were not declared, and a model asked
    /// to use one answered that it has no such capability — which reads as a
    /// missing feature rather than an outage that will pass.
    pub unavailable_mcp_servers: Vec<String>,
}

/// Inputs a caller must supply to build a session.
pub(crate) struct ToolSessionRequest {
    pub projects: Arc<ProjectsService>,
    pub approvals: Arc<ApprovalsService>,
    /// Every MCP server this run may draw tools from. `None` skips MCP entirely
    /// rather than failing.
    pub mcp: Option<Arc<McpRuntimeSet>>,
    pub account: ValidatedAccountId,
    pub conversation_id: ConversationId,
    pub run_id: RunId,
    /// The run's tool-result budget, from its model's context window. Every
    /// tool that pages — files, directories, skills, exact results — sizes its
    /// page to it.
    pub result_budget: crate::tools::ToolResultBudget,
    /// Narrows the registry further when a skill is active. `None` means "no
    /// skill selected", never "allow everything" — the registry composed
    /// below is already the ceiling.
    pub skill_scope: Option<SkillToolScope>,
    /// Standing "allow for session"/"always allow" grants, consulted where a
    /// mutating call would otherwise park. `None` keeps ask-every-time.
    pub standing_approvals: Option<std::sync::Arc<crate::tools::standing::StandingToolApprovals>>,
    /// This device's imported skill library. Contributes
    /// `__internal__get_skill` when it holds at least one enabled skill —
    /// declaring a fetch tool with nothing to fetch spends the model's
    /// attention for nothing.
    pub skills: Arc<SkillStore>,
    /// Where "this MCP server refused its sign-in" is announced. `None` runs
    /// without the prompt — the refusal is still classified and still terminal,
    /// the user simply is not asked. Optional so a test can build a session
    /// without a window to emit into.
    pub reauth: Option<Arc<dyn ReauthNotifier>>,
    /// Exact payload access admitted from this run's authoritative context.
    /// `None` means the range tool is neither registered nor executable.
    pub exact_result: Option<ExactResultToolExecutor>,
    /// Where this run's tool decisions are audited. `None` runs unaudited.
    pub audit: Option<AuditPublisher>,
}

/// Builds the run's tool subsystem. An empty registry is an ordinary session:
/// the provider receives no tool declarations, while the run keeps the same
/// durable context and compaction state machine as a tool-capable session.
///
/// A project is optional and MCP is optional, independently. The tenant
/// workspace's own MCP server needs no local project to answer a question about
/// the tenant's data, and gating it behind one would put this build's chat
/// behind xfalcon-web's for no reason a user could see.
pub(crate) async fn build_tool_session(request: ToolSessionRequest) -> ToolSession {
    let ToolSessionRequest {
        projects,
        approvals,
        mcp,
        account,
        conversation_id,
        run_id,
        result_budget,
        skill_scope,
        standing_approvals,
        skills,
        reauth,
        exact_result,
        audit,
    } = request;

    // Reopening proves the root's identity is still what it was when the
    // project was trusted. A project whose root moved or was swapped since
    // activation must not silently become tool-reachable — and it must not take
    // the MCP tools down with it either, so an unusable project degrades to an
    // MCP-only session rather than to no session.
    let bound_project = projects
        .project_for_conversation(&account, &conversation_id)
        .unwrap_or_else(|error| {
            // Failing closed here is correct, but doing it silently would make
            // a bound chat that quietly runs without its folder indistinguishable
            // from an ordinary one. The user sees the degraded run either way;
            // this is what makes it diagnosable.
            tracing::warn!(%error, "run: local project binding unreadable; running without project tools");
            None
        });
    let project = bound_project.and_then(|project_id| {
        match projects.trusted_root(&account, project_id) {
            Ok(trusted_root) => Some((project_id, trusted_root)),
            Err(error) => {
                tracing::warn!(%error, "run: bound project root unavailable; withholding project tools");
                None
            }
        }
    });
    let search_ready = match project.as_ref() {
        Some((project_id, _)) => match projects.refresh_index(&account, *project_id).await {
            Ok(_) => true,
            Err(error) => {
                // Exact reads and directory listing do not depend on the index,
                // so the run keeps them; only the two search tools are withheld.
                tracing::warn!(%error, "run: project index refresh failed; withholding search tools");
                false
            }
        },
        None => false,
    };

    // `project.*` tools are registered only with a project open. Declaring one
    // the policy engine would deny (`PolicyDenial::ProjectRequired`) spends the
    // model's attention on a tool it cannot use.
    let mut global = match project {
        Some(_) => internal_tool_registry(search_ready),
        None => ToolRegistry::new(),
    };
    // Independent of the project: a skill is a document this app stored
    // itself, so a run with skills and nothing else is still tool-capable.
    let has_skills = skills.enabled().is_ok_and(|enabled| !enabled.is_empty());
    if has_skills {
        if let Ok(descriptor) = get_skill_tool_descriptor() {
            let _ = global.register(descriptor);
        }
    }
    let exact_result = exact_result.filter(ExactResultToolExecutor::is_available);
    if exact_result.is_some() {
        if let Ok(descriptor) = read_tool_result_tool_descriptor(result_budget) {
            let _ = global.register(descriptor);
        }
    }
    let mut unavailable_mcp_servers = Vec::new();
    let mcp = match mcp {
        Some(servers) => {
            let contributions = register_mcp_tools(&mut global, &servers).await;
            unavailable_mcp_servers = contributions
                .iter()
                .filter(|(_, contribution)| matches!(contribution, ServerContribution::Unreachable))
                // The row's display name, so the model names the server the way
                // Settings does. The bundled server carries no DA row and falls
                // back to its slug.
                .map(|(slug, _)| {
                    servers
                        .find(slug)
                        .and_then(|runtime| runtime.identity())
                        .map_or_else(|| slug.clone(), |identity| identity.label.clone())
                })
                .collect();
            contributions
                .iter()
                .any(|(_, contribution)| matches!(contribution, ServerContribution::Served(_)))
                .then_some(servers)
        }
        None => None,
    };
    // The allowlist is the composed registry itself: this app has no per-run
    // tool-selection surface yet, so the narrowing levers that exist are the
    // registry composition above (project tools only when a project is open)
    // and `skill_scope`. Wiring a real per-conversation allowlist to a UI is
    // a separate change; `PolicyEngine` already enforces whatever is passed
    // here, so that change is additive rather than structural.
    let allowlist: HashSet<ToolId> = global
        .iter()
        .map(|descriptor| descriptor.id().clone())
        .collect();
    let registry = effective_registry_for_run(&global, &allowlist, skill_scope.as_ref());
    let has_project_tools = registry
        .iter()
        .any(|descriptor| descriptor.id().as_str().starts_with("project."));
    let (tools, tool_ids) = declare(&registry);

    // `Actor::new` only rejects an empty or over-long subject, and a
    // `ValidatedAccountId` is neither by construction.
    let actor = Actor::new(account.as_str())
        .expect("ValidatedAccountId always satisfies the Actor contract");
    let mut tool_loop = ToolLoop::new(
        registry,
        allowlist,
        actor,
        run_id.clone(),
        project
            .as_ref()
            .map(|(project_id, _)| ProjectContext::new(project_id.to_string())),
        skill_scope,
        ToolLoopBudgets::production(),
    );
    if let Some(standing) = standing_approvals {
        tool_loop = tool_loop.with_standing_approvals(standing);
    }
    if let Some(publisher) = audit {
        tool_loop = tool_loop.with_audit(ToolLoopAudit::from_publisher(publisher));
    }

    // Only a project-bound run can link the reports it saves to a project; a
    // regular chat carries no recorder, so a save in it can never be linked.
    let report_origin = project.as_ref().map(|(project_id, _)| {
        ReportOriginRecorder::new(
            projects.clone(),
            account.clone(),
            *project_id,
            conversation_id.clone(),
        )
    });
    let (project_dispatcher, path_authority) = match project {
        Some((project_id, trusted_root)) => {
            let dispatcher = ProjectToolDispatcher::new(ProjectToolContext {
                projects,
                approvals,
                classifier: Arc::new(SecretClassifier::built_in()),
                account,
                project_id,
                run_id,
                budget: result_budget,
            });
            let authority: Arc<dyn PathAuthority> = Arc::new(trusted_root);
            (Some(dispatcher), Some(authority))
        }
        None => (None, None),
    };
    let executor: Arc<dyn ToolExecutorPort> = Arc::new(CompositeToolExecutor {
        project: project_dispatcher,
        report_origin,
        mcp: mcp.map(|servers| {
            let dispatcher = McpToolDispatcher::new(servers);
            match reauth {
                Some(notifier) => dispatcher.with_reauth_notifier(notifier),
                None => dispatcher,
            }
        }),
        skills: has_skills.then(|| SkillToolDispatcher::new(skills, result_budget)),
        exact_result,
    });

    ToolSession {
        tool_loop,
        executor,
        path_authority,
        has_project_tools,
        tools,
        tool_ids,
        unavailable_mcp_servers,
    }
}

/// Adds whatever every held MCP server publishes to `registry`, and reports what
/// each one contributed.
///
/// The per-server outcome used to be collapsed to a single bool here, which
/// discarded the one fact a reader needs when a server is down: its name. The
/// caller decides both questions from the returned list — whether the
/// dispatcher will have anything to route, and which servers to name as
/// unavailable.
pub(crate) async fn register_mcp_tools(
    registry: &mut ToolRegistry,
    servers: &McpRuntimeSet,
) -> Vec<(String, ServerContribution)> {
    servers.register_tools(registry).await
}

/// Routes each call to the source that owns the tool.
///
/// `ToolLoop` authorized it against one registry and does not know the
/// difference; this is the only place that does.
struct CompositeToolExecutor {
    /// `None` when no project is open. Nothing routed here is registered in
    /// that state, so reaching it at all is a composition bug — reported as a
    /// terminal execution failure rather than silently answered.
    project: Option<ProjectToolDispatcher>,
    /// Present only in a project-bound run. Links a report the assistant saves
    /// to that project once DA has answered.
    report_origin: Option<ReportOriginRecorder>,
    mcp: Option<McpToolDispatcher>,
    /// `None` when the library holds no enabled skill, which is the same
    /// state in which `__internal__get_skill` was never registered.
    skills: Option<SkillToolDispatcher>,
    /// Present only when the run's frozen context admitted ready payloads.
    exact_result: Option<ExactResultToolExecutor>,
}

#[async_trait]
impl ToolExecutorPort for CompositeToolExecutor {
    fn requires_completion(&self, tool_id: &ToolId) -> bool {
        self.project
            .as_ref()
            .is_some_and(|project| project.requires_completion(tool_id))
    }

    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        if tool_id.as_str() == READ_TOOL_RESULT_TOOL_ID {
            let Some(exact_result) = self.exact_result.as_ref() else {
                return Err(ToolExecutionError::Failed(
                    "No exact tool-result payload is admitted for this run.".to_owned(),
                ));
            };
            return exact_result.execute(tool_id, arguments, cancel).await;
        }
        if tool_id.as_str() == GET_SKILL_TOOL_ID {
            let Some(skills) = self.skills.as_ref() else {
                return Err(ToolExecutionError::Failed(
                    "No skills are available on this device.".to_owned(),
                ));
            };
            return skills.execute(tool_id, arguments, cancel).await;
        }
        if tool_id.as_str().starts_with(MCP_TOOL_ID_PREFIX) {
            let Some(mcp) = self.mcp.as_ref() else {
                // Registered from a catalog that is no longer reachable. A
                // terminal failure, not a retry: the session this run was
                // composed with is gone.
                return Err(ToolExecutionError::Disconnected);
            };
            let result = mcp.execute(tool_id, arguments, cancel).await?;
            if let Some(recorder) = self.report_origin.as_ref() {
                recorder.observe(tool_id.as_str(), arguments, &result);
            }
            return Ok(result);
        }
        let Some(project) = self.project.as_ref() else {
            return Err(ToolExecutionError::Failed(
                "This tool needs an open project.".to_owned(),
            ));
        };
        project.execute(tool_id, arguments, cancel).await
    }
}

/// Every tool this app itself provides.
///
/// A descriptor that fails to build is a programmer error in a compile-time
/// constant schema, not a runtime condition — but it must not take the whole
/// run down either, so it is skipped and the rest still register.
fn internal_tool_registry(search_ready: bool) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    if let Ok(descriptors) = project_read_tool_descriptors() {
        for descriptor in descriptors {
            if !search_ready
                && matches!(
                    descriptor.id().as_str(),
                    TOOL_SEARCH_FILES | TOOL_SEARCH_TEXT
                )
            {
                continue;
            }
            let _ = registry.register(descriptor);
        }
    }
    if let Ok(descriptor) = project_propose_patch_tool_descriptor() {
        let _ = registry.register(descriptor);
    }
    if let Ok(descriptor) = project_update_plan_tool_descriptor() {
        let _ = registry.register(descriptor);
    }
    registry
}

/// Turns the effective registry into provider declarations plus the reverse
/// name map.
fn declare(registry: &ToolRegistry) -> (Vec<ProviderToolSchema>, HashMap<String, ToolId>) {
    let mut tools = Vec::with_capacity(registry.len());
    let mut tool_ids = HashMap::with_capacity(registry.len());
    for descriptor in registry.iter() {
        let name = unique_wire_name(descriptor.id(), &tool_ids);
        tool_ids.insert(name.clone(), descriptor.id().clone());
        tools.push(ProviderToolSchema {
            name,
            description: descriptor.description().to_owned(),
            input_schema: descriptor.input_schema().clone(),
        });
    }
    (tools, tool_ids)
}

/// A provider-legal alias for `tool_id`, disambiguated against names already
/// taken.
///
/// Two distinct ids can sanitize to the same alias (`a.b` and `a_b` both
/// become `a_b`). Silently letting the second overwrite the first would make
/// one tool uncallable and route its calls to the other — so a collision
/// takes a numeric suffix instead.
pub(crate) fn unique_wire_name(tool_id: &ToolId, taken: &HashMap<String, ToolId>) -> String {
    let base = wire_tool_name(tool_id);
    if !taken.contains_key(&base) {
        return base;
    }
    for suffix in 2..=99u32 {
        let candidate = with_suffix(&base, suffix);
        if !taken.contains_key(&candidate) {
            return candidate;
        }
    }
    base
}

fn with_suffix(base: &str, suffix: u32) -> String {
    let tail = format!("_{suffix}");
    let mut head = base.to_owned();
    head.truncate(MAX_WIRE_TOOL_NAME_BYTES.saturating_sub(tail.len()));
    head.push_str(&tail);
    head
}

/// Maps a [`ToolId`] to the provider function-name charset.
///
/// Every byte outside `[A-Za-z0-9_-]` becomes `_`, and the result is capped at
/// [`MAX_WIRE_TOOL_NAME_BYTES`]. Truncation operates on bytes, which is safe
/// here because every retained byte is ASCII by construction.
pub(crate) fn wire_tool_name(tool_id: &ToolId) -> String {
    let mut name: String = tool_id
        .as_str()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    name.truncate(MAX_WIRE_TOOL_NAME_BYTES);
    if name.is_empty() {
        // A provider rejects an empty name outright; an id that sanitizes to
        // nothing is pathological, but it still needs *some* callable alias.
        name.push('_');
    }
    name
}
