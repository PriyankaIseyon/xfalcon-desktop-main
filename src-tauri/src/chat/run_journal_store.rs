//! Account-scoped persistence for agent run state transitions (Task 8 / exit gate #4).

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use xfalcon_contracts::{RunId, RunState};

use crate::agent::run::JournalEntry;
use crate::chat::local_projection::LOCAL_PROJECTIONS_DIR;
use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};

const RUN_JOURNAL_DIR: &str = "run-journal";

#[derive(Clone)]
pub struct RunJournalStore {
    app_root: PathBuf,
    account: ValidatedAccountId,
}

impl RunJournalStore {
    pub fn new(app_root: PathBuf, account: ValidatedAccountId) -> Self {
        Self { app_root, account }
    }

    pub fn append_transition(
        &self,
        run_id: &RunId,
        sequence: u64,
        from: RunState,
        to: RunState,
    ) -> std::io::Result<()> {
        let path = self.run_file(run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut encoded = serde_json::to_vec(&PersistedTransition { sequence, from, to })
            .map_err(invalid_journal_data)?;
        encoded.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&encoded)?;
        file.sync_all()
    }

    fn load_persisted(&self, run_id: &RunId) -> std::io::Result<Vec<PersistedTransition>> {
        let path = self.run_file(run_id);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&path)?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Vec::new());
        }
        if bytes
            .iter()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| *byte == b'[')
        {
            let mut stream = serde_json::Deserializer::from_slice(&bytes)
                .into_iter::<Vec<PersistedTransition>>();
            let Some(legacy) = stream.next() else {
                return Ok(Vec::new());
            };
            let mut persisted = legacy.map_err(invalid_journal_data)?;
            let mut appended = parse_json_line_records(&bytes[stream.byte_offset()..])?;
            persisted.append(&mut appended);
            return Ok(persisted);
        }
        parse_json_line_records(&bytes)
    }

    pub fn load_entries(&self, run_id: &RunId) -> std::io::Result<Vec<JournalEntry>> {
        let persisted = self.load_persisted(run_id)?;
        Ok(persisted
            .into_iter()
            .map(|row| JournalEntry {
                run_id: run_id.clone(),
                sequence: row.sequence,
                from: row.from,
                to: row.to,
            })
            .collect())
    }

    pub fn remove_run(&self, run_id: &RunId) {
        let path = self.run_file(run_id);
        let _ = fs::remove_file(path);
    }

    fn run_file(&self, run_id: &RunId) -> PathBuf {
        run_journal_dir(&self.app_root, &self.account).join(run_journal_file_name(run_id))
    }
}

fn parse_json_line_records(bytes: &[u8]) -> std::io::Result<Vec<PersistedTransition>> {
    let ends_with_newline = bytes.ends_with(b"\n");
    let mut records = bytes
        .split(|byte| *byte == b'\n')
        .filter(|record| !record.iter().all(u8::is_ascii_whitespace))
        .peekable();
    let mut persisted = Vec::new();
    while let Some(record) = records.next() {
        match serde_json::from_slice(record) {
            Ok(entry) => persisted.push(entry),
            Err(error) if error.is_eof() && records.peek().is_none() && !ends_with_newline => {
                break;
            }
            Err(error) => return Err(invalid_journal_data(error)),
        }
    }
    Ok(persisted)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedTransition {
    sequence: u64,
    from: RunState,
    to: RunState,
}

fn run_journal_dir(app_root: &Path, account: &ValidatedAccountId) -> PathBuf {
    app_root
        .join("accounts")
        .join(account_directory_name(account.as_str()))
        .join(LOCAL_PROJECTIONS_DIR)
        .join(RUN_JOURNAL_DIR)
}

fn run_journal_file_name(run_id: &RunId) -> String {
    let value = run_id.as_str();
    if !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        format!("{value}.json")
    } else {
        format!("run-{}.json", account_directory_name(value))
    }
}

