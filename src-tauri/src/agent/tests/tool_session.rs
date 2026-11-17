//! Composition root — what a run is actually offered, and under what name.

use std::collections::HashMap;
use std::sync::Arc;

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::tools::ToolId;
use xfalcon_contracts::{ConversationId, RunId};

use crate::agent::tool_loop::ModelToolCall;
use crate::agent::tool_session::{build_tool_session, wire_tool_name, ToolSessionRequest};
use crate::approvals::ApprovalsService;
use crate::audit::{AuditAction, AuditHub, LocalAuditStore};
use crate::clients::da::chat_api::DaChatSession;
use crate::persistence::InMemorySecretStore;
use crate::projects::ProjectsService;
use crate::security::account_cleanup::{
    account_directory_name, CurrentAccountScope, ValidatedAccountId,
};
use crate::skills::store::SkillStore;
use crate::tools::exact_result::{AdmittedToolResultPayload, ExactResultToolExecutor};
use crate::tools::invocation::InMemoryInvocationJournal;
use crate::tools::registry::READ_TOOL_RESULT_TOOL_ID;

fn account() -> ValidatedAccountId {
    ValidatedAccountId::parse("tool-session-account").expect("account")
}

#[derive(Clone, Copy)]
enum WriteInterruption {
    Stop,
    CallTimeout,
    RoundTimeout,
}

