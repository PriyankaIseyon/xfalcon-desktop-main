//! The tenant model catalogue, held for the session rather than re-fetched.
//!
//! `GET /api/v1/chat/models` answers with the models an operator enabled for
//! the tenant. That answer changes when an operator changes it, not between two
//! messages. It nonetheless used to run on the critical path of every send,
//! every conversation create, every follow-up, and every catalogue read — a
//! round trip ahead of the first streamed token, paid for freshness nobody
//! asked for.
//!
//! Removing it does not make sending work offline. `message_send` still needs
//! DA for the conversation, message, and run rows (`decisions.md` ADR-005).
//! What it removes is one round trip, and one way for a send to fail.
//!
//! `workspace_status` owns the fetch now and publishes what it read here; the
//! chat paths read the published snapshot. A chat command that arrives before
//! the first publish fetches once itself, so a cold cache is slow rather than
//! broken.
//!
//! For product models, that fetch is the normal freshness story, and it is narrow:
//! `useWorkspaceConnection` runs it on mount and when Settings changes
//! something that feeds it — **not** on a return to the foreground. So a
//! snapshot normally lasts as long as the window does, and a product model an
//! operator enables mid-session is not seen until one of those two events.
//! Private-model mutations refresh it immediately, and private-model inference
//! refreshes the row before resolving its exact endpoint and key binding.
//!
//! The snapshot is account-scoped. A catalogue is what one tenant enabled for
//! one signed-in user, so a snapshot held for another account is a miss, never
//! a fallback.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::chat_api::{DaChatApiError, DaChatSession};
use super::generated::ModelEntity;

/// Catalogue rows that exist only on this device, merged after DA's rows.
pub(crate) trait LocalModelRows: Send + Sync {
    fn rows(&self, account: &str) -> Vec<ModelEntity>;
}

/// The last catalogue read from DA, and the account it was read for.
#[derive(Default)]
pub(crate) struct ModelCatalogCache {
    held: Mutex<Option<AccountCatalog>>,
    /// Read on every call rather than held, so a change to the local rows
    /// shows at once and not at the next DA refresh.
    local_rows: OnceLock<Arc<dyn LocalModelRows>>,
}

struct AccountCatalog {
    account: String,
    models: Arc<Vec<ModelEntity>>,
}

impl ModelCatalogCache {
    /// Fetches the catalogue and publishes it, replacing whatever was held.
    ///
    /// Workspace refresh, private-model mutations, and private-model inference
    /// use this when they need the current catalog.
    pub(crate) async fn refresh(
        &self,
        da: &DaChatSession,
        account: &str,
    ) -> Result<Arc<Vec<ModelEntity>>, DaChatApiError> {
        let models = self.publish(account, da.list_models().await?);
        Ok(self.with_local_rows(account, models))
    }

    /// The catalogue held for `account`, fetched once when none is held.
    ///
    /// Two callers that miss together both fetch. That is a repeated `GET`
    /// against an idempotent route, which costs less than holding a lock across
    /// the network for the hits that are the normal case.
    pub(crate) async fn read_through(
        &self,
        da: &DaChatSession,
        account: &str,
    ) -> Result<Arc<Vec<ModelEntity>>, DaChatApiError> {
        match self.held_for(account) {
            Some(models) => Ok(self.with_local_rows(account, models)),
            None => self.refresh(da, account).await,
        }
    }

    /// Drops the snapshot. Sign-out must not leave one account's catalogue
    /// readable by the next session in this process.
    pub(crate) fn clear(&self) {
        *self.lock() = None;
    }

    /// Wired once at startup. A second source is ignored.
    pub(crate) fn set_local_rows(&self, source: Arc<dyn LocalModelRows>) {
        let _ = self.local_rows.set(source);
    }

    fn held_for(&self, account: &str) -> Option<Arc<Vec<ModelEntity>>> {
        self.lock()
            .as_ref()
            .filter(|held| held.account == account)
            .map(|held| Arc::clone(&held.models))
    }

    /// The DA rows followed by this account's local rows. With no source, or a
    /// source that has nothing for this account, the DA snapshot itself is
    /// returned, so a device without plan models reads exactly what it did.
    fn with_local_rows(
        &self,
        account: &str,
        models: Arc<Vec<ModelEntity>>,
    ) -> Arc<Vec<ModelEntity>> {
        let Some(source) = self.local_rows.get() else {
            return models;
        };
        let local = source.rows(account);
        if local.is_empty() {
            return models;
        }
        Arc::new(models.iter().cloned().chain(local).collect())
    }

