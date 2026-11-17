use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock},
};

use rand::RngCore;
use std::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

use crate::domain::error::{AppError, AppResult};

const DEFAULT_KEY_VERSION: u32 = 1;
const MAX_ACCOUNT_ID_BYTES: usize = 160;
pub const MAX_KEY_VERSIONS: u32 = 64;
const ACTIVE_VERSION_SUFFIX: &str = "active-version";
static KEYRING_MUTATION_LOCK: Mutex<()> = Mutex::new(());

pub trait SecretStore: Send + Sync + 'static {
    fn load_or_create_active_key(&self, account_id: &str) -> AppResult<(u32, Zeroizing<[u8; 32]>)>;
    fn active_key_version(&self, account_id: &str) -> AppResult<u32>;
    fn account_key(&self, account_id: &str, key_version: u32) -> AppResult<Zeroizing<[u8; 32]>>;
    fn delete_account_keys(&self, account_id: &str) -> AppResult<()>;
}

pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> AppResult<Self> {
        let service = service.into();
        if service.trim().is_empty() || service.len() > 160 {
            return Err(AppError::PersistenceInvalid("keyring service is invalid"));
        }
        Ok(Self { service })
    }

    pub fn provision_active_key(
        &self,
        account_id: &str,
        key_version: u32,
        key: Zeroizing<[u8; 32]>,
    ) -> AppResult<()> {
        let _guard = KEYRING_MUTATION_LOCK
            .lock()
            .map_err(|_| AppError::PersistenceUnavailable)?;
        self.provision_active_key_locked(account_id, key_version, key)
    }

    fn provision_active_key_locked(
        &self,
        account_id: &str,
        key_version: u32,
        key: Zeroizing<[u8; 32]>,
    ) -> AppResult<()> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        let key_entry = self.key_entry(account_id, key_version)?;
        match key_entry.get_secret() {
            Ok(mut existing) => {
                existing.zeroize();
                return Err(AppError::PersistenceKeyVersionConflict);
            }
            Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err(AppError::PersistenceKeyUnavailable),
        }
        match self.active_version_entry(account_id)?.get_password() {
            Ok(active_version) => {
                let active_version: u32 = active_version
                    .parse()
                    .map_err(|_| AppError::PersistenceKeyUnavailable)?;
                if key_version <= active_version {
                    return Err(AppError::PersistenceKeyVersionConflict);
                }
            }
            Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err(AppError::PersistenceKeyUnavailable),
        }
        key_entry
            .set_secret(key.as_ref())
            .map_err(|_| AppError::PersistenceKeyUnavailable)?;
        if let Err(error) = self.set_active_key_version(account_id, key_version) {
            let _ = key_entry.delete_credential();
            return Err(error);
        }
        Ok(())
    }

    fn set_active_key_version(&self, account_id: &str, key_version: u32) -> AppResult<()> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        let mut stored = Zeroizing::new(
            self.key_entry(account_id, key_version)?
                .get_secret()
                .map_err(|_| AppError::PersistenceKeyUnavailable)?,
        );
        if stored.len() != 32 {
            return Err(AppError::PersistenceKeyUnavailable);
        }
        stored.zeroize();
        self.active_version_entry(account_id)?
            .set_password(&key_version.to_string())
            .map_err(|_| AppError::PersistenceKeyUnavailable)
    }

    fn active_version_entry(&self, account_id: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(
            &self.service,
            &format!("{account_id}:{ACTIVE_VERSION_SUFFIX}"),
        )
        .map_err(|_| AppError::PersistenceKeyUnavailable)
    }

    fn key_entry(&self, account_id: &str, key_version: u32) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, &format!("{account_id}:key-v{key_version}"))
            .map_err(|_| AppError::PersistenceKeyUnavailable)
    }
}

impl SecretStore for KeyringSecretStore {
    fn load_or_create_active_key(&self, account_id: &str) -> AppResult<(u32, Zeroizing<[u8; 32]>)> {
        validate_account_id(account_id)?;
        let _guard = KEYRING_MUTATION_LOCK
            .lock()
            .map_err(|_| AppError::PersistenceUnavailable)?;
        match self.active_version_entry(account_id)?.get_password() {
            Ok(value) => {
                let key_version = value
                    .parse()
                    .map_err(|_| AppError::PersistenceKeyUnavailable)?;
                validate_key_version(key_version)?;
                let key = self.account_key(account_id, key_version)?;
                Ok((key_version, key))
            }
            Err(keyring::Error::NoEntry) => {
                let mut key = Zeroizing::new([0_u8; 32]);
                rand::rng().fill_bytes(key.as_mut());
                match self.provision_active_key_locked(account_id, DEFAULT_KEY_VERSION, key) {
                    Ok(()) | Err(AppError::PersistenceKeyVersionConflict) => {
                        let key_version = self.active_key_version(account_id)?;
                        let key = self.account_key(account_id, key_version)?;
                        Ok((key_version, key))
                    }
                    Err(error) => Err(error),
                }
            }
            Err(_) => Err(AppError::PersistenceKeyUnavailable),
        }
    }

