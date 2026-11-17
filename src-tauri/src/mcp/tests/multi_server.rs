//! Several servers at once: whose tools a run gets, under whose namespace, and
//! what happens when one of them is down.
//!
//! Every server here is a real `rmcp` Streamable HTTP server
//! (`super::support::FixtureServer`), so a "server is down" case is a genuinely
//! dead endpoint rather than a mocked error.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::mcp::{McpPermissionMode, BUNDLED_SERVER_ID};
use xfalcon_contracts::tools::{ToolId, ToolRisk};
use zeroize::Zeroizing;

use super::support::{FixtureServer, FixtureState};
use crate::agent::tool_loop::ToolExecutorPort;
use crate::mcp::auth::{McpAuth, McpAuthError, McpAuthProvider};
use crate::mcp::dispatch::McpToolDispatcher;
use crate::mcp::runtime::{
    ExcludedServer, McpRuntime, McpRuntimeSet, ServerContribution, ServerExclusion,
    SessionInvalidation, MAX_CONCURRENT_SERVER_SESSIONS,
};
use crate::mcp::server_resolver::McpServerDescriptor;
use crate::tools::registry::ToolRegistry;

/// Accepts whatever the fixture is handed; the credential path itself is proven
/// in `super::credential_store` and `crate::mcp::auth`'s own tests.
struct StaticAuth;

#[async_trait::async_trait]
impl McpAuthProvider for StaticAuth {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        McpAuth::bind(
            Zeroizing::new("fixture-bearer".to_owned()),
            server.da_origin().as_str(),
            server,
        )
    }
}

fn runtime(slug: &str, mcp_url: &str, mode: McpPermissionMode) -> Arc<McpRuntime> {
    let endpoint = reqwest::Url::parse(mcp_url).expect("valid fixture url");
    Arc::new(McpRuntime::new(
        slug.to_owned(),
        mode,
        McpServerDescriptor::registered(endpoint),
        Arc::new(StaticAuth),
    ))
}

/// A server on a port nothing is listening on. Bound and immediately released,
/// so the address is real and refuses connections rather than hanging.
async fn dead_endpoint() -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let address = listener.local_addr().expect("local addr");
    drop(listener);
    format!("http://{address}/mcp")
}

async fn spawn_server() -> (String, FixtureState, super::support::ServerGuard) {
    let state = FixtureState::default();
    let (url, guard) = FixtureServer::new(0, state.clone()).spawn().await;
    (url, state, guard)
}

#[tokio::test]
async fn runtime_invalidation_reconnects_and_refreshes_the_same_servers_schema() {
    let (url, state, _guard) = spawn_server().await;
    let runtime = runtime("changing", &url, McpPermissionMode::Ask);
    let cancel = CancellationToken::new();

    let (first_session, first_catalog) = runtime
        .connected(&cancel)
        .await
        .expect("first connection and catalog");
    assert!(first_catalog
        .validate_arguments("echo", &json!({"text": "before"}))
        .is_ok());

    state.require_number_for_echo();
    assert_eq!(
        runtime.invalidate(&first_session).await,
        SessionInvalidation::LatestGeneration
    );
    let (second_session, second_catalog) = runtime
        .connected(&cancel)
        .await
        .expect("fresh connection and catalog");

    assert!(!Arc::ptr_eq(&first_session, &second_session));
    assert!(!Arc::ptr_eq(&first_catalog, &second_catalog));
    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 2);
    assert!(second_catalog
        .validate_arguments("echo", &json!({"text": 7}))
        .is_ok());
    assert!(second_catalog
        .validate_arguments("echo", &json!({"text": "stale"}))
        .is_err());

    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !second_session.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the session observes its lifetime cancellation");
    let (third_session, third_catalog) = runtime
        .connected(&CancellationToken::new())
        .await
        .expect("closed session reconnects without reusing its catalog");
    assert!(!Arc::ptr_eq(&second_session, &third_session));
    assert!(!Arc::ptr_eq(&second_catalog, &third_catalog));
    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 3);
    assert!(third_catalog
        .validate_arguments("echo", &json!({"text": 7}))
        .is_ok());
}

struct CountingAuth(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl McpAuthProvider for CountingAuth {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        McpAuth::bind(
            Zeroizing::new("fixture-bearer".to_owned()),
            server.da_origin().as_str(),
            server,
        )
    }
}

