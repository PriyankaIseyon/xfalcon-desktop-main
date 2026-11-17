use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::Client;
use xfalcon_contracts::workspace::{McpServerSummary, WorkspaceStatus};
use xfalcon_contracts::CONTRACT_VERSION;

use crate::auth::{
    build_discovery_client, discover_auth, ensure_fresh_session, AuthConfig, AuthSessionStore,
    DiscoveryError,
};
use crate::clients::da::chat_api::{
    select_model, summarize_mcp_servers, DaChatApiError, DaChatSession,
};
use crate::clients::da::generated::{ModelEntity, WorkspaceBootstrap};
use crate::clients::da::mapping::build_hardened_client;
use crate::clients::da::model_catalog::ModelCatalogCache;
use crate::mcp::McpServerResolver;
use crate::plan_providers::PlanProviders;
use crate::security::account_cleanup::CurrentAccountScope;

/// The longest `status()` waits for the plan model refresh. A slow Claude probe or
/// ChatGPT request must not hold up a healthy tenant read. The rows it writes show
/// on the next read.
const PLAN_REFRESH_LIMIT: Duration = Duration::from_secs(4);
const SETUP_NOTE: &str ="xFalcon connects to your tenant’s bundled Data Analyst MCP automatically after sign-in. You can remove that connection or add custom MCP servers in Settings.";

pub struct WorkspaceService {
    sessions: Arc<AuthSessionStore>,
    accounts: Arc<CurrentAccountScope>,
    /// Published here, not consumed here: this service performs the one
    /// catalogue fetch the app makes, and chat reads what it publishes. See
    /// [`crate::clients::da::model_catalog`].
    catalog: Arc<ModelCatalogCache>,
    client: Client,
    /// The tenant chat client `status()` reads through. Pooled like any
    /// `reqwest::Client`, so the reads reuse one warm connection to the tenant.
    da_client: Client,
    /// Refreshes the device-local plan rows before the catalogue is read. Set
    /// once by the composition root through [`Self::set_plan_providers`].
    plan_providers: OnceLock<Arc<PlanProviders>>,
}

impl WorkspaceService {
    /// `pub(crate)`, like [`crate::chat::ChatService::new`]: the composition
    /// root is the only caller, and the shared catalogue it passes is a
    /// crate-internal type.
    pub(crate) fn new(
        sessions: Arc<AuthSessionStore>,
        accounts: Arc<CurrentAccountScope>,
        catalog: Arc<ModelCatalogCache>,
    ) -> Self {
        Self {
            sessions,
            accounts,
            catalog,
            client: build_discovery_client()
                .expect("hardened workspace discovery client must be constructible"),
            da_client: build_hardened_client()
                .expect("hardened DA workspace client must be constructible"),
            plan_providers: OnceLock::new(),
        }
    }

    /// Wires the device-local plan providers in, after construction, the same
    /// way [`crate::chat::ChatService::set_plan_providers`] does.
    pub(crate) fn set_plan_providers(&self, plan: Arc<PlanProviders>) {
        if self.plan_providers.set(plan).is_err() {
            tracing::warn!("workspace plan providers were already set; keeping the first");
        }
    }

    pub async fn status(&self) -> Result<WorkspaceStatus, WorkspaceServiceError> {
        let config = AuthConfig::from_env().map_err(WorkspaceServiceError::NotConfigured)?;
        // Renewed before the bearer is read. Note this is a mount-and-on-change
        // call (`useWorkspaceConnection`), not a foreground-refresh one, so it
        // is not reliably the first caller to notice an hour has passed —
        // `authenticated_fresh` on the chat paths renews too.
        let _ = ensure_fresh_session(&self.sessions).await;
        let token = self
            .sessions
            .access_token()
            .ok_or(WorkspaceServiceError::AuthRequired)?;
        // The catalogue is published under the account that read it. A session
        // whose account scope is not bound has nowhere to publish to, and is
        // the same state every account-bound command already refuses.
        let account = self
            .accounts
            .current()
            .map_err(|_| WorkspaceServiceError::Unavailable)?
            .ok_or(WorkspaceServiceError::AuthRequired)?;

        let da = DaChatSession::with_client(
            config.da_base_url.as_str(),
            token.as_str(),
            self.da_client.clone(),
        )
        .map_err(|_| WorkspaceServiceError::Unavailable)?;

        // The plan rows must exist before the catalogue is published and read. A
        // refresh that runs past its limit is dropped, which releases its locks, and
        // the rows already offered are kept.
        if let Some(plan) = self.plan_providers.get() {
            if !finished_within(PLAN_REFRESH_LIMIT, plan.refresh_models(&account)).await {
                tracing::warn!("the plan model refresh ran past its limit");
            }
        }
        let (mcp_resource_url, bootstrap, models) =
            read_tenant_state(&self.client, &config, &da, &self.catalog, account.as_str()).await?;
        let selection = select_model(&models).map_err(map_da_error)?;
        let models_available = models
            .iter()
            .filter(|model| {
                model.is_chat && model.is_enabled && model.is_active && model.supports_streaming
            })
            .count() as u32;

        let bundled_endpoint = McpServerResolver::resolve(&config);
        let mcp_servers = summarize_mcp_servers(&bootstrap, bundled_endpoint.endpoint())
            .into_iter()
            .map(|(id, name, transport, enabled)| {
                McpServerSummary::new(id, name, transport, enabled)
            })
            .collect();

        let tenant_host = config.da_base_url.host_str().unwrap_or("tenant").to_owned();

        Ok(WorkspaceStatus {
            version: CONTRACT_VERSION,
            tenant_host,
            mcp_resource_url,
            mcp_servers,
            models_available,
            default_model_id: Some(selection.model_id),
            default_provider: Some(selection.provider),
            // Reaching this line means the session is valid, the tenant's chat
            // data plane answered `workspace_bootstrap` and `models`, and a model
            // to send with is resolved. Nothing further gates composing a
            // message: a missing provider credential is reported by the run that
            // needs it, not by disabling the composer for every conversation.
            chat_ready: true,
            setup_note: SETUP_NOTE.to_owned(),
        })
    }
}

