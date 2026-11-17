use std::collections::{HashMap, HashSet};

use uuid::Uuid;
use xfalcon_contracts::chat::{ToolStepPathV1, ToolStepSummaryV1};
use xfalcon_contracts::{ConversationId, RunId, RunState};

use crate::agent::recovery::{
    pick_strongest_recovery_notice, STARTUP_RECOVERY_NOTICE_AWAIT_USER,
    STARTUP_RECOVERY_NOTICE_RECONCILE,
};
use crate::chat::domain::{
    ChatSnapshot, Conversation, Message, MessageContent, MessageId, MessageRole, RunRecord,
    SyncCursor, ToolCallId, ToolCallRef, ToolCallStatus,
};
use crate::chat::sync::SyncAvailability;
use crate::chat::wire::{
    conversation_view_from, latest_owned_run_state, startup_recovery_from_workspace,
    startup_recovery_hint_from_snapshot, without_foreign_active_run, workspace_bootstrap_from,
    WorkspaceRecoverySnapshot,
};

fn conversation_snapshot(conversation_id: &str, runs: Vec<RunRecord>) -> ChatSnapshot {
    let conversation_id = ConversationId::from(conversation_id);
    let conversation = Conversation::new(conversation_id.clone(), "Fixture");
    let message = Message::try_new(
        MessageId::from_uuid(Uuid::new_v4()),
        conversation_id.clone(),
        1,
        MessageRole::User,
        vec![MessageContent::text("hello")],
        vec![],
    )
    .expect("message")
    .into_completed();
    ChatSnapshot::new(conversation, vec![message], None, runs, SyncCursor::new(1))
}

fn run(conversation_id: &str, run_id: &str, state: RunState) -> RunRecord {
    RunRecord::try_new(
        RunId::from(run_id),
        ConversationId::from(conversation_id),
        state,
        1,
    )
    .expect("run")
}

#[test]
fn startup_recovery_from_workspace_picks_strongest_notice() {
    let quiet = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot("conv-quiet", vec![]),
        active_run_id: None,
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::new(),
    };
    let interrupted = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-interrupted",
            vec![run("conv-interrupted", "run-int", RunState::Interrupted)],
        ),
        active_run_id: Some(RunId::from("run-int")),
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::new(),
    };
    let running = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-running",
            vec![run("conv-running", "run-run", RunState::Running)],
        ),
        active_run_id: Some(RunId::from("run-run")),
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::new(),
    };

    assert_eq!(
        startup_recovery_from_workspace(
            &[quiet, interrupted, running],
            Some(&ConversationId::from("conv-interrupted"))
        )
        .as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
    );
}

#[test]
fn startup_recovery_from_workspace_prefers_reconcile_over_await_user() {
    let await_user = startup_recovery_hint_from_snapshot(
        &conversation_snapshot(
            "conv-a",
            vec![run("conv-a", "run-a", RunState::Interrupted)],
        ),
        Some(&RunId::from("run-a")),
        &HashSet::new(),
        &HashSet::new(),
    );
    let reconcile = Some(STARTUP_RECOVERY_NOTICE_RECONCILE.to_owned());

    assert_eq!(
        pick_strongest_recovery_notice([await_user, reconcile]).as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_RECONCILE)
    );
}

#[test]
fn startup_recovery_from_workspace_empty_entries_returns_none() {
    assert_eq!(startup_recovery_from_workspace(&[], None), None);
}

#[test]
fn workspace_bootstrap_from_maps_recovery_notice_from_workspace_recovery() {
    let interrupted = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-interrupted",
            vec![run("conv-interrupted", "run-int", RunState::Interrupted)],
        ),
        active_run_id: Some(RunId::from("run-int")),
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::new(),
    };
    let recovery_notice = startup_recovery_from_workspace(
        &[interrupted],
        Some(&ConversationId::from("conv-interrupted")),
    );
    let summary = crate::chat::domain::ConversationSummary::from_conversation(&Conversation::new(
        ConversationId::from("conv-interrupted"),
        "Fixture",
    ));
    let project_id = crate::projects::ProjectId::new();
    let bindings = HashMap::from([("conv-interrupted".to_owned(), project_id)]);
    let bootstrap = workspace_bootstrap_from(vec![summary], recovery_notice.clone(), &bindings);
    assert_eq!(
        bootstrap.recovery_notice.as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
    );
    assert_eq!(bootstrap.recovery_notice, recovery_notice);
    assert_eq!(
        bootstrap.conversations[0]
            .project_id
            .as_ref()
            .map(xfalcon_contracts::projects::ProjectId::as_str),
        Some(project_id.to_string().as_str())
    );
}