fn invalid_journal_data(error: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use xfalcon_contracts::RunId;

    #[test]
    fn append_and_reload_round_trip() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-account").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);
        let run_id = RunId::from("run-persist-1");

        store
            .append_transition(&run_id, 1, RunState::Queued, RunState::Running)
            .expect("append");
        store
            .append_transition(&run_id, 2, RunState::Running, RunState::Interrupted)
            .expect("append");

        let entries = store.load_entries(&run_id).expect("load");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].from, RunState::Queued);
        assert_eq!(entries[1].to, RunState::Interrupted);

        store.remove_run(&run_id);
        assert!(store.load_entries(&run_id).expect("load").is_empty());
    }

    #[test]
    fn append_writes_one_durable_record_per_transition() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-records").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);
        let run_id = RunId::from("run-persist-records");

        store
            .append_transition(&run_id, 1, RunState::Draft, RunState::Queued)
            .expect("first append");
        store
            .append_transition(&run_id, 2, RunState::Queued, RunState::Running)
            .expect("second append");

        let encoded = fs::read_to_string(store.run_file(&run_id)).expect("journal text");
        let records: Vec<_> = encoded.lines().collect();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| record.starts_with('{')));
    }

    #[test]
    fn load_retains_durable_records_before_an_incomplete_tail() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-partial").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);
        let run_id = RunId::from("run-persist-partial");

        store
            .append_transition(&run_id, 1, RunState::Draft, RunState::Queued)
            .expect("durable append");
        let mut file = OpenOptions::new()
            .append(true)
            .open(store.run_file(&run_id))
            .expect("journal");
        file.write_all(br#"{"sequence":2"#).expect("partial tail");
        file.sync_all().expect("partial tail sync");

        let entries = store.load_entries(&run_id).expect("recover durable prefix");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].to, RunState::Queued);
    }

    #[test]
    fn load_rejects_a_malformed_unterminated_tail() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-malformed").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);
        let run_id = RunId::from("run-persist-malformed");

        store
            .append_transition(&run_id, 1, RunState::Draft, RunState::Queued)
            .expect("durable append");
        let mut file = OpenOptions::new()
            .append(true)
            .open(store.run_file(&run_id))
            .expect("journal");
        file.write_all(b"not-json").expect("malformed tail");
        file.sync_all().expect("malformed tail sync");

        let error = store
            .load_entries(&run_id)
            .expect_err("malformed tail must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn load_accepts_the_previous_json_array_format() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-legacy").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);
        let run_id = RunId::from("run-persist-legacy");
        let path = store.run_file(&run_id);
        fs::create_dir_all(path.parent().expect("journal parent")).expect("journal directory");
        fs::write(path, br#"[{"sequence":1,"from":"draft","to":"queued"}]"#)
            .expect("legacy journal");

        let entries = store.load_entries(&run_id).expect("load legacy journal");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, RunState::Draft);
        assert_eq!(entries[0].to, RunState::Queued);
    }

    #[test]
    fn append_after_previous_json_array_preserves_all_transitions() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-upgrade").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);
        let run_id = RunId::from("run-persist-upgrade");
        let path = store.run_file(&run_id);
        fs::create_dir_all(path.parent().expect("journal parent")).expect("journal directory");
        fs::write(&path, br#"[{"sequence":1,"from":"draft","to":"queued"}]"#)
            .expect("legacy journal");

        store
            .append_transition(&run_id, 2, RunState::Queued, RunState::Running)
            .expect("append after upgrade");

        let entries = store.load_entries(&run_id).expect("load upgraded journal");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].to, RunState::Queued);
        assert_eq!(entries[1].to, RunState::Running);
    }

    #[test]
    fn run_id_cannot_escape_the_account_journal_directory() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-store-path").expect("account");
        let expected_directory = run_journal_dir(directory.path(), &account);
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);

        let path = store.run_file(&RunId::from("../../outside-account"));

        assert_eq!(path.parent(), Some(expected_directory.as_path()));
    }
}
