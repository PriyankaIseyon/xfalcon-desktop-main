use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xfalcon_contracts::custom_models::CustomModelSampling;

use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};
use crate::settings::write_atomically;

pub(crate) const CUSTOM_MODEL_SETTINGS_FILE: &str = "custom-model-settings.json";
const MAX_SETTINGS_BYTES: u64 = 256 * 1024;
const MAX_SETTINGS_MODELS: usize = 2_048;

fn settings_io_lock() -> &'static RwLock<()> {
    static LOCK: OnceLock<RwLock<()>> = OnceLock::new();
    LOCK.get_or_init(RwLock::default)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CustomModelSettingsDocument {
    version: u16,
    models: BTreeMap<String, CustomModelSampling>,
}

pub(crate) struct CustomModelSettingsStore {
    path: PathBuf,
}

impl CustomModelSettingsStore {
    #[must_use]
    pub(crate) fn new(app_local_data: &Path, account: &ValidatedAccountId) -> Self {
        Self {
            path: app_local_data
                .join("accounts")
                .join(account_directory_name(account.as_str()))
                .join(CUSTOM_MODEL_SETTINGS_FILE),
        }
    }

    pub(crate) fn sampling(&self, model_id: Uuid) -> std::io::Result<CustomModelSampling> {
        let _read = settings_io_lock()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(self
            .read_document()?
            .models
            .remove(&model_id.to_string())
            .unwrap_or_default())
    }

    pub(crate) fn sampling_snapshot(&self) -> std::io::Result<BTreeMap<Uuid, CustomModelSampling>> {
        let _read = settings_io_lock()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.read_document()?
            .models
            .into_iter()
            .map(|(model_id, sampling)| {
                Uuid::parse_str(&model_id)
                    .map(|model_id| (model_id, sampling))
                    .map_err(std::io::Error::other)
            })
            .collect()
    }

    pub(crate) fn set_sampling(
        &self,
        model_id: Uuid,
        sampling: CustomModelSampling,
    ) -> std::io::Result<()> {
        let _write = settings_io_lock()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut document = self.read_document()?;
        if sampling == CustomModelSampling::default() {
            document.models.remove(&model_id.to_string());
        } else {
            document.models.insert(model_id.to_string(), sampling);
        }
        self.write_document(&document)
    }

    pub(crate) fn clear(&self, model_id: Uuid) -> std::io::Result<()> {
        let _write = settings_io_lock()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut document = self.read_document()?;
        document.models.remove(&model_id.to_string());
        self.write_document(&document)
    }

    fn read_document(&self) -> std::io::Result<CustomModelSettingsDocument> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) if metadata.len() <= MAX_SETTINGS_BYTES => metadata,
            Ok(_) => return Err(std::io::Error::other("custom model settings are too large")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CustomModelSettingsDocument {
                    version: 1,
                    models: BTreeMap::new(),
                });
            }
            Err(error) => return Err(error),
        };
        let bytes = fs::read(&self.path)?;
        if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(std::io::Error::other(
                "custom model settings changed while reading",
            ));
        }
        let document: CustomModelSettingsDocument =
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if document.version != 1
            || document.models.len() > MAX_SETTINGS_MODELS
            || document
                .models
                .keys()
                .any(|model_id| Uuid::parse_str(model_id).is_err())
        {
            return Err(std::io::Error::other(
                "invalid custom model settings document",
            ));
        }
        Ok(document)
    }

    fn write_document(&self, document: &CustomModelSettingsDocument) -> std::io::Result<()> {
        if document.models.len() > MAX_SETTINGS_MODELS {
            return Err(std::io::Error::other("too many custom model settings"));
        }
        let bytes = serde_json::to_vec_pretty(document).map_err(std::io::Error::other)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(std::io::Error::other("custom model settings are too large"));
        }
        write_atomically(&self.path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(value: &str) -> ValidatedAccountId {
        ValidatedAccountId::parse(value).expect("valid account")
    }

    #[test]
    fn settings_are_isolated_by_account_and_contain_no_account_or_secret() {
        let root = tempfile::tempdir().expect("tempdir");
        let model_id = Uuid::new_v4();
        let first = CustomModelSettingsStore::new(root.path(), &account("tenant:user-a"));
        let second = CustomModelSettingsStore::new(root.path(), &account("tenant:user-b"));
        first
            .set_sampling(
                model_id,
                CustomModelSampling {
                    temperature: Some(0.4),
                    top_p: Some(0.8),
                },
            )
            .expect("save sampling");

        assert_eq!(
            first.sampling(model_id).expect("first read").temperature,
            Some(0.4)
        );
        assert_eq!(
            second.sampling(model_id).expect("second read"),
            CustomModelSampling::default()
        );
        let text = fs::read_to_string(&first.path).expect("stored settings");
        assert!(!text.contains("tenant:user-a"));
        assert!(!text.to_ascii_lowercase().contains("secret"));
    }

    #[test]
    fn malformed_settings_fail_instead_of_becoming_defaults() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = CustomModelSettingsStore::new(root.path(), &account("tenant:user"));
        fs::create_dir_all(store.path.parent().expect("parent")).expect("directory");
        fs::write(&store.path, b"{broken").expect("seed malformed file");

        assert!(store.sampling(Uuid::new_v4()).is_err());
        assert!(store
            .set_sampling(Uuid::new_v4(), CustomModelSampling::default())
            .is_err());
    }

    #[test]
    fn well_formed_but_unsafe_sampling_and_keys_fail_closed() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = CustomModelSettingsStore::new(root.path(), &account("tenant:user"));
        fs::create_dir_all(store.path.parent().expect("parent")).expect("directory");
        let model_id = Uuid::new_v4();
        fs::write(
            &store.path,
            format!(
                "{{\"version\":1,\"models\":{{\"{model_id}\":{{\"temperature\":3.0,\"top_p\":0.8}}}}}}"
            ),
        )
        .expect("seed invalid sampling");
        assert!(store.sampling(model_id).is_err());

        fs::write(
            &store.path,
            br#"{"version":1,"models":{"not-a-uuid":{"temperature":0.3,"top_p":0.8}}}"#,
        )
        .expect("seed invalid key");
        assert!(store.sampling(model_id).is_err());
    }

    #[test]
    fn clearing_one_model_preserves_other_settings() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = CustomModelSettingsStore::new(root.path(), &account("tenant:user"));
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        for id in [first, second] {
            store
                .set_sampling(
                    id,
                    CustomModelSampling {
                        temperature: Some(0.7),
                        top_p: None,
                    },
                )
                .expect("save");
        }
        store.clear(first).expect("clear first");

        assert_eq!(
            store.sampling(first).expect("first"),
            CustomModelSampling::default()
        );
        assert_eq!(
            store.sampling(second).expect("second").temperature,
            Some(0.7)
        );
    }

    #[test]
    fn snapshot_returns_all_validated_sampling_values() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = CustomModelSettingsStore::new(root.path(), &account("tenant:user"));
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        store
            .set_sampling(
                first,
                CustomModelSampling {
                    temperature: Some(0.4),
                    top_p: None,
                },
            )
            .expect("save first");
        store
            .set_sampling(
                second,
                CustomModelSampling {
                    temperature: None,
                    top_p: Some(0.8),
                },
            )
            .expect("save second");

        let snapshot = store.sampling_snapshot().expect("sampling snapshot");
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[&first].temperature, Some(0.4));
        assert_eq!(snapshot[&second].top_p, Some(0.8));
    }
}
