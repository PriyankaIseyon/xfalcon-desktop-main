use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tempfile::tempdir;
use uuid::Uuid;
use xfalcon_contracts::auth::{IdentityProjection, SessionProjection};
use zeroize::Zeroizing;

use super::local::{compute_mac, Limits, FORMAT_VERSION};
use super::{
    render_diagnostics_section, AuditAction, AuditEvent, AuditHub, AuditOutcome,
    AuditPublishStatus, AuditPublisher, AuditQueue, AuditResourceType, AuditSink, LocalAuditStore,
};
use crate::auth::AuthSessionStore;
use crate::domain::error::{AppError, AppResult};
use crate::persistence::{InMemorySecretStore, Journal, SecretStore};

const KEY_ACCOUNT: &str = "device-audit-chain";

fn secrets() -> Arc<InMemorySecretStore> {
    Arc::new(InMemorySecretStore::default())
}

fn open(root: &Path, secrets: &Arc<InMemorySecretStore>, limits: Limits) -> LocalAuditStore {
    LocalAuditStore::open_with_limits(root.to_path_buf(), secrets.clone(), limits)
        .expect("open audit store")
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn event_at(resource: &str, ts_ms: i64) -> AuditEvent {
    AuditEvent::new(
        Uuid::new_v4(),
        1,
        "actor-1",
        "tenant-1",
        "device-1",
        AuditAction::ProjectRead,
        AuditResourceType::Project,
        resource,
        AuditOutcome::Allowed,
        "policy-1",
        "request-1",
        ts_ms,
    )
    .expect("valid event")
}

fn segment_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(root)
        .expect("read audit directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("audit-") && name.ends_with(".jsonl"))
        })
        .collect();
    paths.sort();
    paths
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .expect("read segment")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn write_lines(path: &Path, lines: &[String]) {
    let mut content = lines.join("\n");
    content.push('\n');
    fs::write(path, content).expect("write segment");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fill(store: &LocalAuditStore, count: usize) {
    let ts = now_ms();
    for index in 1..=count {
        store
            .append(event_at(&format!("res-{index}"), ts))
            .expect("append");
    }
}

#[test]
fn appended_records_verify() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 2);

    let report = store.verify().expect("verify");
    assert!(report.ok);
    assert_eq!(report.records, 2);
    assert_eq!(report.first_seq, Some(1));
    assert_eq!(report.last_seq, Some(2));
    assert_eq!(report.segments, 1);
    assert!(!report.anchored_after_prune);
    assert!(!report.torn_tail_repaired);
    assert_eq!(report.problem, None);
}

#[test]
fn device_id_is_stable_across_reopen() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let first = open(dir.path(), &secrets, Limits::default());
    let id = first.device_id().to_owned();
    assert!(Uuid::parse_str(&id).is_ok());
    drop(first);
    let second = open(dir.path(), &secrets, Limits::default());
    assert_eq!(second.device_id(), id);
}

#[test]
fn a_thousand_records_across_segments_verify_and_recent_returns_the_tail() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(
        dir.path(),
        &secrets,
        Limits {
            segment_max_bytes: 4096,
            ..Limits::default()
        },
    );
    fill(&store, 1000);

    assert!(segment_paths(dir.path()).len() > 1);
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 1000);
    assert_eq!(report.last_seq, Some(1000));

    let tail = store.recent(5).expect("recent");
    assert_eq!(
        tail.iter().map(|record| record.seq()).collect::<Vec<_>>(),
        vec![996, 997, 998, 999, 1000]
    );
    assert_eq!(store.recent(5000).expect("recent all").len(), 1000);
    assert!(store.recent(0).expect("recent none").is_empty());
}

#[test]
fn an_edited_byte_in_the_middle_is_named_by_sequence() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 10);

    let path = segment_paths(dir.path()).remove(0);
    let mut lines = read_lines(&path);
    lines[4] = lines[4].replace("\"res-5\"", "\"res-x\"");
    write_lines(&path, &lines);

    let report = store.verify().expect("verify");
    assert!(!report.ok);
    assert_eq!(
        report.problem.as_deref(),
        Some("record 5 fails its integrity check")
    );
    assert_eq!(report.records, 4);
    assert!(
        !report.problem.unwrap_or_default().contains("res-x"),
        "the problem must not contain event content"
    );
}

#[test]
fn a_deleted_middle_line_fails() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 10);

    let path = segment_paths(dir.path()).remove(0);
    let mut lines = read_lines(&path);
    lines.remove(4);
    write_lines(&path, &lines);

    let report = store.verify().expect("verify");
    assert!(!report.ok);
    assert_eq!(
        report.problem.as_deref(),
        Some("sequence jumps from 4 to 6")
    );
}