#[tokio::test]
async fn invalidation_and_reconnect_authentication_do_not_wait_for_catalog_refresh() {
    let (url, state, _guard) = spawn_server().await;
    let auth_calls = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(McpRuntime::new(
        "changing".to_owned(),
        McpPermissionMode::Ask,
        McpServerDescriptor::registered(reqwest::Url::parse(&url).expect("fixture url")),
        Arc::new(CountingAuth(Arc::clone(&auth_calls))),
    ));
    let cancel = CancellationToken::new();
    let (old_session, first_catalog) = runtime.connected(&cancel).await.expect("first connection");
    state.require_number_for_echo();
    let gate = state.pause_next_list_tools();
    state.push_tool_list_changed().await;

    let refresh_runtime = Arc::clone(&runtime);
    let refresh_state = state.clone();
    let refresh = tokio::spawn(async move {
        loop {
            let result = refresh_runtime.connected(&CancellationToken::new()).await;
            if result.is_err() || refresh_state.list_tools_call_count.load(Ordering::SeqCst) > 1 {
                break result;
            }
            tokio::task::yield_now().await;
        }
    });
    tokio::time::timeout(Duration::from_secs(5), gate.wait_until_entered())
        .await
        .expect("real catalog refresh reaches the blocked listing");

    let mut invalidation = Box::pin(runtime.invalidate(&old_session));
    let invalidation_poll = futures_util::poll!(&mut invalidation);
    let mut reconnect = Box::pin(runtime.connected(&cancel));
    let reconnect_poll = futures_util::poll!(&mut reconnect);
    let auth_calls_during_listing = auth_calls.load(Ordering::SeqCst);
    gate.release();
    drop(invalidation);

    assert_eq!(
        invalidation_poll,
        Poll::Ready(SessionInvalidation::LatestGeneration)
    );
    assert!(matches!(reconnect_poll, Poll::Pending));
    assert_eq!(
        auth_calls_during_listing, 2,
        "replacement authentication starts while the old listing is blocked"
    );
    let (replacement, catalog) = tokio::time::timeout(Duration::from_secs(5), reconnect)
        .await
        .expect("replacement completes after the old listing")
        .expect("replacement refreshes its catalog");
    tokio::time::timeout(Duration::from_secs(5), refresh)
        .await
        .expect("old refresh completes")
        .expect("refresh task completes")
        .expect("old listing succeeds");
    assert!(!Arc::ptr_eq(&old_session, &replacement));
    assert!(!Arc::ptr_eq(&first_catalog, &catalog));
    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 3);
    assert!(catalog
        .validate_arguments("echo", &json!({"text": 7}))
        .is_ok());
    assert!(catalog
        .validate_arguments("echo", &json!({"text": "stale"}))
        .is_err());
    cancel.cancel();
}

#[tokio::test]
async fn stale_repeated_invalidation_preserves_the_replacement_session_and_catalog() {
    let (url, state, _guard) = spawn_server().await;
    let runtime = runtime("changing", &url, McpPermissionMode::Ask);
    let cancel = CancellationToken::new();

    let (stale_session, _stale_catalog) = runtime
        .connected(&cancel)
        .await
        .expect("first connection and catalog");
    assert_eq!(
        runtime.invalidate(&stale_session).await,
        SessionInvalidation::LatestGeneration
    );
    let (replacement_session, replacement_catalog) = runtime
        .connected(&cancel)
        .await
        .expect("replacement connection and catalog");

    // Two failures from the discarded session can complete after the
    // replacement is live. Neither may evict the replacement.
    assert_eq!(
        runtime.invalidate(&stale_session).await,
        SessionInvalidation::StaleGeneration
    );
    assert_eq!(
        runtime.invalidate(&stale_session).await,
        SessionInvalidation::StaleGeneration
    );
    let (current_session, current_catalog) = runtime
        .connected(&cancel)
        .await
        .expect("replacement stays current");

    assert!(Arc::ptr_eq(&replacement_session, &current_session));
    assert!(Arc::ptr_eq(&replacement_catalog, &current_catalog));
    assert_eq!(state.list_tools_call_count.load(Ordering::SeqCst), 2);

    assert_eq!(
        runtime.invalidate(&replacement_session).await,
        SessionInvalidation::LatestGeneration
    );
    assert_eq!(
        runtime.invalidate(&stale_session).await,
        SessionInvalidation::StaleGeneration,
        "the old generation stays stale after the replacement is also cleared"
    );
}

