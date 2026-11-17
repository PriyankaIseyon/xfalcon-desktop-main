use std::sync::Arc;

use rusqlite::Connection;
use tempfile::tempdir;

use super::{
    AppendOutcome, CachedSecretStore, InMemorySecretStore, Journal, LocalSqliteSynchronous,
    NewJournalEvent, SecretStore, MAX_KEY_VERSIONS,
};

const ACCOUNT_ID: &str = "account-1";
const RUN_ID: &str = "run-1";

fn event(event_id: &str, sequence: u64, payload: &[u8]) -> NewJournalEvent {
    NewJournalEvent {
        account_id: ACCOUNT_ID.to_owned(),
        event_id: event_id.to_owned(),
        run_id: RUN_ID.to_owned(),
        sequence,
        event_type: "assistant_text_delta".to_owned(),
        state: "received".to_owned(),
        payload: payload.to_vec(),
        created_at_ms: 1_783_500_000_000 + sequence as i64,
    }
}

fn secret_store() -> Arc<InMemorySecretStore> {
    let store = Arc::new(InMemorySecretStore::default());
    store
        .put_key(ACCOUNT_ID, [0xA5; 32])
        .expect("store account key");
    store
}

#[test]
fn local_store_open_preserves_normal_and_full_synchronous_profiles() {
    let directory = tempdir().expect("temporary store directory");
    let normal = super::open_local_sqlite_store(
        &directory.path().join("normal.sqlite3"),
        LocalSqliteSynchronous::Normal,
        |_| Ok(()),
    )
    .expect("open normal-sync store");
    let normal_mode: i64 = normal
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .expect("read normal-sync mode");
    assert_eq!(normal_mode, 1, "NORMAL is SQLite synchronous mode 1");

    let full = super::open_local_sqlite_store(
        &directory.path().join("full.sqlite3"),
        LocalSqliteSynchronous::Full,
        |_| Ok(()),
    )
    .expect("open full-sync store");
    let full_mode: i64 = full
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .expect("read full-sync mode");
    assert_eq!(full_mode, 2, "FULL is SQLite synchronous mode 2");
}

#[test]
fn journal_encrypts_payloads_and_replays_ordered_events() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let journal = Journal::open(&database, secret_store()).expect("open journal");

    let second = br#"{"message":"second"}"#;
    let first = br#"{"message":"sensitive"}"#;
    assert_eq!(
        journal.append(event("event-2", 2, second)).expect("append"),
        AppendOutcome::Inserted
    );
    assert_eq!(
        journal.append(event("event-1", 1, first)).expect("append"),
        AppendOutcome::Inserted
    );

    let stored_payload: Vec<u8> = Connection::open(&database)
        .expect("read journal database")
        .query_row(
            "SELECT ciphertext FROM journal_events WHERE event_id = ?1",
            ["event-1"],
            |row| row.get(0),
        )
        .expect("stored ciphertext");
    assert_ne!(stored_payload, first);
    assert!(!stored_payload
        .windows(first.len())
        .any(|window| window == first));

    let replay = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect("replay journal");
    assert_eq!(
        replay
            .iter()
            .map(|event| (event.sequence, event.payload.as_slice()))
            .collect::<Vec<_>>(),
        vec![(1, first.as_slice()), (2, second.as_slice())]
    );
}