struct ReleaseWriteOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl ReleaseWriteOnDrop {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for ReleaseWriteOnDrop {
    fn drop(&mut self) {
        self.release();
    }
}

/// Drives the actual composite executor, filesystem writer and encrypted
/// approval ledger through the production tool loop. Only time and the
/// writer's post-claim test barrier are controlled.
async fn interrupted_local_write(interruption: WriteInterruption) {
    use crate::agent::tool_loop::{
        last_journaled_status, ModelToolCall, ToolCallOutcome, ToolLoop, ToolLoopBudgets,
    };
    use crate::tools::invocation::InMemoryInvocationJournal;
    use crate::tools::policy::{Actor, ProjectContext};
    use crate::tools::registry::{project_propose_patch_tool_descriptor, ToolRegistry};
    use std::sync::Mutex;
    use std::time::Duration;
    use xfalcon_contracts::tools::{InvocationId, ToolInvocationStatus};

    let root = tempdir().unwrap();
    let project_root = tempdir().unwrap();
    let scope = Arc::new(CurrentAccountScope::default());
    scope.bind_native(account().as_str()).unwrap();
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets.put_key(account().as_str(), [0x33; 32]).unwrap();
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let mut approvals = ApprovalsService::new(root.path().to_path_buf(), scope, secrets);
    let entered = Arc::new(tokio::sync::Notify::new());
    let hook_entered = entered.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let mut release = ReleaseWriteOnDrop(Some(release));
    let wait = Mutex::new(wait);
    approvals.resolution_hook = Some(Arc::new(move |_| {
        hook_entered.notify_one();
        wait.lock().unwrap().recv().expect("release writer");
    }));
    let view = projects
        .create(&project_root.path().to_string_lossy(), None)
        .await
        .unwrap();
    let conversation_id = ConversationId::from("conversation-local-write");
    projects
        .bind_active_conversation(&account(), &conversation_id)
        .unwrap();
    let run_id = RunId::from("run-local-write");
    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals: Arc::new(approvals),
        mcp: None,
        account: account(),
        conversation_id,
        run_id: run_id.clone(),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(root.path(), &account())),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;
    let tool_id = ToolId::from("project.propose_patch");
    let mut registry = ToolRegistry::new();
    registry
        .register(project_propose_patch_tool_descriptor().unwrap())
        .unwrap();
    const PREFLIGHT_WAIT: Duration = Duration::from_secs(30);
    let (tool_call_timeout, round_timeout) = match interruption {
        WriteInterruption::Stop => (Duration::from_secs(120), Duration::from_secs(180)),
        WriteInterruption::CallTimeout => (Duration::from_secs(60), Duration::from_secs(120)),
        WriteInterruption::RoundTimeout => (Duration::from_secs(120), Duration::from_secs(60)),
    };
    let mut tool_loop = ToolLoop::new(
        registry,
        std::collections::HashSet::from([tool_id.clone()]),
        Actor::new(account().as_str()).unwrap(),
        run_id,
        Some(ProjectContext::new(view.id.as_str())),
        None,
        ToolLoopBudgets {
            tool_call_timeout,
            round_timeout,
            ..ToolLoopBudgets::production()
        },
    );
    let invocation_id = InvocationId::from("write-1");
    let make_call = || ModelToolCall {
        invocation_id: invocation_id.clone(),
        tool_id: tool_id.clone(),
        arguments: serde_json::json!({"operations": [{
            "kind": "create", "path": "new.txt", "content": "complete write"
        }]}),
    };
    let call = make_call();
    let cancel = CancellationToken::new();
    let job_cancel = cancel.clone();
    let executor = session.executor;
    let authority = session.path_authority;
    let mut job = tokio::spawn(async move {
        let mut journal = InMemoryInvocationJournal::new();
        let result = tool_loop
            .run_round(
                vec![call],
                executor.as_ref(),
                authority.as_deref(),
                &mut journal,
                &job_cancel,
            )
            .await;
        (tool_loop, journal, result, executor, authority)
    });
    let entry = tokio::time::timeout(PREFLIGHT_WAIT, async {
        tokio::select! {
            _ = entered.notified() => Ok(()),
            completed = &mut job => Err(completed),
        }
    })
    .await;
    match entry {
        Ok(Ok(())) => {}
        Ok(Err(Ok((_, _, result, _, _)))) => {
            panic!("tool loop completed before the write hook entered: {result:?}")
        }
        Ok(Err(Err(error))) => {
            panic!("tool loop task failed before the write hook entered: {error}")
        }
        Err(_) => panic!("timed out waiting {PREFLIGHT_WAIT:?} for the write hook to enter"),
    }
    // Pause only after the real write has started. Auto-advancing time before
    // its blocking preflight finishes would test a queued cancellation instead.
    tokio::time::pause();
    match interruption {
        WriteInterruption::Stop => cancel.cancel(),
        WriteInterruption::CallTimeout => {
            tokio::time::advance(tool_call_timeout + Duration::from_secs(1)).await
        }
        WriteInterruption::RoundTimeout => {
            tokio::time::advance(round_timeout + Duration::from_secs(1)).await
        }
    }
    tokio::task::yield_now().await;
    assert!(
        !job.is_finished(),
        "started write must keep its completion owner"
    );
    // Blocking filesystem jobs use wall time. Resume before waiting for one,
    // so Tokio does not auto-advance another deadline while that job finishes.
    tokio::time::resume();
    release.release();
    let (mut tool_loop, mut journal, result, executor, authority) = job.await.unwrap();
    let round = result.expect("real mutation outcome is returned before round failure");
    assert!(matches!(
        round.outcomes[0],
        ToolCallOutcome::Completed { .. }
    ));
    assert_eq!(
        tool_loop.round_timed_out(),
        matches!(interruption, WriteInterruption::RoundTimeout)
    );
    assert_eq!(
        cancel.is_cancelled(),
        matches!(interruption, WriteInterruption::Stop)
    );
    let entries = journal
        .entries_for(&invocation_id)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        last_journaled_status(&entries, &invocation_id),
        Some(ToolInvocationStatus::Completed)
    );
    assert_eq!(
        std::fs::read(project_root.path().join("new.txt")).unwrap(),
        b"complete write"
    );
    let connection = rusqlite::Connection::open(
        root.path()
            .join("accounts")
            .join(account_directory_name(account().as_str()))
            .join(crate::projects::PROJECT_DATA_DIR)
            .join("approvals.sqlite3"),
    )
    .unwrap();
    let status: String = connection
        .query_row("SELECT status FROM project_mutation_approvals", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        status, "executed",
        "approval and invocation ledgers agree with disk"
    );
    let replay = tool_loop
        .run_round(
            vec![make_call()],
            executor.as_ref(),
            authority.as_deref(),
            &mut journal,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        replay.outcomes[0],
        ToolCallOutcome::Completed { .. }
    ));
    assert_eq!(
        journal.entries_for(&invocation_id).len(),
        entries.len(),
        "completed invocation is replayed"
    );
}

#[tokio::test]
async fn stop_during_a_local_write_journals_the_actual_completed_write() {
    interrupted_local_write(WriteInterruption::Stop).await;
}

#[tokio::test]
async fn call_timeout_during_a_local_write_journals_the_actual_completed_write() {
    interrupted_local_write(WriteInterruption::CallTimeout).await;
}

