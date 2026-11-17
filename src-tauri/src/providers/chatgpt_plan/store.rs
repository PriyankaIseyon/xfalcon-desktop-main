//! Keeps one ChatGPT plan sign-in on this device. `meta.json` holds the
//! non-secret state: the host id, the active client id, and the profiles. Each
//! issued client id's tokens sit in their own sealed session file. Both live
//! in the account's `chatgpt-plan/` directory (`PLAN_DIR`). Account cleanup
//! removes that directory in its session-vault handler, before it deletes the
//! account keys that seal these files.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::persistence::crypto::{
    decrypt_plan_session, encrypt_plan_session, PlanSessionAuthenticatedMetadata,
};
use crate::persistence::SecretStore;
use crate::providers::chatgpt_plan::oauth;
use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};

/// The per-account directory that holds this store's files.
pub(crate) const PLAN_DIR: &str = "chatgpt-plan";
const META_FILE: &str = "meta.json";
const MAX_META_BYTES: u64 = 64 * 1024;
const MAX_SESSION_BYTES: u64 = 256 * 1024;

/// The tokens and identity of one sign-in. Only its sealed form reaches disk.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    pub(crate) client_id: String,
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) id_token: Option<String>,
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_at: i64,
    pub(crate) earliest_refresh_at: Option<i64>,
    pub(crate) scopes: Vec<String>,
    pub(crate) saved_at: i64,
}

impl fmt::Debug for SessionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionRecord")
            .field("client_id", &self.client_id)
            .field("subject", &"<redacted>")
            .field("email", &self.email.as_ref().map(|_| "<redacted>"))
            .field("id_token", &self.id_token.as_ref().map(|_| "<redacted>"))
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .field("earliest_refresh_at", &self.earliest_refresh_at)
            .field("scopes", &self.scopes)
            .field("saved_at", &self.saved_at)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Meta {
    pub(crate) host_id: Option<String>,
    pub(crate) active_client_id: Option<String>,
    pub(crate) welcome_acknowledged: bool,
    pub(crate) profiles: Vec<Profile>,
}