#[test]
fn deleting_the_head_of_the_oldest_segment_fails() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 10);

    // Records 1 to 3 cut from the only, and so the oldest, segment. The chain
    // still links from record 3's MAC, so only the file name can refuse it.
    let path = segment_paths(dir.path()).remove(0);
    let mut lines = read_lines(&path);
    lines.drain(..3);
    write_lines(&path, &lines);

    let report = store.verify().expect("verify");
    assert!(!report.ok);
    assert_eq!(
        report.problem.as_deref(),
        Some("segment 1 begins with record 4")
    );
}

#[test]
fn deleting_the_whole_oldest_segment_is_reported_as_pruning() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(
        dir.path(),
        &secrets,
        Limits {
            segment_max_bytes: 600,
            ..Limits::default()
        },
    );
    fill(&store, 20);
    assert!(segment_paths(dir.path()).len() > 1);

    fs::remove_file(segment_paths(dir.path()).remove(0)).expect("delete oldest segment");

    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert!(report.anchored_after_prune);
    assert!(report.first_seq.is_some_and(|seq| seq > 1));
}

#[test]
fn a_first_record_with_a_zero_link_must_be_record_one() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    open(&root, &secrets, Limits::default());

    // A record 5 that claims to start the log. Its MAC is valid for those
    // fields, so only the sequence rule can refuse it.
    let version = secrets
        .active_key_version(KEY_ACCOUNT)
        .expect("active key version");
    let key = secrets.account_key(KEY_ACCOUNT, version).expect("key");
    let ts = now_ms();
    let event = event_at("res-5", ts);
    let event_json = serde_json::to_vec(&event).expect("event json");
    let prev = [0_u8; 32];
    let mac = compute_mac(&key, FORMAT_VERSION, 5, ts, &prev, &event_json);
    let record = serde_json::json!({
        "v": FORMAT_VERSION,
        "seq": 5,
        "ts_ms": ts,
        "prev": hex(&prev),
        "mac": hex(mac.as_bytes()),
        "event": event,
    });
    fs::write(
        root.join("audit-00000000000000000005.jsonl"),
        format!("{record}\n"),
    )
    .expect("write forged segment");

    let store = open(&root, &secrets, Limits::default());
    let report = store.verify().expect("verify");
    assert!(!report.ok);
    assert_eq!(
        report.problem.as_deref(),
        Some("record 5 has no predecessor and is not record 1")
    );
}

#[test]
fn swapped_lines_fail() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 10);

    let path = segment_paths(dir.path()).remove(0);
    let mut lines = read_lines(&path);
    lines.swap(3, 4);
    write_lines(&path, &lines);

    let report = store.verify().expect("verify");
    assert!(!report.ok);
    assert!(report.problem.is_some());
}

#[test]
fn a_log_verified_with_a_different_key_fails() {
    let dir = tempdir().expect("temp dir");
    let original = secrets();
    let store = open(dir.path(), &original, Limits::default());
    fill(&store, 3);
    drop(store);

    let other = secrets();
    let reopened = open(dir.path(), &other, Limits::default());
    let report = reopened.verify().expect("verify");
    assert!(!report.ok);
    assert_eq!(
        report.problem.as_deref(),
        Some("record 1 fails its integrity check")
    );
}

#[test]
fn a_truncated_last_line_is_repaired_on_reopen_and_the_chain_continues() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 3);
    drop(store);

    let path = segment_paths(dir.path()).remove(0);
    let bytes = fs::read(&path).expect("read segment");
    fs::write(&path, &bytes[..bytes.len() - 10]).expect("truncate last line");

    let store = open(dir.path(), &secrets, Limits::default());
    let repaired = store.verify().expect("verify");
    assert!(repaired.ok, "{:?}", repaired.problem);
    assert!(repaired.torn_tail_repaired);
    assert_eq!(repaired.records, 2);

    store
        .append(event_at("res-next", now_ms()))
        .expect("append after repair");
    let report = store.verify().expect("verify after append");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 3);
    assert_eq!(report.last_seq, Some(3));
}

#[test]
fn a_month_boundary_starts_a_new_segment_and_keeps_the_chain() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    let january = Utc
        .with_ymd_and_hms(2026, 1, 31, 23, 59, 59)
        .single()
        .expect("january instant")
        .timestamp_millis();
    store.append(event_at("res-jan", january)).expect("append");
    store
        .append(event_at("res-feb", january + 2_000))
        .expect("append across month");

    assert_eq!(segment_paths(dir.path()).len(), 2);
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 2);
}