#[tokio::test]
async fn round_timeout_during_a_local_write_journals_the_actual_completed_write() {
    interrupted_local_write(WriteInterruption::RoundTimeout).await;
}

// ---------------------------------------------------------------------
// Wire names
// ---------------------------------------------------------------------

#[test]
fn a_dotted_tool_id_never_reaches_the_wire_as_written() {
    // OpenAI and Anthropic both reject a function name outside
    // `^[a-zA-Z0-9_-]{1,64}$`, and every internal id is dotted — sending one
    // verbatim would fail the whole request, not just the tool.
    let name = wire_tool_name(&ToolId::from("project.read_file"));

    assert_eq!(name, "project_read_file");
    assert!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "{name} is not a legal provider function name"
    );
}

#[test]
fn an_over_long_tool_id_is_truncated_to_the_provider_limit() {
    let name = wire_tool_name(&ToolId::from("a".repeat(200).as_str()));

    assert_eq!(name.len(), 64);
}

#[test]
fn a_tool_id_that_sanitizes_to_nothing_still_gets_a_callable_name() {
    let name = wire_tool_name(&ToolId::from("...."));

    assert!(
        !name.is_empty(),
        "a provider rejects an empty function name"
    );
}

// ---------------------------------------------------------------------
// Session construction
// ---------------------------------------------------------------------

/// The MCP-only case (a run made tool-capable by a server alone, with no
/// project) is proven in `crate::mcp::tests::multi_server`, against a real
/// fixture server — the harness for that lives there.
#[tokio::test]
async fn neither_a_project_nor_an_mcp_server_means_no_tool_session() {
    let root = tempdir().expect("tempdir");
    let scope = Arc::new(CurrentAccountScope::default());
    let secrets = Arc::new(InMemorySecretStore::default());
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        root.path().to_path_buf(),
        scope,
        secrets,
    ));

    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        // No server at all: with no project either, nothing is registrable.
        // Reaching for a live MCP connection would make this a network test.
        mcp: None,
        account: account(),
        conversation_id: ConversationId::from("conversation-unbound"),
        run_id: RunId::from("run-1"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        // An empty skill library: this test's claim is that NOTHING is
        // registrable, and an imported skill would itself make the run
        // tool-capable.
        skills: Arc::new(SkillStore::open(root.path(), &account())),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;

    assert!(session.tools.is_empty());
    assert!(session.tool_ids.is_empty());
}

#[tokio::test]
async fn ready_context_payload_alone_registers_the_exact_range_tool() {
    let root = tempdir().expect("tempdir");
    let scope = Arc::new(CurrentAccountScope::default());
    let secrets = Arc::new(InMemorySecretStore::default());
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        root.path().to_path_buf(),
        scope,
        secrets,
    ));
    let conversation_id = uuid::Uuid::new_v4();
    let exact_result = ExactResultToolExecutor::new(
        DaChatSession::for_test("http://127.0.0.1:9", "unused-token").expect("DA session"),
        conversation_id,
        [AdmittedToolResultPayload {
            id: uuid::Uuid::new_v4(),
            byte_length: 10,
            sha256: "00".repeat(32),
            media_type: "application/json".to_owned(),
            serialization_format: "json".to_owned(),
            serialization_version: 1,
            content_encoding: "identity".to_owned(),
        }],
        crate::tools::ToolResultBudget::floor(),
    );

    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account(),
        conversation_id: ConversationId::from(conversation_id.to_string()),
        run_id: RunId::from("run-exact-result"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(root.path(), &account())),
        reauth: None,
        exact_result: Some(exact_result),
        audit: None,
    })
    .await;

    assert_eq!(session.tools.len(), 1);
    assert_eq!(session.tools[0].name, READ_TOOL_RESULT_TOOL_ID);
    assert_eq!(
        session.tool_ids.get(READ_TOOL_RESULT_TOOL_ID),
        Some(&ToolId::from(READ_TOOL_RESULT_TOOL_ID))
    );
}

// ---------------------------------------------------------------------
// Name collisions
// ---------------------------------------------------------------------

#[test]
fn two_ids_that_sanitize_alike_do_not_collapse_into_one_callable_name() {
    // `a.b` and `a_b` both sanitize to `a_b`. Letting the second overwrite the
    // first would make one tool permanently uncallable *and* silently route
    // its calls to the other.
    let mut taken: HashMap<String, ToolId> = HashMap::new();
    let first = crate::agent::tool_session::unique_wire_name(&ToolId::from("a.b"), &taken);
    taken.insert(first.clone(), ToolId::from("a.b"));
    let second = crate::agent::tool_session::unique_wire_name(&ToolId::from("a_b"), &taken);

    assert_eq!(first, "a_b");
    assert_ne!(first, second);
}