/// One issued client id this device has signed in with. Its `email` is the
/// login hint for the next sign-in on that client, so it is kept here rather than
/// in the sealed record, which a sign-out removes. Neither it nor the subject is
/// printed by `Debug`.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Profile {
    pub(crate) client_id: String,
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
}

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Profile")
            .field("client_id", &self.client_id)
            .field("subject", &"<redacted>")
            .field("email", &self.email.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum StoreError {
    #[error("the plan store is unavailable")]
    Unavailable,
    /// A file or its key could not be read. Nothing shows the stored data is wrong,
    /// so the caller keeps it: no delete, no overwrite.
    #[error("a stored sign-in could not be read")]
    Unreadable,
    /// The stored bytes do not authenticate or parse.
    #[error("a stored sign-in is damaged")]
    Corrupt,
}

/// The session file's envelope. The AAD schema version is kept beside the
/// ciphertext so the reader can rebuild the authenticated data it was sealed with.
#[derive(Serialize, Deserialize)]
struct SealedSession {
    aad_schema_version: u32,
    key_version: u32,
    nonce: String,
    ciphertext: String,
}

pub(crate) struct PlanStore {
    root: PathBuf,
    account_id: String,
    secrets: Arc<dyn SecretStore>,
}

impl PlanStore {
    pub(crate) fn new(
        app_local_data: &Path,
        account: &ValidatedAccountId,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            root: app_local_data
                .join("accounts")
                .join(account_directory_name(account.as_str()))
                .join(PLAN_DIR),
            account_id: account.as_str().to_owned(),
            secrets,
        }
    }

    /// A missing file is an empty store. Any other failure is an error, and the
    /// caller must not write `meta.json` after one: an unreadable file may still
    /// hold the profiles and the host id.
    pub(crate) fn load_meta(&self) -> Result<Meta, StoreError> {
        let Some(bytes) = read_bounded(&self.root.join(META_FILE), MAX_META_BYTES)
            .map_err(|_| StoreError::Unreadable)?
        else {
            return Ok(Meta::default());
        };
        serde_json::from_slice(&bytes).map_err(|_| StoreError::Corrupt)
    }

    pub(crate) fn save_meta(&self, meta: &Meta) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(meta).map_err(|_| StoreError::Unavailable)?;
        self.write_atomic(&self.root.join(META_FILE), &bytes)
    }

    /// The device's host id. It is created once and then kept in `meta.json`.
    pub(crate) fn host_id(&self) -> Result<String, StoreError> {
        let mut meta = self.load_meta()?;
        if let Some(host_id) = meta.host_id.clone() {
            return Ok(host_id);
        }
        let host_id = oauth::new_host_id();
        meta.host_id = Some(host_id.clone());
        self.save_meta(&meta)?;
        Ok(host_id)
    }

    pub(crate) fn load_session(
        &self,
        client_id: &str,
    ) -> Result<Option<SessionRecord>, StoreError> {
        let Some(bytes) = read_bounded(&self.session_path(client_id), MAX_SESSION_BYTES)
            .map_err(|_| StoreError::Unreadable)?
        else {
            return Ok(None);
        };
        let sealed: SealedSession =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::Corrupt)?;
        let nonce = STANDARD
            .decode(&sealed.nonce)
            .map_err(|_| StoreError::Corrupt)?;
        let ciphertext = STANDARD
            .decode(&sealed.ciphertext)
            .map_err(|_| StoreError::Corrupt)?;
        // The keyring does not tell a missing key from a locked one. Both read as
        // Unreadable, so the sealed record is kept for when the key can be read.
        let key = self
            .secrets
            .account_key(&self.account_id, sealed.key_version)
            .map_err(|_| StoreError::Unreadable)?;
        let metadata = PlanSessionAuthenticatedMetadata {
            account_id: &self.account_id,
            client_id,
        };
        let plaintext = Zeroizing::new(
            decrypt_plan_session(
                &key,
                sealed.aad_schema_version,
                sealed.key_version,
                &nonce,
                &metadata,
                &ciphertext,
            )
            .map_err(|_| StoreError::Corrupt)?,
        );
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|_| StoreError::Corrupt)
    }

    pub(crate) fn save_session(&self, record: &SessionRecord) -> Result<(), StoreError> {
        let plaintext =
            Zeroizing::new(serde_json::to_vec(record).map_err(|_| StoreError::Unavailable)?);
        let (key_version, key) = self
            .secrets
            .load_or_create_active_key(&self.account_id)
            .map_err(|_| StoreError::Unavailable)?;
        let metadata = PlanSessionAuthenticatedMetadata {
            account_id: &self.account_id,
            client_id: &record.client_id,
        };
        let sealed = encrypt_plan_session(&key, key_version, &metadata, &plaintext)
            .map_err(|_| StoreError::Unavailable)?;
        let envelope = SealedSession {
            aad_schema_version: sealed.aad_schema_version,
            key_version,
            nonce: STANDARD.encode(&sealed.nonce),
            ciphertext: STANDARD.encode(&sealed.ciphertext),
        };
        let bytes = serde_json::to_vec(&envelope).map_err(|_| StoreError::Unavailable)?;
        self.write_atomic(&self.session_path(&record.client_id), &bytes)
    }

    /// Removing a session that is already gone is not an error.
    pub(crate) fn delete_session(&self, client_id: &str) -> Result<(), StoreError> {
        remove_if_present(&self.session_path(client_id))
    }

    /// Removes every sealed session in the directory, whichever client it belongs
    /// to. A session that no profile names is removed too.
    pub(crate) fn delete_all_sessions(&self) -> Result<(), StoreError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(StoreError::Unavailable),
        };
        for entry in entries {
            let entry = entry.map_err(|_| StoreError::Unavailable)?;
            let is_session = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("session-") && name.ends_with(".bin"));
            if is_session {
                remove_if_present(&entry.path())?;
            }
        }
        Ok(())
    }

    /// The path of `meta.json`, for tests that damage the file.
    #[cfg(test)]
    pub(crate) fn meta_path(&self) -> PathBuf {
        self.root.join(META_FILE)
    }

    /// The file name is a digest, so a directory listing shows no client id.
    fn session_path(&self, client_id: &str) -> PathBuf {
        let digest = format!("{:x}", Sha256::digest(client_id.as_bytes()));
        self.root.join(format!("session-{}.bin", &digest[..32]))
    }

    /// The bytes land in a temporary file beside `path` and are then renamed
    /// over it, so a reader sees the old file or the new one, never a mix.
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root).map_err(|_| StoreError::Unavailable)?;
        #[cfg(unix)]
        {
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(|_| StoreError::Unavailable)?;
        }
        let mut file =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|_| StoreError::Unavailable)?;
        #[cfg(unix)]
        {
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| StoreError::Unavailable)?;
        }
        file.write_all(bytes)
            .and_then(|()| file.as_file().sync_all())
            .map_err(|_| StoreError::Unavailable)?;
        file.persist(path).map_err(|_| StoreError::Unavailable)?;
        // A rename is durable only once its directory entry is flushed. This is
        // the std form of `projects::platform::fsync_dir_after_replace`, which is
        // not reachable from here. Windows has no such flush through std, and its
        // rename is already a durable OS transaction, so this runs on unix only.
        #[cfg(unix)]
        {
            fs::File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| StoreError::Unavailable)?;
        }
        Ok(())
    }
}

