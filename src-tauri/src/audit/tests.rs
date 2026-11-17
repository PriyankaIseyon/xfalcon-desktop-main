use std::sync::Arc;

use rusqlite::Connection;
use tempfile::tempdir;
use uuid::Uuid;

use async_trait::async_trait;

use super::{
    AuditAction, AuditEvent, AuditOutcome, AuditPublishStatus, AuditQueue, AuditResourceType,
    AuditSink, DaAuditResponse, DaAuditSink, DaAuditTransport,
};
use crate::domain::error::AppResult;
use crate::persistence::{AppendOutcome, InMemorySecretStore, Journal, NewJournalEvent};

const ACCOUNT_ID: &str = "account-1";

fn audit_event(event_id: Uuid, sequence: u64, resource_id: &str) -> AuditEvent {
    AuditEvent::new(
        event_id,
        sequence,
        "actor-1",
        "tenant-1",
        "device-1",
        AuditAction::ProjectRead,
        AuditResourceType::Project,
        resource_id,
        AuditOutcome::Allowed,
        "policy-1",
        format!("request-{sequence}"),
        1_783_500_000_000 + sequence as i64,
    )
    .expect("valid content-free event")
}

#[test]
fn audit_schema_rejects_content_bearing_or_unbounded_fields() {
    assert!(AuditAction::parse("project read with user content").is_err());
    assert!(AuditAction::parse("invented.action").is_err());
    assert!(AuditEvent::new(
        Uuid::new_v4(),
        1,
        "actor\nmessage-content",
        "tenant-1",
        "device-1",
        AuditAction::ProjectRead,
        AuditResourceType::Project,
        "resource-1",
        AuditOutcome::Allowed,
        "policy-1",
        "request-1",
        1_783_500_000_000,
    )
    .is_err());
}

#[test]
fn audit_outcomes_match_the_unchanged_da_constraint() {
    let outcomes = [
        (AuditOutcome::Allowed, "allowed"),
        (AuditOutcome::ApprovalRequired, "approval_required"),
        (AuditOutcome::Approved, "approved"),
        (AuditOutcome::Rejected, "rejected"),
        (AuditOutcome::Denied, "denied"),
        (AuditOutcome::Updated, "updated"),
        (AuditOutcome::Success, "success"),
        (AuditOutcome::Failure, "failure"),
    ];
    for (outcome, expected) in outcomes {
        assert_eq!(
            serde_json::to_value(outcome).expect("serialize audit outcome"),
            serde_json::Value::String(expected.to_owned())
        );
    }
}

#[test]
fn audit_queue_is_encrypted_bounded_ordered_and_duplicate_safe() {
    let directory = tempdir().expect("temporary audit directory");
    let database = directory.path().join("audit.sqlite3");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT_ID, [0xC3; 32])
        .expect("store account key");
    let journal = Arc::new(Journal::open(&database, secrets).expect("open journal"));
    let queue = AuditQueue::new(ACCOUNT_ID, 2, journal).expect("create audit queue");
    let first_id = Uuid::new_v4();
    let first = audit_event(first_id, 1, "resource-sensitive-canary");
    let second = audit_event(Uuid::new_v4(), 2, "resource-2");

    assert_eq!(
        queue.enqueue(&second).expect("enqueue second"),
        AppendOutcome::Inserted
    );
    assert_eq!(
        queue.enqueue(&first).expect("enqueue first"),
        AppendOutcome::Inserted
    );
    assert_eq!(
        queue.enqueue(&first).expect("duplicate first"),
        AppendOutcome::Duplicate
    );
    assert_eq!(
        queue
            .enqueue(&audit_event(Uuid::new_v4(), 3, "resource-3"))
            .expect("bounded enqueue"),
        AppendOutcome::CapacityReached
    );

    let database_bytes = std::fs::read(&database).expect("read encrypted database");
    assert!(!database_bytes
        .windows(b"resource-sensitive-canary".len())
        .any(|window| window == b"resource-sensitive-canary"));
    let stored_ciphertext: Vec<u8> = Connection::open(&database)
        .expect("inspect database")
        .query_row(
            "SELECT ciphertext FROM journal_events WHERE event_id = ?1",
            [first_id.to_string()],
            |row| row.get(0),
        )
        .expect("stored ciphertext");
    assert!(!stored_ciphertext
        .windows(b"resource-sensitive-canary".len())
        .any(|window| window == b"resource-sensitive-canary"));

    let replay = queue.events_after(0).expect("replay audit queue");
    assert_eq!(
        replay.iter().map(AuditEvent::sequence).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(queue.acknowledge(&replay[0]).expect("acknowledge first"));
    assert_eq!(
        queue
            .enqueue(&audit_event(Uuid::new_v4(), 3, "resource-3"))
            .expect("enqueue after acknowledgement"),
        AppendOutcome::Inserted
    );
}

#[test]
fn audit_queue_purge_is_account_scoped() {
    let directory = tempdir().expect("temporary audit directory");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key(ACCOUNT_ID, [0xC3; 32])
        .expect("store first key");
    secrets
        .put_key("account-2", [0xD4; 32])
        .expect("store second key");
    let journal = Arc::new(
        Journal::open(directory.path().join("audit.sqlite3"), secrets).expect("open journal"),
    );
    journal
        .append(NewJournalEvent {
            account_id: ACCOUNT_ID.to_owned(),
            event_id: "run-event-1".to_owned(),
            run_id: "run-1".to_owned(),
            sequence: 1,
            event_type: "assistant_text_delta".to_owned(),
            state: "received".to_owned(),
            payload: b"run-state".to_vec(),
            created_at_ms: 1_783_500_000_001,
        })
        .expect("append non-audit run event");
    let first = AuditQueue::new(ACCOUNT_ID, 8, journal.clone()).expect("first queue");
    let second = AuditQueue::new("account-2", 8, journal.clone()).expect("second queue");
    first
        .enqueue(&audit_event(Uuid::new_v4(), 1, "resource-1"))
        .expect("enqueue first");
    second
        .enqueue(&audit_event(Uuid::new_v4(), 1, "resource-2"))
        .expect("enqueue second");

    assert_eq!(first.purge().expect("purge first account"), 1);
    assert!(first.events_after(0).expect("first replay").is_empty());
    assert_eq!(
        journal
            .events_after(ACCOUNT_ID, "run-1", 0)
            .expect("non-audit replay")
            .len(),
        1
    );
    assert_eq!(second.events_after(0).expect("second replay").len(), 1);
}

struct FakeAuditTransport(DaAuditResponse);

#[async_trait]
impl DaAuditTransport for FakeAuditTransport {
    async fn create_audit_event(&self, _event: &AuditEvent) -> AppResult<DaAuditResponse> {
        Ok(self.0)
    }
}

#[tokio::test]
async fn audit_sink_surfaces_inherited_authorization_controls() {
    let event = audit_event(Uuid::new_v4(), 1, "resource-1");
    for response in [DaAuditResponse::Unauthorized, DaAuditResponse::Forbidden] {
        let sink = DaAuditSink::new(FakeAuditTransport(response));
        assert_eq!(
            sink.publish(&event).await.expect("publish status"),
            AuditPublishStatus::InheritedControl
        );
    }
    let sink = DaAuditSink::new(FakeAuditTransport(DaAuditResponse::Created));
    assert_eq!(
        sink.publish(&event).await.expect("publish status"),
        AuditPublishStatus::Published
    );
}