fn contribution(contributions: &[(String, ServerContribution)], slug: &str) -> ServerContribution {
    contributions
        .iter()
        .find(|(name, _)| name == slug)
        .map(|(_, contribution)| *contribution)
        .unwrap_or_else(|| panic!("{slug} is absent from {contributions:?}"))
}

#[tokio::test]
async fn two_servers_publishing_the_same_tool_name_register_as_two_distinct_tools() {
    let (bundled_url, _bundled_state, _bundled_guard) = spawn_server().await;
    let (acme_url, _acme_state, _acme_guard) = spawn_server().await;
    let servers = McpRuntimeSet::new(
        Some(runtime(
            BUNDLED_SERVER_ID,
            &bundled_url,
            McpPermissionMode::Ask,
        )),
        vec![runtime("acme-crm", &acme_url, McpPermissionMode::Ask)],
    );
    let mut registry = ToolRegistry::new();

    let contributions = servers.register_tools(&mut registry).await;

    // Both fixtures advertise `echo`; without the per-server namespace one
    // would have shadowed the other and the run would have silently lost a tool.
    assert!(registry.get(&ToolId::from("mcp.tenant.echo")).is_some());
    assert!(registry.get(&ToolId::from("mcp.acme-crm.echo")).is_some());
    assert!(matches!(
        contribution(&contributions, BUNDLED_SERVER_ID),
        ServerContribution::Served(_)
    ));
    assert!(matches!(
        contribution(&contributions, "acme-crm"),
        ServerContribution::Served(_)
    ));
}

#[tokio::test]
async fn one_server_being_down_does_not_take_another_servers_tools_with_it() {
    let (live_url, _live_state, _live_guard) = spawn_server().await;
    let servers = McpRuntimeSet::new(
        Some(runtime(
            BUNDLED_SERVER_ID,
            &live_url,
            McpPermissionMode::Ask,
        )),
        vec![runtime(
            "offline",
            &dead_endpoint().await,
            McpPermissionMode::Ask,
        )],
    );
    let mut registry = ToolRegistry::new();

    let contributions = servers.register_tools(&mut registry).await;

    assert!(
        registry.get(&ToolId::from("mcp.tenant.echo")).is_some(),
        "the reachable server's tools must still be registered"
    );
    assert!(registry.get(&ToolId::from("mcp.offline.echo")).is_none());
    assert_eq!(
        contribution(&contributions, "offline"),
        ServerContribution::Unreachable,
        "the failure is recorded against the server that failed, not swallowed"
    );
}

#[tokio::test]
async fn a_denied_server_is_never_held_in_the_set_at_all() {
    let (bundled_url, _bundled_state, _bundled_guard) = spawn_server().await;
    let (denied_url, denied_state, _denied_guard) = spawn_server().await;
    let servers = McpRuntimeSet::new(
        Some(runtime(
            BUNDLED_SERVER_ID,
            &bundled_url,
            McpPermissionMode::Ask,
        )),
        vec![runtime("denied", &denied_url, McpPermissionMode::Deny)],
    );
    let mut registry = ToolRegistry::new();

    let contributions = servers.register_tools(&mut registry).await;

    assert!(servers.find("denied").is_none());
    assert_eq!(
        servers.excluded(),
        [ExcludedServer {
            slug: "denied".to_owned(),
            reason: ServerExclusion::ToolsWithheld,
        }]
    );
    assert!(contributions.iter().all(|(slug, _)| slug != "denied"));
    assert!(registry.get(&ToolId::from("mcp.denied.echo")).is_none());
    assert_eq!(
        denied_state.list_tools_call_count.load(Ordering::SeqCst),
        0,
        "a denied server must not even be connected — nothing it says can be used"
    );
}

#[tokio::test]
async fn an_allow_server_still_pauses_a_tool_it_did_not_annotate_read_only() {
    let (url, _state, _guard) = spawn_server().await;
    let servers = McpRuntimeSet::new(
        Some(runtime(BUNDLED_SERVER_ID, &url, McpPermissionMode::Allow)),
        Vec::new(),
    );
    let mut registry = ToolRegistry::new();

    servers.register_tools(&mut registry).await;

    // The fixture annotates nothing, so `allow` buys nothing: every one of its
    // tools lands in the tier that requires an approval token.
    let echo = registry
        .get(&ToolId::from("mcp.tenant.echo"))
        .expect("echo is registered");
    assert_eq!(echo.risk(), ToolRisk::Mutating);
}