    fn active_key_version(&self, account_id: &str) -> AppResult<u32> {
        validate_account_id(account_id)?;
        let value = self
            .active_version_entry(account_id)?
            .get_password()
            .map_err(|_| AppError::PersistenceKeyUnavailable)?;
        let key_version = value
            .parse()
            .map_err(|_| AppError::PersistenceKeyUnavailable)?;
        validate_key_version(key_version)?;
        Ok(key_version)
    }

    fn account_key(&self, account_id: &str, key_version: u32) -> AppResult<Zeroizing<[u8; 32]>> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        let mut stored = Zeroizing::new(
            self.key_entry(account_id, key_version)?
                .get_secret()
                .map_err(|_| AppError::PersistenceKeyUnavailable)?,
        );
        if stored.len() != 32 {
            return Err(AppError::PersistenceKeyUnavailable);
        }
        let mut key = Zeroizing::new([0_u8; 32]);
        key.copy_from_slice(stored.as_slice());
        stored.zeroize();
        Ok(key)
    }

    fn delete_account_keys(&self, account_id: &str) -> AppResult<()> {
        validate_account_id(account_id)?;
        let _guard = KEYRING_MUTATION_LOCK
            .lock()
            .map_err(|_| AppError::PersistenceUnavailable)?;
        match self.active_version_entry(account_id)?.get_password() {
            Ok(value) => {
                let active_version = value
                    .parse()
                    .map_err(|_| AppError::PersistenceKeyUnavailable)?;
                validate_key_version(active_version)?;
            }
            Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err(AppError::PersistenceKeyUnavailable),
        }
        for key_version in 1..=MAX_KEY_VERSIONS {
            if let Err(error) = self.key_entry(account_id, key_version)?.delete_credential() {
                if !matches!(error, keyring::Error::NoEntry) {
                    return Err(AppError::PersistenceKeyUnavailable);
                }
            }
        }
        if let Err(error) = self.active_version_entry(account_id)?.delete_credential() {
            if !matches!(error, keyring::Error::NoEntry) {
                return Err(AppError::PersistenceKeyUnavailable);
            }
        }
        Ok(())
    }
}

/// Wraps a [`SecretStore`] so an account's device key is fetched from the
/// backing store once per process rather than once per cryptographic
/// operation.
///
/// Every sealed record -- each journal event, chat projection row, report,
/// project index entry, approval, and vault credential -- resolves its key
/// through this trait. Without a cache that is one OS keychain round trip per
/// record: on macOS a `securityd` IPC each time, and on any build whose code
/// signature the keychain ACL does not recognize, one *user-facing
/// authorization prompt* each time. Decrypting a conversation therefore asked
/// the user to approve keychain access once per message. Caching moves that to
/// at most one prompt per account per store per launch.
///
/// The cache holds key material for the life of the process. That is a
/// deliberate, narrow widening of the exposure window, not a new exposure:
/// the same key was already resident in this process on every seal and open,
/// and the app cannot read its own storage without it. Entries stay in
/// [`Zeroizing`], so eviction and process teardown wipe them, and
/// [`Self::delete_account_keys`] evicts before it deletes so a failed delete
/// cannot leave a live key cached behind a store that no longer has one.
///
/// There is no rotation path in the app today -- `provision_active_key` has no
/// production caller -- so a cached active version cannot go stale behind this
/// decorator. Adding rotation means invalidating this cache in the same
/// change; the `key_version`-keyed map below is already shaped for it, since a
/// rotated key lands under a new key.
pub struct CachedSecretStore {
    inner: Arc<dyn SecretStore>,
    active_versions: RwLock<HashMap<String, u32>>,
    keys: RwLock<StoredAccountKeys>,
}

/// How many distinct accounts may hold cached key material at once.
///
/// One signed-in account is the norm and switching accounts is rare, so this
/// is a memory bound rather than a working-set target: past it the cache stops
/// admitting entries and calls fall through to `inner`, which is slower but
/// still correct.
const MAX_CACHED_ACCOUNTS: usize = 8;

impl CachedSecretStore {
    #[must_use]
    pub fn new(inner: Arc<dyn SecretStore>) -> Self {
        Self {
            inner,
            active_versions: RwLock::new(HashMap::new()),
            keys: RwLock::new(HashMap::new()),
        }
    }