    fn publish(&self, account: &str, models: Vec<ModelEntity>) -> Arc<Vec<ModelEntity>> {
        let models = Arc::new(
            models
                .into_iter()
                .filter(|model| {
                    model.provider != "custom_openai"
                        || account.split_once(':').is_some_and(|(tenant, user)| {
                            model.tenant_id == tenant
                                && model.owner_user_id.as_deref() == Some(user)
                        })
                })
                .collect::<Vec<_>>(),
        );
        // Do not retain an empty answer: a later read must retry. Clear the
        // previous snapshot too, or a refresh after revocation would leave old
        // models visible through read_through.
        *self.lock() = if models.is_empty() {
            None
        } else {
            Some(AccountCatalog {
                account: account.to_owned(),
                models: Arc::clone(&models),
            })
        };
        models
    }

    fn lock(&self) -> MutexGuard<'_, Option<AccountCatalog>> {
        self.held
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::clients::da::chat_api::select_model_by_id;
    use crate::plan_providers::models::{claude_rows, PlanModelSource};
    use crate::providers::claude_runtime::adapter::CLAUDE_MAX_OUTPUT_TOKENS;

    const ROUTE: &str = "/api/v1/chat/models";
    const ACCOUNT: &str = "tenant-1:user-1";
    const BEARER: &str = "model-catalog-local-rows-bearer";
    const PLAN_IDS: [&str; 3] = [
        "claude_code/claude-opus-5-5",
        "claude_code/claude-sonnet-5-5",
        "claude_code/claude-haiku-5-5",
    ];

    fn da_page() -> Value {
        json!({
            "items": [{
                "id": "00000000-0000-0000-0000-0000000000aa",
                "model_id": "sonnet",
                "provider": "openrouter",
                "provider_parent": "openrouter",
                "provider_sub": null,
                "display_name": "Tenant model",
                "context_length": 1_000_000,
                "max_output_tokens": 64_000,
                "input_price_per_m": 1.0,
                "output_price_per_m": 2.0,
                "released_at": null,
                "is_chat": true,
                "is_enabled": true,
                "supports_streaming": true,
                "supports_tools": true,
                "supports_vision": false,
                "metadata": {},
                "created_at": null,
                "updated_at": null,
                "is_active": true,
                "tenant_id": "tenant-1",
                "supports_reasoning": false,
                "supports_extended_thinking": false,
                "reasoning_param_style": "none",
                "default_thinking_budget_tokens": null,
                "owner_user_id": null,
                "base_url": null,
                "upstream_model_id": null,
                "provider_key_id": null
            }]
        })
    }