#[test]
fn the_set_stops_holding_sessions_at_its_cap_and_says_which_servers_it_dropped() {
    // Endpoints are never dialed here — the cap is applied at set construction,
    // before any connection exists.
    let bundled = runtime(
        BUNDLED_SERVER_ID,
        "https://tenant.example/mcp",
        McpPermissionMode::Ask,
    );
    let registered: Vec<_> = (0..MAX_CONCURRENT_SERVER_SESSIONS + 2)
        .map(|index| {
            runtime(
                &format!("server-{index}"),
                "https://server.example/mcp",
                McpPermissionMode::Ask,
            )
        })
        .collect();

    let servers = McpRuntimeSet::new(Some(bundled), registered);

    assert_eq!(servers.len(), MAX_CONCURRENT_SERVER_SESSIONS);
    let dropped: Vec<&str> = servers
        .excluded()
        .iter()
        .filter(|excluded| excluded.reason == ServerExclusion::SessionLimit)
        .map(|excluded| excluded.slug.as_str())
        .collect();
    assert_eq!(dropped, ["server-31", "server-32", "server-33"]);
    assert!(
        servers.find(BUNDLED_SERVER_ID).is_some(),
        "the tenant workspace is never the server the cap drops"
    );
}

#[tokio::test]
async fn a_call_reaches_the_server_whose_namespace_owns_the_tool() {
    let (first_url, first_state, _first_guard) = spawn_server().await;
    let (second_url, second_state, _second_guard) = spawn_server().await;
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime(
            BUNDLED_SERVER_ID,
            &first_url,
            McpPermissionMode::Ask,
        )),
        vec![runtime("acme-crm", &second_url, McpPermissionMode::Ask)],
    ));
    let dispatcher = McpToolDispatcher::new(Arc::clone(&servers));

    let result = dispatcher
        .execute(
            &ToolId::from("mcp.acme-crm.echo"),
            &json!({"text": "routed"}),
            CancellationToken::new(),
        )
        .await
        .expect("the call reaches a connected server");

    assert!(result.structured().is_some());
    assert_eq!(
        second_state.call_count.load(Ordering::SeqCst),
        1,
        "the namespaced id names the second server"
    );
    assert_eq!(
        first_state.call_count.load(Ordering::SeqCst),
        0,
        "and nothing was sent to the first"
    );
}

#[tokio::test]
async fn a_tool_id_naming_no_connected_server_is_refused_rather_than_guessed() {
    let (url, state, _guard) = spawn_server().await;
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime(BUNDLED_SERVER_ID, &url, McpPermissionMode::Ask)),
        Vec::new(),
    ));
    let dispatcher = McpToolDispatcher::new(servers);

    for id in ["mcp.ghost.echo", "mcp.echo", "project.read_file"] {
        let outcome = dispatcher
            .execute(
                &ToolId::from(id),
                &json!({"text": "hi"}),
                CancellationToken::new(),
            )
            .await;
        assert!(outcome.is_err(), "{id} must not be routed anywhere");
    }
    assert_eq!(
        state.call_count.load(Ordering::SeqCst),
        0,
        "no unroutable id may fall back to some other server"
    );
}

#[tokio::test]
async fn a_run_keeps_no_dispatcher_when_not_one_server_served_it() {
    // `crate::agent::tool_session` drops the MCP half of its executor when
    // nothing registered, so a call can never be routed at a dispatcher whose
    // servers are all gone.
    let servers = McpRuntimeSet::new(
        Some(runtime(
            BUNDLED_SERVER_ID,
            &dead_endpoint().await,
            McpPermissionMode::Ask,
        )),
        Vec::new(),
    );
    let mut registry = ToolRegistry::new();

    let contributions =
        crate::agent::tool_session::register_mcp_tools(&mut registry, &servers).await;

    assert!(!contributions
        .iter()
        .any(|(_, contribution)| matches!(contribution, ServerContribution::Served(_))));
    // The outcome is reported per server rather than collapsed, so a run can
    // name the server that is down instead of silently offering fewer tools.
    assert!(contributions
        .iter()
        .all(|(_, contribution)| matches!(contribution, ServerContribution::Unreachable)));
    assert!(registry.is_empty());
}