/// The projection is the only place a step label may be composed, so it is also
/// the only place one can be lost: a mapping that dropped it would leave the
/// card with nothing to render and no way to notice.
#[test]
fn the_wire_view_carries_a_calls_step_summary() {
    let conversation_id = ConversationId::from("conversation-step");
    let summary = ToolStepSummaryV1::new(
        "Listed 2 entries",
        vec![
            ToolStepPathV1 {
                path: "src/features/cards.tsx".to_owned(),
                is_sensitive: false,
            },
            ToolStepPathV1 {
                path: "src/features/id_ed25519".to_owned(),
                is_sensitive: true,
            },
        ],
        2,
    );
    let call = ToolCallRef::try_new(
        ToolCallId::try_new("call_a").expect("a provider call id"),
        "project_list_directory",
        ToolCallStatus::Answered,
    )
    .expect("call")
    .with_step(Some(summary.clone()));
    let message = Message::try_new(
        MessageId::from_uuid(Uuid::new_v4()),
        conversation_id.clone(),
        1,
        MessageRole::Assistant,
        vec![],
        vec![call],
    )
    .expect("message")
    .into_completed();
    let snapshot = ChatSnapshot::new(
        Conversation::new(conversation_id, "Fixture"),
        vec![message],
        None,
        vec![],
        SyncCursor::new(1),
    );

    let view = conversation_view_from(snapshot, SyncAvailability::Live);
    let projected = &view.messages[0].tool_calls[0];
    assert_eq!(projected.step, Some(summary));
    // The wire name keeps its place as the row's verbatim secondary identifier:
    // a label cannot be matched against a tool catalog.
    assert_eq!(projected.name, "project_list_directory");
}

/// A renderer reload -- a hot reload in development, a re-auth, a reconnect --
/// re-runs the workspace bootstrap while a run is still streaming. Before this,
/// that bootstrap reported the live run as interrupted and the UI replaced the
/// answer being written with a recovery card, while the backend went on and
/// logged the same run `state=Completed`.
#[test]
fn a_run_this_process_drives_raises_no_recovery_notice() {
    let live = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-live",
            vec![run("conv-live", "run-live", RunState::Running)],
        ),
        active_run_id: None,
        live_run_ids: HashSet::from(["run-live".to_owned()]),
        foreign_run_ids: HashSet::new(),
    };

    assert_eq!(
        startup_recovery_from_workspace(&[live], Some(&ConversationId::from("conv-live"))),
        None
    );
}

/// The same exemption for the other client. xFalcon Web executes its own runs
/// against these conversations, so an in-flight run this device never journaled
/// is someone else's healthy answer, not evidence a process died here.
///
/// `active_run_id` is `None` on purpose: that is the shape this case actually
/// produces, and it is the dangerous one. `startup_recovery_for_states` answers
/// "something is in flight and nothing is active" with an unconditional notice,
/// so excluding a foreign run from the active slot alone would still raise a
/// crash banner. Only removing it from the run set is enough.
#[test]
fn a_run_another_client_drives_raises_no_recovery_notice() {
    let foreign = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-foreign",
            vec![run("conv-foreign", "run-web", RunState::Running)],
        ),
        active_run_id: None,
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::from(["run-web".to_owned()]),
    };

    assert_eq!(
        startup_recovery_from_workspace(&[foreign], Some(&ConversationId::from("conv-foreign"))),
        None
    );
}

/// Per run here too: this device's own abandoned run still needs its banner
/// while the other client streams beside it.
#[test]
fn an_abandoned_run_beside_a_foreign_one_still_raises_the_notice() {
    let mixed = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-mixed-foreign",
            vec![
                run("conv-mixed-foreign", "run-web", RunState::Running),
                run("conv-mixed-foreign", "run-dead", RunState::Interrupted),
            ],
        ),
        active_run_id: Some(RunId::from("run-dead")),
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::from(["run-web".to_owned()]),
    };

    assert_eq!(
        startup_recovery_from_workspace(
            &[mixed],
            Some(&ConversationId::from("conv-mixed-foreign"))
        )
        .as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
    );
}

/// The exemption is per run, not per conversation: a conversation can hold a
/// live run and a run a dead process abandoned, and the second still needs the
/// banner.
#[test]
fn a_dead_run_beside_a_live_one_still_raises_the_notice() {
    let mixed = WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "conv-mixed",
            vec![
                run("conv-mixed", "run-live", RunState::Running),
                run("conv-mixed", "run-dead", RunState::Interrupted),
            ],
        ),
        active_run_id: Some(RunId::from("run-dead")),
        live_run_ids: HashSet::from(["run-live".to_owned()]),
        foreign_run_ids: HashSet::new(),
    };

    assert_eq!(
        startup_recovery_from_workspace(&[mixed], Some(&ConversationId::from("conv-mixed")))
            .as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
    );
}