/// Waits at most `limit` for `work`. Work that runs past it is dropped, and `false`
/// is returned.
async fn finished_within<F: Future<Output = ()>>(limit: Duration, work: F) -> bool {
    tokio::time::timeout(limit, work).await.is_ok()
}

/// The three independent tenant reads behind `status()`, run concurrently.
/// Returns the protected resource URL, the workspace bootstrap, and the model
/// catalogue. The first failure in the order discovery, bootstrap, models is
/// the one reported, and it is returned as soon as every read before it has
/// finished.
async fn read_tenant_state(
    client: &Client,
    config: &AuthConfig,
    da: &DaChatSession,
    catalog: &ModelCatalogCache,
    account: &str,
) -> Result<(String, WorkspaceBootstrap, Arc<Vec<ModelEntity>>), WorkspaceServiceError> {
    // The catalogue refresh is the app's one catalogue fetch. Every chat path
    // reads the snapshot it publishes rather than repeating the request.
    let discovered = discover_auth(client, config);
    let bootstrap = da.workspace_bootstrap();
    let models = catalog.refresh(da, account);
    tokio::pin!(discovered);
    tokio::pin!(bootstrap);
    tokio::pin!(models);
    let mut early_bootstrap = None;
    let mut early_models = None;

    let discovered = loop {
        tokio::select! {
            result = &mut discovered => break result,
            result = &mut bootstrap, if early_bootstrap.is_none() => early_bootstrap = Some(result),
            result = &mut models, if early_models.is_none() => early_models = Some(result),
        }
    };
    let discovered = discovered.map_err(WorkspaceServiceError::from)?;

    let bootstrap = loop {
        if let Some(result) = early_bootstrap.take() {
            break result;
        }
        tokio::select! {
            result = &mut bootstrap => break result,
            result = &mut models, if early_models.is_none() => early_models = Some(result),
        }
    };
    let bootstrap = bootstrap.map_err(map_da_error)?;

    // The models read may have already published the account's own model
    // catalog when an earlier stage fails. That is harmless: the catalog is
    // keyed by the same account and is re-read on demand.
    let models = match early_models {
        Some(result) => result,
        None => models.await,
    };
    let models = models.map_err(map_da_error)?;
    Ok((discovered.resource, bootstrap, models))
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceServiceError {
    #[error("sign in required")]
    AuthRequired,
    #[error("{0}")]
    NotConfigured(#[from] crate::auth::AuthConfigError),
    #[error("could not reach the tenant authorization service")]
    DiscoveryFailed,
    #[error("tenant workspace is unavailable")]
    Unavailable,
    #[error("tenant session expired; sign in again")]
    Unauthorized,
    #[error("tenant workspace response was invalid")]
    InvalidResponse,
}

fn map_da_error(error: DaChatApiError) -> WorkspaceServiceError {
    match error {
        DaChatApiError::Unauthorized => WorkspaceServiceError::Unauthorized,
        DaChatApiError::InvalidResponse => WorkspaceServiceError::InvalidResponse,
        DaChatApiError::NoModel
        | DaChatApiError::CustomModelUnavailable
        | DaChatApiError::CustomModelCapacityUnsupported => WorkspaceServiceError::Unavailable,
        DaChatApiError::NotFound
        | DaChatApiError::RequestFailed
        | DaChatApiError::Conflict
        | DaChatApiError::ContextConflict
        | DaChatApiError::ContextSnapshotStale
        | DaChatApiError::IdempotencyConflict
        | DaChatApiError::PayloadTooLarge
        | DaChatApiError::StorageQuotaExceeded
        | DaChatApiError::StorageUnavailable
        | DaChatApiError::PayloadNotReady
        | DaChatApiError::PayloadIntegrityFailed
        | DaChatApiError::Validation
        | DaChatApiError::RateLimited
        | DaChatApiError::Timeout
        | DaChatApiError::UnsupportedCompactionFormat => WorkspaceServiceError::Unavailable,
    }
}

impl From<DiscoveryError> for WorkspaceServiceError {
    fn from(_: DiscoveryError) -> Self {
        Self::DiscoveryFailed
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use reqwest::Url;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn work_that_never_finishes_is_dropped_at_the_limit() {
        let started = Instant::now();

        let finished =
            finished_within(Duration::from_millis(20), std::future::pending::<()>()).await;

        assert!(!finished);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn work_that_finishes_within_the_limit_is_kept() {
        let finished = finished_within(Duration::from_secs(5), async {}).await;

        assert!(finished);
    }

    #[tokio::test]
    async fn the_workspace_discovery_client_does_not_follow_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/target", server.uri())),
            )
            .mount(&server)
            .await;
        // Expected to be hit zero times: a followed redirect would fail the mock.
        Mock::given(method("GET"))
            .and(path("/target"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let service = WorkspaceService::new(
            Arc::new(AuthSessionStore::new()),
            Arc::new(CurrentAccountScope::default()),
            Arc::new(ModelCatalogCache::default()),
        );

        let response = service
            .client
            .get(format!("{}/redirect", server.uri()))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 302);
    }

    const DELAY: Duration = Duration::from_millis(300);
    const HAIKU: &str = "~anthropic/claude-haiku-latest";

    /// One tenant DA server (protected resource, bootstrap, models) and one
    /// issuer server (OpenID configuration). The caller's `prm` response names
    /// the authorization server it wants.
    async fn mount_tenant(
        da: &MockServer,
        issuer: &MockServer,
        prm: ResponseTemplate,
        bootstrap: ResponseTemplate,
        models: ResponseTemplate,
    ) {
        let issuer_uri = issuer.uri();
        let openid = ResponseTemplate::new(200)
            .set_delay(DELAY)
            .set_body_json(serde_json::json!({
                "issuer": issuer_uri,
                "authorization_endpoint": format!("{issuer_uri}/login/oauth/authorize"),
                "token_endpoint": format!("{issuer_uri}/api/login/oauth/access_token"),
                "jwks_uri": format!("{issuer_uri}/keys/current")
            }));
        mount(issuer, "/.well-known/openid-configuration", openid).await;
        mount(da, "/.well-known/oauth-protected-resource", prm).await;
        mount(da, "/api/v1/chat/bootstrap/workspace", bootstrap).await;
        mount(da, "/api/v1/chat/models", models).await;
    }

    async fn mount(server: &MockServer, route: &str, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .mount(server)
            .await;
    }

    fn protected_resource(issuer: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_delay(DELAY)
            .set_body_json(serde_json::json!({
                "resource": "https://tenant.example/mcp",
                "authorization_servers": [issuer]
            }))
    }

    fn bootstrap_ok() -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_delay(DELAY)
            .set_body_json(serde_json::json!({
                "mcp_servers": [{
                    "id": "additional",
                    "name": "Acme CRM",
                    "transport": "http",
                    "url": "https://mcp.acme.example/mcp",
                    "is_enabled": true
                }]
            }))
    }

    /// Every field of the pinned `ModelEntity` row, as the models route sends it.
    fn models_ok() -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_delay(DELAY)
            .set_body_json(serde_json::json!({
                "items": [{
                    "id": "00000000-0000-0000-0000-0000000000aa",
                    "model_id": HAIKU,
                    "provider": "openrouter",
                    "provider_parent": "openrouter",
                    "provider_sub": null,
                    "display_name": "Haiku",
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
            }))
    }

    fn tenant_config(da: &MockServer, issuer: &MockServer) -> AuthConfig {
        AuthConfig {
            da_base_url: Url::parse(&da.uri()).expect("da url"),
            casdoor_issuer: Url::parse(&issuer.uri()).expect("issuer"),
            client_id: "desktop-client".to_owned(),
            organization: "tenant".to_owned(),
            loopback_port: 19847,
            mcp_client_id: None,
            mcp_client_secret: None,
        }
    }

    #[tokio::test]
    async fn the_three_tenant_reads_return_their_values_without_serial_latency() {
        let da = MockServer::start().await;
        let issuer = MockServer::start().await;
        mount_tenant(
            &da,
            &issuer,
            protected_resource(&issuer.uri()),
            bootstrap_ok(),
            models_ok(),
        )
        .await;

        let client = build_discovery_client().expect("client");
        let session = DaChatSession::for_test(&da.uri(), "bearer").expect("session");
        let catalog = ModelCatalogCache::default();
        let config = tenant_config(&da, &issuer);

        let started = Instant::now();
        let (resource, bootstrap, models) =
            read_tenant_state(&client, &config, &session, &catalog, "tenant-1:user-1")
                .await
                .expect("tenant reads");
        let elapsed = started.elapsed();
        eprintln!("read_tenant_state with 300 ms per read took {elapsed:?}");

        assert_eq!(resource, "https://tenant.example/mcp");
        assert_eq!(bootstrap.mcp_servers.as_ref().map(Vec::len), Some(1));
        assert_eq!(models.first().map(|m| m.model_id.as_str()), Some(HAIKU));
        assert!(
            elapsed < Duration::from_millis(700),
            "tenant reads ran serially: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn a_discovery_failure_is_reported_before_a_bootstrap_failure() {
        let da = MockServer::start().await;
        let issuer = MockServer::start().await;
        mount_tenant(
            &da,
            &issuer,
            ResponseTemplate::new(500),
            ResponseTemplate::new(401),
            ResponseTemplate::new(500),
        )
        .await;

        let client = build_discovery_client().expect("client");
        let session = DaChatSession::for_test(&da.uri(), "bearer").expect("session");
        let config = tenant_config(&da, &issuer);

        let result = read_tenant_state(
            &client,
            &config,
            &session,
            &ModelCatalogCache::default(),
            "tenant-1:user-1",
        )
        .await;
        assert!(matches!(
            result,
            Err(WorkspaceServiceError::DiscoveryFailed)
        ));
    }

    #[tokio::test]
    async fn a_bootstrap_failure_is_reported_before_a_model_list_failure() {
        let da = MockServer::start().await;
        let issuer = MockServer::start().await;
        mount_tenant(
            &da,
            &issuer,
            protected_resource(&issuer.uri()),
            ResponseTemplate::new(401),
            ResponseTemplate::new(500),
        )
        .await;

        let client = build_discovery_client().expect("client");
        let session = DaChatSession::for_test(&da.uri(), "bearer").expect("session");
        let config = tenant_config(&da, &issuer);

        let result = read_tenant_state(
            &client,
            &config,
            &session,
            &ModelCatalogCache::default(),
            "tenant-1:user-1",
        )
        .await;
        assert!(matches!(result, Err(WorkspaceServiceError::Unauthorized)));
    }

    #[tokio::test]
    async fn a_discovery_failure_does_not_wait_for_slow_bootstrap_and_models() {
        let da = MockServer::start().await;
        let issuer = MockServer::start().await;
        let slow = ResponseTemplate::new(500).set_delay(Duration::from_secs(3));
        mount_tenant(&da, &issuer, ResponseTemplate::new(500), slow.clone(), slow).await;

        let client = build_discovery_client().expect("client");
        let session = DaChatSession::for_test(&da.uri(), "bearer").expect("session");
        let config = tenant_config(&da, &issuer);

        let started = Instant::now();
        let result = read_tenant_state(
            &client,
            &config,
            &session,
            &ModelCatalogCache::default(),
            "tenant-1:user-1",
        )
        .await;
        let elapsed = started.elapsed();

        assert!(matches!(
            result,
            Err(WorkspaceServiceError::DiscoveryFailed)
        ));
        assert!(
            elapsed < Duration::from_millis(1500),
            "discovery failure waited for the slow reads: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn a_bootstrap_failure_does_not_wait_for_slow_models() {
        let da = MockServer::start().await;
        let issuer = MockServer::start().await;
        let slow_models = ResponseTemplate::new(500).set_delay(Duration::from_secs(3));
        mount_tenant(
            &da,
            &issuer,
            protected_resource(&issuer.uri()),
            ResponseTemplate::new(401),
            slow_models,
        )
        .await;

        let client = build_discovery_client().expect("client");
        let session = DaChatSession::for_test(&da.uri(), "bearer").expect("session");
        let config = tenant_config(&da, &issuer);

        let started = Instant::now();
        let result = read_tenant_state(
            &client,
            &config,
            &session,
            &ModelCatalogCache::default(),
            "tenant-1:user-1",
        )
        .await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(WorkspaceServiceError::Unauthorized)));
        assert!(
            elapsed < Duration::from_millis(1500),
            "bootstrap failure waited for the slow models read: {elapsed:?}"
        );
    }
}