#[test]
fn duplicate_event_ids_are_idempotent_without_overwriting_ciphertext() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let journal = Journal::open(&database, secret_store()).expect("open journal");

    assert_eq!(
        journal
            .append(event("event-1", 1, br#"{"message":"original"}"#))
            .expect("first append"),
        AppendOutcome::Inserted
    );
    assert_eq!(
        journal
            .append(event("event-1", 1, br#"{"message":"replacement"}"#))
            .expect("duplicate append"),
        AppendOutcome::Duplicate
    );

    let replay = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect("replay journal");
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].payload, br#"{"message":"original"}"#);
}

#[test]
fn wrong_account_key_and_ciphertext_tampering_fail_closed() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let store = secret_store();
    let journal = Journal::open(&database, store.clone()).expect("open journal");
    journal
        .append(event("event-1", 1, br#"{"message":"sensitive"}"#))
        .expect("append");

    store
        .replace_key_for_test(ACCOUNT_ID, 1, [0x5A; 32])
        .expect("replace account key");
    let wrong_key_error = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect_err("wrong account key must fail");
    assert_eq!(wrong_key_error.code(), "persistence_decryption_failed");

    store
        .replace_key_for_test(ACCOUNT_ID, 1, [0xA5; 32])
        .expect("restore account key");
    Connection::open(&database)
        .expect("open journal database")
        .execute(
            "UPDATE journal_events SET ciphertext = zeroblob(length(ciphertext)) WHERE event_id = ?1",
            ["event-1"],
        )
        .expect("tamper ciphertext");
    let tamper_error = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect_err("tampered ciphertext must fail");
    assert_eq!(tamper_error.code(), "persistence_decryption_failed");
}

#[test]
fn account_purge_removes_only_the_requested_accounts_events() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let store = secret_store();
    store
        .put_key("account-2", [0x3C; 32])
        .expect("store second account key");
    let journal = Journal::open(&database, store).expect("open journal");
    journal
        .append(event("event-1", 1, br#"{"message":"first"}"#))
        .expect("append first account");
    let mut second_account = event("event-2", 1, br#"{"message":"second"}"#);
    second_account.account_id = "account-2".to_owned();
    journal
        .append(second_account)
        .expect("append second account");

    journal.purge_account(ACCOUNT_ID).expect("purge account");

    assert!(journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect("first account replay")
        .is_empty());
    assert_eq!(
        journal
            .events_after("account-2", RUN_ID, 0)
            .expect("second account replay")
            .len(),
        1
    );
}

#[test]
fn key_rotation_preserves_replay_of_rows_encrypted_with_historical_keys() {
    let directory = tempdir().expect("temporary journal directory");
    let store = secret_store();
    let journal =
        Journal::open(directory.path().join("journal.sqlite3"), store.clone()).expect("open");
    journal
        .append(event("event-1", 1, br#"{"message":"version-one"}"#))
        .expect("append version one");
    store
        .put_versioned_key(ACCOUNT_ID, 2, [0xB6; 32])
        .expect("store version two");
    store
        .set_active_key_version(ACCOUNT_ID, 2)
        .expect("activate version two");
    journal
        .append(event("event-2", 2, br#"{"message":"version-two"}"#))
        .expect("append version two");

    let replay = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect("replay across key versions");
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].payload, br#"{"message":"version-one"}"#);
    assert_eq!(replay[1].payload, br#"{"message":"version-two"}"#);
}

#[test]
fn restart_reopens_the_journal_without_losing_encrypted_events() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let store = secret_store();
    {
        let journal = Journal::open(&database, store.clone()).expect("open first journal");
        journal
            .append(event("event-1", 1, br#"{"message":"durable"}"#))
            .expect("append durable event");
    }

    let reopened = Journal::open(&database, store).expect("reopen journal");
    let replay = reopened
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect("replay reopened journal");
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].state, "received");
    assert_eq!(replay[0].payload, br#"{"message":"durable"}"#);
}

#[test]
fn sequence_zero_and_collisions_are_rejected() {
    let directory = tempdir().expect("temporary journal directory");
    let journal = Journal::open(directory.path().join("journal.sqlite3"), secret_store())
        .expect("open journal");

    let zero_error = journal
        .append(event("event-zero", 0, b"zero"))
        .expect_err("zero sequence must fail");
    assert_eq!(zero_error.code(), "persistence_invalid");
    journal
        .append(event("event-1", 1, b"first"))
        .expect("append first sequence");
    let collision = journal
        .append(event("event-2", 1, b"collision"))
        .expect_err("sequence collision must fail");
    assert_eq!(collision.code(), "persistence_database_failed");
}

#[test]
fn plaintext_state_and_timestamp_are_authenticated() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let journal = Journal::open(&database, secret_store()).expect("open journal");
    journal
        .append(event("event-1", 1, b"bound"))
        .expect("append event");

    Connection::open(&database)
        .expect("open journal database")
        .execute(
            "UPDATE journal_events SET state = 'tampered', created_at_ms = created_at_ms + 1
             WHERE event_id = ?1",
            ["event-1"],
        )
        .expect("tamper authenticated metadata");
    let error = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect_err("metadata tampering must fail");
    assert_eq!(error.code(), "persistence_decryption_failed");
}

#[test]
fn replay_pages_expose_an_explicit_continuation_cursor() {
    let directory = tempdir().expect("temporary journal directory");
    let journal = Journal::open(directory.path().join("journal.sqlite3"), secret_store())
        .expect("open journal");
    for sequence in 1..=513 {
        journal
            .append(event(&format!("event-{sequence}"), sequence, b"x"))
            .expect("append paged event");
    }

    let first_page = journal
        .events_after(ACCOUNT_ID, RUN_ID, 0)
        .expect("first replay page");
    assert_eq!(first_page.len(), 512);
    assert!(first_page.has_more());
    assert_eq!(first_page.next_sequence(), Some(512));

    let second_page = journal
        .events_after(
            ACCOUNT_ID,
            RUN_ID,
            first_page.next_sequence().expect("first page continuation"),
        )
        .expect("second replay page");
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].sequence, 513);
    assert!(!second_page.has_more());
}

#[test]
fn account_key_deletion_is_bounded_idempotent_and_account_scoped() {
    let store = InMemorySecretStore::default();
    store.put_key(ACCOUNT_ID, [0xA5; 32]).expect("first key");
    store
        .put_versioned_key(ACCOUNT_ID, 2, [0xB6; 32])
        .expect("rotated key");
    store
        .set_active_key_version(ACCOUNT_ID, 2)
        .expect("activate rotated key");
    store
        .put_key("neighbor-account", [0x3C; 32])
        .expect("neighbor key");

    store
        .delete_account_keys(ACCOUNT_ID)
        .expect("delete account keys");
    store
        .delete_account_keys(ACCOUNT_ID)
        .expect("missing keys are idempotent");

    assert_eq!(
        store
            .active_key_version(ACCOUNT_ID)
            .expect_err("deleted marker")
            .code(),
        "persistence_key_unavailable"
    );
    assert_eq!(
        store
            .account_key(ACCOUNT_ID, 2)
            .expect_err("deleted key")
            .code(),
        "persistence_key_unavailable"
    );
    assert_eq!(
        store
            .active_key_version("neighbor-account")
            .expect("neighbor marker"),
        1
    );
}

#[test]
fn key_versions_above_the_cleanup_bound_are_rejected() {
    let store = InMemorySecretStore::default();
    let error = store
        .put_versioned_key(ACCOUNT_ID, MAX_KEY_VERSIONS + 1, [0xA5; 32])
        .expect_err("unbounded key version must fail closed");
    assert_eq!(error.code(), "persistence_invalid");
}

#[test]
fn legacy_active_key_above_cleanup_bound_fails_without_deleting_marker_or_key() {
    let store = InMemorySecretStore::default();
    let legacy_version = MAX_KEY_VERSIONS + 1;
    store
        .insert_legacy_key_for_test(ACCOUNT_ID, legacy_version, [0xD7; 32])
        .expect("legacy key fixture");

    let error = store
        .delete_account_keys(ACCOUNT_ID)
        .expect_err("legacy marker must fail closed");

    assert_eq!(error.code(), "persistence_invalid");
    assert_eq!(
        store
            .raw_active_version_for_test(ACCOUNT_ID)
            .expect("marker remains"),
        legacy_version
    );
    assert!(store.contains_key_for_test(ACCOUNT_ID, legacy_version));
}

#[test]
fn explicit_journal_close_joins_worker_before_database_file_removal() {
    let directory = tempdir().expect("temporary journal directory");
    let database = directory.path().join("journal.sqlite3");
    let journal = Journal::open(&database, secret_store()).expect("open journal");
    journal
        .append(event("event-close", 1, b"payload"))
        .expect("append before close");

    journal.close().expect("acknowledged close");
    std::fs::remove_file(&database).expect("closed SQLite file can be removed");
    assert!(!database.exists());
}

/// Counts every call that reaches the backing store, so a test can assert how
/// many keychain round trips a sequence of operations really costs.
struct CountingSecretStore {
    inner: InMemorySecretStore,
    reads: std::sync::atomic::AtomicUsize,
    deletes: std::sync::atomic::AtomicUsize,
}

impl CountingSecretStore {
    fn new() -> Self {
        Self {
            inner: InMemorySecretStore::default(),
            reads: std::sync::atomic::AtomicUsize::new(0),
            deletes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn deletes(&self) -> usize {
        self.deletes.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn count_read(&self) {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl SecretStore for Arc<CountingSecretStore> {
    fn load_or_create_active_key(
        &self,
        account_id: &str,
    ) -> crate::domain::error::AppResult<(u32, zeroize::Zeroizing<[u8; 32]>)> {
        self.count_read();
        self.inner.load_or_create_active_key(account_id)
    }

    fn active_key_version(&self, account_id: &str) -> crate::domain::error::AppResult<u32> {
        self.count_read();
        self.inner.active_key_version(account_id)
    }

    fn account_key(
        &self,
        account_id: &str,
        key_version: u32,
    ) -> crate::domain::error::AppResult<zeroize::Zeroizing<[u8; 32]>> {
        self.count_read();
        self.inner.account_key(account_id, key_version)
    }

    fn delete_account_keys(&self, account_id: &str) -> crate::domain::error::AppResult<()> {
        self.deletes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.delete_account_keys(account_id)
    }
}

#[test]
fn repeated_key_access_reaches_the_backing_store_once_per_account() {
    let counting = Arc::new(CountingSecretStore::new());
    let cached = CachedSecretStore::new(Arc::new(counting.clone()) as Arc<dyn SecretStore>);

    let (key_version, first) = cached
        .load_or_create_active_key(ACCOUNT_ID)
        .expect("provision the account key");
    let reads_after_provision = counting.reads();

    // The access pattern a conversation load produces: one `account_key` per
    // sealed record, plus the `active_key_version` a seal resolves first.
    for _ in 0..50 {
        let key = cached
            .account_key(ACCOUNT_ID, key_version)
            .expect("read the cached key");
        assert_eq!(*key, *first, "the cache must return the same key");
        assert_eq!(
            cached
                .active_key_version(ACCOUNT_ID)
                .expect("read the cached version"),
            key_version
        );
        let (repeat_version, repeat_key) = cached
            .load_or_create_active_key(ACCOUNT_ID)
            .expect("reload the cached key");
        assert_eq!(repeat_version, key_version);
        assert_eq!(*repeat_key, *first);
    }

    assert_eq!(
        counting.reads(),
        reads_after_provision,
        "150 key accesses after provisioning must not reach the keychain again"
    );
}

#[test]
fn deleting_an_account_evicts_its_cached_key_before_delegating() {
    let counting = Arc::new(CountingSecretStore::new());
    let cached = CachedSecretStore::new(Arc::new(counting.clone()) as Arc<dyn SecretStore>);

    let (key_version, _) = cached
        .load_or_create_active_key(ACCOUNT_ID)
        .expect("provision the account key");
    cached
        .delete_account_keys(ACCOUNT_ID)
        .expect("delete the account keys");
    assert_eq!(counting.deletes(), 1, "the delete must reach the store");

    // The key is gone from both layers: a cached copy surviving eviction would
    // still decrypt this account's records after cleanup claimed to erase it.
    assert!(
        cached.account_key(ACCOUNT_ID, key_version).is_err(),
        "a deleted account must not be served from the cache"
    );
    assert!(
        cached.active_key_version(ACCOUNT_ID).is_err(),
        "a deleted account must not report a cached active version"
    );
}

#[test]
fn a_cached_store_serves_distinct_keys_per_account() {
    let counting = Arc::new(CountingSecretStore::new());
    let cached = CachedSecretStore::new(Arc::new(counting.clone()) as Arc<dyn SecretStore>);

    let (first_version, first) = cached
        .load_or_create_active_key("account-1")
        .expect("first account key");
    let (second_version, second) = cached
        .load_or_create_active_key("account-2")
        .expect("second account key");

    assert_ne!(*first, *second, "accounts must not share a device key");
    assert_eq!(
        *cached
            .account_key("account-1", first_version)
            .expect("first account cached key"),
        *first
    );
    assert_eq!(
        *cached
            .account_key("account-2", second_version)
            .expect("second account cached key"),
        *second
    );

    // Evicting one account must not disturb the other.
    cached
        .delete_account_keys("account-1")
        .expect("delete the first account");
    assert!(cached.account_key("account-1", first_version).is_err());
    assert_eq!(
        *cached
            .account_key("account-2", second_version)
            .expect("second account survives"),
        *second
    );
}

#[test]
fn an_invalid_account_id_is_rejected_before_the_backing_store_is_touched() {
    let counting = Arc::new(CountingSecretStore::new());
    let cached = CachedSecretStore::new(Arc::new(counting.clone()) as Arc<dyn SecretStore>);

    for rejected in ["", "account/1", "account 1", "account\u{7f}1"] {
        assert!(
            cached.load_or_create_active_key(rejected).is_err(),
            "{rejected:?} must not be accepted"
        );
        assert!(cached.account_key(rejected, 1).is_err());
        assert!(cached.active_key_version(rejected).is_err());
        assert!(cached.delete_account_keys(rejected).is_err());
    }
    assert_eq!(
        counting.reads(),
        0,
        "validation must run before the keychain is consulted"
    );
    assert_eq!(counting.deletes(), 0);
}

#[test]
fn a_key_version_outside_the_cleanup_bound_is_rejected_by_the_cache() {
    let counting = Arc::new(CountingSecretStore::new());
    let cached = CachedSecretStore::new(Arc::new(counting.clone()) as Arc<dyn SecretStore>);

    assert!(cached.account_key(ACCOUNT_ID, 0).is_err());
    assert!(cached
        .account_key(ACCOUNT_ID, MAX_KEY_VERSIONS + 1)
        .is_err());
    assert_eq!(counting.reads(), 0);
}
