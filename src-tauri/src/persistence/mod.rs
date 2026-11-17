use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use rusqlite::Connection;

use crate::domain::error::AppResult;

pub(crate) mod crypto;
mod journal;
mod secret_store;

pub(crate) use crypto::{
    decrypt_approval, decrypt_backup, decrypt_project_index, decrypt_project_trust,
    decrypt_projection, encrypt_approval, encrypt_backup, encrypt_project_index,
    encrypt_project_trust, encrypt_projection, ApprovalAuthenticatedMetadata,
    BackupAuthenticatedMetadata, ProjectIndexAuthenticatedMetadata,
    ProjectTrustAuthenticatedMetadata, ProjectionAuthenticatedMetadata,
};
pub use journal::{AppendOutcome, Journal, JournalEvent, NewJournalEvent, ReplayPage};
#[cfg(test)]
pub use secret_store::InMemorySecretStore;
pub use secret_store::MAX_KEY_VERSIONS;
pub use secret_store::{CachedSecretStore, KeyringSecretStore, SecretStore};

#[cfg(test)]
mod tests;

pub const CHAT_PROJECTION_MIGRATION: &str = concat!(
    include_str!("migrations/0002_chat_projection.sql"),
    include_str!("migrations/0009_chat_title_sources.sql"),
    include_str!("migrations/0012_chat_deletions.sql"),
);
pub const CHAT_PROJECTION_SCHEMA_VERSION: u32 = 3;
pub const PROJECT_TRUST_MIGRATION: &str = concat!(
    include_str!("migrations/0003_projects.sql"),
    include_str!("migrations/0007_project_conversation_bindings.sql"),
    include_str!("migrations/0013_report_kit_projects.sql"),
);
pub const PROJECT_TRUST_SCHEMA_VERSION: u32 = 3;
pub const PROJECT_INDEX_MIGRATION: &str = include_str!("migrations/0004_project_index.sql");
pub const PROJECT_INDEX_SCHEMA_VERSION: u32 = 1;
pub const PROJECT_BACKUP_MIGRATION: &str = include_str!("migrations/0010_project_backups.sql");
pub const PROJECT_BACKUP_SCHEMA_VERSION: u32 = 1;
// Two files concatenated into one migration blob (rather than a second,
// independently-run `execute_batch`) because `crate::approvals::service`'s
// `migrate` re-runs this entire string, unconditionally, on any connection
// whose `user_version` is below `PROJECT_APPROVAL_SCHEMA_VERSION` -- an
// already-migrated-to-version-1 local database (`CREATE TABLE IF NOT
// EXISTS`/`CREATE TRIGGER IF NOT EXISTS` throughout, so re-applying 0005's
// own statements is a no-op) picks up 0006's history table and archive
// trigger the same way a brand-new database does, with no separate
// version-1-to-version-2 migration path to maintain.
pub const PROJECT_APPROVAL_MIGRATION: &str = concat!(
    include_str!("migrations/0005_project_mutation_approvals.sql"),
    include_str!("migrations/0006_project_mutation_approval_history.sql"),
);
pub const PROJECT_APPROVAL_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy)]
pub(crate) enum LocalSqliteSynchronous {
    Normal,
    Full,
}

/// Opens one of the app's local SQLite stores with its per-connection
/// settings, then applies the store's schema migration while holding a
/// process-local lock for this path. Every store opens a fresh connection per
/// operation. SQLite's WAL mode is persistent database state, so setting it
/// on every fresh connection races when parallel project tools open the same
/// new or existing database. The per-path lock makes the mode check, initial
/// migration, and the first switch to WAL a single local operation.
pub(crate) fn open_local_sqlite_store(
    path: &Path,
    synchronous: LocalSqliteSynchronous,
    migrate: impl FnOnce(&mut Connection) -> AppResult<()>,
) -> AppResult<Connection> {
    let setup_lock = sqlite_setup_lock(path);
    let _setup_guard = setup_lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let journal_mode =
        connection.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    match synchronous {
        LocalSqliteSynchronous::Normal => connection.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?,
        LocalSqliteSynchronous::Full => connection.execute_batch(
            "PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;",
        )?,
    }
    migrate(&mut connection)?;
    Ok(connection)
}

fn sqlite_setup_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}
