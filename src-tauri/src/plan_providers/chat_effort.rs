//! The reasoning level each Claude chat keeps, per account.
//!
//! A new chat starts with the device-wide `ClaudeEffort` setting. The first
//! Claude turn of a chat stores that level, and every later turn uses the
//! stored level even after the setting changes. A missing, damaged, oversized
//! or newer file reads as no chats, so this store never blocks a chat.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use xfalcon_contracts::plan_providers::ClaudeEffort;

use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};
use crate::settings::write_atomically;

pub(crate) const CHAT_EFFORT_FILE: &str = "claude-chat-effort.json";
const DOCUMENT_VERSION: u8 = 1;
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;
const MAX_CHATS: usize = 2_048;
const MAX_CONVERSATION_ID_BYTES: usize = 64;

fn effort_io_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Mutex::default)
}

#[derive(Deserialize, Serialize)]
struct ChatEffort {
    id: String,
    effort: ClaudeEffort,
}

#[derive(Serialize)]
struct ChatEffortDocument {
    version: u8,
    chats: Vec<ChatEffort>,
}

pub(crate) struct ChatEffortStore {
    path: PathBuf,
}

impl ChatEffortStore {
    #[must_use]
    pub(crate) fn new(app_local_data: &Path, account: &ValidatedAccountId) -> Self {
        Self {
            path: app_local_data
                .join("accounts")
                .join(account_directory_name(account.as_str()))
                .join(CHAT_EFFORT_FILE),
        }
    }

    /// The level this chat keeps. The first call for a conversation decides its
    /// level for good: it stores `current` and returns it. Every later call
    /// returns the stored level and ignores `current`.
    ///
    /// Never fails. A conversation id that is not 1 to 64 ASCII letters, digits,
    /// `-` or `_` is not stored, and `current` is returned as it is.
    pub(crate) fn lock_or_get(&self, conversation_id: &str, current: ClaudeEffort) -> ClaudeEffort {
        if !is_valid_conversation_id(conversation_id) {
            return current;
        }
        let _lock = effort_io_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut chats = read_chats(&self.path);
        if let Some(chat) = chats.iter().find(|chat| chat.id == conversation_id) {
            return chat.effort;
        }
        chats.push(ChatEffort {
            id: conversation_id.to_owned(),
            effort: current,
        });
        let oldest_kept = chats.len().saturating_sub(MAX_CHATS);
        chats.drain(..oldest_kept);
        if let Err(error) = write_chats(&self.path, chats) {
            tracing::warn!(error = %error, "could not save a chat's Claude effort level");
        }
        current
    }
}

/// A missing, unreadable, oversized, damaged or other-version file reads as no
/// chats. Within a valid document, an entry with a bad id or an unknown level
/// is dropped alone.
fn read_chats(path: &Path) -> Vec<ChatEffort> {
    let Ok(metadata) = fs::metadata(path) else {
        return Vec::new();
    };
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Vec::new();
    }
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let Ok(Value::Object(mut document)) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    if document.remove("version") != Some(Value::from(DOCUMENT_VERSION)) {
        return Vec::new();
    }
    let Some(Value::Array(entries)) = document.remove("chats") else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<ChatEffort>(entry).ok())
        .filter(|chat| is_valid_conversation_id(&chat.id))
        .collect()
}

fn write_chats(path: &Path, chats: Vec<ChatEffort>) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&ChatEffortDocument {
        version: DOCUMENT_VERSION,
        chats,
    })
    .map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(std::io::Error::other("chat effort document is too large"));
    }
    write_atomically(path, &bytes)
}