#[test]
fn historical_recovery_does_not_raise_a_notice_on_an_empty_or_different_chat() {
    let entries = [WorkspaceRecoverySnapshot {
        snapshot: conversation_snapshot(
            "history",
            vec![run("history", "dead", RunState::Interrupted)],
        ),
        active_run_id: Some(RunId::from("dead")),
        live_run_ids: HashSet::new(),
        foreign_run_ids: HashSet::new(),
    }];
    assert_eq!(startup_recovery_from_workspace(&entries, None), None);
    assert_eq!(
        startup_recovery_from_workspace(&entries, Some(&ConversationId::from("other"))),
        None
    );
    assert_eq!(
        startup_recovery_from_workspace(&entries, Some(&ConversationId::from("history")))
            .as_deref(),
        Some(STARTUP_RECOVERY_NOTICE_AWAIT_USER)
    );
}

#[test]
fn recovered_run_remains_actionable_only_in_its_own_account_and_conversation() {
    use crate::chat::cold_start_recovery::RecoveredRuns;
    let mut recovered = RecoveredRuns::default();
    recovered.remember(
        "account",
        &ConversationId::from("history"),
        RunId::from("dead"),
    );
    let cancelled =
        conversation_snapshot("history", vec![run("history", "dead", RunState::Cancelled)]);
    assert_eq!(recovered.for_snapshot("other-account", &cancelled), None);
    let other = conversation_snapshot("other", vec![run("other", "other-run", RunState::Running)]);
    assert_eq!(recovered.for_snapshot("account", &other), None);
    assert_eq!(
        recovered.for_snapshot("account", &cancelled),
        Some(RunId::from("dead"))
    );
}

#[test]
fn newer_or_completed_run_permanently_retires_recovered_notice() {
    use crate::chat::cold_start_recovery::RecoveredRuns;
    for (latest_id, state) in [
        ("dead", RunState::Completed),
        ("new-live", RunState::Running),
        ("new-completed", RunState::Completed),
    ] {
        let mut recovered = RecoveredRuns::default();
        recovered.remember(
            "account",
            &ConversationId::from("history"),
            RunId::from("dead"),
        );
        let snapshot = conversation_snapshot("history", vec![run("history", latest_id, state)]);
        assert_eq!(recovered.for_snapshot("account", &snapshot), None);
        let stale =
            conversation_snapshot("history", vec![run("history", "dead", RunState::Cancelled)]);
        assert_eq!(recovered.for_snapshot("account", &stale), None);
    }
}

/// The composer-lock guard. `conversation_view_from` names the first in-flight
/// run as the view's active run; for a run another client drives, this window
/// will never receive an event that ends it, so the pointer has to go or the
/// composer stays held on a spinner nothing here can finish.
#[test]
fn a_foreign_run_is_not_the_views_active_run() {
    let snapshot = conversation_snapshot(
        "conv-web",
        vec![run("conv-web", "run-web", RunState::Running)],
    );
    let view = conversation_view_from(snapshot, SyncAvailability::Live);
    assert_eq!(
        view.active_run_id.as_ref().map(RunId::as_str),
        Some("run-web"),
        "fixture precondition: an in-flight run is the active run by default"
    );

    let guarded = without_foreign_active_run(view, &HashSet::from(["run-web".to_owned()]));
    assert_eq!(guarded.active_run_id, None);
}

/// The guard is per run: this device's own in-flight run must still drive the
/// view, or the fix would disable the composer's own progress reporting.
#[test]
fn an_owned_run_survives_the_foreign_active_run_guard() {
    let snapshot = conversation_snapshot(
        "conv-mine",
        vec![run("conv-mine", "run-mine", RunState::Running)],
    );
    let view = conversation_view_from(snapshot, SyncAvailability::Live);

    let guarded = without_foreign_active_run(view, &HashSet::from(["run-web".to_owned()]));
    assert_eq!(
        guarded.active_run_id.as_ref().map(RunId::as_str),
        Some("run-mine")
    );
}

/// The reported run state skips past a foreign run to the newest one this
/// window owns. Reporting the foreign run's `Running` would start a spinner
/// with no terminal event behind it.
#[test]
fn the_reported_run_state_skips_a_foreign_run() {
    let snapshot = conversation_snapshot(
        "conv-both",
        vec![
            run("conv-both", "run-mine", RunState::Completed),
            run("conv-both", "run-web", RunState::Running),
        ],
    );

    let reported = latest_owned_run_state(&snapshot, &HashSet::from(["run-web".to_owned()]));
    assert_eq!(
        reported.map(|(id, state)| (id.as_str().to_owned(), state)),
        Some(("run-mine".to_owned(), RunState::Completed))
    );
}

/// A conversation whose only in-flight run belongs to another client reports
/// no run state at all, rather than borrowing one that is not this window's.
#[test]
fn a_conversation_with_only_a_foreign_run_reports_no_run_state() {
    let snapshot = conversation_snapshot(
        "conv-web-only",
        vec![run("conv-web-only", "run-web", RunState::Running)],
    );

    assert_eq!(
        latest_owned_run_state(&snapshot, &HashSet::from(["run-web".to_owned()])),
        None
    );
}