#[test]
fn a_size_rollover_starts_new_segments_and_keeps_the_chain() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(
        dir.path(),
        &secrets,
        Limits {
            segment_max_bytes: 1000,
            ..Limits::default()
        },
    );
    fill(&store, 10);

    assert!(segment_paths(dir.path()).len() > 1);
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 10);
}

#[test]
fn retention_prunes_the_oldest_segment_and_reports_the_anchor() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(
        dir.path(),
        &secrets,
        Limits {
            segment_max_bytes: 600,
            ..Limits::default()
        },
    );
    let old = Utc
        .with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
        .single()
        .expect("old instant")
        .timestamp_millis();
    for index in 0..6 {
        store
            .append(event_at(&format!("old-{index}"), old))
            .expect("append old");
    }
    // Old records are already past retention, so each rollover during this
    // phase prunes the segment before it; only the newest one survives.
    assert_eq!(segment_paths(dir.path()).len(), 1);
    let before = store.verify().expect("verify before new records");
    assert!(before.ok, "{:?}", before.problem);
    assert!(before.anchored_after_prune);

    fill(&store, 6);

    // The 2020 segment is pruned at the month rollover, so the first retained
    // record is seq 7 (the 2020 records were 1 through 6).
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert!(report.anchored_after_prune);
    assert_eq!(report.records, 6);
    assert_eq!(report.first_seq, Some(7));
}

#[test]
fn the_total_size_limit_prunes_oldest_segments_but_never_the_newest() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(
        dir.path(),
        &secrets,
        Limits {
            segment_max_bytes: 600,
            total_max_bytes: 1000,
            ..Limits::default()
        },
    );
    fill(&store, 20);

    assert!(segment_paths(dir.path()).len() <= 3);
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert!(report.anchored_after_prune);
    assert_eq!(report.last_seq, Some(20));
}

#[test]
fn concurrent_appends_keep_the_chain_valid_and_gap_free() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(
        dir.path(),
        &secrets,
        Limits {
            segment_max_bytes: 8192,
            ..Limits::default()
        },
    ));
    let ts = now_ms();
    let handles: Vec<_> = (0..8)
        .map(|thread_index| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for index in 0..200 {
                    store
                        .append(event_at(&format!("t{thread_index}-{index}"), ts))
                        .expect("concurrent append");
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("append thread");
    }

    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 1600);
    assert_eq!(report.first_seq, Some(1));
    assert_eq!(report.last_seq, Some(1600));
}

#[test]
fn identity_defaults_and_can_be_set_and_cleared() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let hub = AuditHub::new(Arc::new(open(dir.path(), &secrets, Limits::default())));
    let store_for_reads = open(dir.path(), &secrets, Limits::default());

    hub.record(
        AuditAction::AuthLogin,
        AuditResourceType::Account,
        "account-1",
        AuditOutcome::Success,
        "policy-1",
        "request-1",
    );
    hub.set_identity("user@example.com", "tenant-9");
    hub.record(
        AuditAction::ProjectRead,
        AuditResourceType::Project,
        "project-1",
        AuditOutcome::Allowed,
        "policy-1",
        "request-2",
    );
    hub.clear_identity();
    hub.record(
        AuditAction::AuthLogout,
        AuditResourceType::Session,
        "session-1",
        AuditOutcome::Success,
        "policy-1",
        "request-3",
    );

    let records = store_for_reads.recent(3).expect("recent");
    assert_eq!(records[0].event().actor_id(), "unauthenticated");
    assert_eq!(records[0].event().tenant_id(), "unconfigured");
    assert_eq!(records[1].event().actor_id(), "_REDACTED_EMAIL_");
    assert!(!records[1].event().actor_id().contains("user@example.com"));
    assert!(!records[1].event().actor_id().contains("user_example.com"));
    assert!(!records[1].event().actor_id().contains("user@example"));
    assert!(!records[1].event().actor_id().contains("user_example"));
    assert_eq!(records[1].event().tenant_id(), "tenant-9");
    assert_eq!(records[2].event().actor_id(), "unauthenticated");
    assert_eq!(hub.dropped_events(), 0);
}

fn signed_in() -> SessionProjection {
    SessionProjection::authenticated(
        IdentityProjection::new("user-1", "tenant-1", "User", None),
        120_000,
    )
}