    fn cached_key(
        &self,
        account_id: &str,
        key_version: u32,
    ) -> AppResult<Option<Zeroizing<[u8; 32]>>> {
        Ok(self
            .keys
            .read()
            .map_err(map_poison)?
            .get(&(account_id.to_owned(), key_version))
            .map(|key| Zeroizing::new(**key)))
    }

    fn admit(
        &self,
        account_id: &str,
        key_version: u32,
        key: &Zeroizing<[u8; 32]>,
    ) -> AppResult<()> {
        let mut keys = self.keys.write().map_err(map_poison)?;
        let versioned_id = (account_id.to_owned(), key_version);
        if !keys.contains_key(&versioned_id) && keys.len() >= MAX_CACHED_ACCOUNTS * 2 {
            return Ok(());
        }
        keys.insert(versioned_id, Zeroizing::new(**key));
        let mut active = self.active_versions.write().map_err(map_poison)?;
        if active.len() < MAX_CACHED_ACCOUNTS || active.contains_key(account_id) {
            active.insert(account_id.to_owned(), key_version);
        }
        Ok(())
    }

    fn evict(&self, account_id: &str) -> AppResult<()> {
        self.keys
            .write()
            .map_err(map_poison)?
            .retain(|(cached_account, _), _| cached_account != account_id);
        self.active_versions
            .write()
            .map_err(map_poison)?
            .remove(account_id);
        Ok(())
    }
}

impl SecretStore for CachedSecretStore {
    fn load_or_create_active_key(&self, account_id: &str) -> AppResult<(u32, Zeroizing<[u8; 32]>)> {
        validate_account_id(account_id)?;
        let cached_version = self
            .active_versions
            .read()
            .map_err(map_poison)?
            .get(account_id)
            .copied();
        if let Some(key_version) = cached_version {
            if let Some(key) = self.cached_key(account_id, key_version)? {
                return Ok((key_version, key));
            }
        }
        let (key_version, key) = self.inner.load_or_create_active_key(account_id)?;
        self.admit(account_id, key_version, &key)?;
        Ok((key_version, key))
    }

    fn active_key_version(&self, account_id: &str) -> AppResult<u32> {
        validate_account_id(account_id)?;
        if let Some(key_version) = self
            .active_versions
            .read()
            .map_err(map_poison)?
            .get(account_id)
            .copied()
        {
            return Ok(key_version);
        }
        let key_version = self.inner.active_key_version(account_id)?;
        let key = self.inner.account_key(account_id, key_version)?;
        self.admit(account_id, key_version, &key)?;
        Ok(key_version)
    }

    fn account_key(&self, account_id: &str, key_version: u32) -> AppResult<Zeroizing<[u8; 32]>> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        if let Some(key) = self.cached_key(account_id, key_version)? {
            return Ok(key);
        }
        let key = self.inner.account_key(account_id, key_version)?;
        self.admit(account_id, key_version, &key)?;
        Ok(key)
    }

    /// Evicts before delegating: a delete that fails partway must not leave a
    /// usable key cached for an account the backing store has begun forgetting.
    fn delete_account_keys(&self, account_id: &str) -> AppResult<()> {
        validate_account_id(account_id)?;
        self.evict(account_id)?;
        self.inner.delete_account_keys(account_id)
    }
}

impl std::fmt::Debug for CachedSecretStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CachedSecretStore")
            .finish_non_exhaustive()
    }
}

type VersionedAccountKey = (String, u32);
type StoredAccountKeys = HashMap<VersionedAccountKey, Zeroizing<[u8; 32]>>;

#[cfg(test)]
#[derive(Default)]
pub struct InMemorySecretStore {
    keys: RwLock<StoredAccountKeys>,
    active_versions: RwLock<HashMap<String, u32>>,
}

#[cfg(test)]
impl InMemorySecretStore {
    pub fn put_key(&self, account_id: &str, key: [u8; 32]) -> AppResult<()> {
        self.put_versioned_key(account_id, DEFAULT_KEY_VERSION, key)?;
        self.set_active_key_version(account_id, DEFAULT_KEY_VERSION)
    }