#[tokio::test]
async fn a_server_alone_makes_a_run_tool_capable_with_no_project_open() {
    // The tenant workspace's own MCP server answers questions about the
    // tenant's data and needs no local project to do it. Gating it behind one
    // would leave a signed-in user with a chat that has no data access at all —
    // the state xfalcon-web has never been in.
    let (url, state, _guard) = spawn_server().await;
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime(BUNDLED_SERVER_ID, &url, McpPermissionMode::Ask)),
        Vec::new(),
    ));
    let root = tempfile::tempdir().expect("tempdir");
    let scope = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
    let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());
    let account = crate::security::account_cleanup::ValidatedAccountId::parse("mcp-only-account")
        .expect("account");

    let session = crate::agent::tool_session::build_tool_session(
        crate::agent::tool_session::ToolSessionRequest {
            projects: Arc::new(crate::projects::ProjectsService::new(
                root.path().to_path_buf(),
                scope.clone(),
                secrets.clone(),
            )),
            approvals: Arc::new(crate::approvals::ApprovalsService::new(
                root.path().to_path_buf(),
                scope,
                secrets,
            )),
            mcp: Some(servers),
            account,
            conversation_id: xfalcon_contracts::ConversationId::from("conversation-mcp-only"),
            run_id: xfalcon_contracts::RunId::from("run-mcp-only"),
            result_budget: crate::tools::ToolResultBudget::floor(),
            skill_scope: None,
            standing_approvals: None,
            reauth: None,
            exact_result: None,
            audit: None,
            skills: Arc::new(crate::skills::store::SkillStore::open(
                root.path(),
                &crate::security::account_cleanup::ValidatedAccountId::parse("skills-unused")
                    .expect("account"),
            )),
        },
    )
    .await;

    assert!(
        !session.tools.is_empty(),
        "the model must be told about the tools the server published"
    );
    assert!(
        session.tool_ids.values().all(|id| id
            .as_str()
            .starts_with(crate::mcp::runtime::MCP_TOOL_ID_PREFIX)),
        "no project is open, so no `project.*` tool may be declared: {:?}",
        session.tool_ids
    );
    assert!(
        session.path_authority.is_none(),
        "no project means no trusted root, and a stand-in authority would be \
         claiming to answer a question it cannot"
    );
    assert!(!session.has_project_tools);

    // The declared tool is not merely named — it dispatches.
    let tool_id = session
        .tool_ids
        .values()
        .find(|id| id.as_str().ends_with(".echo"))
        .cloned()
        .expect("the fixture publishes `echo`");
    let outcome = session
        .executor
        .execute(&tool_id, &json!({"text": "hi"}), CancellationToken::new())
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_project_tool_in_a_server_only_session_fails_closed() {
    // Nothing registers a `project.*` tool without a project, so this id can
    // only arrive from a model hallucinating a name or a composition bug. Both
    // must be a reported failure rather than a dispatch at a half-built
    // executor.
    let (url, _state, _guard) = spawn_server().await;
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime(BUNDLED_SERVER_ID, &url, McpPermissionMode::Ask)),
        Vec::new(),
    ));
    let root = tempfile::tempdir().expect("tempdir");
    let scope = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
    let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());

    let session = crate::agent::tool_session::build_tool_session(
        crate::agent::tool_session::ToolSessionRequest {
            projects: Arc::new(crate::projects::ProjectsService::new(
                root.path().to_path_buf(),
                scope.clone(),
                secrets.clone(),
            )),
            approvals: Arc::new(crate::approvals::ApprovalsService::new(
                root.path().to_path_buf(),
                scope,
                secrets,
            )),
            mcp: Some(servers),
            account: crate::security::account_cleanup::ValidatedAccountId::parse("mcp-only-2")
                .expect("account"),
            conversation_id: xfalcon_contracts::ConversationId::from("conversation-mcp-only-2"),
            run_id: xfalcon_contracts::RunId::from("run-mcp-only-2"),
            result_budget: crate::tools::ToolResultBudget::floor(),
            skill_scope: None,
            standing_approvals: None,
            reauth: None,
            exact_result: None,
            audit: None,
            skills: Arc::new(crate::skills::store::SkillStore::open(
                root.path(),
                &crate::security::account_cleanup::ValidatedAccountId::parse("skills-unused")
                    .expect("account"),
            )),
        },
    )
    .await;

    let outcome = session
        .executor
        .execute(
            &ToolId::from("project.read_file"),
            &json!({"path": "a.txt"}),
            CancellationToken::new(),
        )
        .await;

    assert!(
        matches!(
            outcome,
            Err(crate::agent::tool_loop::ToolExecutionError::Failed(_))
        ),
        "{outcome:?}"
    );
}