    async fn tenant() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(200).set_body_json(da_page()))
            .mount(&server)
            .await;
        server
    }

    async fn tenant_with(rows: Vec<Value>) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "items": rows })))
            .mount(&server)
            .await;
        server
    }

    /// A DA row that names a device model and carries the provider fields a
    /// device row carries. It copies the Sonnet row, so its output limit is
    /// 64,000 where the device row's is 128,000. Each row needs its own `id`,
    /// since DA refuses a page that repeats one.
    fn da_plan_row(id: &str, model_id: &str, provider: &str) -> Value {
        let mut row = da_page()["items"][0].clone();
        row["id"] = json!(id);
        row["model_id"] = json!(model_id);
        row["provider"] = json!(provider);
        row["provider_parent"] = json!(provider);
        row
    }

    fn session(server: &MockServer) -> DaChatSession {
        DaChatSession::for_test(&server.uri(), BEARER).expect("valid loopback DA session")
    }

    async fn requests(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .expect("the mock records requests")
            .len()
    }

    fn model_ids(models: &[ModelEntity]) -> Vec<String> {
        models.iter().map(|model| model.model_id.clone()).collect()
    }

    fn wire_claude_rows(catalog: &ModelCatalogCache) {
        let source = PlanModelSource::new();
        source.set_claude(claude_rows());
        catalog.set_local_rows(Arc::new(source));
    }

    /// With no local source, a refresh and a held read hand back the same
    /// allocation the snapshot was published as.
    #[tokio::test]
    async fn without_a_local_source_the_catalogue_is_the_same_allocation() {
        let server = tenant().await;
        let da = session(&server);
        let catalog = ModelCatalogCache::default();

        let published = catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");
        let read = catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("held read answers");

        assert!(Arc::ptr_eq(&published, &read));
        assert_eq!(requests(&server).await, 1);
    }

    /// A source with nothing for this account changes nothing either.
    #[tokio::test]
    async fn a_local_source_with_no_rows_returns_the_same_allocation() {
        let server = tenant().await;
        let da = session(&server);
        let catalog = ModelCatalogCache::default();
        catalog.set_local_rows(Arc::new(PlanModelSource::new()));

        let published = catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");
        let read = catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("held read answers");

        assert!(Arc::ptr_eq(&published, &read));
        assert_eq!(model_ids(&read), vec!["sonnet"]);
    }

    /// Local rows follow the DA rows on a refresh and on a held read. The held
    /// snapshot keeps only what DA sent, so local rows are read fresh each time.
    #[tokio::test]
    async fn local_rows_follow_the_da_rows_and_are_not_held() {
        let server = tenant().await;
        let da = session(&server);
        let catalog = ModelCatalogCache::default();
        wire_claude_rows(&catalog);
        let expected: Vec<&str> = std::iter::once("sonnet").chain(PLAN_IDS).collect();

        let refreshed = catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");
        let read = catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("held read answers");

        assert_eq!(model_ids(&refreshed), expected);
        assert_eq!(model_ids(&read), expected);
        assert_eq!(requests(&server).await, 1);
        let held = catalog.held_for(ACCOUNT).expect("the DA snapshot is held");
        assert_eq!(model_ids(&held), vec!["sonnet"]);
    }

    /// Clearing drops the DA snapshot only. The next read fetches again and
    /// still appends the local rows, because the source stays wired.
    #[tokio::test]
    async fn clear_drops_the_da_snapshot_and_keeps_the_local_source() {
        let server = tenant().await;
        let da = session(&server);
        let catalog = ModelCatalogCache::default();
        wire_claude_rows(&catalog);
        catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");

        catalog.clear();
        let read = catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("the next read fetches");

        assert_eq!(requests(&server).await, 2);
        let expected: Vec<&str> = std::iter::once("sonnet").chain(PLAN_IDS).collect();
        assert_eq!(model_ids(&read), expected);
    }

    /// A DA row that names a device model is dropped when DA's rows are read,
    /// so no selection can run it, with or without a device source. With the
    /// device source wired, the device's row is the one a selection reads.
    #[tokio::test]
    async fn a_da_row_that_names_a_plan_model_is_dropped_and_never_selected() {
        let server = tenant_with(vec![
            da_page()["items"][0].clone(),
            da_plan_row(
                "00000000-0000-0000-0000-0000000000bb",
                "claude_code/claude-opus-5-5",
                "claude_code",
            ),
            da_plan_row(
                "00000000-0000-0000-0000-0000000000cc",
                "  chatgpt_plan/attacker-slug ",
                "chatgpt_plan",
            ),
        ])
        .await;
        let da = session(&server);
        let catalog = ModelCatalogCache::default();

        let without_device = catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");
        assert_eq!(model_ids(&without_device), vec!["sonnet"]);
        assert_eq!(
            select_model_by_id(&without_device, Some("claude_code/claude-opus-5-5")),
            Err(DaChatApiError::NoModel)
        );
        assert_eq!(
            select_model_by_id(&without_device, Some("chatgpt_plan/attacker-slug")),
            Err(DaChatApiError::NoModel)
        );

        wire_claude_rows(&catalog);
        let with_device = catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");
        let expected: Vec<&str> = std::iter::once("sonnet").chain(PLAN_IDS).collect();
        assert_eq!(model_ids(&with_device), expected);
        let selected = select_model_by_id(&with_device, Some("claude_code/claude-opus-5-5"))
            .expect("the device row is selectable");
        assert_eq!(selected.provider, "claude_code");
        assert_eq!(selected.max_output_tokens, CLAUDE_MAX_OUTPUT_TOKENS);
        assert_eq!(requests(&server).await, 2);
    }

    /// The source is wired once at startup, so a second one is ignored.
    #[tokio::test]
    async fn a_second_local_source_is_ignored() {
        let server = tenant().await;
        let da = session(&server);
        let catalog = ModelCatalogCache::default();
        wire_claude_rows(&catalog);
        catalog.set_local_rows(Arc::new(PlanModelSource::new()));

        let read = catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("refresh publishes");

        assert_eq!(model_ids(&read).len(), 1 + PLAN_IDS.len());
    }
}
