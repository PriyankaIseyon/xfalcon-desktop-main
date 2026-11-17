use tempfile::TempDir;

use super::{is_day, KeepsakeStore};

fn store() -> (TempDir, KeepsakeStore) {
    let root = TempDir::new().expect("temp root");
    let store = KeepsakeStore::new(root.path().to_path_buf());
    (root, store)
}

#[test]
fn the_greeting_is_claimed_once_a_day() {
    let (_root, store) = store();
    assert!(store.claim_greeting("2026-10-01").expect("first claim"));
    assert!(!store.claim_greeting("2026-10-01").expect("same day"));
    assert!(store.claim_greeting("2026-10-02").expect("next day"));
    assert!(!store.claim_greeting("2026-10-02").expect("same day again"));
}

#[test]
fn a_claim_survives_a_restart() {
    let (root, store) = store();
    assert!(store.claim_greeting("2026-10-01").expect("claim"));
    let reopened = KeepsakeStore::new(root.path().to_path_buf());
    assert!(!reopened
        .claim_greeting("2026-10-01")
        .expect("same day after restart"));
}

#[test]
fn only_a_day_shaped_value_is_taken() {
    for bad in [
        "",
        "today",
        "2026-1-1",
        "2026/10/01",
        "2026-10-011",
        "20261001",
        "2026-10-0a",
    ] {
        assert!(!is_day(bad), "{bad}");
        let (_root, store) = store();
        assert!(store.claim_greeting(bad).is_err(), "{bad}");
    }
    assert!(is_day("2026-10-01"));
}

#[test]
fn messages_are_counted_in_order_and_kept() {
    let (root, store) = store();
    assert_eq!(store.count_message().expect("first"), 1);
    assert_eq!(store.count_message().expect("second"), 2);
    let reopened = KeepsakeStore::new(root.path().to_path_buf());
    assert_eq!(reopened.count_message().expect("third"), 3);
}

#[test]
fn counting_and_greeting_do_not_overwrite_each_other() {
    let (_root, store) = store();
    assert!(store.claim_greeting("2026-10-01").expect("claim"));
    assert_eq!(store.count_message().expect("count"), 1);
    assert!(!store.claim_greeting("2026-10-01").expect("still claimed"));
    assert_eq!(store.count_message().expect("count"), 2);
}

#[test]
fn a_damaged_file_reads_as_nothing_kept_and_is_repaired_by_the_next_write() {
    let (root, store) = store();
    std::fs::write(root.path().join("keepsakes.json"), b"{ not json").expect("seed");
    assert_eq!(store.count_message().expect("count after damage"), 1);
    assert!(store
        .claim_greeting("2026-10-01")
        .expect("claim after repair"));
    let raw = std::fs::read(root.path().join("keepsakes.json")).expect("read back");
    assert!(serde_json::from_slice::<serde_json::Value>(&raw).is_ok());
}

#[test]
fn a_file_from_a_build_that_kept_fewer_fields_still_reads() {
    let (root, store) = store();
    std::fs::write(
        root.path().join("keepsakes.json"),
        br#"{"messages_sent":41}"#,
    )
    .expect("seed");
    assert_eq!(store.count_message().expect("count"), 42);
}

#[test]
fn simultaneous_counts_all_land() {
    let (_root, store) = store();
    let store = std::sync::Arc::new(store);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let store = std::sync::Arc::clone(&store);
            std::thread::spawn(move || {
                for _ in 0..5 {
                    store.count_message().expect("count");
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(store.count_message().expect("final"), 41);
}