#[test]
fn every_end_of_an_authenticated_session_clears_the_audit_identity() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = Arc::new(AuditHub::new(Arc::clone(&store)));
    let sessions = AuthSessionStore::new();
    let hook_hub = Arc::clone(&hub);
    sessions.set_session_end_hook(move || hook_hub.clear_identity());

    // Logout and cancel clear; a refused renewal clears; a new sign-in
    // replaces the session through `set`.
    let endings: [fn(&AuthSessionStore); 3] = [
        |sessions| sessions.clear(),
        |sessions| sessions.set(SessionProjection::authenticating()),
        |sessions| sessions.set(SessionProjection::unauthenticated()),
    ];
    for end in endings {
        sessions.set_authenticated(signed_in(), "access-1", Some("refresh-1".to_owned()));
        hub.set_identity("user-1", "tenant-1");
        end(&sessions);
        hub.record(
            AuditAction::ProjectRead,
            AuditResourceType::Project,
            "project-1",
            AuditOutcome::Allowed,
            "policy-1",
            "request-1",
        );
    }

    let records = store.recent(3).expect("recent");
    assert_eq!(records.len(), 3);
    for record in records {
        assert_eq!(record.event().actor_id(), "unauthenticated");
        assert_eq!(record.event().tenant_id(), "unconfigured");
    }
}

#[test]
fn renewing_a_session_keeps_the_audit_identity() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = Arc::new(AuditHub::new(Arc::clone(&store)));
    let sessions = AuthSessionStore::new();
    let hook_hub = Arc::clone(&hub);
    sessions.set_session_end_hook(move || hook_hub.clear_identity());

    sessions.set_authenticated(signed_in(), "access-1", Some("refresh-1".to_owned()));
    hub.set_identity("user-1", "tenant-1");
    // A renewal replaces the bearer while the same person stays signed in.
    sessions.set_authenticated(signed_in(), "access-2", Some("refresh-2".to_owned()));
    hub.record(
        AuditAction::ProjectRead,
        AuditResourceType::Project,
        "project-1",
        AuditOutcome::Allowed,
        "policy-1",
        "request-1",
    );

    let record = store.recent(1).expect("recent").remove(0);
    assert_eq!(record.event().actor_id(), "user-1");
}

#[test]
fn out_of_charset_resource_ids_are_sanitized_not_dropped() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = AuditHub::new(Arc::clone(&store));

    hub.record(
        AuditAction::ToolExecute,
        AuditResourceType::ToolCall,
        "inv/with spaces\nand",
        AuditOutcome::Success,
        "policy-1",
        "request-1",
    );

    let records = store.recent(1).expect("recent");
    assert_eq!(records[0].event().resource_id(), "inv_with_spaces_and");
    assert_eq!(hub.dropped_events(), 0);
}

#[test]
fn a_failed_append_is_counted_as_dropped_and_does_not_panic() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let hub = AuditHub::new(Arc::new(open(&root, &secrets, Limits::default())));

    fs::remove_dir_all(&root).expect("remove audit directory");
    hub.record(
        AuditAction::ToolDeny,
        AuditResourceType::ToolCall,
        "inv-1",
        AuditOutcome::Denied,
        "policy-1",
        "request-1",
    );

    assert_eq!(hub.dropped_events(), 1);
}

#[test]
fn new_enum_strings_round_trip_through_parse_as_str_and_serde() {
    let actions = [
        (AuditAction::AuthLoginFailed, "auth.login_failed"),
        (AuditAction::McpServerCreate, "mcp.server_create"),
        (AuditAction::McpServerUpdate, "mcp.server_update"),
        (AuditAction::McpServerDelete, "mcp.server_delete"),
        (AuditAction::McpSignIn, "mcp.sign_in"),
        (AuditAction::McpSignOut, "mcp.sign_out"),
        (AuditAction::SkillImport, "skill.import"),
        (AuditAction::SkillDelete, "skill.delete"),
        (AuditAction::DataExport, "data.export"),
        (AuditAction::DataErase, "data.erase"),
        (AuditAction::DiagnosticsExport, "diagnostics.export"),
    ];
    for (action, wire) in actions {
        assert_eq!(action.as_str(), wire);
        assert_eq!(AuditAction::parse(wire).expect("parse"), action);
        assert_eq!(
            serde_json::to_value(action).expect("serialize"),
            serde_json::Value::String(wire.to_owned())
        );
        assert_eq!(
            serde_json::from_value::<AuditAction>(serde_json::Value::String(wire.to_owned()))
                .expect("deserialize"),
            action
        );
    }

    let resource_types = [
        (AuditResourceType::Skill, "skill"),
        (AuditResourceType::Export, "export"),
        (AuditResourceType::LocalData, "local_data"),
        (AuditResourceType::Diagnostics, "diagnostics"),
    ];
    for (resource_type, wire) in resource_types {
        assert_eq!(resource_type.as_str(), wire);
        assert_eq!(
            serde_json::to_value(resource_type).expect("serialize"),
            serde_json::Value::String(wire.to_owned())
        );
    }
}