/// Removing a file that is already gone is not an error.
fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(StoreError::Unavailable),
    }
}

/// A store double for tests. It delegates to a real in-memory store and fails the
/// key operations a test arms, standing in for a locked keychain or a failed write.
#[cfg(test)]
pub(crate) struct FlakySecrets {
    pub(crate) inner: std::sync::Arc<crate::persistence::InMemorySecretStore>,
    /// How many of the next key creations fail.
    pub(crate) failing_creates: std::sync::atomic::AtomicUsize,
    /// Whether key reads fail.
    pub(crate) failing_reads: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl FlakySecrets {
    pub(crate) fn new(
        inner: &std::sync::Arc<crate::persistence::InMemorySecretStore>,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: inner.clone(),
            failing_creates: std::sync::atomic::AtomicUsize::new(0),
            failing_reads: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

#[cfg(test)]
impl SecretStore for FlakySecrets {
    fn load_or_create_active_key(
        &self,
        account_id: &str,
    ) -> crate::domain::error::AppResult<(u32, Zeroizing<[u8; 32]>)> {
        use std::sync::atomic::Ordering;
        let fails = self
            .failing_creates
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok();
        if fails {
            return Err(crate::domain::error::AppError::PersistenceUnavailable);
        }
        self.inner.load_or_create_active_key(account_id)
    }

    fn active_key_version(&self, account_id: &str) -> crate::domain::error::AppResult<u32> {
        self.inner.active_key_version(account_id)
    }

    fn account_key(
        &self,
        account_id: &str,
        key_version: u32,
    ) -> crate::domain::error::AppResult<Zeroizing<[u8; 32]>> {
        if self.failing_reads.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::domain::error::AppError::PersistenceKeyUnavailable);
        }
        self.inner.account_key(account_id, key_version)
    }

    fn delete_account_keys(&self, account_id: &str) -> crate::domain::error::AppResult<()> {
        self.inner.delete_account_keys(account_id)
    }
}

/// Reads a whole file of at most `limit` bytes. `Ok(None)` when it does not exist.
fn read_bounded(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("the file is larger than its limit"));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::InMemorySecretStore;

    const CLIENT: &str = "oaiapp_client_one";
    const ACCESS: &str = "access-token-value";
    const REFRESH: &str = "refresh-token-value";
    const ID: &str = "id-token-value";
    const SUBJECT: &str = "user-123";
    const EMAIL: &str = "person@example.com";

    fn secrets() -> Arc<InMemorySecretStore> {
        Arc::new(InMemorySecretStore::default())
    }

    fn plan_store(root: &Path, account: &str, secrets: &Arc<InMemorySecretStore>) -> PlanStore {
        let account = ValidatedAccountId::parse(account).expect("account");
        PlanStore::new(root, &account, secrets.clone())
    }