fn is_valid_conversation_id(id: &str) -> bool {
    (1..=MAX_CONVERSATION_ID_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account(value: &str) -> ValidatedAccountId {
        ValidatedAccountId::parse(value).expect("valid account")
    }

    fn written_document(store: &ChatEffortStore) -> Value {
        serde_json::from_slice(&fs::read(&store.path).expect("stored document"))
            .expect("json document")
    }

    fn seed(store: &ChatEffortStore, bytes: &[u8]) {
        fs::create_dir_all(store.path.parent().expect("parent")).expect("directory");
        fs::write(&store.path, bytes).expect("seed document");
    }

    #[test]
    fn the_first_call_stores_and_returns_the_current_level() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));

        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Low),
            ClaudeEffort::Low
        );
        assert_eq!(
            written_document(&store)["chats"],
            json!([{"id": "chat-a", "effort": "low"}])
        );
    }

    #[test]
    fn a_later_call_returns_the_stored_level_whatever_the_setting_is() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));

        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Low),
            ClaudeEffort::Low
        );
        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Max),
            ClaudeEffort::Low
        );
    }

    #[test]
    fn conversations_keep_their_levels_independently() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));

        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Low),
            ClaudeEffort::Low
        );
        assert_eq!(
            store.lock_or_get("chat-b", ClaudeEffort::Max),
            ClaudeEffort::Max
        );
        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Max),
            ClaudeEffort::Low
        );
        assert_eq!(
            store.lock_or_get("chat-b", ClaudeEffort::Low),
            ClaudeEffort::Max
        );
    }

    #[test]
    fn accounts_keep_their_levels_independently() {
        let root = tempfile::tempdir().expect("tempdir");
        let first = ChatEffortStore::new(root.path(), &account("tenant:user-a"));
        let second = ChatEffortStore::new(root.path(), &account("tenant:user-b"));
        assert_ne!(
            account_directory_name("tenant:user-a"),
            account_directory_name("tenant:user-b")
        );

        assert_eq!(
            first.lock_or_get("chat-a", ClaudeEffort::Low),
            ClaudeEffort::Low
        );
        assert_eq!(
            second.lock_or_get("chat-a", ClaudeEffort::Max),
            ClaudeEffort::Max
        );
        assert_eq!(
            first.lock_or_get("chat-a", ClaudeEffort::Max),
            ClaudeEffort::Low
        );
    }

    #[test]
    fn the_lock_survives_a_new_store_over_the_same_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let first = ChatEffortStore::new(root.path(), &account("tenant:user"));
        assert_eq!(
            first.lock_or_get("chat-a", ClaudeEffort::Low),
            ClaudeEffort::Low
        );

        let second = ChatEffortStore::new(root.path(), &account("tenant:user"));
        assert_eq!(
            second.lock_or_get("chat-a", ClaudeEffort::Max),
            ClaudeEffort::Low
        );
    }

    #[test]
    fn an_invalid_conversation_id_returns_the_current_level_and_writes_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));
        let too_long = "a".repeat(65);

        for id in ["", too_long.as_str(), "has space", "a/b", "line\nbreak"] {
            assert_eq!(
                store.lock_or_get(id, ClaudeEffort::Max),
                ClaudeEffort::Max,
                "id {id:?}"
            );
        }
        assert!(!store.path.exists());
    }

    #[test]
    fn a_conversation_id_of_64_characters_is_stored() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));
        let id = "a".repeat(64);

        assert_eq!(store.lock_or_get(&id, ClaudeEffort::Low), ClaudeEffort::Low);
        assert_eq!(store.lock_or_get(&id, ClaudeEffort::Max), ClaudeEffort::Low);
    }

    #[test]
    fn a_damaged_or_other_version_file_counts_as_empty_and_is_replaced() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));

        for bytes in [
            b"{broken".as_slice(),
            br#"{"version":2,"chats":[{"id":"chat-a","effort":"max"}]}"#.as_slice(),
        ] {
            seed(&store, bytes);
            assert_eq!(
                store.lock_or_get("chat-a", ClaudeEffort::Low),
                ClaudeEffort::Low
            );
            assert_eq!(
                written_document(&store),
                json!({"version": 1, "chats": [{"id": "chat-a", "effort": "low"}]})
            );
        }
    }

    #[test]
    fn a_file_over_256_kib_counts_as_empty() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));
        let mut padded = br#"{"version":1,"chats":[{"id":"chat-a","effort":"max"}]}"#.to_vec();
        padded.resize(256 * 1024 + 1, b' ');
        seed(&store, &padded);

        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Low),
            ClaudeEffort::Low
        );
        assert_eq!(
            written_document(&store)["chats"],
            json!([{"id": "chat-a", "effort": "low"}])
        );
    }

    #[test]
    fn a_bad_entry_is_dropped_alone_and_its_conversation_locks_again() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));
        seed(
            &store,
            br#"{"version":1,"chats":[{"id":"chat-a","effort":"low"},{"id":"chat-b","effort":"ultracode"},{"id":"bad id","effort":"low"},{"id":"chat-c","effort":"max"}]}"#,
        );

        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::High),
            ClaudeEffort::Low
        );
        assert_eq!(
            store.lock_or_get("chat-c", ClaudeEffort::High),
            ClaudeEffort::Max
        );
        assert_eq!(
            store.lock_or_get("chat-b", ClaudeEffort::High),
            ClaudeEffort::High
        );
        assert_eq!(
            written_document(&store)["chats"],
            json!([
                {"id": "chat-a", "effort": "low"},
                {"id": "chat-c", "effort": "max"},
                {"id": "chat-b", "effort": "high"}
            ])
        );
    }

    #[test]
    fn past_2048_chats_the_oldest_is_dropped_and_locks_again() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));
        for index in 0..2_049 {
            let id = format!("chat-{index}");
            assert_eq!(
                store.lock_or_get(&id, ClaudeEffort::High),
                ClaudeEffort::High
            );
        }

        let document = written_document(&store);
        let chats = document["chats"].as_array().expect("chats");
        assert_eq!(chats.len(), 2_048);
        assert_eq!(chats[0]["id"], "chat-1");
        assert_eq!(chats[2_047]["id"], "chat-2048");
        assert_eq!(
            store.lock_or_get("chat-2048", ClaudeEffort::Low),
            ClaudeEffort::High
        );
        assert_eq!(
            store.lock_or_get("chat-0", ClaudeEffort::Low),
            ClaudeEffort::Low
        );
    }

    #[test]
    fn the_written_file_is_a_version_1_document_and_leaves_no_temporary_file() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = ChatEffortStore::new(root.path(), &account("tenant:user"));
        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::Xhigh),
            ClaudeEffort::Xhigh
        );

        assert_eq!(
            written_document(&store),
            json!({"version": 1, "chats": [{"id": "chat-a", "effort": "xhigh"}]})
        );
        let names: Vec<String> = fs::read_dir(store.path.parent().expect("parent"))
            .expect("account directory")
            .map(|entry| {
                entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, [CHAT_EFFORT_FILE]);
    }
}