/// Skills are the third thing that can make a run tool-capable, alongside a
/// project and an MCP server (spec: chat-run-activity). This is the case that
/// matters on a laptop with neither: the user imported skills, so the model
/// must be able to fetch them.
#[tokio::test]
async fn an_imported_skill_alone_makes_a_run_tool_capable() {
    let root = tempdir().expect("tempdir");
    let scope = Arc::new(CurrentAccountScope::default());
    let secrets = Arc::new(InMemorySecretStore::default());
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        root.path().to_path_buf(),
        scope,
        secrets,
    ));

    let skills = Arc::new(SkillStore::open(root.path(), &account()));
    let chosen = root.path().join("ida-dashboard.md");
    std::fs::write(
        &chosen,
        "---\nname: ida-dashboard\ndescription: \"Build dashboards.\"\n---\nBody.\n",
    )
    .expect("seed skill");
    let outcome = skills.import(&[chosen]);
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account(),
        conversation_id: ConversationId::from("conversation-skills"),
        run_id: RunId::from("run-skills-only"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: skills.clone(),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;

    // The wire name has to be web's exactly, or the ported prompt's "you MUST
    // call get_skill(name)" names a tool the model was never given.
    assert!(
        session
            .tools
            .iter()
            .any(|tool| tool.name == "__internal__get_skill"),
        "declared: {:?}",
        session.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    // A skill the user turned off takes the tool with it: there is nothing
    // left to fetch, and declaring a fetch tool would invite a call that
    // can only fail.
    skills.set_enabled("ida-dashboard", false).expect("disable");
    let none = build_tool_session(ToolSessionRequest {
        projects: Arc::new(ProjectsService::new(
            root.path().to_path_buf(),
            Arc::new(CurrentAccountScope::default()),
            Arc::new(InMemorySecretStore::default()),
        )),
        approvals: Arc::new(ApprovalsService::new(
            root.path().to_path_buf(),
            Arc::new(CurrentAccountScope::default()),
            Arc::new(InMemorySecretStore::default()),
        )),
        mcp: None,
        account: account(),
        conversation_id: ConversationId::from("conversation-no-skills"),
        run_id: RunId::from("run-no-skills"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        reauth: None,
        exact_result: None,
        audit: None,
        skills,
    })
    .await;
    assert!(none.tools.is_empty());
}

#[tokio::test]
async fn project_tools_use_the_conversation_binding_not_the_ambient_active_project() {
    let root = tempdir().expect("tempdir");
    let first_root = tempdir().expect("first project");
    let second_root = tempdir().expect("second project");
    std::fs::write(first_root.path().join("README.md"), "first\n").expect("seed first");
    std::fs::write(second_root.path().join("README.md"), "second\n").expect("seed second");

    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native(account().as_str())
        .expect("bind current account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(account().as_str(), [0x33; 32])
        .expect("account key");
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        root.path().to_path_buf(),
        scope,
        secrets,
    ));
    projects
        .create(
            &first_root.path().to_string_lossy(),
            Some("First".to_owned()),
        )
        .await
        .expect("trust first");
    let conversation_id = ConversationId::from("conversation-first");
    projects
        .bind_active_conversation(&account(), &conversation_id)
        .expect("bind conversation to first");
    projects
        .create(
            &second_root.path().to_string_lossy(),
            Some("Second".to_owned()),
        )
        .await
        .expect("trust and activate second");

    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account(),
        conversation_id,
        run_id: RunId::from("run-bound-project"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(root.path(), &account())),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;
    assert!(session.has_project_tools);

    let outcome = session
        .executor
        .execute(
            &ToolId::from("project.read_file"),
            &serde_json::json!({"path": "README.md"}),
            CancellationToken::new(),
        )
        .await
        .expect("read bound project");
    assert_eq!(
        outcome.structured().expect("structured")["content"],
        "first\n"
    );

    let search = session
        .executor
        .execute(
            &ToolId::from("project.search_files"),
            &serde_json::json!({"query": "README"}),
            CancellationToken::new(),
        )
        .await
        .expect("search indexed project");
    assert_eq!(
        search.structured().expect("structured")["results"]
            .as_array()
            .expect("results")
            .len(),
        1,
        "building a project tool session must complete an index pass first"
    );
}

#[tokio::test]
async fn an_index_failure_withholds_search_but_keeps_exact_project_reads() {
    let root = tempdir().expect("tempdir");
    let project_root = tempdir().expect("project");
    std::fs::write(project_root.path().join("README.md"), "readable\n").expect("seed project");

    let scope = Arc::new(CurrentAccountScope::default());
    scope
        .bind_native(account().as_str())
        .expect("bind current account");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(account().as_str(), [0x44; 32])
        .expect("account key");
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        root.path().to_path_buf(),
        scope,
        secrets,
    ));
    projects
        .create(&project_root.path().to_string_lossy(), None)
        .await
        .expect("trust project");
    let conversation_id = ConversationId::from("conversation-index-failure");
    projects
        .bind_active_conversation(&account(), &conversation_id)
        .expect("bind conversation");

    // A directory where SQLite needs a file deterministically makes the
    // index pass fail without affecting the separately stored trusted root.
    let index_path = root
        .path()
        .join("accounts")
        .join(account_directory_name(account().as_str()))
        .join("project-data")
        .join("project-index.sqlite3");
    std::fs::create_dir_all(&index_path).expect("block index database path");

    let session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account(),
        conversation_id,
        run_id: RunId::from("run-index-failure"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(root.path(), &account())),
        reauth: None,
        exact_result: None,
        audit: None,
    })
    .await;

    assert!(session.has_project_tools);
    assert!(session
        .tool_ids
        .values()
        .any(|id| id.as_str() == "project.read_file"));
    assert!(session
        .tool_ids
        .values()
        .all(|id| !matches!(id.as_str(), "project.search_files" | "project.search_text")));
    let outcome = session
        .executor
        .execute(
            &ToolId::from("project.read_file"),
            &serde_json::json!({"path": "README.md"}),
            CancellationToken::new(),
        )
        .await
        .expect("exact read is index-independent");
    assert_eq!(
        outcome.structured().expect("structured")["content"],
        "readable\n"
    );
}