    fn session_record(client_id: &str) -> SessionRecord {
        SessionRecord {
            client_id: client_id.to_owned(),
            subject: SUBJECT.to_owned(),
            email: Some(EMAIL.to_owned()),
            id_token: Some(ID.to_owned()),
            access_token: ACCESS.to_owned(),
            refresh_token: Some(REFRESH.to_owned()),
            expires_at: 1_800_003_600,
            earliest_refresh_at: Some(1_800_001_800),
            scopes: vec!["openid".to_owned(), "chatgpt.tokens.use.direct".to_owned()],
            saved_at: 1_800_000_000,
        }
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[test]
    fn session_round_trips_and_deletes() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let store = plan_store(dir.path(), "person:tenant", &secrets);
        assert!(store.load_session(CLIENT).expect("load").is_none());

        store.save_session(&session_record(CLIENT)).expect("save");
        let loaded = store
            .load_session(CLIENT)
            .expect("load")
            .expect("saved record");
        assert_eq!(loaded.client_id, CLIENT);
        assert_eq!(loaded.subject, SUBJECT);
        assert_eq!(loaded.email.as_deref(), Some(EMAIL));
        assert_eq!(loaded.id_token.as_deref(), Some(ID));
        assert_eq!(loaded.access_token, ACCESS);
        assert_eq!(loaded.refresh_token.as_deref(), Some(REFRESH));
        assert_eq!(loaded.expires_at, 1_800_003_600);
        assert_eq!(loaded.earliest_refresh_at, Some(1_800_001_800));
        assert_eq!(loaded.scopes.len(), 2);
        assert_eq!(loaded.saved_at, 1_800_000_000);

        store.delete_session(CLIENT).expect("delete");
        assert!(store.load_session(CLIENT).expect("load").is_none());
        store
            .delete_session(CLIENT)
            .expect("a missing file is fine");
    }

    #[test]
    fn sealed_session_file_holds_no_token_or_identity_text() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let store = plan_store(dir.path(), "person:tenant", &secrets);
        store.save_session(&session_record(CLIENT)).expect("save");