#[test]
fn diagnostics_section_shows_integrity_and_only_the_listed_fields() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = AuditHub::new(Arc::clone(&store));
    hub.set_identity("user-1", "tenant-1");
    for index in 1..=3 {
        hub.record(
            AuditAction::ProjectRead,
            AuditResourceType::Project,
            &format!("res-{index}"),
            AuditOutcome::Allowed,
            "policy-1",
            "request-1",
        );
    }

    let rendered = render_diagnostics_section(&store, Some(&hub), 10);
    assert!(rendered.starts_with("## 4. Security Audit Trail"));
    assert!(rendered.contains("Integrity: verified"));
    assert!(rendered.contains("- Records: 3"));
    assert!(rendered.contains("- Dropped events this session: 0"));
    assert!(rendered.contains("seq=1 project.read project:res-1 outcome=allowed actor=user-1"));
    assert!(!rendered.contains('{'));
    assert!(!rendered.contains("mac"));
    assert!(!rendered.contains("prev"));

    let path = segment_paths(dir.path()).remove(0);
    let last: serde_json::Value =
        serde_json::from_str(read_lines(&path).last().expect("last line"))
            .expect("parse last line");
    let mac = last["mac"].as_str().expect("mac");
    assert!(!rendered.contains(mac));
    let version = secrets
        .active_key_version(KEY_ACCOUNT)
        .expect("active key version");
    let key = secrets.account_key(KEY_ACCOUNT, version).expect("key");
    assert!(!rendered.contains(&hex(&key[..])));
}

#[test]
fn diagnostics_section_shows_failed_for_a_tampered_log() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    fill(&store, 4);

    let path = segment_paths(dir.path()).remove(0);
    let mut lines = read_lines(&path);
    lines[1] = lines[1].replace("\"res-2\"", "\"res-x\"");
    write_lines(&path, &lines);

    let rendered = render_diagnostics_section(&store, None, 10);
    assert!(rendered.contains("Integrity: FAILED — record 2 fails its integrity check"));
    assert!(rendered.contains("- Dropped events this session: not tracked"));
    assert!(!rendered.contains('{'));
    let integrity = rendered
        .lines()
        .find(|line| line.starts_with("Integrity:"))
        .expect("integrity line");
    assert!(!integrity.contains("res-x"));
}

#[test]
fn diagnostics_section_reports_an_unreadable_store_with_its_error_kind() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fs::remove_dir_all(&root).expect("remove audit directory");

    let rendered = render_diagnostics_section(&store, None, 10);
    assert!(rendered.contains("_The audit trail could not be read._"));
    assert!(rendered.contains("audit_storage_failed"));
}

#[tokio::test]
async fn a_hub_bound_publisher_clone_shares_one_chain_and_reads_identity_at_publish_time() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = Arc::new(AuditHub::new(Arc::clone(&store)));
    let publisher = hub.publisher();
    let clone = publisher.clone();

    hub.set_identity("user-2", "tenant-2");
    publisher
        .publish(
            AuditAction::ProjectRead,
            AuditResourceType::Project,
            "project-a",
            AuditOutcome::Allowed,
            "policy-1",
            "request-1",
        )
        .await;
    clone
        .publish(
            AuditAction::ProjectRead,
            AuditResourceType::Project,
            "project-b",
            AuditOutcome::Allowed,
            "policy-1",
            "request-2",
        )
        .await;

    let records = store.recent(2).expect("recent");
    assert_eq!(
        records
            .iter()
            .map(|record| record.seq())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(records[0].event().actor_id(), "user-2");
    assert_eq!(records[1].event().resource_id(), "project-b");
    assert_eq!(records[1].event().device_id(), store.device_id());
    assert!(store.verify().expect("verify").ok);
}

/// Rewrites one top-level JSON field of the record at `index` in a segment.
fn edit_record_field(path: &Path, index: usize, field: &str, value: serde_json::Value) {
    let mut lines = read_lines(path);
    let mut record: serde_json::Value = serde_json::from_str(&lines[index]).expect("parse record");
    record[field] = value;
    lines[index] = record.to_string();
    write_lines(path, &lines);
}

#[test]
fn editing_any_stored_field_of_a_middle_record_fails_by_its_position() {
    let edits = [
        ("v", serde_json::json!(2)),
        ("seq", serde_json::json!(50)),
        ("ts_ms", serde_json::json!(0)),
        ("prev", serde_json::json!("0".repeat(64))),
        ("mac", serde_json::json!("0".repeat(64))),
    ];
    for (field, value) in edits {
        let dir = tempdir().expect("temp dir");
        let secrets = secrets();
        let store = open(dir.path(), &secrets, Limits::default());
        fill(&store, 10);

        let path = segment_paths(dir.path()).remove(0);
        edit_record_field(&path, 4, field, value);

        let report = store.verify().expect("verify");
        assert!(!report.ok, "{field}");
        assert_eq!(
            report.problem.as_deref(),
            Some("record 5 fails its integrity check"),
            "{field}"
        );
        assert_eq!(report.records, 4, "{field}");
    }
}

