//! What boot recovery may and may not cancel on the source of truth.
//!
//! `finalize_abandoned_runs` writes `status=cancelled` to DA. xFalcon Web
//! drives its own runs against the same conversations, so the difference
//! between "a dead process on this device left this behind" and "another
//! client is streaming this right now" is the difference between recovery and
//! destroying a live answer in the other client.
//!
//! Every assertion here counts requests at the DA boundary. The returned patch
//! list cannot show a request that was already sent, and the request is the
//! part that is not undoable.

use super::*;
use crate::persistence::InMemorySecretStore;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn chat_service(directory: &std::path::Path) -> ChatService {
    let sessions = Arc::new(AuthSessionStore::new());
    let accounts = Arc::new(CurrentAccountScope::default());
    accounts.bind_native("account-1").unwrap();
    let secrets = Arc::new(InMemorySecretStore::default());
    ChatService::new(
        sessions.clone(),
        accounts.clone(),
        directory.to_owned(),
        secrets.clone(),
        Arc::new(CredentialRuntime::new(sessions, secrets.clone())),
        Arc::new(crate::clients::da::model_catalog::ModelCatalogCache::default()),
        Arc::new(ProjectsService::new(
            directory.to_owned(),
            accounts.clone(),
            secrets.clone(),
        )),
        Arc::new(ApprovalsService::new(
            directory.to_owned(),
            accounts,
            secrets,
        )),
    )
}

/// Run ids are real UUIDs because the cancel path parses them before it
/// reaches DA; a placeholder id would skip the request this file is about.
fn run(run_id: &str, conversation: &ConversationId, state: RunState) -> RunRecord {
    RunRecord::try_new(
        RunId::from(run_id.to_owned()),
        conversation.clone(),
        state,
        1,
    )
    .expect("run record")
}

/// Refuses every request with a 500 and expects none: mounted so that a cancel
/// this test does not want is a failure rather than a silent connection error.
async fn expect_no_requests(server: &MockServer) {
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(server)
        .await;
}

/// The rule the two paths above turn on, read from a real journal on disk
/// rather than asserted against a literal. A journal file that was never
/// written is how another client's run looks from here, and it must not be
/// confused with a run this device started and abandoned.
#[test]
fn a_run_with_no_journal_file_is_not_this_devices() {
    let directory = tempfile::tempdir().expect("tempdir");
    let accounts = CurrentAccountScope::default();
    accounts.bind_native("account-1").expect("account");
    let account = accounts.current().expect("scope").expect("bound");
    let journal = RunJournalStore::new(directory.path().to_owned(), account);

    let never_written = RunId::from(Uuid::new_v4().to_string());
    assert!(
        !device_journaled_run(&journal.load_entries(&never_written).unwrap_or_default()),
        "a run with no journal on this device is another client's"
    );

    let journaled = RunId::from(Uuid::new_v4().to_string());
    journal
        .append_transition(&journaled, 1, RunState::Queued, RunState::Running)
        .expect("append");
    assert!(
        device_journaled_run(&journal.load_entries(&journaled).unwrap_or_default()),
        "a run this device journaled is this device's to recover"
    );
}

#[tokio::test]
async fn a_run_another_client_is_driving_is_never_cancelled() {
    let directory = tempfile::tempdir().expect("tempdir");
    let chat = chat_service(directory.path());
    let server = MockServer::start().await;
    expect_no_requests(&server).await;
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");

    let conversation = ConversationId::from(Uuid::new_v4().to_string());
    let run_id = Uuid::new_v4().to_string();

    let patches = chat
        .finalize_abandoned_runs(
            &da,
            &conversation,
            &[run(&run_id, &conversation, RunState::Running)],
            &HashSet::from([run_id.clone()]),
        )
        .await;

    assert!(
        patches.is_empty(),
        "a run this device holds no journal for must not be projected as cancelled"
    );
}

/// The state a run sits in while it waits for a person to approve a tool call.
/// It can be held for a long time, which is exactly the window in which its
/// owner walks over to the other client.
#[tokio::test]
async fn a_foreign_run_awaiting_approval_is_never_cancelled() {
    let directory = tempfile::tempdir().expect("tempdir");
    let chat = chat_service(directory.path());
    let server = MockServer::start().await;
    expect_no_requests(&server).await;
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");

    let conversation = ConversationId::from(Uuid::new_v4().to_string());
    let run_id = Uuid::new_v4().to_string();

    let patches = chat
        .finalize_abandoned_runs(
            &da,
            &conversation,
            &[run(&run_id, &conversation, RunState::AwaitingApproval)],
            &HashSet::from([run_id.clone()]),
        )
        .await;

    assert!(patches.is_empty());
}

#[tokio::test]
async fn a_run_this_device_abandoned_is_cancelled_on_da() {
    let directory = tempfile::tempdir().expect("tempdir");
    let chat = chat_service(directory.path());
    let server = MockServer::start().await;
    let conversation = ConversationId::from(Uuid::new_v4().to_string());
    let run_id = Uuid::new_v4().to_string();

    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/chat/runs/{run_id}/status")))
        .and(body_json(serde_json::json!({ "status": "cancelled" })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");

    let patches = chat
        .finalize_abandoned_runs(
            &da,
            &conversation,
            &[run(&run_id, &conversation, RunState::Running)],
            &HashSet::new(),
        )
        .await;

    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].state(), &RunState::Cancelled);
}

/// The realistic shape after a crash: this device left one run behind, and the
/// other client is mid-answer on another in the same conversation. Recovery
/// has to separate them, not pick one rule for both.
#[tokio::test]
async fn an_abandoned_run_is_finalized_beside_a_live_foreign_one() {
    let directory = tempfile::tempdir().expect("tempdir");
    let chat = chat_service(directory.path());
    let server = MockServer::start().await;
    let conversation = ConversationId::from(Uuid::new_v4().to_string());
    let abandoned = Uuid::new_v4().to_string();
    let foreign = Uuid::new_v4().to_string();

    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/chat/runs/{abandoned}/status")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    // Named separately so a cancel aimed at the live run fails loudly here
    // rather than being absorbed by the matcher above.
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/chat/runs/{foreign}/status")))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");

    let patches = chat
        .finalize_abandoned_runs(
            &da,
            &conversation,
            &[
                run(&abandoned, &conversation, RunState::Running),
                run(&foreign, &conversation, RunState::Running),
            ],
            &HashSet::from([foreign.clone()]),
        )
        .await;

    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].run_id().as_str(), abandoned);
}

#[tokio::test]
async fn a_finished_run_is_left_alone() {
    let directory = tempfile::tempdir().expect("tempdir");
    let chat = chat_service(directory.path());
    let server = MockServer::start().await;
    expect_no_requests(&server).await;
    let da = DaChatSession::for_test(&server.uri(), "token").expect("session");

    let conversation = ConversationId::from(Uuid::new_v4().to_string());
    let run_id = Uuid::new_v4().to_string();

    let patches = chat
        .finalize_abandoned_runs(
            &da,
            &conversation,
            &[run(&run_id, &conversation, RunState::Completed)],
            &HashSet::new(),
        )
        .await;

    assert!(patches.is_empty());
}