/// A run over a real fixture server, with a project bound to the conversation or
/// not, and the executor it dispatches through.
async fn save_session(
    bind_project: bool,
) -> (
    crate::agent::tool_session::ToolSession,
    Arc<crate::projects::ProjectsService>,
    crate::security::account_cleanup::ValidatedAccountId,
    tempfile::TempDir,
    tempfile::TempDir,
    super::support::ServerGuard,
) {
    let (url, _state, guard) = spawn_server().await;
    let servers = Arc::new(McpRuntimeSet::new(
        Some(runtime(BUNDLED_SERVER_ID, &url, McpPermissionMode::Ask)),
        Vec::new(),
    ));
    let root = tempfile::tempdir().expect("tempdir");
    let project_root = tempfile::tempdir().expect("project root");
    let scope = Arc::new(crate::security::account_cleanup::CurrentAccountScope::default());
    scope.bind_native("origin-account").expect("bind account");
    let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());
    secrets
        .put_key("origin-account", [0x33; 32])
        .expect("account key");
    let account = crate::security::account_cleanup::ValidatedAccountId::parse("origin-account")
        .expect("account");
    let projects = Arc::new(crate::projects::ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let conversation_id = xfalcon_contracts::ConversationId::from("conversation-origin");
    if bind_project {
        let created = projects
            .create(
                &project_root.path().to_string_lossy(),
                Some("Sales".to_owned()),
            )
            .await
            .expect("trust project");
        let project_id = created.id.as_str().parse().expect("project id");
        projects
            .bind_conversation(&account, &conversation_id, project_id)
            .expect("bind conversation");
    }
    let session = crate::agent::tool_session::build_tool_session(
        crate::agent::tool_session::ToolSessionRequest {
            projects: projects.clone(),
            approvals: Arc::new(crate::approvals::ApprovalsService::new(
                root.path().to_path_buf(),
                scope,
                secrets,
            )),
            mcp: Some(servers),
            account: account.clone(),
            conversation_id,
            run_id: xfalcon_contracts::RunId::from("run-origin"),
            result_budget: crate::tools::ToolResultBudget::floor(),
            skill_scope: None,
            standing_approvals: None,
            reauth: None,
            exact_result: None,
            audit: None,
            skills: Arc::new(crate::skills::store::SkillStore::open(
                root.path(),
                &crate::security::account_cleanup::ValidatedAccountId::parse("skills-unused")
                    .expect("account"),
            )),
        },
    )
    .await;
    (session, projects, account, root, project_root, guard)
}

async fn call_save(
    session: &crate::agent::tool_session::ToolSession,
    arguments: serde_json::Value,
) {
    let tool_id = session
        .tool_ids
        .values()
        .find(|id| id.as_str().ends_with(".ida_save_dashboard"))
        .cloned()
        .expect("the fixture publishes the save tool");
    let outcome = session
        .executor
        .execute(&tool_id, &arguments, CancellationToken::new())
        .await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
async fn a_save_through_a_project_bound_run_links_the_new_report_to_that_project() {
    let (session, projects, account, _root, _project_root, _guard) = save_session(true).await;

    call_save(&session, json!({"name": "Weekly"})).await;

    let linked = projects.report_origins(&account).expect("origins");
    assert_eq!(linked.len(), 1, "{linked:?}");
    assert_eq!(linked[0].0, "kit-fixture");
    assert_eq!(linked[0].2, "Sales");
}

#[tokio::test]
async fn a_save_through_a_regular_run_links_nothing() {
    let (session, projects, account, _root, _project_root, _guard) = save_session(false).await;

    call_save(&session, json!({"name": "Weekly"})).await;

    assert!(projects
        .report_origins(&account)
        .expect("origins")
        .is_empty());
}

#[tokio::test]
async fn a_redefine_through_a_project_bound_run_links_nothing() {
    let (session, projects, account, _root, _project_root, _guard) = save_session(true).await;

    call_save(&session, json!({"name": "Weekly", "kit_id": "kit-fixture"})).await;

    assert!(projects
        .report_origins(&account)
        .expect("origins")
        .is_empty());
}