#[test]
fn a_record_without_detail_serializes_without_a_detail_key() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());
    store.append(event_at("res-1", now_ms())).expect("append");

    let line = read_lines(&segment_paths(dir.path()).remove(0)).remove(0);
    assert!(!line.contains("\"detail\""));
    let value = serde_json::to_value(event_at("res-1", now_ms())).expect("serialize");
    assert!(value.get("detail").is_none());
}

#[test]
fn detail_is_validated_stored_and_round_trips() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = open(dir.path(), &secrets, Limits::default());

    let event = event_at("res-1", now_ms())
        .with_detail("tool-call.7:a")
        .expect("valid detail");
    assert_eq!(event.detail(), Some("tool-call.7:a"));
    assert!(event_at("res-1", now_ms())
        .with_detail("has space")
        .is_err());
    assert!(event_at("res-1", now_ms()).with_detail("").is_err());
    assert!(event_at("res-1", now_ms())
        .with_detail("d".repeat(161))
        .is_err());
    assert!(event_at("res-1", now_ms())
        .with_detail("d".repeat(160))
        .is_ok());

    store.append(event).expect("append");
    let record = store.recent(1).expect("recent").remove(0);
    assert_eq!(record.event().detail(), Some("tool-call.7:a"));
    assert!(store.verify().expect("verify").ok);
}

#[test]
fn record_with_detail_is_sanitized_and_printed_by_diagnostics() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = AuditHub::new(Arc::clone(&store));
    hub.set_identity("user-1", "tenant-1");

    hub.record_with_detail(
        AuditAction::ToolExecute,
        AuditResourceType::ToolCall,
        "inv-1",
        AuditOutcome::Success,
        "policy-1",
        "request-1",
        "tool id/x",
    );

    let record = store.recent(1).expect("recent").remove(0);
    assert_eq!(record.event().detail(), Some("tool_id_x"));
    assert_eq!(hub.dropped_events(), 0);
    let rendered = render_diagnostics_section(&store, Some(&hub), 10);
    assert!(rendered.contains("outcome=success actor=user-1 detail=tool_id_x\n"));
    assert!(rendered.contains("- Previous log damaged and set aside: no\n"));
}

#[tokio::test]
async fn publish_with_detail_in_hub_mode_stores_the_detail() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let store = Arc::new(open(dir.path(), &secrets, Limits::default()));
    let hub = Arc::new(AuditHub::new(Arc::clone(&store)));
    let publisher = hub.publisher();

    publisher
        .publish_with_detail(
            AuditAction::ToolExecute,
            AuditResourceType::ToolCall,
            "inv-1",
            AuditOutcome::Success,
            "policy-1",
            "request-1",
            "tool-7",
        )
        .await;

    let record = store.recent(1).expect("recent").remove(0);
    assert_eq!(record.event().detail(), Some("tool-7"));
    assert!(store.verify().expect("verify").ok);
}

struct RecordingSink {
    published: Mutex<Vec<AuditEvent>>,
}

#[async_trait]
impl AuditSink for RecordingSink {
    async fn publish(&self, event: &AuditEvent) -> AppResult<AuditPublishStatus> {
        self.published.lock().expect("lock").push(event.clone());
        Ok(AuditPublishStatus::Published)
    }
}

#[tokio::test]
async fn publish_with_detail_in_spool_mode_behaves_as_publish() {
    let directory = tempdir().expect("temp dir");
    let secrets = Arc::new(InMemorySecretStore::default());
    secrets
        .put_key("account-1", [0xC3; 32])
        .expect("store account key");
    let journal = Arc::new(
        Journal::open(directory.path().join("audit.sqlite3"), secrets).expect("open journal"),
    );
    let queue = Arc::new(AuditQueue::new("account-1", 8, journal).expect("queue"));
    let sink = Arc::new(RecordingSink {
        published: Mutex::new(Vec::new()),
    });
    let publisher = AuditPublisher::new(
        "actor-1",
        "tenant-1",
        "device-1",
        sink.clone(),
        queue.clone(),
    );

    publisher
        .publish_with_detail(
            AuditAction::ToolExecute,
            AuditResourceType::ToolCall,
            "inv-1",
            AuditOutcome::Success,
            "policy-1",
            "request-1",
            "tool-7",
        )
        .await;
    publisher
        .publish(
            AuditAction::ToolExecute,
            AuditResourceType::ToolCall,
            "inv-2",
            AuditOutcome::Success,
            "policy-1",
            "request-2",
        )
        .await;

    let published = sink.published.lock().expect("lock");
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|event| event.detail().is_none()));
    assert!(queue.events_after(0).expect("replay").is_empty());
}

