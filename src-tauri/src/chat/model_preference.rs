use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};

pub(crate) const MODEL_PREFERENCE_FILE: &str = "chat-model.json";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelPreferenceDocument {
    version: u16,
    model_id: String,
}

pub(crate) struct ModelPreferenceStore {
    path: PathBuf,
}

impl ModelPreferenceStore {
    #[must_use]
    pub(crate) fn new(app_local_data: &Path, account: &ValidatedAccountId) -> Self {
        Self {
            path: app_local_data
                .join("accounts")
                .join(account_directory_name(account.as_str()))
                .join(MODEL_PREFERENCE_FILE),
        }
    }

    #[must_use]
    pub(crate) fn selected(&self) -> Option<String> {
        let document: ModelPreferenceDocument =
            serde_json::from_slice(&fs::read(&self.path).ok()?).ok()?;
        let model_id = document.model_id.trim();
        (document.version == 1 && !model_id.is_empty() && model_id.len() <= 256)
            .then(|| model_id.to_owned())
    }

    pub(crate) fn set_selected(&self, model_id: &str) -> std::io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| std::io::Error::other("model preference path has no parent"))?;
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(&ModelPreferenceDocument {
            version: 1,
            model_id: model_id.to_owned(),
        })
        .map_err(std::io::Error::other)?;
        let temporary = self.path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(temporary, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_path_contains_only_the_account_digest() {
        let root = tempfile::tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("person:tenant").expect("account");
        let store = ModelPreferenceStore::new(root.path(), &account);

        store
            .set_selected("openai/gpt-5.6-luna")
            .expect("preference write");

        assert_eq!(store.selected().as_deref(), Some("openai/gpt-5.6-luna"));
        assert!(!store.path.to_string_lossy().contains("person:tenant"));
        let bytes = fs::read(&store.path).expect("preference bytes");
        assert!(!String::from_utf8_lossy(&bytes).contains("person:tenant"));
    }

    #[test]
    fn malformed_preferences_do_not_become_model_ids() {
        let root = tempfile::tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("person:tenant").expect("account");
        let store = ModelPreferenceStore::new(root.path(), &account);
        fs::create_dir_all(store.path.parent().expect("parent")).expect("directory");
        fs::write(&store.path, b"{broken").expect("seed");

        assert_eq!(store.selected(), None);
    }
}