/// A run's audit publisher reaches its tool loop: the first decision the loop
/// takes lands in the local log, with the tool id as its detail.
#[tokio::test]
async fn a_run_audits_its_tool_decisions_into_the_local_log() {
    let root = tempdir().expect("tempdir");
    let scope = Arc::new(CurrentAccountScope::default());
    let secrets = Arc::new(InMemorySecretStore::default());
    let projects = Arc::new(ProjectsService::new(
        root.path().to_path_buf(),
        scope.clone(),
        secrets.clone(),
    ));
    let approvals = Arc::new(ApprovalsService::new(
        root.path().to_path_buf(),
        scope,
        secrets.clone(),
    ));
    let store = LocalAuditStore::open(root.path().join("audit"), secrets).expect("open audit log");
    let hub = Arc::new(AuditHub::new(Arc::new(store)));

    let mut session = build_tool_session(ToolSessionRequest {
        projects,
        approvals,
        mcp: None,
        account: account(),
        conversation_id: ConversationId::from("conversation-audited"),
        run_id: RunId::from("run-audited"),
        result_budget: crate::tools::ToolResultBudget::floor(),
        skill_scope: None,
        standing_approvals: None,
        skills: Arc::new(SkillStore::open(root.path(), &account())),
        reauth: None,
        exact_result: None,
        audit: Some(hub.publisher()),
    })
    .await;

    // No tool is registered in this session, so the call is denied at once.
    let mut journal = InMemoryInvocationJournal::new();
    session
        .tool_loop
        .run_round(
            vec![ModelToolCall {
                invocation_id: "inv-audited".into(),
                tool_id: ToolId::from("no.such.tool"),
                arguments: serde_json::json!({}),
            }],
            session.executor.as_ref(),
            None,
            &mut journal,
            &CancellationToken::new(),
        )
        .await
        .expect("round completes");

    let records = hub.store().recent(10).expect("read audit log");
    assert_eq!(records.len(), 1);
    let event = records[0].event();
    assert_eq!(event.action(), AuditAction::ToolDeny);
    assert_eq!(event.resource_id(), "inv-audited");
    assert_eq!(event.request_id(), "run-audited");
    assert_eq!(event.detail(), Some("no.such.tool"));
}