fn set_aside_directories(parent: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(parent)
        .expect("read parent directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("audit.damaged-"))
        })
        .collect();
    paths.sort();
    paths
}

/// Appends one complete line to the newest segment.
fn append_line(path: &Path, line: &str) {
    let mut content = fs::read_to_string(path).expect("read segment");
    content.push_str(line);
    content.push('\n');
    fs::write(path, content).expect("write segment");
}

/// Every entry name in `root`, sorted.
fn entry_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .expect("read audit directory")
        .map(|entry| {
            entry
                .expect("directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn a_damaged_log_is_set_aside_and_a_fresh_chain_starts() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 3);
    drop(store);
    append_line(&segment_paths(&root).remove(0), "not an audit record");

    let store = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    )
    .expect("recover from damage");
    assert!(store.recovered_from_damage());
    store
        .append(event_at("res-fresh", now_ms()))
        .expect("append to fresh log");
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 1);
    assert_eq!(report.first_seq, Some(1));
    assert!(!report.anchored_after_prune);

    let set_aside = set_aside_directories(dir.path());
    assert_eq!(set_aside.len(), 1);
    assert_eq!(segment_paths(&set_aside[0]).len(), 1);
    assert_eq!(read_lines(&segment_paths(&set_aside[0])[0]).len(), 4);
}

#[test]
fn an_unreadable_segment_fails_the_open_without_touching_the_log() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 2);
    drop(store);
    let segment = segment_paths(&root).remove(0);
    let before = fs::read(&segment).expect("read segment");
    // A directory where a segment file should be: reading it fails on every
    // platform, including for root, where a permission change would not.
    let unreadable = root.join("audit-00000000000000000099.jsonl");
    fs::create_dir(&unreadable).expect("unreadable segment");

    let result = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    );
    assert!(matches!(result, Err(AppError::AuditStorageFailed)));
    assert!(set_aside_directories(dir.path()).is_empty());
    assert_eq!(fs::read(&segment).expect("read segment"), before);

    // The next launch, once the obstruction is gone, opens the same history.
    fs::remove_dir(&unreadable).expect("remove unreadable segment");
    let store = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    )
    .expect("reopen after the obstruction is gone");
    assert!(!store.recovered_from_damage());
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert_eq!(report.records, 2);
}

#[test]
fn garbage_in_the_middle_of_the_newest_segment_is_set_aside() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 5);
    drop(store);
    let path = segment_paths(&root).remove(0);
    let mut lines = read_lines(&path);
    lines[2] = "garbage".to_owned();
    write_lines(&path, &lines);

    let store = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    )
    .expect("recover from damage");
    assert!(store.recovered_from_damage());
    assert_eq!(set_aside_directories(dir.path()).len(), 1);
}

#[test]
fn bytes_after_the_last_newline_are_cut_and_the_log_opens() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 3);
    drop(store);

    let path = segment_paths(&root).remove(0);
    let original = fs::read(&path).expect("read segment");
    let mut torn = original.clone();
    torn.extend_from_slice(b"{\"v\":1,\"seq\":4,\"ts_m");
    fs::write(&path, &torn).expect("append an unfinished write");

    let store = open(&root, &secrets, Limits::default());
    let report = store.verify().expect("verify");
    assert!(report.ok, "{:?}", report.problem);
    assert!(report.torn_tail_repaired);
    assert_eq!(report.records, 3);
    assert_eq!(fs::read(&path).expect("read segment"), original);
}

#[test]
fn a_complete_corrupt_last_record_is_never_dropped() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 3);
    drop(store);

    // The last line is complete (newline-terminated) but no longer parses.
    let path = segment_paths(&root).remove(0);
    let mut lines = read_lines(&path);
    let last = lines.len() - 1;
    let corrupted = lines[last][..lines[last].len() - 10].to_owned();
    lines[last] = corrupted.clone();
    write_lines(&path, &lines);

    assert!(
        LocalAuditStore::open_with_limits(root.clone(), secrets.clone(), Limits::default())
            .is_err()
    );

    let store = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    )
    .expect("recover from damage");
    assert!(store.recovered_from_damage());
    let set_aside = set_aside_directories(dir.path());
    assert_eq!(set_aside.len(), 1);
    let kept = read_lines(&segment_paths(&set_aside[0])[0]);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept.last(), Some(&corrupted));
}