    pub fn put_versioned_key(
        &self,
        account_id: &str,
        key_version: u32,
        key: [u8; 32],
    ) -> AppResult<()> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        let versioned_id = (account_id.to_owned(), key_version);
        let mut keys = self.keys.write().map_err(map_poison)?;
        if keys.contains_key(&versioned_id) {
            return Err(AppError::PersistenceKeyVersionConflict);
        }
        if let Some(active_version) = self
            .active_versions
            .read()
            .map_err(map_poison)?
            .get(account_id)
        {
            if key_version <= *active_version {
                return Err(AppError::PersistenceKeyVersionConflict);
            }
        }
        keys.insert(versioned_id, Zeroizing::new(key));
        Ok(())
    }

    pub fn set_active_key_version(&self, account_id: &str, key_version: u32) -> AppResult<()> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        if !self
            .keys
            .read()
            .map_err(map_poison)?
            .contains_key(&(account_id.to_owned(), key_version))
        {
            return Err(AppError::PersistenceKeyUnavailable);
        }
        self.active_versions
            .write()
            .map_err(map_poison)?
            .insert(account_id.to_owned(), key_version);
        Ok(())
    }

    pub fn replace_key_for_test(
        &self,
        account_id: &str,
        key_version: u32,
        key: [u8; 32],
    ) -> AppResult<()> {
        let mut keys = self.keys.write().map_err(map_poison)?;
        let entry = keys
            .get_mut(&(account_id.to_owned(), key_version))
            .ok_or(AppError::PersistenceKeyUnavailable)?;
        **entry = key;
        Ok(())
    }

    pub fn insert_legacy_key_for_test(
        &self,
        account_id: &str,
        key_version: u32,
        key: [u8; 32],
    ) -> AppResult<()> {
        validate_account_id(account_id)?;
        self.keys
            .write()
            .map_err(map_poison)?
            .insert((account_id.to_owned(), key_version), Zeroizing::new(key));
        self.active_versions
            .write()
            .map_err(map_poison)?
            .insert(account_id.to_owned(), key_version);
        Ok(())
    }

    pub fn raw_active_version_for_test(&self, account_id: &str) -> Option<u32> {
        self.active_versions
            .read()
            .ok()
            .and_then(|versions| versions.get(account_id).copied())
    }

    pub fn contains_key_for_test(&self, account_id: &str, key_version: u32) -> bool {
        self.keys
            .read()
            .map(|keys| keys.contains_key(&(account_id.to_owned(), key_version)))
            .unwrap_or(false)
    }
}

#[cfg(test)]
impl SecretStore for InMemorySecretStore {
    fn load_or_create_active_key(&self, account_id: &str) -> AppResult<(u32, Zeroizing<[u8; 32]>)> {
        validate_account_id(account_id)?;
        if let Ok(key_version) = self.active_key_version(account_id) {
            return Ok((key_version, self.account_key(account_id, key_version)?));
        }
        let mut key = [0_u8; 32];
        rand::rng().fill_bytes(&mut key);
        match self.put_key(account_id, key) {
            Ok(()) | Err(AppError::PersistenceKeyVersionConflict) => {
                let key_version = self.active_key_version(account_id)?;
                Ok((key_version, self.account_key(account_id, key_version)?))
            }
            Err(error) => Err(error),
        }
    }

    fn active_key_version(&self, account_id: &str) -> AppResult<u32> {
        validate_account_id(account_id)?;
        self.active_versions
            .read()
            .map_err(map_poison)?
            .get(account_id)
            .copied()
            .ok_or(AppError::PersistenceKeyUnavailable)
    }

    fn account_key(&self, account_id: &str, key_version: u32) -> AppResult<Zeroizing<[u8; 32]>> {
        validate_account_id(account_id)?;
        validate_key_version(key_version)?;
        self.keys
            .read()
            .map_err(map_poison)?
            .get(&(account_id.to_owned(), key_version))
            .map(|key| Zeroizing::new(**key))
            .ok_or(AppError::PersistenceKeyUnavailable)
    }

    fn delete_account_keys(&self, account_id: &str) -> AppResult<()> {
        validate_account_id(account_id)?;
        if let Some(active_version) = self
            .active_versions
            .read()
            .map_err(map_poison)?
            .get(account_id)
            .copied()
        {
            validate_key_version(active_version)?;
        }
        self.keys
            .write()
            .map_err(map_poison)?
            .retain(|(stored_account, _), _| stored_account != account_id);
        self.active_versions
            .write()
            .map_err(map_poison)?
            .remove(account_id);
        Ok(())
    }
}

fn validate_account_id(account_id: &str) -> AppResult<()> {
    if account_id.is_empty()
        || account_id.len() > MAX_ACCOUNT_ID_BYTES
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::PersistenceInvalid("account_id is invalid"));
    }
    Ok(())
}

fn validate_key_version(key_version: u32) -> AppResult<()> {
    if key_version == 0 || key_version > MAX_KEY_VERSIONS {
        return Err(AppError::PersistenceInvalid(
            "key_version is outside the supported cleanup bound",
        ));
    }
    Ok(())
}

fn map_poison<T>(_: PoisonError<T>) -> AppError {
    AppError::PersistenceUnavailable
}