        let bytes = fs::read(store.session_path(CLIENT)).expect("session file");
        for text in [ACCESS, REFRESH, ID, SUBJECT, EMAIL] {
            assert!(!contains_bytes(&bytes, text.as_bytes()), "{text} on disk");
        }
    }

    #[test]
    fn a_session_file_moved_to_another_client_or_account_reads_as_corrupt() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let first = plan_store(dir.path(), "person:tenant", &secrets);
        first.save_session(&session_record(CLIENT)).expect("save");

        fs::copy(
            first.session_path(CLIENT),
            first.session_path("oaiapp_client_two"),
        )
        .expect("copy to another client");
        assert_eq!(
            first.load_session("oaiapp_client_two").err(),
            Some(StoreError::Corrupt)
        );

        // The second account has its own key, so only the account binding fails.
        let second = plan_store(dir.path(), "person:other", &secrets);
        second
            .save_session(&session_record(CLIENT))
            .expect("second account saves");
        fs::copy(first.session_path(CLIENT), second.session_path(CLIENT))
            .expect("copy to another account");
        assert_eq!(second.load_session(CLIENT).err(), Some(StoreError::Corrupt));
    }

    #[test]
    fn host_id_is_created_once_and_survives_a_new_store_instance() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let first = plan_store(dir.path(), "person:tenant", &secrets)
            .host_id()
            .expect("host id");
        assert!(first.starts_with("urn:uuid:"));

        let again = plan_store(dir.path(), "person:tenant", &secrets)
            .host_id()
            .expect("host id");
        assert_eq!(first, again);
    }

    #[test]
    fn meta_round_trips_through_the_file() {
        let dir = tempfile::tempdir().expect("dir");
        let store = plan_store(dir.path(), "person:tenant", &secrets());
        let meta = Meta {
            host_id: Some("urn:uuid:host".to_owned()),
            active_client_id: Some(CLIENT.to_owned()),
            welcome_acknowledged: true,
            profiles: vec![Profile {
                client_id: CLIENT.to_owned(),
                subject: SUBJECT.to_owned(),
                email: Some(EMAIL.to_owned()),
            }],
        };
        store.save_meta(&meta).expect("save");
        assert_eq!(store.load_meta().expect("load"), meta);
    }

    #[test]
    fn a_profile_written_without_an_email_reads_back_with_none() {
        let dir = tempfile::tempdir().expect("dir");
        let store = plan_store(dir.path(), "person:tenant", &secrets());
        fs::create_dir_all(&store.root).expect("root");
        let legacy = format!(
            r#"{{"host_id":null,"active_client_id":null,"welcome_acknowledged":false,"profiles":[{{"client_id":"{CLIENT}","subject":"{SUBJECT}"}}]}}"#
        );
        fs::write(store.root.join(META_FILE), legacy).expect("legacy meta");

        let profiles = store.load_meta().expect("load").profiles;
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].email, None);
    }

    #[test]
    fn missing_meta_reads_as_empty() {
        let dir = tempfile::tempdir().expect("dir");
        let store = plan_store(dir.path(), "person:tenant", &secrets());
        assert_eq!(
            store.load_meta().expect("missing is empty"),
            Meta::default()
        );
    }

    #[test]
    fn corrupt_meta_is_an_error_not_an_empty_store() {
        let dir = tempfile::tempdir().expect("dir");
        let store = plan_store(dir.path(), "person:tenant", &secrets());
        fs::create_dir_all(&store.root).expect("root");
        fs::write(store.root.join(META_FILE), b"{not json").expect("corrupt file");
        assert_eq!(store.load_meta().err(), Some(StoreError::Corrupt));
    }

    #[test]
    fn oversized_or_unreadable_meta_is_unreadable() {
        let dir = tempfile::tempdir().expect("dir");
        let store = plan_store(dir.path(), "person:tenant", &secrets());
        fs::create_dir_all(&store.root).expect("root");
        let meta_path = store.root.join(META_FILE);

        // Valid JSON, so only the size bound makes this file unreadable.
        let mut oversized = br#"{"welcome_acknowledged":true,"pad":""#.to_vec();
        oversized.resize(oversized.len() + MAX_META_BYTES as usize, b'x');
        oversized.extend_from_slice(br#""}"#);
        fs::write(&meta_path, &oversized).expect("oversized file");
        assert_eq!(store.load_meta().err(), Some(StoreError::Unreadable));

        // A directory where the file belongs cannot be read as one.
        fs::remove_file(&meta_path).expect("remove");
        fs::create_dir(&meta_path).expect("directory in place of the file");
        assert_eq!(store.load_meta().err(), Some(StoreError::Unreadable));
    }

    #[test]
    fn an_unreadable_meta_file_is_kept_and_no_host_id_is_made_over_it() {
        let dir = tempfile::tempdir().expect("dir");
        let store = plan_store(dir.path(), "person:tenant", &secrets());
        fs::create_dir_all(store.root.join(META_FILE)).expect("directory in place of meta");

        assert_eq!(store.host_id().err(), Some(StoreError::Unreadable));
        assert!(store.root.join(META_FILE).is_dir(), "meta was replaced");
    }

    #[test]
    fn a_session_file_that_cannot_be_read_is_unreadable_and_kept() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let store = plan_store(dir.path(), "person:tenant", &secrets);
        fs::create_dir_all(store.session_path(CLIENT)).expect("directory in place of session");

        assert_eq!(
            store.load_session(CLIENT).err(),
            Some(StoreError::Unreadable)
        );
        assert!(store.session_path(CLIENT).is_dir(), "session was removed");
    }

    #[test]
    fn a_locked_keychain_is_unreadable_and_the_sealed_record_is_kept() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        plan_store(dir.path(), "person:tenant", &secrets)
            .save_session(&session_record(CLIENT))
            .expect("save");
        let flaky = FlakySecrets::new(&secrets);
        let account = ValidatedAccountId::parse("person:tenant").expect("account");
        let locked = PlanStore::new(dir.path(), &account, flaky.clone());

        flaky
            .failing_reads
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            locked.load_session(CLIENT).err(),
            Some(StoreError::Unreadable)
        );
        assert!(locked.session_path(CLIENT).is_file(), "record was removed");

        flaky
            .failing_reads
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(locked.load_session(CLIENT).expect("unlocked").is_some());
    }

    #[test]
    fn delete_all_sessions_removes_every_session_file_and_keeps_meta() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let store = plan_store(dir.path(), "person:tenant", &secrets);
        store.save_session(&session_record(CLIENT)).expect("save");
        store
            .save_session(&session_record("oaiapp_client_two"))
            .expect("save second");
        store
            .save_meta(&Meta {
                welcome_acknowledged: true,
                ..Meta::default()
            })
            .expect("meta");

        store.delete_all_sessions().expect("delete all");

        assert!(store.load_session(CLIENT).expect("load").is_none());
        assert!(store
            .load_session("oaiapp_client_two")
            .expect("load")
            .is_none());
        assert!(store.load_meta().expect("meta kept").welcome_acknowledged);
    }
}