#[test]
fn two_corrupt_trailing_lines_are_set_aside_without_panicking() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 3);
    drop(store);
    let path = segment_paths(&root).remove(0);
    append_line(&path, "first corrupt line");
    append_line(&path, "second corrupt line");

    let store = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    )
    .expect("recover from damage");
    assert!(store.recovered_from_damage());
    let set_aside = set_aside_directories(dir.path());
    assert_eq!(set_aside.len(), 1);
    assert_eq!(read_lines(&segment_paths(&set_aside[0])[0]).len(), 5);
}

#[test]
fn quarantining_a_damaged_log_keeps_the_device_id() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 3);
    let id = store.device_id().to_owned();
    drop(store);
    append_line(&segment_paths(&root).remove(0), "not an audit record");

    let store = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        secrets.clone(),
        Limits::default(),
    )
    .expect("recover from damage");
    assert!(store.recovered_from_damage());
    assert_eq!(store.device_id(), id);
    assert_eq!(
        fs::read_to_string(root.join("device-id")).expect("device id file"),
        id
    );
}

#[test]
fn a_malformed_device_id_does_not_quarantine_the_log() {
    for content in ["", "not-a-device-id"] {
        let dir = tempdir().expect("temp dir");
        let secrets = secrets();
        let root = dir.path().join("audit");
        let store = open(&root, &secrets, Limits::default());
        fill(&store, 2);
        drop(store);
        fs::write(root.join("device-id"), content).expect("damage device id");

        let store = LocalAuditStore::open_or_quarantine_with_limits(
            root.clone(),
            secrets.clone(),
            Limits::default(),
        )
        .expect("open with a malformed device id");
        assert!(!store.recovered_from_damage());
        assert!(set_aside_directories(dir.path()).is_empty());
        assert!(Uuid::parse_str(store.device_id()).is_ok());
        let report = store.verify().expect("verify");
        assert!(report.ok, "{:?}", report.problem);
        assert_eq!(report.records, 2);

        // The new id is written in full, and no temporary file is left behind.
        assert_eq!(
            fs::read_to_string(root.join("device-id")).expect("read device id"),
            store.device_id()
        );
        assert!(entry_names(&root)
            .iter()
            .all(|name| name == "device-id" || name.starts_with("audit-")));
    }
}

#[test]
fn at_most_three_damaged_logs_are_kept_after_four_recoveries() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 1);
    drop(store);

    for _ in 0..4 {
        append_line(&segment_paths(&root).remove(0), "not an audit record");
        let store = LocalAuditStore::open_or_quarantine_with_limits(
            root.clone(),
            secrets.clone(),
            Limits::default(),
        )
        .expect("recover from damage");
        assert!(store.recovered_from_damage());
        fill(&store, 1);
    }

    assert_eq!(set_aside_directories(dir.path()).len(), 3);
    let store = open(&root, &secrets, Limits::default());
    assert!(store.verify().expect("verify").ok);
    assert!(!store.recovered_from_damage());
}

/// A key store whose keychain is unavailable.
struct UnavailableKeys;

impl SecretStore for UnavailableKeys {
    fn load_or_create_active_key(
        &self,
        _account_id: &str,
    ) -> AppResult<(u32, Zeroizing<[u8; 32]>)> {
        Err(AppError::PersistenceKeyUnavailable)
    }

    fn active_key_version(&self, _account_id: &str) -> AppResult<u32> {
        Err(AppError::PersistenceKeyUnavailable)
    }

    fn account_key(&self, _account_id: &str, _key_version: u32) -> AppResult<Zeroizing<[u8; 32]>> {
        Err(AppError::PersistenceKeyUnavailable)
    }

    fn delete_account_keys(&self, _account_id: &str) -> AppResult<()> {
        Err(AppError::PersistenceKeyUnavailable)
    }
}

#[test]
fn a_keychain_failure_is_returned_and_never_sets_the_log_aside() {
    let dir = tempdir().expect("temp dir");
    let secrets = secrets();
    let root = dir.path().join("audit");
    let store = open(&root, &secrets, Limits::default());
    fill(&store, 2);
    drop(store);

    let result = LocalAuditStore::open_or_quarantine_with_limits(
        root.clone(),
        Arc::new(UnavailableKeys),
        Limits::default(),
    );
    match result {
        Ok(_) => panic!("a keychain failure must not open a store"),
        Err(error) => assert!(matches!(error, AppError::PersistenceKeyUnavailable)),
    }
    assert!(set_aside_directories(dir.path()).is_empty());
    let path = segment_paths(&root).remove(0);
    assert_eq!(read_lines(&path).len(), 2);
}
