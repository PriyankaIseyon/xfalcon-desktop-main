use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::chat::{
    ChatConversationView, ChatErrorCode, ChatMcpSelectionView, ChatModelCatalogView,
    ChatOperationError, ChatSubmitResult, ChatWorkspaceBootstrap,
};
use xfalcon_contracts::credentials::{ModelCapabilitiesView, ModelView, ProviderId};
use xfalcon_contracts::custom_models::{
    CustomModelCreateInput, CustomModelDiscoveryInput, CustomModelDiscoveryResult,
    CustomModelSampling, CustomModelUpdateInput, CustomModelView, ExistingCustomModel,
};
use xfalcon_contracts::inference::{FinishReason, InferenceErrorCode, InferenceEvent};
use xfalcon_contracts::mcp::McpServerId;
use xfalcon_contracts::tools::{InvocationId, ToolId};
use xfalcon_contracts::{ConversationId, RunId, RunState};
use zeroize::Zeroizing;

use crate::agent::context::{ContextMessage, ContextRole};
use crate::agent::turn_driver::McpContextScope;
use crate::agent::{
    build_system_prompt, build_tool_session, recovery::is_in_flight_run_state, run::JournalEntry,
    LocalTurnDriver, LocalTurnDriverConfig, PromptProject, PromptSkill, Run, RunBudgets,
    RunEngineError, RunEngineOutcome, SystemPromptInputs, ToolLoopError, ToolSessionRequest,
};
use crate::app::events::{emit_chat_inference_event, emit_chat_projection_event};
use crate::approvals::ApprovalsService;
use crate::auth::{canonical_account_id, ensure_fresh_session, AuthConfig, AuthSessionStore};
use crate::chat::cold_start_recovery::{interrupted_run_patch_from_journal, RecoveredRuns};
use crate::chat::da_repository::DaChatRepository;
use crate::chat::domain::{ChatProjectionEvent, ChatSnapshot, RunRecord, MAX_MESSAGE_TEXT_BYTES};
use crate::chat::history::context_from_snapshot;
use crate::chat::invocation_emit::EmittingInvocationJournal;
use crate::chat::local_projection::{
    EncryptedChatProjection, CHAT_PROJECTION_DB_FILE, LOCAL_PROJECTIONS_DIR,
};
use crate::chat::mcp_selection::{BindingError, McpConversationBinding};
use crate::chat::persisting_run_journal::PersistingEmittingRunJournal;
use crate::chat::run_journal_store::RunJournalStore;
use crate::chat::sync::ChatSyncService;
use crate::chat::title::{derive_title, generate_model_title, TitleSource};
use crate::chat::tool_run::{ChatToolRun, ParkedToolRuns, ToolRun, WatchdogLimits};
use crate::chat::wire::{
    conversation_view_from, is_active_run, latest_owned_run_state, projection_run_state_changed,
    projection_snapshot_replaced, startup_recovery_from_workspace, without_foreign_active_run,
    workspace_bootstrap_from, WorkspaceRecoverySnapshot, STARTUP_RECOVERY_CONVERSATION_LIMIT,
};
use crate::clients::da::chat_api::{
    plan_model_provider, select_model, select_model_by_id, select_offered_model, supported_models,
    ContextSnapshot, CustomModelApiError, CustomModelRoute, DaChatApiError, DaChatSession,
    ModelExecutionMode, ModelSelection, SONNET_MODEL_ID,
};
use crate::clients::da::generated::RunFinalize;
use crate::clients::da::mapping::build_hardened_client;
use crate::clients::da::model_catalog::ModelCatalogCache;
use crate::clients::da::provider_keys::DaCredentialRepository;
use crate::credentials::service::{
    CredentialRepository, CredentialServiceError, ResolvedCredential, MAX_PROVIDER_KEY_BYTES,
};
use crate::credentials::CredentialRuntime;
use crate::mcp::commands::McpServerRuntime;
use crate::mcp::dispatch::WindowReauthNotifier;
use crate::mcp::runtime::McpRuntimeSet;
use crate::persistence::SecretStore;
use crate::plan_providers::PlanProviders;
use crate::projects::ProjectsService;
use crate::providers::gateway;
use crate::providers::{
    adapter_for, custom_openai_adapter_with_sampling, verify_custom_model_connection,
    CustomModelConnectionError, ProviderAdapter, ProviderError, ProviderToolSchema,
};
use crate::security::account_cleanup::{
    account_directory_name, CurrentAccountScope, ValidatedAccountId,
};
use crate::skills::store::SkillStore;

use super::custom_model_settings::CustomModelSettingsStore;
use super::model_preference::ModelPreferenceStore;

#[cfg(all(test, unix))]
#[path = "tests/deleted_projects.rs"]
mod deleted_project_tests;

#[cfg(test)]
#[path = "tests/foreign_runs.rs"]
mod foreign_run_tests;

/// Longest prompt the composer will submit.
///
/// Defined as [`MAX_MESSAGE_TEXT_BYTES`], never as its own number: the value a
/// prompt is checked against here has to be the value the domain will accept
/// when the message is constructed, or a prompt clears this check and fails
/// several network round trips later. `pub(crate)` so tests assert against the
/// bound itself rather than restating it.
pub(crate) const MAX_PROMPT_LEN: usize = MAX_MESSAGE_TEXT_BYTES;
const MAX_IN_FLIGHT_RUNS: usize = 8;
/// One total bound covers catalog reads and all exact-key resolutions in the modal.
const CUSTOM_MODEL_DUPLICATE_CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const RUN_WALL_CLOCK: Duration = super::tool_run::RUN_WALL_CLOCK;
/// Shown when the account has no openable key for the model's provider. Names
/// the one action that resolves it — a device key sealed under a rotated device
/// secret reads as absent, and "re-enter" is what fixes that case.
const MISSING_INFERENCE_KEY: &str =
    "No usable key for this model's provider. Add or re-enter one in Settings → Providers.";

/// What a submission does to the conversation before its run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnRevision {
    /// A new turn at the end of the conversation.
    None,
    /// The turn at `cutoff` and everything after it is removed, and the prompt
    /// takes its place. The reader edited their own message.
    Replace { cutoff: Uuid },
    /// Everything from `cutoff` on is removed and the prompt already stored
    /// above it runs again. No new user row is written.
    Rerun { cutoff: Uuid },
}

impl TurnRevision {
    pub(crate) fn writes_user_message(&self) -> bool {
        matches!(self, Self::None | Self::Replace { .. })
    }

    pub(crate) fn cutoff(&self) -> Option<Uuid> {
        match self {
            Self::None => None,
            Self::Replace { cutoff } | Self::Rerun { cutoff } => Some(*cutoff),
        }
    }
}

struct LocalRunRequest {
    app: tauri::AppHandle,
    config: AuthConfig,
    bearer: Zeroizing<String>,
    da_client: reqwest::Client,
    tenant_id: String,
    account: ValidatedAccountId,
    conversation_id: ConversationId,
    da_run_id: Uuid,
    run_id_string: String,
    model_id: String,
    custom: Option<CustomModelRoute>,
    context_length: i64,
    max_output_tokens: i64,
    execution_mode: ModelExecutionMode,
    provider: xfalcon_contracts::credentials::ProviderId,
    credential: ResolvedCredential,
    cancel: CancellationToken,
    /// Whether the message this run answers was sent with files. Its run cannot
    /// go ahead without them; any other run can, when the attachment list is
    /// briefly unreadable (spec: chat-attachments).
    attachments_required: bool,
    /// Frozen per-conversation server set for a selected chat. None keeps the
    /// existing lazy all-server path and cache behavior.
    mcp_scope: Option<McpContextScope>,
    auth_generation: u64,
}

pub(super) struct CapturedChatAuth<'a> {
    pub(super) config: &'a AuthConfig,
    pub(super) token: &'a str,
    pub(super) account: &'a ValidatedAccountId,
    pub(super) generation: u64,
}

/// What is stored as the text of a message sent with files and no words. It
/// names nothing, so a conversation is never titled from it.
const ATTACHMENT_ONLY_BODY: &str = "Attached files.";

/// The most attachment names one title basis lists.
const MAX_TITLED_ATTACHMENTS: usize = 3;

struct TitleUpgradeRequest {
    app: Option<tauri::AppHandle>,
    config: AuthConfig,
    bearer: Zeroizing<String>,
    da_client: reqwest::Client,
    tenant_id: String,
    account: ValidatedAccountId,
    conversation_id: ConversationId,
    model: ModelSelection,
    provider: xfalcon_contracts::credentials::ProviderId,
    /// The credential the run itself resolved on the send path.
    credential: ResolvedCredential,
    opening_message: String,
}

#[derive(Clone)]
struct ActiveLocalRun {
    account: ValidatedAccountId,
    conversation_id: ConversationId,
    cancel: CancellationToken,
    teardown_failed: bool,
}

type RunCancelRegistry = Arc<AsyncMutex<HashMap<String, ActiveLocalRun>>>;

fn has_active_run_for_conversation(
    runs: &HashMap<String, ActiveLocalRun>,
    conversation_id: &ConversationId,
) -> bool {
    runs.values()
        .any(|run| run.conversation_id == *conversation_id)
}

/// Why a chat operation failed.
///
/// Every `Display` string here is what the reader sees. It reaches the surface
/// through [`ChatOperationError`], which carries it verbatim, and through
/// `ChatSubmitResult::failed` on the send path. Lowercase engineer tokens used
/// to leak this way — "chat is unavailable" was the whole explanation a reader
/// got for a workspace that was mid-deploy — so a new variant earns a sentence
/// and, when there is one, the next action.
#[derive(Debug, thiserror::Error)]
pub enum ChatServiceError {
    #[error("Sign in to use chat.")]
    AuthRequired,
    #[error("Auth is not configured on this device.")]
    NotConfigured,
    #[error("Session expired. Sign in again.")]
    SessionExpired,
    /// The workspace answered outside its contract. Retrying reaches the same
    /// answer, so this sentence does not invite one.
    #[error("The workspace could not complete that. Reload xFalcon, then try again.")]
    Unavailable,
    /// The workspace could not be reached, answered 5xx, timed out, or rate
    /// limited — the one failure shape that passes on its own.
    #[error("The workspace is not responding right now. Try again in a moment.")]
    TemporarilyUnavailable,
    #[error("No account is active on this device.")]
    NoActiveAccount,
    #[error("A message is already sending. Wait for it to finish.")]
    SendInFlight,
    #[error("Too many active runs. Cancel one and try again.")]
    TooManyRuns,
    /// The server authoritatively answered that the conversation does not
    /// exist for this identity. The surface self-heals on
    /// [`ChatErrorCode::NotFound`] rather than on this text, so this is prose
    /// like every other variant.
    #[error("That conversation no longer exists.")]
    NotFound,
    #[error("{0}")]
    Message(String),
}

impl ChatServiceError {
    /// The decisions the prose cannot carry: self-healing a dropped row, and
    /// telling a failure that passes on its own from one that does not.
    pub fn code(&self) -> ChatErrorCode {
        match self {
            Self::AuthRequired => ChatErrorCode::AuthRequired,
            Self::NotConfigured => ChatErrorCode::NotConfigured,
            Self::SessionExpired => ChatErrorCode::SessionExpired,
            Self::Unavailable => ChatErrorCode::ServiceUnavailable,
            Self::TemporarilyUnavailable => ChatErrorCode::TemporarilyUnavailable,
            Self::NoActiveAccount => ChatErrorCode::NoActiveAccount,
            Self::SendInFlight => ChatErrorCode::SendInFlight,
            Self::TooManyRuns => ChatErrorCode::TooManyRuns,
            Self::NotFound => ChatErrorCode::NotFound,
            Self::Message(_) => ChatErrorCode::Failed,
        }
    }
}

/// The wire form every fallible chat command rejects with.
///
/// One conversion in one place, so no command can reintroduce the
/// `map_err(|error| error.to_string())` that threw the code away and left the
/// frontend matching on substrings.
impl From<ChatServiceError> for ChatOperationError {
    fn from(error: ChatServiceError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

/// Tenant chat — sync, projections, and message submit.
///
/// Every run executes here, on this device ([`run_local`], or
/// [`crate::agent::run_with_tool_loop`] when the run is tool-capable), using the
/// account's DA-resolved credential for the chosen model's provider. DA's
/// `/api/v1/chat/*` is a data plane — it stores conversations, runs, and tool
/// calls, and calls no model provider — so a run this device cannot execute is a
/// run nothing executes. [`Self::message_send`] therefore resolves the credential
/// before it writes anything, and reports the missing key instead of queueing
/// work that could never complete.
///
/// [`crate::agent::RunEngine::poll_da_run`] remains in the engine for the inverse
/// case — observing a run some other client is executing — and has no caller
/// here: with no tenant-side executor there is nothing to observe.
pub struct ChatService {
    sessions: Arc<AuthSessionStore>,
    accounts: Arc<CurrentAccountScope>,
    app_local_data: PathBuf,
    secrets: Arc<dyn SecretStore>,
    credentials: Arc<CredentialRuntime>,
    /// The tenant catalogue the workspace refresh publishes. Read, never
    /// fetched on a send path: see [`crate::clients::da::model_catalog`].
    model_catalog: Arc<ModelCatalogCache>,
    /// Owns the device-local conversation binding lifecycle and composes each
    /// run's tool session. DA remains unaware of this project state.
    projects: Arc<ProjectsService>,
    approvals: Arc<ApprovalsService>,
    run_cancels: RunCancelRegistry,
    /// Tool-capable runs paused awaiting a human decision on one of their
    /// tool calls. See [`crate::chat::tool_run`] for why the whole run has to
    /// be held rather than rebuilt on resume.
    parked_runs: Arc<ParkedToolRuns>,
    /// The MCP servers a run may draw tools from — the bundled tenant server
    /// plus whatever the account has registered.
    ///
    /// Sessions are held per server for the app's lifetime, not per run: an MCP
    /// session is a real connection with a handshake and a server-side session
    /// id (see [`crate::mcp::runtime`]). This is a distinct instance from the
    /// one the `mcp_server_*` command surface manages: both read the same DA
    /// rows and the same device credential store, but the Settings surface
    /// dials on demand while a run reuses a warm session, and neither should be
    /// able to invalidate the other's connections.
    mcp: Arc<McpServerRuntime>,
    /// The one HTTP client for this service's tenant chat calls. A
    /// `reqwest::Client` keeps its own connection pool, so every session built
    /// from a clone of it reuses one warm connection to the tenant.
    da_client: reqwest::Client,
    send_in_flight: Arc<AsyncMutex<bool>>,
    title_writes: Arc<AsyncMutex<()>>,
    model_preference_writes: Arc<AsyncMutex<()>>,
    #[cfg(test)]
    auth_config_override: Mutex<Option<AuthConfig>>,
    #[cfg(test)]
    custom_model_connection_failure: Mutex<Option<CustomModelConnectionError>>,
    deleted_project_cleanup: AsyncMutex<()>,
    active_conversations: Arc<Mutex<HashMap<String, ConversationId>>>,
    recovered_runs: Mutex<RecoveredRuns>,
    /// Standing tool-approval grants + the product's auto-approve default —
    /// see `crate::tools::standing`. Threaded into every run's tool session.
    standing_approvals: Arc<crate::tools::standing::StandingToolApprovals>,
    /// Where each run's tool decisions are audited. Set once by the
    /// composition root through [`Self::set_audit`]; empty means unaudited.
    audit: OnceLock<crate::audit::AuditPublisher>,
    /// The device-local plan providers (Claude runtime, ChatGPT plan). Set once
    /// by the composition root through [`Self::set_plan_providers`]; empty means
    /// a plan model is refused before any send.
    plan_providers: OnceLock<Arc<PlanProviders>>,
}

impl ChatService {
    /// `pub(crate)`: the composition root is the only caller, and the shared
    /// catalogue it passes is a crate-internal type.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        sessions: Arc<AuthSessionStore>,
        accounts: Arc<CurrentAccountScope>,
        app_local_data: PathBuf,
        secrets: Arc<dyn SecretStore>,
        credentials: Arc<CredentialRuntime>,
        model_catalog: Arc<ModelCatalogCache>,
        projects: Arc<ProjectsService>,
        approvals: Arc<ApprovalsService>,
    ) -> Self {
        let mcp = Arc::new(McpServerRuntime::with_vault(
            sessions.clone(),
            app_local_data.clone(),
            credentials.vault(),
        ));
        let standing_approvals = Arc::new(crate::tools::standing::StandingToolApprovals::load(
            app_local_data.clone(),
        ));
        Self {
            sessions,
            accounts,
            app_local_data,
            secrets,
            credentials,
            model_catalog,
            projects,
            approvals,
            run_cancels: Arc::new(AsyncMutex::new(HashMap::new())),
            parked_runs: Arc::new(ParkedToolRuns::default()),
            mcp,
            da_client: build_hardened_client()
                .expect("hardened DA chat client must be constructible"),
            send_in_flight: Arc::new(AsyncMutex::new(false)),
            title_writes: Arc::new(AsyncMutex::new(())),
            model_preference_writes: Arc::new(AsyncMutex::new(())),
            #[cfg(test)]
            auth_config_override: Mutex::new(None),
            #[cfg(test)]
            custom_model_connection_failure: Mutex::new(None),
            deleted_project_cleanup: AsyncMutex::new(()),
            active_conversations: Arc::new(Mutex::new(HashMap::new())),
            recovered_runs: Mutex::new(RecoveredRuns::default()),
            standing_approvals,
            audit: OnceLock::new(),
            plan_providers: OnceLock::new(),
        }
    }

    /// Wires the local audit log in. The composition root builds this service
    /// inside `AppRuntimeState::new`, so it reaches the service only after
    /// construction, through `AppRuntimeState::chat`.
    pub(crate) fn set_audit(&self, audit: crate::audit::AuditPublisher) {
        if self.audit.set(audit).is_err() {
            tracing::warn!("chat audit publisher was already set; keeping the first");
        }
    }

    /// Wires the device-local plan providers in, after construction, for the
    /// same reason as [`Self::set_audit`].
    pub(crate) fn set_plan_providers(&self, plan: Arc<PlanProviders>) {
        if self.plan_providers.set(plan).is_err() {
            tracing::warn!("plan providers were already set; keeping the first");
        }
    }

    pub(crate) fn plan_providers(&self) -> Option<Arc<PlanProviders>> {
        self.plan_providers.get().cloned()
    }

    pub fn reset_active_conversation(&self) {
        if let Ok((_, _, _, account)) = self.authenticated() {
            self.active_conversations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(account.as_str());
        }
    }

    pub async fn has_active_work(&self) -> bool {
        *self.send_in_flight.lock().await || !self.run_cancels.lock().await.is_empty()
    }

    pub(crate) fn app_local_data(&self) -> &Path {
        &self.app_local_data
    }

    pub async fn model_catalog(
        &self,
        conversation_id: Option<ConversationId>,
    ) -> Result<ChatModelCatalogView, ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let models = self
            .read_models(&da, account.as_str())
            .await
            .map_err(map_da_error)?;
        let (selected, selection_locked) = if let Some(conversation_id) = conversation_id {
            self.ensure_conversation_access(&conversation_id).await?;
            let conversation = da
                .get_conversation(parse_conversation_uuid(&conversation_id)?)
                .await
                .map_err(map_da_error)?;
            if conversation.tenant_id != tenant_id {
                return Err(ChatServiceError::Message(
                    "The conversation does not belong to this account.".to_owned(),
                ));
            }
            let stored = conversation
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(SONNET_MODEL_ID);
            (
                select_model_by_id(&models, Some(stored)).map_err(map_da_error)?,
                true,
            )
        } else {
            (self.preferred_model(&account, &models).await?, false)
        };

        let view = model_catalog_view(&models, &selected, selection_locked);
        self.ensure_model_catalog_account(&account)?;
        Ok(view)
    }

    pub async fn custom_model_list(&self) -> Result<Vec<CustomModelView>, ChatServiceError> {
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let _read = self.model_preference_writes.lock().await;
        self.ensure_model_catalog_account(&account)?;
        let da = self.da_session(&config, &token)?;
        let models = da
            .list_custom_models_for_management()
            .await
            .map_err(map_custom_model_error)?;
        self.ensure_model_catalog_account(&account)?;
        custom_model_views(
            &models,
            account.as_str(),
            &CustomModelSettingsStore::new(&self.app_local_data, &account),
        )
    }

    pub async fn custom_model_discover(
        &self,
        input: CustomModelDiscoveryInput,
    ) -> Result<CustomModelDiscoveryResult, ChatServiceError> {
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let base_url =
            crate::providers::normalize_base_url_input(&input.base_url).map_err(|_| {
                ChatServiceError::Message(
                    "Enter a valid API base URL, such as http://localhost:8000/v1.".to_owned(),
                )
            })?;
        let secret = Zeroizing::new(input.secret);
        validate_custom_model_secret(&secret)?;
        let existing_models = self
            .existing_custom_models_for_connection(
                &config,
                &token,
                &account,
                CustomModelConnectionCandidate {
                    base_url: &base_url,
                    secret: &secret,
                    upstream_model_id: None,
                    exclude_id: None,
                },
            )
            .await?;
        let mut result = crate::providers::discover_models(base_url, secret.to_string())
            .await
            .map_err(|error| ChatServiceError::Message(error.to_string()))?;
        self.ensure_model_catalog_account(&account)?;
        result.existing_models = existing_models;
        Ok(result)
    }

    async fn existing_custom_models_for_connection(
        &self,
        config: &AuthConfig,
        token: &str,
        account: &ValidatedAccountId,
        candidate: CustomModelConnectionCandidate<'_>,
    ) -> Result<Vec<ExistingCustomModel>, ChatServiceError> {
        let check = async {
            self.ensure_model_catalog_account(account)?;
            let da = self.da_session(config, token)?;
            let models = da
                .list_custom_models_for_management()
                .await
                .map_err(map_custom_model_error)?;
            self.ensure_model_catalog_account(account)?;
            let snapshot = self
                .sessions
                .authenticated_snapshot()
                .ok_or(ChatServiceError::AuthRequired)?;
            if canonical_account_id(&snapshot.identity)
                .map_err(|_| ChatServiceError::AuthRequired)?
                != account.as_str()
            {
                return Err(ChatServiceError::AuthRequired);
            }
            #[cfg(test)]
            let repository = if config.da_base_url.scheme() == "http"
                && config
                    .da_base_url
                    .host_str()
                    .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                    .is_some_and(|address| address.is_loopback())
            {
                DaCredentialRepository::for_test(
                    config.da_base_url.as_str(),
                    token,
                    &snapshot.identity,
                    CUSTOM_MODEL_DUPLICATE_CHECK_TIMEOUT,
                )
            } else {
                DaCredentialRepository::new(
                    config.da_base_url.as_str(),
                    token,
                    &snapshot.identity,
                    self.da_client.clone(),
                )
            };
            #[cfg(not(test))]
            let repository = DaCredentialRepository::new(
                config.da_base_url.as_str(),
                token,
                &snapshot.identity,
                self.da_client.clone(),
            );
            let repository = repository.map_err(|_| ChatServiceError::TemporarilyUnavailable)?;
            let matches = existing_custom_model_connections(
                &models,
                account.as_str(),
                candidate,
                &repository,
            )
            .await?;
            self.ensure_model_catalog_account(account)?;
            Ok(matches)
        };
        // The modal must stay responsive even when several stored keys need resolving.
        tokio::time::timeout(CUSTOM_MODEL_DUPLICATE_CHECK_TIMEOUT, check)
            .await
            .map_err(|_| {
                ChatServiceError::Message(
                    "Could not check for existing private models. Try again.".to_owned(),
                )
            })?
            .map_err(custom_model_preflight_error)
    }

    pub async fn custom_model_create(
        &self,
        mut input: CustomModelCreateInput,
    ) -> Result<CustomModelView, ChatServiceError> {
        input.base_url =
            crate::providers::normalize_base_url_input(&input.base_url).map_err(|_| {
                ChatServiceError::Message(
                    "Enter a valid API base URL, such as http://localhost:8000/v1.".to_owned(),
                )
            })?;
        validate_custom_model_secret(&input.secret)?;
        validate_custom_model_configuration(
            &input.base_url,
            &input.upstream_model_id,
            &input.display_name,
            input.context_length,
            input.max_output_tokens,
            &input.sampling,
        )?;
        let sampling = input.sampling.clone();
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let _write = self.model_preference_writes.lock().await;
        self.ensure_model_catalog_account(&account)?;
        let duplicates = self
            .existing_custom_models_for_connection(
                &config,
                &token,
                &account,
                CustomModelConnectionCandidate {
                    base_url: &input.base_url,
                    secret: &input.secret,
                    upstream_model_id: Some(&input.upstream_model_id),
                    exclude_id: None,
                },
            )
            .await?;
        if let Some(duplicate) = duplicates.first() {
            return Err(custom_model_duplicate_error(duplicate, &input.secret));
        }
        self.verify_custom_model_connection(
            &input.base_url,
            &input.upstream_model_id,
            &input.secret,
            sampling.clone(),
            input.max_output_tokens,
        )
        .await
        .map_err(|error| ChatServiceError::Message(error.to_string()))?;
        self.ensure_model_catalog_account(&account)?;
        let da = self.da_session(&config, &token)?;
        let model = da
            .create_custom_model(input)
            .await
            .map_err(map_custom_model_error)?;
        self.ensure_model_catalog_account(&account)?;
        self.refresh_custom_model_catalog(&da, account.as_str())
            .await;
        self.ensure_model_catalog_account(&account)?;
        let settings = CustomModelSettingsStore::new(&self.app_local_data, &account);
        settings
            .set_sampling(model.id, sampling.clone())
            .map_err(|_| ChatServiceError::TemporarilyUnavailable)?;
        let view = custom_model_view(&model, account.as_str(), sampling)
            .ok_or(ChatServiceError::Unavailable)?;
        Ok(view)
    }

    pub async fn custom_model_update(
        &self,
        id: String,
        mut input: CustomModelUpdateInput,
    ) -> Result<CustomModelView, ChatServiceError> {
        let id = Uuid::parse_str(&id).map_err(|_| {
            ChatServiceError::Message("Select a valid private model to edit.".to_owned())
        })?;
        if let Some(base_url) = input.base_url.as_mut() {
            *base_url = crate::providers::normalize_base_url_input(base_url).map_err(|_| {
                ChatServiceError::Message(
                    "Enter a valid API base URL, such as http://localhost:8000/v1.".to_owned(),
                )
            })?;
        }
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let _write = self.model_preference_writes.lock().await;
        self.ensure_model_catalog_account(&account)?;
        let da = self.da_session(&config, &token)?;
        let current = da
            .list_custom_models_for_management()
            .await
            .map_err(map_custom_model_error)
            .map_err(custom_model_preflight_error)?
            .into_iter()
            .find(|model| model.id == id)
            .ok_or(ChatServiceError::NotFound)?;
        self.ensure_model_catalog_account(&account)?;
        validate_custom_model_update(&current, &input)?;
        validate_custom_model_update_connection_security(&current, &input)?;
        let requires_connection_test =
            custom_model_update_requires_connection_test(&current, &input);
        if requires_connection_test {
            let base_url = input
                .base_url
                .as_deref()
                .or(current.base_url.as_deref())
                .ok_or_else(|| ChatServiceError::Message("Enter an API base URL.".to_owned()))?;
            let upstream_model_id = input
                .upstream_model_id
                .as_deref()
                .or(current.upstream_model_id.as_deref())
                .ok_or_else(|| ChatServiceError::Message("Enter a model ID.".to_owned()))?;
            let max_output_tokens = input
                .max_output_tokens
                .or(current.max_output_tokens)
                .ok_or_else(|| {
                    ChatServiceError::Message("Enter a maximum output token limit.".to_owned())
                })?;
            let settings = CustomModelSettingsStore::new(&self.app_local_data, &account);
            let candidate_sampling = match input.sampling.as_ref() {
                Some(sampling) => sampling.clone(),
                None => settings.sampling(id).map_err(|_| {
                    ChatServiceError::Message(
                        "Could not load this private model's settings. Try again.".to_owned(),
                    )
                })?,
            };
            let stored_credential = if input.secret.is_none() {
                let provider_key_id = current.provider_key_id.ok_or_else(|| {
                    ChatServiceError::Message(
                        "Enter the API key again to test this private model.".to_owned(),
                    )
                })?;
                let credential = self
                    .credentials
                    .resolve_exact_custom_for_inference(&account, provider_key_id)
                    .await
                    .map_err(map_custom_model_connection_credential_error)?;
                let expected_base_url = current.base_url.as_deref().ok_or_else(|| {
                    ChatServiceError::Message(
                        "Enter the API key again to test this private model.".to_owned(),
                    )
                })?;
                if !custom_model_connection_credential_matches(
                    provider_key_id,
                    expected_base_url,
                    &credential,
                ) {
                    return Err(ChatServiceError::Message(
                        "Enter the API key again to test this private model.".to_owned(),
                    ));
                }
                Some(credential)
            } else {
                None
            };
            let secret = input
                .secret
                .as_deref()
                .or_else(|| {
                    stored_credential
                        .as_ref()
                        .map(|credential| credential.secret.as_str())
                })
                .ok_or_else(|| {
                    ChatServiceError::Message(
                        "Enter the API key again to test this private model.".to_owned(),
                    )
                })?;
            if input.base_url.is_some()
                || input.secret.is_some()
                || input.upstream_model_id.is_some()
            {
                let duplicates = self
                    .existing_custom_models_for_connection(
                        &config,
                        &token,
                        &account,
                        CustomModelConnectionCandidate {
                            base_url,
                            secret,
                            upstream_model_id: Some(upstream_model_id),
                            exclude_id: Some(id),
                        },
                    )
                    .await?;
                if let Some(duplicate) = duplicates.first() {
                    return Err(custom_model_duplicate_error(duplicate, secret));
                }
            }
            self.verify_custom_model_connection(
                base_url,
                upstream_model_id,
                secret,
                candidate_sampling,
                max_output_tokens,
            )
            .await
            .map_err(|error| ChatServiceError::Message(error.to_string()))?;
            self.ensure_model_catalog_account(&account)?;
        }
        let sampling = input.sampling.clone();
        let has_da_update = input.base_url.is_some()
            || input.secret.is_some()
            || input.is_enabled.is_some()
            || input.is_active.is_some()
            || input.upstream_model_id.is_some()
            || input.display_name.is_some()
            || input.context_length.is_some()
            || input.max_output_tokens.is_some()
            || input.supports_tools.is_some()
            || input.supports_vision.is_some();
        let (model, da_mutated) = if has_da_update {
            (
                da.update_custom_model(id, input)
                    .await
                    .map_err(map_custom_model_error)?,
                true,
            )
        } else {
            (current, false)
        };
        self.ensure_model_catalog_account(&account)?;
        if da_mutated {
            self.refresh_custom_model_catalog(&da, account.as_str())
                .await;
            self.ensure_model_catalog_account(&account)?;
        }
        if model.id != id {
            return Err(ChatServiceError::Unavailable);
        }
        let settings = CustomModelSettingsStore::new(&self.app_local_data, &account);
        if let Some(sampling) = sampling {
            settings
                .set_sampling(id, sampling)
                .map_err(|_| ChatServiceError::TemporarilyUnavailable)?;
        }
        let sampling = settings
            .sampling(id)
            .map_err(|_| ChatServiceError::TemporarilyUnavailable)?;
        let view = custom_model_view(&model, account.as_str(), sampling)
            .ok_or(ChatServiceError::Unavailable)?;
        Ok(view)
    }

    pub async fn custom_model_delete(&self, id: String) -> Result<(), ChatServiceError> {
        let id = Uuid::parse_str(&id).map_err(|_| {
            ChatServiceError::Message("Select a valid private model to remove.".to_owned())
        })?;
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let _write = self.model_preference_writes.lock().await;
        self.ensure_model_catalog_account(&account)?;
        let da = self.da_session(&config, &token)?;
        da.delete_custom_model(id)
            .await
            .map_err(map_custom_model_error)?;
        self.ensure_model_catalog_account(&account)?;
        self.refresh_custom_model_catalog(&da, account.as_str())
            .await;
        self.ensure_model_catalog_account(&account)?;
        CustomModelSettingsStore::new(&self.app_local_data, &account)
            .clear(id)
            .map_err(|_| ChatServiceError::TemporarilyUnavailable)?;
        Ok(())
    }

    fn ensure_model_catalog_account(
        &self,
        expected: &ValidatedAccountId,
    ) -> Result<(), ChatServiceError> {
        match self.authenticated() {
            Ok((_, _, _, current)) if &current == expected => Ok(()),
            _ => {
                self.model_catalog.clear();
                Err(ChatServiceError::AuthRequired)
            }
        }
    }

    async fn verify_custom_model_connection(
        &self,
        base_url: &str,
        upstream_model_id: &str,
        secret: &str,
        sampling: CustomModelSampling,
        max_output_tokens: i64,
    ) -> Result<(), CustomModelConnectionError> {
        #[cfg(test)]
        if let Some(error) = *self
            .custom_model_connection_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            return Err(error);
        }
        verify_custom_model_connection(
            base_url,
            upstream_model_id,
            secret,
            sampling,
            max_output_tokens,
        )
        .await
    }

    /// Points [`Self::authenticated`] at a test DA without the https-only
    /// environment. Tests in `chat::tests` only.
    #[cfg(test)]
    pub(super) fn set_auth_config_for_test(&self, config: AuthConfig) {
        *self
            .auth_config_override
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(config);
    }

    #[cfg(test)]
    fn set_custom_model_test_overrides(
        &self,
        config: AuthConfig,
        failure: CustomModelConnectionError,
    ) {
        *self
            .auth_config_override
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(config);
        *self
            .custom_model_connection_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(failure);
    }

    async fn refresh_custom_model_catalog(&self, da: &DaChatSession, account: &str) {
        if self.model_catalog.refresh(da, account).await.is_err() {
            // A successful mutation is not retried as an apparent failure. Drop
            // the stale snapshot so the next catalog read fetches the new state.
            self.model_catalog.clear();
        }
    }

    pub async fn default_model_set(
        &self,
        model_id: String,
    ) -> Result<ChatModelCatalogView, ChatServiceError> {
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let models = self
            .read_models(&da, account.as_str())
            .await
            .map_err(map_da_error)?;
        let selected = select_offered_model(&models, &model_id).map_err(map_da_error)?;
        self.ensure_model_catalog_account(&account)?;
        let _write = self.model_preference_writes.lock().await;
        self.ensure_model_catalog_account(&account)?;
        ModelPreferenceStore::new(&self.app_local_data, &account)
            .set_selected(&selected.model_id)
            .map_err(|_| {
                ChatServiceError::Message("Could not save the default model.".to_owned())
            })?;
        let view = model_catalog_view(&models, &selected, false);
        self.ensure_model_catalog_account(&account)?;
        Ok(view)
    }

    /// The catalogue, for a read that selects a model from it. Claude's rows stay
    /// offered up to 10 minutes after their last `Ready` probe, and a failed check
    /// keeps them. This read renews them first: a stale probe is re-run in the
    /// background when rows exist, and waited for up to 4 s only when there are none.
    /// So a model picked from this catalogue still resolves.
    async fn read_models(
        &self,
        da: &DaChatSession,
        account: &str,
    ) -> Result<Arc<Vec<crate::clients::da::generated::ModelEntity>>, DaChatApiError> {
        if let Some(plan) = self.plan_providers() {
            plan.renew_claude_rows().await;
        }
        self.model_catalog.read_through(da, account).await
    }

    async fn preferred_model(
        &self,
        account: &ValidatedAccountId,
        models: &[crate::clients::da::generated::ModelEntity],
    ) -> Result<ModelSelection, ChatServiceError> {
        let _read = self.model_preference_writes.lock().await;
        let saved = ModelPreferenceStore::new(&self.app_local_data, account).selected();
        saved
            .as_deref()
            .and_then(|model_id| select_offered_model(models, model_id).ok())
            .map_or_else(|| select_model(models).map_err(map_da_error), Ok)
    }

    pub(super) async fn locked_conversation_model(
        &self,
        da: &DaChatSession,
        tenant_id: &str,
        expected_auth: &CapturedChatAuth<'_>,
        conversation_id: &ConversationId,
        models: &[crate::clients::da::generated::ModelEntity],
        selected_mcp_server_id: Option<&McpServerId>,
    ) -> Result<
        (
            Uuid,
            ModelSelection,
            McpConversationBinding,
            Option<Arc<McpRuntimeSet>>,
        ),
        ChatServiceError,
    > {
        let conversation_uuid = match parse_conversation_uuid(conversation_id) {
            Ok(uuid) => uuid,
            Err(error) => {
                // An access failure still outranks a malformed id, as before.
                self.ensure_conversation_access(conversation_id).await?;
                return Err(error);
            }
        };
        // The access check and the read have no data dependency, so they run
        // together. An access failure is returned as soon as it is known,
        // without waiting for the read. Errors keep the old precedence: access,
        // then read, then tenant.
        let read = da.get_conversation(conversation_uuid);
        tokio::pin!(read);
        let access = self.ensure_conversation_access(conversation_id);
        tokio::pin!(access);
        let mut early_read = None;
        let access_result = loop {
            tokio::select! {
                result = &mut access => break result,
                result = &mut read, if early_read.is_none() => early_read = Some(result),
            }
        };
        access_result?;
        let conversation = match early_read {
            Some(result) => result,
            None => read.await,
        }
        .map_err(map_da_error)?;
        if conversation.tenant_id != tenant_id {
            return Err(ChatServiceError::Message(
                "The conversation does not belong to this account.".to_owned(),
            ));
        }
        let binding = binding_from_metadata(conversation.metadata)?;
        validate_requested_selection(&binding, selected_mcp_server_id)?;
        // Do this before the legacy model-lock write below. A repointed or
        // unavailable server must fail before any DA mutation for this send.
        if binding.server_id().is_some() {
            self.ensure_captured_auth_context(expected_auth)?;
        }
        let mcp_runtime_set = self
            .runtime_for_binding(expected_auth.config, &binding)
            .await?;
        if binding.server_id().is_some() {
            self.ensure_captured_auth_context(expected_auth)?;
        }

        if let Some(model_id) = conversation
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let model = select_model_by_id(models, Some(model_id)).map_err(|error| {
                if matches!(
                    error,
                    DaChatApiError::CustomModelUnavailable
                        | DaChatApiError::CustomModelCapacityUnsupported
                ) {
                    return map_da_error(error);
                }
                // A plan model is turned on in Settings, not in the tenant catalog.
                if plan_model_provider(model_id).is_some() {
                    return ChatServiceError::Message(
                        "This conversation uses a plan model that is not available right now. Turn the plan on or sign in again in Settings → Models & connections."
                            .to_owned(),
                    );
                }
                ChatServiceError::Message(
                    "This conversation's model is not available. Enable the same model in the tenant catalog."
                        .to_owned(),
                )
            })?;
            return Ok((conversation_uuid, model, binding, mcp_runtime_set));
        }

        // Conversations created before model locking always ran the Sonnet
        // native-compaction path. Freeze that same model before another row is
        // written; selecting the current preference here would turn an old
        // context into a cross-protocol conversation.
        let model = select_model_by_id(models, Some(SONNET_MODEL_ID)).map_err(|_| {
            ChatServiceError::Message(
                "This existing conversation requires Sonnet, but Sonnet is not available."
                    .to_owned(),
            )
        })?;
        da.set_conversation_model(conversation_uuid, &model.model_id)
            .await
            .map_err(map_da_error)?;
        Ok((conversation_uuid, model, binding, mcp_runtime_set))
    }

    /// The in-flight runs in `snapshot` that this device neither drives nor
    /// journaled — another client's live work.
    ///
    /// One implementation for both entry points that need it.
    /// [`Self::workspace_bootstrap`] uses it to keep boot recovery off those
    /// runs; [`Self::open_and_emit`] uses it so opening the conversation does
    /// not hold the composer on a run this window will never see finish.
    /// Two copies of this rule would drift, and the destructive half is the
    /// one that would drift silently.
    async fn foreign_run_ids(
        &self,
        journal: &RunJournalStore,
        snapshot: &ChatSnapshot,
    ) -> HashSet<String> {
        let mut foreign = HashSet::new();
        for run in snapshot
            .runs()
            .iter()
            .filter(|run| is_in_flight_run_state(run.state().clone()))
        {
            // Checked before the journal: a run this process drives is alive
            // here, whatever its journal says, and is never another client's.
            if self.drives_run(run.run_id().as_str()).await {
                continue;
            }
            if !device_journaled_run(&journal.load_entries(run.run_id()).unwrap_or_default()) {
                foreign.insert(run.run_id().as_str().to_owned());
            }
        }
        foreign
    }

    /// Cancels on DA the in-flight runs a dead process on THIS device left
    /// behind, and returns a projection patch for each one it cancelled.
    ///
    /// Split out of [`Self::workspace_bootstrap`] so a wire test can assert
    /// exactly which runs reach DA — the seam
    /// `cleanup_deleted_projects_for_account` opens for the same reason.
    /// Applying the patches stays with the caller, which owns the snapshot.
    ///
    /// `foreign_run_ids` is the exemption that matters: see the call site for
    /// why a run this device holds no journal for is never this device's to
    /// finalize.
    pub(crate) async fn finalize_abandoned_runs(
        &self,
        da: &DaChatSession,
        conversation_id: &ConversationId,
        runs: &[RunRecord],
        foreign_run_ids: &HashSet<String>,
    ) -> Vec<RunRecord> {
        let mut patches = Vec::new();
        for run in runs
            .iter()
            .filter(|run| is_in_flight_run_state(run.state().clone()))
        {
            let run_id = run.run_id().as_str();
            if self.drives_run(run_id).await {
                continue;
            }
            if foreign_run_ids.contains(run_id) {
                tracing::info!(
                    run = %run_id,
                    conversation = %conversation_id.as_str(),
                    state = ?run.state(),
                    "bootstrap: left an in-flight run this device has no journal for"
                );
                continue;
            }
            if let Ok(uuid) = Uuid::parse_str(run_id) {
                let _ = da.cancel_run(uuid).await;
            }
            tracing::info!(
                run = %run_id,
                conversation = %conversation_id.as_str(),
                state = ?run.state(),
                "bootstrap: finalized zombie run left by a previous process"
            );
            if let Ok(patch) = RunRecord::try_new(
                run.run_id().clone(),
                run.conversation_id().clone(),
                RunState::Cancelled,
                run.sequence().saturating_add(1),
            ) {
                patches.push(patch);
            }
        }
        patches
    }

    pub async fn workspace_bootstrap(
        &self,
        app: Option<tauri::AppHandle>,
    ) -> Result<ChatWorkspaceBootstrap, ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let sync = self
            .sync_service(&config, &token, &tenant_id, account.clone())
            .await?;
        let workspace = sync.workspace_bootstrap().await.map_err(map_chat_error)?;

        let summaries: Vec<_> = workspace.conversations().to_vec();
        let active_id = self
            .active_conversations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(account.as_str())
            .cloned();

        let mut reconcile_ids: Vec<ConversationId> = summaries
            .iter()
            .take(STARTUP_RECOVERY_CONVERSATION_LIMIT)
            .map(|summary| summary.id().clone())
            .collect();

        for summary in summaries.iter().skip(STARTUP_RECOVERY_CONVERSATION_LIMIT) {
            let id = summary.id().clone();
            if reconcile_ids.iter().any(|existing| existing == &id) {
                continue;
            }
            let might_have_in_flight = match sync.peek_runs(id.clone()).await {
                Ok(Some(runs)) => runs
                    .iter()
                    .any(|run| is_in_flight_run_state(run.state().clone())),
                _ => false,
            };
            if might_have_in_flight {
                reconcile_ids.push(id);
            }
        }

        let mut recovery_entries = Vec::new();
        let mut active_reconciled = None;

        let journal_store = RunJournalStore::new(self.app_local_data.clone(), account.clone());
        let da = self.da_session(&config, &token)?;

        // These are independent cloud reads. Running them serially made first
        // arrival pay multiple network round trips in sequence. Keep no more
        // than the startup recovery limit in flight at once.
        let reconciled_conversations = sync
            .reconcile_conversations(reconcile_ids, STARTUP_RECOVERY_CONVERSATION_LIMIT)
            .await;

        for (conversation_id, reconciled) in reconciled_conversations {
            let Ok(mut reconciled) = reconciled else {
                continue;
            };
            let in_flight_runs: Vec<_> = reconciled
                .snapshot
                .runs()
                .iter()
                .filter(|run| is_in_flight_run_state(run.state().clone()))
                .cloned()
                .collect();
            let mut live_run_ids: HashSet<String> = HashSet::new();
            // xFalcon Web drives its own runs server-side, so a DA row with no
            // local transition log is another client's live work, not a crash
            // this device is entitled to clean up. `chat-run-recovery` already
            // scopes boot recovery to a "journal-tail run -- one whose local
            // transition log ends non-terminal"; this set is what enforces
            // that scope on the three paths below.
            let foreign_run_ids = self
                .foreign_run_ids(&journal_store, &reconciled.snapshot)
                .await;
            for run in in_flight_runs {
                // A run this process drives is streaming right now. Its journal
                // says Queued -> Running exactly as a dead process's would, so
                // the journal alone cannot tell them apart -- only ownership can.
                if self.drives_run(run.run_id().as_str()).await {
                    live_run_ids.insert(run.run_id().as_str().to_owned());
                    continue;
                }
                if foreign_run_ids.contains(run.run_id().as_str()) {
                    continue;
                }
                let entries = journal_store.load_entries(run.run_id()).unwrap_or_default();
                let Some(patch) = interrupted_run_patch_from_journal(&entries, &run) else {
                    continue;
                };
                if sync
                    .apply_projection_event(ChatProjectionEvent::UpsertRun(patch))
                    .await
                    .is_ok()
                {
                    if let Ok(Some(updated)) = sync.peek_local(conversation_id.clone()).await {
                        reconciled.snapshot = updated;
                    }
                }
            }

            // Capture the previous session's interruption before finalization
            // erases it. Keep its retry identity scoped to this conversation
            // for this boot, so unrelated chats never inherit the notice.
            let pre_finalize_active_run = reconciled
                .snapshot
                .runs()
                .iter()
                .filter(|run| !live_run_ids.contains(run.run_id().as_str()))
                // A run another client is driving was never interrupted. Saying
                // so would report someone else's healthy answer as this
                // device's crash.
                .filter(|run| !foreign_run_ids.contains(run.run_id().as_str()))
                .find(|run| is_in_flight_run_state(run.state().clone()))
                .map(|run| run.run_id().clone());
            if let Some(run_id) = pre_finalize_active_run.as_ref() {
                self.recovered_runs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remember(account.as_str(), &conversation_id, run_id.clone());
            }
            recovery_entries.push(WorkspaceRecoverySnapshot {
                snapshot: reconciled.snapshot.clone(),
                active_run_id: pre_finalize_active_run,
                live_run_ids,
                foreign_run_ids: foreign_run_ids.clone(),
            });

            // Zombie finalization, for runs this device abandoned. An
            // in-flight DA row that THIS process is not driving (not
            // cancellable, not parked here) but whose local journal shows this
            // device once drove it belongs to a process that no longer exists
            // and can never progress. Left alone it re-trips the recovery
            // banner on every launch, forever. Cancel it durably on DA and in
            // the local projection.
            //
            // Two exemptions, and the second is load-bearing. Live runs:
            // everything this process drives sits in `run_cancels` from
            // creation to conclusion, and paused ones in `parked_runs`.
            // Foreign runs: this device is NOT the only thing that executes
            // runs -- xFalcon Web runs its own engine against the same
            // conversations (see `RunCreate` in the generated DA contract,
            // which documents the web run engine minting run ids for its SSE
            // stream). Cancelling one of those here is a durable
            // `status=cancelled` write to the source of truth that stops a
            // live answer mid-stream in the other client, recorded as the
            // user's own cancel. A run this device cannot prove it owns is
            // never this device's to finalize.
            let snapshot_runs = reconciled.snapshot.runs().to_vec();
            let mut finalized_any = false;
            for patch in self
                .finalize_abandoned_runs(&da, &conversation_id, &snapshot_runs, &foreign_run_ids)
                .await
            {
                if sync
                    .apply_projection_event(ChatProjectionEvent::UpsertRun(patch))
                    .await
                    .is_ok()
                {
                    finalized_any = true;
                }
            }
            if finalized_any {
                if let Ok(Some(updated)) = sync.peek_local(conversation_id.clone()).await {
                    reconciled.snapshot = updated;
                }
            }

            let recovered_run = self
                .recovered_runs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .for_snapshot(account.as_str(), &reconciled.snapshot);
            let mut view = without_foreign_active_run(
                conversation_view_from(reconciled.snapshot.clone(), reconciled.availability),
                &foreign_run_ids,
            );
            if let Some(run_id) = recovered_run.as_ref() {
                view.active_run_id = Some(run_id.clone());
            }
            let run_state = recovered_run
                .map(|id| (id, RunState::Interrupted))
                .or_else(|| latest_owned_run_state(&reconciled.snapshot, &foreign_run_ids));
            if active_id.as_ref() == Some(&conversation_id) {
                active_reconciled = Some((view, run_state));
            }
        }

        let deleted = sync.deleted_conversations().await.map_err(map_chat_error)?;
        let summaries = summaries
            .into_iter()
            .filter(|summary| !deleted.contains(summary.id()))
            .collect();
        recovery_entries.retain(|entry| !deleted.contains(entry.snapshot.conversation().id()));
        if active_reconciled
            .as_ref()
            .is_some_and(|(view, _)| deleted.contains(&view.conversation_id))
        {
            active_reconciled = None;
        }
        let recovery_notice =
            startup_recovery_from_workspace(&recovery_entries, active_id.as_ref());

        if let Some(app) = app {
            if let Some((view, run_state)) = active_reconciled {
                let conversation_id = view.conversation_id.clone();
                let _ = emit_chat_projection_event(&app, &projection_snapshot_replaced(view));
                if let Some((run_id, state)) = run_state {
                    let _ = emit_chat_projection_event(
                        &app,
                        &projection_run_state_changed(conversation_id, run_id, state),
                    );
                }
            }
        }

        let bindings = self
            .projects
            .conversation_bindings(&account)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "workspace: local project bindings unavailable");
                HashMap::new()
            });
        Ok(workspace_bootstrap_from(
            summaries,
            recovery_notice,
            &bindings,
        ))
    }

    /// Whether this process is driving `run_id` right now.
    ///
    /// Everything this process executes sits in `run_cancels` from creation to
    /// conclusion, and a run paused on an approval sits in `parked_runs`. A run
    /// in either is alive: it is not a zombie left by a dead process, and it is
    /// not a run to recover.
    async fn drives_run(&self, run_id: &str) -> bool {
        self.run_cancels.lock().await.contains_key(run_id)
            || self.parked_runs.contains(run_id).await
    }

    pub async fn conversation_create(
        &self,
        app: tauri::AppHandle,
        opening_message: Option<String>,
        mcp_server_id: Option<McpServerId>,
    ) -> Result<ChatConversationView, ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let auth_generation = self.sessions.generation();
        let conversation_id = self
            .create_conversation_row(
                &config,
                &token,
                &account,
                opening_message.as_deref(),
                mcp_server_id,
                auth_generation,
            )
            .await?;
        self.set_active_conversation(&account, conversation_id.clone());
        self.open_and_emit(&app, &config, &token, &tenant_id, account, conversation_id)
            .await
    }

    /// Creates a conversation for files attached before anything was sent,
    /// without opening it.
    ///
    /// DA needs a conversation before it will take an attachment, but the
    /// person is still on the screen they were on: the chat becomes current
    /// only when its first message is sent (spec: chat-attachments).
    pub async fn conversation_reserve(
        &self,
        mcp_server_id: Option<McpServerId>,
    ) -> Result<ConversationId, ChatServiceError> {
        let (config, token, _tenant_id, account) = self.authenticated_fresh().await?;
        let auth_generation = self.sessions.generation();
        self.create_conversation_row(
            &config,
            &token,
            &account,
            None,
            mcp_server_id,
            auth_generation,
        )
        .await
    }

    /// Reads the persisted MCP selection from DA's conversation metadata.
    ///
    /// The local transcript projection does not carry metadata, so it cannot
    /// authorize a selected runtime.
    pub async fn mcp_selection_get(
        &self,
        conversation_id: ConversationId,
    ) -> Result<ChatMcpSelectionView, ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        self.ensure_conversation_access(&conversation_id).await?;
        let conversation_uuid = parse_conversation_uuid(&conversation_id)?;
        let da = self.da_session(&config, &token)?;
        let conversation = da
            .get_conversation(conversation_uuid)
            .await
            .map_err(map_da_error)?;
        if !conversation_owned_by(&conversation, &tenant_id, &account) {
            return Err(ChatServiceError::NotFound);
        }
        let binding = binding_from_metadata(conversation.metadata)?;
        Ok(binding.view())
    }

    /// Validates an attachment target against its immutable persisted scope.
    pub async fn mcp_selection_validate(
        &self,
        conversation_id: ConversationId,
        selected: Option<McpServerId>,
    ) -> Result<(), ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        self.ensure_conversation_access(&conversation_id).await?;
        let conversation_uuid = parse_conversation_uuid(&conversation_id)?;
        let da = self.da_session(&config, &token)?;
        let conversation = da
            .get_conversation(conversation_uuid)
            .await
            .map_err(map_da_error)?;
        if !conversation_owned_by(&conversation, &tenant_id, &account) {
            return Err(ChatServiceError::NotFound);
        }
        let binding = binding_from_metadata(conversation.metadata)?;
        validate_requested_selection(&binding, selected.as_ref())?;
        self.runtime_for_binding(&config, &binding).await?;
        Ok(())
    }

    async fn binding_for_new_selection(
        &self,
        config: &AuthConfig,
        selected: Option<McpServerId>,
    ) -> Result<(McpConversationBinding, Option<Arc<McpRuntimeSet>>), ChatServiceError> {
        let Some(server_id) = selected else {
            return Ok((McpConversationBinding::all(), None));
        };
        let (runtime, endpoint) = self
            .mcp
            .runtime_set_for_server(config, server_id.as_str(), None)
            .await
            .map_err(map_mcp_selection_error)?;
        let binding =
            McpConversationBinding::server(server_id, endpoint).map_err(map_mcp_binding_error)?;
        Ok((binding, Some(runtime)))
    }

    async fn runtime_for_binding(
        &self,
        config: &AuthConfig,
        binding: &McpConversationBinding,
    ) -> Result<Option<Arc<McpRuntimeSet>>, ChatServiceError> {
        let Some(server_id) = binding.server_id() else {
            return Ok(None);
        };
        let endpoint = binding.endpoint().ok_or_else(invalid_mcp_binding_error)?;
        self.mcp
            .runtime_set_for_server(config, server_id.as_str(), Some(endpoint))
            .await
            .map(|(runtime, _)| Some(runtime))
            .map_err(map_mcp_selection_error)
    }

    /// The DA row, its project binding, and its title authority.
    ///
    /// The authority is recorded as `Derived` whether or not there is a title
    /// yet: a conversation created before its first message -- for an
    /// attachment -- is titled from that message when it arrives, never left
    /// as "New Chat" for being created early.
    async fn create_conversation_row(
        &self,
        config: &AuthConfig,
        token: &str,
        account: &ValidatedAccountId,
        opening_message: Option<&str>,
        mcp_server_id: Option<McpServerId>,
        auth_generation: u64,
    ) -> Result<ConversationId, ChatServiceError> {
        // A new binding must not arrive after deletion cleanup took its snapshot.
        let _project_cleanup = self.deleted_project_cleanup.lock().await;
        // Freeze selection before the network request: a sidebar switch while
        // DA creates the row must not retarget the new chat to another folder.
        let selected_project = self.projects.active_project(account);
        let (binding, _) = self
            .binding_for_new_selection(config, mcp_server_id)
            .await?;
        if binding.server_id().is_some() {
            self.ensure_captured_auth_context(&CapturedChatAuth {
                config,
                token,
                account,
                generation: auth_generation,
            })?;
        }
        let da = self.da_session(config, token)?;
        let models = self
            .read_models(&da, account.as_str())
            .await
            .map_err(map_da_error)?;
        let model = self.preferred_model(account, &models).await?;
        let title = opening_message.and_then(derive_title);
        if binding.server_id().is_some() {
            self.ensure_captured_auth_context(&CapturedChatAuth {
                config,
                token,
                account,
                generation: auth_generation,
            })?;
        }
        let created = da
            .create_conversation(
                title.as_deref(),
                Some(&model.model_id),
                Some(binding.metadata()),
                binding.server_id().is_none(),
            )
            .await
            .map_err(map_da_error)?;
        let conversation_id = ConversationId::from(created.id.to_string());
        if selected_project.is_some_and(|project_id| {
            self.projects
                .bind_conversation(account, &conversation_id, project_id)
                .is_err()
        }) {
            // A project chat without its local authority would look valid but
            // execute as an unrelated ordinary chat. Roll back the just-made
            // DA row rather than creating that split-brain state.
            let _ = da.delete_conversation(created.id).await;
            return Err(ChatServiceError::Unavailable);
        }
        let recorded = EncryptedChatProjection::open(
            projection_db_path(&self.app_local_data, account),
            self.secrets.clone(),
            account.clone(),
        )
        .and_then(|projection| {
            projection.record_new_title_source(&conversation_id, TitleSource::Derived)
        });
        if !matches!(recorded, Ok(true)) {
            let _ = da.delete_conversation(created.id).await;
            let _ = self.projects.unbind_conversation(account, &conversation_id);
            return Err(ChatServiceError::Unavailable);
        }
        Ok(conversation_id)
    }

    pub async fn conversation_search(
        &self,
        query: String,
    ) -> Result<Vec<ConversationId>, ChatServiceError> {
        let query = query.trim().to_owned();
        if query.is_empty() || query.chars().count() > 200 {
            return Err(ChatServiceError::Message(
                "Search must contain between 1 and 200 characters.".to_owned(),
            ));
        }
        // Local sealed rows only: a DA round-trip here made the palette fail
        // offline and searched a different corpus than the sidebar.
        let (_, _, _, account) = self.authenticated_local()?;
        let path = projection_db_path(&self.app_local_data, &account);
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || {
            EncryptedChatProjection::open(path, secrets, account)
                .map_err(map_chat_error)?
                .search_message_bodies(&query)
                .map_err(map_chat_error)
        })
        .await
        .map_err(|_| ChatServiceError::Unavailable)?
    }

    pub async fn conversation_open(
        &self,
        app: tauri::AppHandle,
        conversation_id: ConversationId,
    ) -> Result<ChatConversationView, ChatServiceError> {
        // No pre-flight `ensure_conversation_access` here: that guard is
        // itself a reconcile, and `open_and_emit` reconciles anyway — the
        // open IS the access check. Mutating commands keep their pre-flight
        // because they must not run against a foreign or dead id.
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        match self
            .open_and_emit(
                &app,
                &config,
                &token,
                &tenant_id,
                account.clone(),
                conversation_id.clone(),
            )
            .await
        {
            Ok(view) => {
                if let Err(error) = self
                    .projects
                    .sync_active_to_conversation(&account, &conversation_id)
                {
                    tracing::warn!(%error, "conversation open: local project binding unavailable");
                    self.projects.clear_active_project(&account);
                }
                self.set_active_conversation(&account, conversation_id);
                Ok(view)
            }
            Err(ChatServiceError::NotFound) => {
                self.remove_missing_conversation(&account, conversation_id, Some(&app))
                    .await?;
                Err(ChatServiceError::NotFound)
            }
            other => other,
        }
    }

    /// Returns the account's sealed local projection without spending the
    /// bearer or changing any navigation authority.
    ///
    /// The caller always follows this with `conversation_open`; this is only
    /// the fast read that lets a known transcript render while that one
    /// authoritative reconcile is in flight.
    pub async fn conversation_preview(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Option<ChatConversationView>, ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_local()?;
        let sync = self
            .sync_service(&config, &token, &tenant_id, account)
            .await?;
        Ok(sync
            .peek_local(conversation_id)
            .await
            .map_err(map_chat_error)?
            .map(|snapshot| {
                conversation_view_from(snapshot, crate::chat::sync::SyncAvailability::Stale)
            }))
    }

    pub async fn conversation_rename(
        &self,
        app: tauri::AppHandle,
        conversation_id: ConversationId,
        title: String,
    ) -> Result<ChatConversationView, ChatServiceError> {
        self.ensure_conversation_access(&conversation_id).await?;
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(ChatServiceError::Message(
                "Title cannot be empty.".to_owned(),
            ));
        }
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let _title_write = self.title_writes.lock().await;
        EncryptedChatProjection::open(
            projection_db_path(&self.app_local_data, &account),
            self.secrets.clone(),
            account,
        )
        .and_then(|projection| projection.advance_title_source(&conversation_id, TitleSource::User))
        .map_err(map_chat_error)?;
        let da = self.da_session(&config, &token)?;
        let uuid = parse_conversation_uuid(&conversation_id)?;
        da.set_conversation_title(uuid, trimmed)
            .await
            .map_err(map_da_error)?;
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        self.open_and_emit(&app, &config, &token, &tenant_id, account, conversation_id)
            .await
    }

    pub async fn message_feedback_list(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Vec<xfalcon_contracts::chat::ChatMessageFeedback>, ChatServiceError> {
        let (config, token, tenant_id, _) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let conversation = da
            .get_conversation(parse_conversation_uuid(&conversation_id)?)
            .await
            .map_err(map_da_error)?;
        if conversation.tenant_id != tenant_id {
            return Err(ChatServiceError::AuthRequired);
        }
        da.message_feedback_list(parse_conversation_uuid(&conversation_id)?)
            .await
            .map_err(map_da_error)
    }

    pub async fn message_feedback_set(
        &self,
        conversation_id: ConversationId,
        message_id: Uuid,
        rating: Option<i32>,
    ) -> Result<Option<xfalcon_contracts::chat::ChatMessageFeedback>, ChatServiceError> {
        let (config, token, tenant_id, _) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let conversation = da
            .get_conversation(parse_conversation_uuid(&conversation_id)?)
            .await
            .map_err(map_da_error)?;
        if conversation.tenant_id != tenant_id {
            return Err(ChatServiceError::AuthRequired);
        }
        da.message_feedback_set(
            parse_conversation_uuid(&conversation_id)?,
            message_id,
            rating,
            &tenant_id,
        )
        .await
        .map_err(map_da_error)
    }

    pub async fn conversation_archive(
        &self,
        app: tauri::AppHandle,
        conversation_id: ConversationId,
        archived: bool,
    ) -> Result<ChatConversationView, ChatServiceError> {
        self.ensure_conversation_access(&conversation_id).await?;
        let (config, token, _, _) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let uuid = parse_conversation_uuid(&conversation_id)?;
        da.set_conversation_archived(uuid, archived)
            .await
            .map_err(map_da_error)?;
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        self.open_and_emit(&app, &config, &token, &tenant_id, account, conversation_id)
            .await
    }

    /// Pins or unpins one conversation on the durable row, then refreshes the
    /// workspace list so the sidebar regroups immediately.
    pub async fn conversation_set_pinned(
        &self,
        app: tauri::AppHandle,
        conversation_id: ConversationId,
        pinned: bool,
    ) -> Result<ChatWorkspaceBootstrap, ChatServiceError> {
        self.ensure_conversation_access(&conversation_id).await?;
        let (config, token, _, _) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let uuid = parse_conversation_uuid(&conversation_id)?;
        da.set_conversation_pinned(uuid, pinned)
            .await
            .map_err(map_da_error)?;
        self.workspace_bootstrap(Some(app)).await
    }

    /// Delete cloud conversations only for roots with confirmed local deletion.
    /// Keep trust and unfinished bindings on failure so the next list can retry.
    pub async fn cleanup_deleted_projects(
        &self,
        app: Option<tauri::AppHandle>,
    ) -> Result<(), ChatServiceError> {
        let _cleanup = self.deleted_project_cleanup.lock().await;
        let Some(account) = self
            .accounts
            .current()
            .map_err(|_| ChatServiceError::Unavailable)?
        else {
            return Ok(());
        };
        let deleted = self
            .projects
            .deleted_projects(&account)
            .map_err(|error| ChatServiceError::Message(error.to_string()))?;
        if deleted.is_empty() {
            return Ok(());
        }
        let bindings = self
            .projects
            .conversation_bindings(&account)
            .map_err(|error| ChatServiceError::Message(error.to_string()))?;
        let mut pending = Vec::new();
        for project in deleted {
            if bindings.values().any(|bound| *bound == project) {
                pending.push(project);
            } else {
                self.ensure_deleted_project(&account, project)?;
                self.projects
                    .revoke_for_account(&account, project)
                    .await
                    .map_err(|error| ChatServiceError::Message(error.to_string()))?;
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        let (config, token, _, authenticated_account) = self.authenticated_fresh().await?;
        if authenticated_account != account {
            return Err(ChatServiceError::AuthRequired);
        }
        let da = self.da_session(&config, &token)?;
        self.cleanup_deleted_projects_for_account(&account, &da, pending, app.as_ref())
            .await
    }

    async fn cleanup_deleted_projects_for_account(
        &self,
        account: &ValidatedAccountId,
        da: &DaChatSession,
        deleted: Vec<crate::projects::ProjectId>,
        app: Option<&tauri::AppHandle>,
    ) -> Result<(), ChatServiceError> {
        let local = EncryptedChatProjection::open(
            projection_db_path(&self.app_local_data, account),
            self.secrets.clone(),
            account.clone(),
        )
        .map_err(map_chat_error)?;
        for project in deleted {
            let bindings = self
                .projects
                .conversation_bindings(account)
                .map_err(|error| ChatServiceError::Message(error.to_string()))?;
            let mut conversations: Vec<_> = bindings
                .into_iter()
                .filter_map(|(id, bound)| (bound == project).then_some(id))
                .collect();
            conversations.sort();
            for id in conversations {
                self.ensure_deleted_project(account, project)?;
                let conversation_id = ConversationId::from(id);
                self.delete_conversation_for_account(
                    account,
                    da,
                    &local,
                    conversation_id,
                    app,
                    Some(project),
                )
                .await?;
            }
            self.ensure_deleted_project(account, project)?;
            self.projects
                .revoke_for_account(account, project)
                .await
                .map_err(|error| ChatServiceError::Message(error.to_string()))?;
        }
        Ok(())
    }

    async fn delete_conversation_for_account(
        &self,
        account: &ValidatedAccountId,
        da: &DaChatSession,
        local: &EncryptedChatProjection,
        conversation_id: ConversationId,
        app: Option<&tauri::AppHandle>,
        deleted_project: Option<crate::projects::ProjectId>,
    ) -> Result<(), ChatServiceError> {
        self.ensure_current_account(account)?;
        self.stop_conversation_runs(account, &conversation_id, Duration::from_secs(10))
            .await?;
        self.ensure_current_account(account)?;
        if let Some(project) = deleted_project {
            self.ensure_deleted_project(account, project)?;
        }
        let conversation_uuid = parse_conversation_uuid(&conversation_id)?;
        match da.delete_conversation(conversation_uuid).await {
            Ok(()) | Err(DaChatApiError::NotFound) => {}
            Err(error) => return Err(map_da_error(error)),
        }
        // What a deleted conversation's summary says has no reason to outlive it.
        crate::agent::summary::forget_summary(conversation_uuid);
        self.finish_conversation_removal(account, local, conversation_id, app)
            .await
    }

    async fn remove_missing_conversation(
        &self,
        account: &ValidatedAccountId,
        conversation_id: ConversationId,
        app: Option<&tauri::AppHandle>,
    ) -> Result<(), ChatServiceError> {
        // A cloud 404 can arrive while a local run still executes tools.
        // Serialize against admission and drain its driver before delinking.
        let _cleanup = self.deleted_project_cleanup.lock().await;
        self.ensure_current_account(account)?;
        self.stop_conversation_runs(account, &conversation_id, Duration::from_secs(10))
            .await?;
        self.ensure_current_account(account)?;
        let local = EncryptedChatProjection::open(
            projection_db_path(&self.app_local_data, account),
            self.secrets.clone(),
            account.clone(),
        )
        .map_err(map_chat_error)?;
        self.finish_conversation_removal(account, &local, conversation_id, app)
            .await
    }

    async fn finish_conversation_removal(
        &self,
        account: &ValidatedAccountId,
        local: &EncryptedChatProjection,
        conversation_id: ConversationId,
        app: Option<&tauri::AppHandle>,
    ) -> Result<(), ChatServiceError> {
        use crate::chat::repository::LocalChatProjection;
        local
            .apply(ChatProjectionEvent::RemoveConversation {
                conversation_id: conversation_id.clone(),
            })
            .await
            .map_err(map_chat_error)?;
        {
            let mut active = self
                .active_conversations
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if active.get(account.as_str()) == Some(&conversation_id) {
                active.remove(account.as_str());
            }
        }
        // Announce each durable deletion before a later unbind/project/chat can
        // fail. Account identity prevents old-session events affecting new UI.
        if let Some(app) = app {
            if let Err(error) = emit_chat_projection_event(
                app,
                &xfalcon_contracts::chat::ChatProjectionEvent::ConversationRemoved {
                    version: xfalcon_contracts::CONTRACT_VERSION,
                    account_id: account.as_str().to_owned(),
                    conversation_id: conversation_id.clone(),
                },
            ) {
                tracing::warn!(%error, "conversation removal event could not be delivered");
                return Err(ChatServiceError::Message(
                    "Conversation deleted, but the view could not be updated. Try again."
                        .to_owned(),
                ));
            }
        }
        self.projects
            .unbind_conversation(account, &conversation_id)
            .map_err(|error| ChatServiceError::Message(error.to_string()))?;
        Ok(())
    }

    fn ensure_current_account(&self, account: &ValidatedAccountId) -> Result<(), ChatServiceError> {
        if self
            .accounts
            .current()
            .map_err(|_| ChatServiceError::Unavailable)?
            .as_ref()
            != Some(account)
        {
            return Err(ChatServiceError::AuthRequired);
        }
        Ok(())
    }

    async fn stop_conversation_runs(
        &self,
        account: &ValidatedAccountId,
        conversation_id: &ConversationId,
        timeout: Duration,
    ) -> Result<(), ChatServiceError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let runs: Vec<_> = self
                .run_cancels
                .lock()
                .await
                .iter()
                .filter(|(_, run)| {
                    run.account == *account && run.conversation_id == *conversation_id
                })
                .map(|(id, run)| (id.clone(), run.cancel.clone(), run.teardown_failed))
                .collect();
            if runs.is_empty() {
                return Ok(());
            }
            for (run_id, cancel, teardown_failed) in runs {
                if teardown_failed {
                    return Err(ChatServiceError::Message(
                        "Could not stop a conversation run. Restart the app and try again."
                            .to_owned(),
                    ));
                }
                cancel.cancel();
                if let Some(parked) = self.parked_runs.take(&run_id).await {
                    abandon_parked_run(
                        &self.run_cancels,
                        &self.app_local_data,
                        account,
                        run_id,
                        parked,
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(project_run_stop_timeout());
            }
            // Registry removal follows the driver's final projection reconciliation.
            // It has no completion notifier; recheck until teardown releases it.
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_millis(25)).min(deadline),
            )
            .await;
        }
    }

    fn ensure_deleted_project(
        &self,
        account: &ValidatedAccountId,
        project: crate::projects::ProjectId,
    ) -> Result<(), ChatServiceError> {
        if self
            .accounts
            .current()
            .map_err(|_| ChatServiceError::Unavailable)?
            .as_ref()
            != Some(account)
        {
            return Err(ChatServiceError::AuthRequired);
        }
        if !self
            .projects
            .deleted_projects(account)
            .map_err(|error| ChatServiceError::Message(error.to_string()))?
            .contains(&project)
        {
            return Err(ChatServiceError::Message(
                "Project deletion could no longer be confirmed.".to_owned(),
            ));
        }
        Ok(())
    }

    pub async fn conversation_delete(
        &self,
        conversation_id: ConversationId,
        app: Option<tauri::AppHandle>,
    ) -> Result<(), ChatServiceError> {
        let _cleanup = self.deleted_project_cleanup.lock().await;
        let (config, token, _, account) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let local = EncryptedChatProjection::open(
            projection_db_path(&self.app_local_data, &account),
            self.secrets.clone(),
            account.clone(),
        )
        .map_err(map_chat_error)?;
        // DELETE itself enforces DA access. Avoid a preflight GET: a previous
        // DELETE may already have succeeded before local cleanup failed.
        self.delete_conversation_for_account(
            &account,
            &da,
            &local,
            conversation_id,
            app.as_ref(),
            None,
        )
        .await
    }

    pub async fn submit_prompt(
        &self,
        prompt: String,
        mcp_server_id: Option<McpServerId>,
    ) -> ChatSubmitResult {
        self.message_send(None, prompt, None, Vec::new(), mcp_server_id)
            .await
    }

    pub async fn message_send(
        &self,
        conversation_id: Option<ConversationId>,
        prompt: String,
        app: Option<tauri::AppHandle>,
        attachment_ids: Vec<Uuid>,
        mcp_server_id: Option<McpServerId>,
    ) -> ChatSubmitResult {
        let trimmed = prompt.trim();
        if trimmed.is_empty() && attachment_ids.is_empty() {
            return ChatSubmitResult::failed("Enter a message to send.");
        }
        if trimmed.len() > MAX_PROMPT_LEN {
            return ChatSubmitResult::failed("Message is too long.");
        }

        {
            let mut in_flight = self.send_in_flight.lock().await;
            if *in_flight {
                return ChatSubmitResult::failed("Wait for the current message to finish sending.");
            }
            *in_flight = true;
        }

        let body = if trimmed.is_empty() {
            ATTACHMENT_ONLY_BODY
        } else {
            trimmed
        };
        let result = self
            .message_send_inner(
                conversation_id,
                body,
                TurnRevision::None,
                app,
                attachment_ids,
                mcp_server_id,
            )
            .await;

        *self.send_in_flight.lock().await = false;
        result
    }

    pub async fn message_edit(
        &self,
        app: Option<tauri::AppHandle>,
        conversation_id: ConversationId,
        message_id: String,
        text: String,
        mcp_server_id: Option<McpServerId>,
    ) -> Result<ChatSubmitResult, ChatServiceError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(ChatSubmitResult::failed("Enter a message to send."));
        }
        if trimmed.len() > MAX_PROMPT_LEN {
            return Ok(ChatSubmitResult::failed("Message is too long."));
        }
        let message_uuid = Uuid::parse_str(&message_id)
            .map_err(|_| ChatServiceError::Message("Message id is invalid.".to_owned()))?;

        self.ensure_conversation_access(&conversation_id).await?;
        let (config, token, tenant_id, _) = self.authenticated_fresh().await?;
        let target_message = self
            .da_session(&config, &token)?
            .revision_message(
                Uuid::parse_str(conversation_id.as_str())
                    .map_err(|_| ChatServiceError::Unavailable)?,
                message_uuid,
                &tenant_id,
            )
            .await
            .map_err(map_revision_error)?;

        if !target_message.role.eq_ignore_ascii_case("user") {
            return Err(ChatServiceError::Message(
                "Only user messages can be edited.".to_owned(),
            ));
        }

        {
            let mut in_flight = self.send_in_flight.lock().await;
            if *in_flight {
                return Ok(ChatSubmitResult::failed(
                    "Wait for the current message to finish sending.",
                ));
            }
            *in_flight = true;
        }

        let result = self
            .message_send_inner(
                Some(conversation_id),
                trimmed,
                TurnRevision::Replace {
                    cutoff: message_uuid,
                },
                app,
                Vec::new(),
                mcp_server_id,
            )
            .await;

        *self.send_in_flight.lock().await = false;
        Ok(result)
    }

    pub async fn message_regenerate(
        &self,
        app: Option<tauri::AppHandle>,
        conversation_id: ConversationId,
        message_id: String,
        mcp_server_id: Option<McpServerId>,
    ) -> Result<ChatSubmitResult, ChatServiceError> {
        let message_uuid = Uuid::parse_str(&message_id)
            .map_err(|_| ChatServiceError::Message("Message id is invalid.".to_owned()))?;

        self.ensure_conversation_access(&conversation_id).await?;
        let (config, token, tenant_id, _) = self.authenticated_fresh().await?;
        let messages = self
            .da_session(&config, &token)?
            .revision_history(
                Uuid::parse_str(conversation_id.as_str())
                    .map_err(|_| ChatServiceError::Unavailable)?,
                message_uuid,
                &tenant_id,
            )
            .await
            .map_err(map_revision_error)?;
        let user_idx = messages
            .iter()
            .rposition(|m| m.role.eq_ignore_ascii_case("user"))
            .ok_or_else(|| {
                ChatServiceError::Message("No user message found to regenerate from.".to_owned())
            })?;
        let user_text = messages[user_idx].content.clone();
        if user_text.trim().is_empty() {
            return Err(ChatServiceError::Message(
                "User message has no text to regenerate.".to_owned(),
            ));
        }
        if user_text.len() > MAX_PROMPT_LEN {
            return Err(ChatServiceError::Message("Message is too long.".to_owned()));
        }
        let cutoff_uuid = messages
            .get(user_idx + 1)
            .ok_or_else(|| {
                ChatServiceError::Message("No response found to regenerate.".to_owned())
            })?
            .id;

        {
            let mut in_flight = self.send_in_flight.lock().await;
            if *in_flight {
                return Ok(ChatSubmitResult::failed(
                    "Wait for the current message to finish sending.",
                ));
            }
            *in_flight = true;
        }

        let result = self
            .message_send_inner(
                Some(conversation_id),
                &user_text,
                TurnRevision::Rerun {
                    cutoff: cutoff_uuid,
                },
                app,
                Vec::new(),
                mcp_server_id,
            )
            .await;

        *self.send_in_flight.lock().await = false;
        Ok(result)
    }

    pub async fn run_cancel(
        &self,
        run_id: String,
        app: Option<&tauri::AppHandle>,
    ) -> Result<(), ChatServiceError> {
        let account = self
            .accounts
            .current()
            .map_err(|_| ChatServiceError::Unavailable)?
            .ok_or_else(|| {
                ChatServiceError::Message("No active run found for that id.".to_owned())
            })?;
        let token = {
            let cancels = self.run_cancels.lock().await;
            match cancels.get(&run_id) {
                Some(run) if run.account == account => Some(run.cancel.clone()),
                Some(_) => return Err(ChatServiceError::AuthRequired),
                None => None,
            }
        };
        let Some(cancel) = token else {
            // Completion can remove the local handle before the window receives
            // its final projection. Reconcile the authorized durable run instead
            // of treating a missing handle as proof that Stop failed.
            let (config, bearer, tenant_id, authenticated_account) =
                self.authenticated_fresh().await?;
            if authenticated_account != account {
                return Err(ChatServiceError::AuthRequired);
            }
            let uuid = Uuid::parse_str(&run_id)
                .map_err(|_| ChatServiceError::Message("Run id is invalid.".to_owned()))?;
            let da = self.da_session(&config, &bearer)?;
            let run = da.get_run(uuid).await.map_err(map_da_error)?;
            let conversation_id = ConversationId::from(run.conversation_id.to_string());
            self.ensure_conversation_access(&conversation_id).await?;
            if self
                .accounts
                .current()
                .map_err(|_| ChatServiceError::Unavailable)?
                .as_ref()
                != Some(&account)
            {
                return Err(ChatServiceError::AuthRequired);
            }
            if !crate::agent::da_events::is_terminal_run_status(&run.status) {
                da.cancel_run(uuid).await.map_err(map_da_error)?;
            }
            if let Some(app) = app {
                self.open_and_emit(app, &config, &bearer, &tenant_id, account, conversation_id)
                    .await?;
            }
            return Ok(());
        };
        cancel.cancel();
        // Stop must reach the executing tool even while another operation
        // owns the admission/cleanup gate. Only parked ownership needs it.
        let _project_cleanup = self.deleted_project_cleanup.lock().await;
        // A run parked on an approval is not being driven by anything, so
        // cancelling its token alone would leave it in the map forever with
        // its invocations stuck `AwaitingApproval` and its DA run
        // non-terminal. Take it out and close it down explicitly.
        if let Some(mut parked) = self.parked_runs.take(&run_id).await {
            parked
                .abandon("run cancelled while awaiting approval")
                .await;
            self.run_cancels.lock().await.remove(&run_id);
        }
        if let Ok((config, bearer, _, authenticated_account)) = self.authenticated_fresh().await {
            if authenticated_account != account {
                return Ok(());
            }
            if let Ok(uuid) = Uuid::parse_str(&run_id) {
                if let Ok(da) = DaChatSession::with_client(
                    config.da_base_url.as_str(),
                    bearer.as_str(),
                    self.da_client.clone(),
                ) {
                    let _ = da.cancel_run(uuid).await;
                }
            }
        }
        Ok(())
    }

    pub async fn run_retry(
        &self,
        app: Option<tauri::AppHandle>,
        run_id: String,
        mcp_server_id: Option<McpServerId>,
    ) -> Result<ChatSubmitResult, ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let uuid = Uuid::parse_str(&run_id)
            .map_err(|_| ChatServiceError::Message("Run id is invalid.".to_owned()))?;
        let run = da.get_run(uuid).await.map_err(map_da_error)?;
        let conversation_id = ConversationId::from(run.conversation_id.to_string());
        self.ensure_conversation_access(&conversation_id).await?;
        let conversation = da
            .get_conversation(run.conversation_id)
            .await
            .map_err(map_da_error)?;
        if !conversation_owned_by(&conversation, &tenant_id, &account) {
            return Err(ChatServiceError::NotFound);
        }
        let binding = binding_from_metadata(conversation.metadata)?;
        validate_requested_selection(&binding, mcp_server_id.as_ref())?;
        self.runtime_for_binding(&config, &binding).await?;
        // At-most-once answers (spec: chat-run-recovery): a run whose
        // finalize landed already has its answer durable in the
        // conversation. Re-submitting would execute the turn a second time
        // and write a duplicate answer row — the exact bug this guard
        // exists for. Reconcile the view so the existing answer is on
        // screen and report settled instead.
        if run.status.eq_ignore_ascii_case("completed") {
            let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
            if let Some(app) = app.as_ref() {
                let _ = self
                    .open_and_emit(
                        app,
                        &config,
                        &token,
                        &tenant_id,
                        account,
                        conversation_id.clone(),
                    )
                    .await;
            }
            return Ok(ChatSubmitResult::already_settled(conversation_id));
        }
        let input = run.input_message;
        if input.trim().is_empty() {
            return Err(ChatServiceError::Message(
                "Run has no input to retry.".to_owned(),
            ));
        }
        Ok(self
            .message_send(Some(conversation_id), input, app, Vec::new(), mcp_server_id)
            .await)
    }

    pub(crate) async fn follow_ups_context(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<
        (
            ModelSelection,
            ProviderId,
            ResolvedCredential,
            ValidatedAccountId,
        ),
        ChatServiceError,
    > {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let da = self.da_session(&config, &token)?;
        let conversation = da
            .get_conversation(parse_conversation_uuid(conversation_id)?)
            .await
            .map_err(map_da_error)?;
        if conversation.tenant_id != tenant_id {
            return Err(ChatServiceError::AuthRequired);
        }
        let models = self
            .model_catalog
            .read_through(&da, account.as_str())
            .await
            .map_err(map_da_error)?;
        let model = select_model_by_id(
            &models,
            Some(conversation.model.as_deref().unwrap_or(SONNET_MODEL_ID)),
        )
        .map_err(map_da_error)?;
        let (provider, credential, model) = self
            .inference_credential(&account, &model)
            .await
            .map_err(ChatServiceError::Message)?;
        let (_, _, _, current_account) = self.authenticated()?;
        if current_account.as_str() != account.as_str() {
            return Err(ChatServiceError::AuthRequired);
        }
        Ok((model, provider, credential, account))
    }

    /// The provider and key this device would execute a run with, or the reason
    /// it cannot — resolved *before* a conversation, message, or run row is
    /// written.
    ///
    /// Nothing else will execute a run THIS device creates. DA's
    /// `/api/v1/chat/*` is a data plane: it stores runs and calls no model
    /// provider, and nothing in it advances a `queued` row (see
    /// `crate::agent::executor`'s module doc comment for the trace). A run
    /// created here without a usable key would therefore sit queued until its
    /// deadline — ten minutes of apparent work that could never produce an
    /// answer. One sentence now is the honest trade.
    ///
    /// Scoped to this device's own runs on purpose. xFalcon Web runs its own
    /// engine against these same conversations, so "this device is the only
    /// thing that executes a run" is false and must not be written here again:
    /// boot recovery once reasoned from exactly that sentence and cancelled
    /// another client's live answer on the source of truth
    /// (`Self::finalize_abandoned_runs`).
    pub(crate) async fn inference_credential(
        &self,
        account: &ValidatedAccountId,
        model: &ModelSelection,
    ) -> Result<(ProviderId, ResolvedCredential, ModelSelection), String> {
        let Some(provider) = crate::agent::da_events::map_provider_id(&model.provider) else {
            return Err(format!(
                "This build cannot run {} models. Choose a model served by a provider it supports.",
                model.provider
            ));
        };
        if matches!(provider, ProviderId::ClaudeCode | ProviderId::ChatgptPlan) {
            let plan = self
                .plan_providers()
                .ok_or_else(|| "Plan models are not available on this device.".to_owned())?;
            return plan
                .credential(provider, account)
                .map(|credential| (provider, credential, model.clone()));
        }
        if provider != ProviderId::CustomOpenAi {
            return self
                .credentials
                .resolve_for_inference(account, provider)
                .await
                .map(|credential| (provider, credential, model.clone()))
                .map_err(inference_credential_error);
        }
        model.custom.as_ref().ok_or_else(|| {
            "This private model is incomplete. Edit or remove it in Settings → Models.".to_owned()
        })?;
        let (config, token, _, current_account) = self
            .authenticated_fresh()
            .await
            .map_err(|_| "Sign in again to use this private model.".to_owned())?;
        if current_account.as_str() != account.as_str() {
            return Err("The active account changed. Try again.".to_owned());
        }
        let da = self
            .da_session(&config, &token)
            .map_err(|_| "Could not refresh the private model. Try again shortly.".to_owned())?;
        // Private rows may change in another client while this window is open.
        // Read the current row for each call, then bind its exact key and URL.
        let (selected, credential) = resolve_custom_model_binding(
            || async {
                let models = self
                    .model_catalog
                    .refresh(&da, account.as_str())
                    .await
                    .map_err(|_| {
                        "Could not refresh the private model. Try again shortly.".to_owned()
                    })?;
                select_model_by_id(&models, Some(&model.model_id)).map_err(|error| {
                    self.model_catalog.clear();
                    match error {
                        DaChatApiError::CustomModelCapacityUnsupported => map_da_error(error),
                        _ => map_da_error(DaChatApiError::CustomModelUnavailable),
                    }
                    .to_string()
                })
            },
            |key_id| {
                self.credentials
                    .resolve_exact_custom_for_inference(account, key_id)
            },
        )
        .await?;
        self.ensure_model_catalog_account(account)
            .map_err(|_| "The active account changed. Try again.".to_owned())?;
        Ok((provider, credential, selected))
    }

    pub(crate) async fn message_send_inner(
        &self,
        conversation_id: Option<ConversationId>,
        trimmed: &str,
        revision: TurnRevision,
        app: Option<tauri::AppHandle>,
        attachment_ids: Vec<Uuid>,
        mcp_server_id: Option<McpServerId>,
    ) -> ChatSubmitResult {
        let _project_cleanup = self.deleted_project_cleanup.lock().await;
        let (config, token, tenant_id, account) = match self.authenticated_fresh().await {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(%error, "message_send: no usable session");
                return match error {
                    ChatServiceError::AuthRequired | ChatServiceError::NoActiveAccount => {
                        ChatSubmitResult::failed("Sign in to send messages.")
                    }
                    ChatServiceError::NotConfigured => {
                        ChatSubmitResult::failed("Auth is not configured on this device.")
                    }
                    ChatServiceError::SessionExpired => {
                        ChatSubmitResult::failed("Session expired. Sign in again.")
                    }
                    other => ChatSubmitResult::failed(other.to_string()),
                };
            }
        };
        let auth_generation = self.sessions.generation();
        let expected_auth = CapturedChatAuth {
            config: &config,
            token: token.as_str(),
            account: &account,
            generation: auth_generation,
        };

        if self.run_cancels.lock().await.len() >= MAX_IN_FLIGHT_RUNS {
            return ChatSubmitResult::failed("Too many active runs. Cancel one and try again.");
        }

        let da = match self.da_session(&config, &token) {
            Ok(session) => session,
            Err(error) => {
                tracing::error!(%error, "message_send: DA session construction failed");
                return ChatSubmitResult::failed("Could not reach the tenant chat API.");
            }
        };

        // Normally a snapshot read with no network on it at all. The
        // workspace refresh publishes the catalogue on app load, so a send only
        // reaches DA here when it beats that refresh or the refresh failed.
        let models = match self.read_models(&da, account.as_str()).await {
            Ok(models) => models,
            Err(DaChatApiError::Unauthorized) => {
                tracing::warn!("message_send: DA rejected the bearer on list_models");
                return ChatSubmitResult::failed("Session expired. Sign in again.");
            }
            Err(error) => {
                tracing::error!(%error, "message_send: list_models failed");
                return ChatSubmitResult::failed("Could not load tenant models.");
            }
        };
        let resolved_id = match conversation_id {
            Some(id) => Some(id),
            None => self
                .active_conversations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(account.as_str())
                .cloned(),
        };

        let (existing_uuid, model, existing_binding, existing_runtime) = if let Some(id) =
            resolved_id.as_ref()
        {
            match self
                .locked_conversation_model(
                    &da,
                    &tenant_id,
                    &expected_auth,
                    id,
                    &models,
                    mcp_server_id.as_ref(),
                )
                .await
            {
                Ok((uuid, model, binding, runtime)) => (Some(uuid), model, Some(binding), runtime),
                Err(error) => {
                    tracing::warn!(%error, conversation = %id.as_str(), "message_send: locked model resolution failed");
                    return ChatSubmitResult::failed(error.to_string());
                }
            }
        } else {
            match self.preferred_model(&account, &models).await {
                Ok(model) => (None, model, None, None),
                Err(error) => {
                    tracing::warn!(%error, "message_send: no supported catalog model");
                    return ChatSubmitResult::failed(
                        "No supported chat model is available. Enable Haiku, Sonnet, Grok, Luna, or Sol in the tenant catalog.",
                    );
                }
            }
        };

        let (binding, mcp_runtime_set) = match existing_binding {
            Some(binding) => (binding, existing_runtime),
            None => match self.binding_for_new_selection(&config, mcp_server_id).await {
                Ok(selection) => selection,
                Err(error) => return ChatSubmitResult::failed(error.to_string()),
            },
        };
        let mcp_scope = match (
            binding.server_id(),
            binding.endpoint(),
            mcp_runtime_set.as_ref(),
        ) {
            (Some(server_id), Some(endpoint), Some(runtimes)) => Some(McpContextScope {
                server_id: server_id.as_str().to_owned(),
                endpoint: endpoint.to_owned(),
                runtimes: Arc::clone(runtimes),
            }),
            (None, None, None) => None,
            _ => {
                return ChatSubmitResult::failed(
                    "This chat's MCP server selection is invalid. Start a new chat.",
                );
            }
        };
        if binding.server_id().is_some()
            && self.ensure_captured_auth_context(&expected_auth).is_err()
        {
            return ChatSubmitResult::failed("The active account changed. Try again.");
        }

        let (provider, credential, model) = match self.inference_credential(&account, &model).await
        {
            Ok(resolved) => resolved,
            Err(message) => {
                tracing::warn!(
                    model = %model.model_id,
                    provider = %model.provider,
                    "message_send: no usable inference credential"
                );
                return ChatSubmitResult::failed(message);
            }
        };
        if binding.server_id().is_some()
            && self.ensure_captured_auth_context(&expected_auth).is_err()
        {
            return ChatSubmitResult::failed("The active account changed. Try again.");
        }

        let (conversation_uuid, created_with_derived_title) = if let Some(uuid) = existing_uuid {
            (uuid, false)
        } else {
            if binding.server_id().is_some()
                && self.ensure_captured_auth_context(&expected_auth).is_err()
            {
                return ChatSubmitResult::failed("The active account changed. Try again.");
            }
            let title = derive_title(trimmed);
            match da
                .create_conversation(
                    title.as_deref(),
                    Some(&model.model_id),
                    Some(binding.metadata()),
                    binding.server_id().is_none(),
                )
                .await
            {
                Ok(conversation) => (conversation.id, title.is_some()),
                Err(DaChatApiError::Unauthorized) => {
                    return ChatSubmitResult::failed("Session expired. Sign in again.");
                }
                Err(error) => {
                    tracing::error!(%error, "message_send: create_conversation failed");
                    return ChatSubmitResult::failed("Could not create a conversation.");
                }
            }
        };

        let conversation_id = resolved_id
            .clone()
            .unwrap_or_else(|| ConversationId::from(conversation_uuid.to_string()));
        let conversation_has_active_run = {
            let runs = self.run_cancels.lock().await;
            has_active_run_for_conversation(&runs, &conversation_id)
        };
        if conversation_has_active_run {
            return ChatSubmitResult::failed(
                "Wait for this conversation's current run to finish or cancel it first.",
            );
        }
        if created_with_derived_title {
            let recorded = EncryptedChatProjection::open(
                projection_db_path(&self.app_local_data, &account),
                self.secrets.clone(),
                account.clone(),
            )
            .and_then(|projection| {
                projection.record_new_title_source(&conversation_id, TitleSource::Derived)
            });
            if !matches!(recorded, Ok(true)) {
                let _ = da.delete_conversation(conversation_uuid).await;
                return ChatSubmitResult::failed("Could not save the conversation title.");
            }
        }
        let upgrade_claimed = match EncryptedChatProjection::open(
            projection_db_path(&self.app_local_data, &account),
            self.secrets.clone(),
            account.clone(),
        )
        .and_then(|projection| projection.begin_title_upgrade(&conversation_id))
        {
            Ok(claimed) => claimed,
            Err(error) => {
                tracing::warn!(
                    conversation = %conversation_id.as_str(),
                    %error,
                    "conversation title: upgrade claim could not be stored"
                );
                false
            }
        };
        // Words the title is made from: what the person wrote, or, for files
        // sent with nothing written, the files' names.
        let title_basis = if upgrade_claimed && trimmed == ATTACHMENT_ONLY_BODY {
            attachment_title_basis(&da, conversation_uuid, &attachment_ids).await
        } else {
            Some(trimmed.to_owned())
        };
        // A conversation that existed before this message -- created for an
        // attachment -- was created without a title, so its first message
        // gives it one now; the model's title replaces it later as for any
        // new chat.
        if upgrade_claimed && existing_uuid.is_some() {
            if let Some(title) = title_basis.as_deref().and_then(derive_title) {
                if binding.server_id().is_some()
                    && self.ensure_captured_auth_context(&expected_auth).is_err()
                {
                    return ChatSubmitResult::failed("The active account changed. Try again.");
                }
                if let Err(error) = da.set_conversation_title(conversation_uuid, &title).await {
                    tracing::warn!(
                        conversation = %conversation_id.as_str(),
                        %error,
                        "conversation title: first-message title could not be saved"
                    );
                }
            }
        }
        if let Some(cutoff) = revision.cutoff() {
            let sync = match self
                .sync_service(&config, &token, &tenant_id, account.clone())
                .await
            {
                Ok(sync) => sync,
                Err(error) => {
                    tracing::error!(%error, "revision: sync_service failed");
                    return match error {
                        ChatServiceError::SessionExpired => {
                            ChatSubmitResult::failed("Session expired. Sign in again.")
                        }
                        _ => ChatSubmitResult::failed("Could not revise conversation."),
                    };
                }
            };

            let reconciled = match sync.reconcile_conversation(conversation_id.clone()).await {
                Ok(reconciled) => reconciled,
                Err(error) => {
                    tracing::error!(
                        %error,
                        conversation = %conversation_id.as_str(),
                        "revision: pre-cut reconcile failed"
                    );
                    return ChatSubmitResult::failed(
                        "Could not access conversation history to revise.",
                    );
                }
            };

            if binding.server_id().is_some()
                && self.ensure_captured_auth_context(&expected_auth).is_err()
            {
                return ChatSubmitResult::failed("The active account changed. Try again.");
            }

            for run in reconciled.snapshot.runs() {
                if is_active_run(run) {
                    let run_id_str = run.run_id().as_str();
                    if let Some(cancel) = self
                        .run_cancels
                        .lock()
                        .await
                        .get(run_id_str)
                        .map(|run| run.cancel.clone())
                    {
                        cancel.cancel();
                    }
                    if let Some(mut parked) = self.parked_runs.take(run_id_str).await {
                        parked.abandon("run cancelled while revising turn").await;
                    }
                    if let Ok(uuid) = Uuid::parse_str(run_id_str) {
                        let _ = da.cancel_run(uuid).await;
                    }
                }
            }

            let pre_cut_runs: std::collections::HashSet<Uuid> = reconciled
                .snapshot
                .runs()
                .iter()
                .filter_map(|r| Uuid::parse_str(r.run_id().as_str()).ok())
                .collect();

            if binding.server_id().is_some()
                && self.ensure_captured_auth_context(&expected_auth).is_err()
            {
                return ChatSubmitResult::failed("The active account changed. Try again.");
            }
            if let Err(error) = da.truncate_conversation(conversation_uuid, cutoff).await {
                tracing::error!(
                    %error,
                    conversation = %conversation_uuid,
                    "revision: truncate_conversation failed"
                );
                return match error {
                    DaChatApiError::Unauthorized => {
                        ChatSubmitResult::failed("Session expired. Sign in again.")
                    }
                    _ => ChatSubmitResult::failed("Could not revise conversation."),
                };
            }

            let reconciled_after = match sync.reconcile_conversation(conversation_id.clone()).await
            {
                Ok(reconciled_after) => {
                    if let Some(ref app_handle) = app {
                        let view = conversation_view_from(
                            reconciled_after.snapshot.clone(),
                            reconciled_after.availability,
                        );
                        let _ = emit_chat_projection_event(
                            app_handle,
                            &projection_snapshot_replaced(view),
                        );
                    }
                    Some(reconciled_after)
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        conversation = %conversation_id.as_str(),
                        "revision: post-cut reconcile warning; continuing with revision"
                    );
                    None
                }
            };

            if let Some(after) = reconciled_after {
                let post_cut_runs: std::collections::HashSet<Uuid> = after
                    .snapshot
                    .runs()
                    .iter()
                    .filter_map(|run| Uuid::parse_str(run.run_id().as_str()).ok())
                    .collect();

                let journal_store =
                    RunJournalStore::new(self.app_local_data.clone(), account.clone());
                for removed_run in pre_cut_runs.difference(&post_cut_runs) {
                    journal_store
                        .remove_run(&xfalcon_contracts::RunId::from(removed_run.to_string()));
                }
            }
        }

        if revision.writes_user_message() {
            if binding.server_id().is_some()
                && self.ensure_captured_auth_context(&expected_auth).is_err()
            {
                return ChatSubmitResult::failed("The active account changed. Try again.");
            }
            // One key per submission (spec: chat-run-recovery): a transport retry
            // of this same submission replays the key and the server dedupes the
            // row, so "response lost after commit" can never double-write.
            let submission_key = Uuid::new_v4().to_string();
            let inserted = match da
                .create_user_message_idempotent(
                    conversation_uuid,
                    trimmed,
                    Some(&submission_key),
                    if attachment_ids.is_empty() {
                        None
                    } else {
                        Some(attachment_ids.clone())
                    },
                )
                .await
            {
                Ok(inserted) => inserted,
                Err(error) => {
                    tracing::error!(%error, conversation = %conversation_uuid, "message_send: create_user_message failed");
                    return match error {
                        DaChatApiError::Unauthorized => {
                            ChatSubmitResult::failed("Session expired. Sign in again.")
                        }
                        _ => ChatSubmitResult::failed("Could not save your message."),
                    };
                }
            };
            // DA links an attachment only when it is ready and in this
            // conversation, and reports how many it linked. A run started
            // with fewer would answer without the files the person sent.
            // DA counts each id once, so the ids sent are counted once too.
            let sent = attachment_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len();
            let linked = inserted.attachments_linked.unwrap_or(0);
            if usize::try_from(linked).ok() != Some(sent) {
                tracing::error!(
                    conversation = %conversation_uuid,
                    sent,
                    linked,
                    "message_send: not every attachment was linked"
                );
                return ChatSubmitResult::failed(
                    "Your message was saved, but not every attached file could be added to it. Attach the files again and resend.",
                );
            }
        }

        if binding.server_id().is_some()
            && self.ensure_captured_auth_context(&expected_auth).is_err()
        {
            return ChatSubmitResult::failed("The active account changed. Try again.");
        }
        let run_id = match da
            .create_run(conversation_uuid, &model.model_id, &model.provider, trimmed)
            .await
        {
            Ok(run_id) => run_id,
            Err(DaChatApiError::Unauthorized) => {
                return ChatSubmitResult::failed("Session expired. Sign in again.");
            }
            Err(error) => {
                tracing::error!(%error, conversation = %conversation_uuid, "message_send: create_run failed");
                return ChatSubmitResult::failed(
                    "Message saved, but the agent run could not start. Check provider credentials in the tenant console.",
                );
            }
        };
        tracing::info!(
            run = %run_id,
            conversation = %conversation_uuid,
            model = %model.model_id,
            provider = %model.provider,
            "message_send: run created, starting local execution"
        );

        self.set_active_conversation(&account, conversation_id.clone());
        let run_id_string = run_id.to_string();

        // Registered BEFORE any further await: `run_cancels` membership is
        // what marks a run as owned by this process, and a concurrent
        // `workspace_bootstrap` (sidebar refresh) finalizes any in-flight run
        // it does not find there as a zombie.
        let cancel = CancellationToken::new();
        self.run_cancels.lock().await.insert(
            run_id_string.clone(),
            ActiveLocalRun {
                account: account.clone(),
                conversation_id: conversation_id.clone(),
                cancel: cancel.clone(),
                teardown_failed: false,
            },
        );

        // Reconcile is the *view*'s concern: it refreshes the local projection
        // the UI renders. The run's own context is read from DA inside
        // `spawn_local_run`, which is where the durable tool rounds live.
        match self
            .sync_service(&config, &token, &tenant_id, account.clone())
            .await
        {
            Ok(sync) => match sync.reconcile_conversation(conversation_id.clone()).await {
                Ok(reconciled) => {
                    if let Some(app) = app.clone() {
                        let view =
                            conversation_view_from(reconciled.snapshot, reconciled.availability);
                        let _ =
                            emit_chat_projection_event(&app, &projection_snapshot_replaced(view));
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, conversation = %conversation_id.as_str(), "message_send: post-submit reconcile failed; transcript may lag");
                }
            },
            Err(error) => {
                tracing::error!(%error, "message_send: sync service unavailable; transcript will not refresh");
            }
        }

        // Conversation-scoped run announcement, emitted BEFORE the run task
        // spawns: the UI store admits run/inference/tool events only for the
        // run bound to the conversation it is showing, and this event is that
        // binding. Emitting it after the spawn would race the run's first
        // deltas and drop them as foreign.
        if let Some(app) = app.as_ref() {
            let _ = emit_chat_projection_event(
                app,
                &projection_run_state_changed(
                    conversation_id.clone(),
                    xfalcon_contracts::RunId::from(run_id_string.clone()),
                    xfalcon_contracts::RunState::Queued,
                ),
            );
        }

        // A plan model's calls spend the plan's quota, and a title the person did
        // not ask for is one such call. The conversation keeps the title it has.
        let title_upgrade = match (upgrade_claimed, title_basis.as_ref()) {
            (true, Some(_)) if is_plan_provider(provider) => None,
            (true, Some(opening_message)) => Some(TitleUpgradeRequest {
                app: app.clone(),
                config: config.clone(),
                bearer: token.clone(),
                da_client: self.da_client.clone(),
                tenant_id: tenant_id.clone(),
                account: account.clone(),
                conversation_id: conversation_id.clone(),
                model: model.clone(),
                provider,
                credential: credential.clone(),
                opening_message: opening_message.clone(),
            }),
            _ => None,
        };

        if let Some(app) = app {
            self.spawn_local_run(LocalRunRequest {
                app,
                config,
                bearer: token,
                da_client: self.da_client.clone(),
                tenant_id,
                account,
                conversation_id: conversation_id.clone(),
                da_run_id: run_id,
                run_id_string: run_id_string.clone(),
                model_id: model.model_id.clone(),
                custom: model.custom.clone(),
                context_length: model.context_length,
                max_output_tokens: model.max_output_tokens,
                execution_mode: model.execution_mode,
                provider,
                credential,
                cancel,
                attachments_required: !attachment_ids.is_empty(),
                mcp_scope,
                auth_generation,
            });
        }
        if let Some(request) = title_upgrade {
            self.spawn_title_upgrade(request);
        }

        ChatSubmitResult::submitted(conversation_id, run_id_string)
    }

    async fn ensure_conversation_access(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<(), ChatServiceError> {
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let sync = self
            .sync_service(&config, &token, &tenant_id, account)
            .await?;
        sync.reconcile_conversation(conversation_id.clone())
            .await
            .map_err(map_chat_error)?;
        Ok(())
    }

    async fn open_and_emit(
        &self,
        app: &tauri::AppHandle,
        config: &AuthConfig,
        token: &str,
        tenant_id: &str,
        account: ValidatedAccountId,
        conversation_id: ConversationId,
    ) -> Result<ChatConversationView, ChatServiceError> {
        let sync = self
            .sync_service(config, token, tenant_id, account.clone())
            .await?;
        let reconciled = sync
            .reconcile_conversation(conversation_id)
            .await
            .map_err(map_chat_error)?;
        let recovered_run = self
            .recovered_runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .for_snapshot(account.as_str(), &reconciled.snapshot);
        // The same exemption the workspace bootstrap applies. Opening a
        // conversation another client is mid-answer in must not hand this
        // window a run it will never see finish: nothing here follows that
        // stream, so the composer would stay held on a spinner with no
        // terminal event behind it.
        let journal_store = RunJournalStore::new(self.app_local_data.clone(), account.clone());
        let foreign_run_ids = self
            .foreign_run_ids(&journal_store, &reconciled.snapshot)
            .await;
        let run_state = recovered_run
            .clone()
            .map(|id| (id, RunState::Interrupted))
            .or_else(|| latest_owned_run_state(&reconciled.snapshot, &foreign_run_ids));
        let mut view = without_foreign_active_run(
            conversation_view_from(reconciled.snapshot, reconciled.availability),
            &foreign_run_ids,
        );
        if let Some(run_id) = recovered_run.as_ref() {
            view.active_run_id = Some(run_id.clone());
        }
        let _ = emit_chat_projection_event(app, &projection_snapshot_replaced(view.clone()));
        if let Some((run_id, state)) = run_state {
            let _ = emit_chat_projection_event(
                app,
                &projection_run_state_changed(view.conversation_id.clone(), run_id, state),
            );
        }
        Ok(view)
    }

    /// [`Self::authenticated`], for a read served only from this device's sealed
    /// rows. No server sees the bearer on such a read, so a dead session — past
    /// expiry with no refresh token, so no request could ever succeed again — is
    /// refused here, as the data plane would refuse it
    /// (see `AuthSessionStore::owns_bound_account`). Checked after
    /// [`Self::authenticated`] so every other refusal keeps its own code.
    fn authenticated_local(
        &self,
    ) -> Result<(AuthConfig, Zeroizing<String>, String, ValidatedAccountId), ChatServiceError> {
        let authenticated = self.authenticated()?;
        if !self.sessions.owns_bound_account(&self.accounts) {
            return Err(ChatServiceError::SessionExpired);
        }
        Ok(authenticated)
    }

    /// [`Self::authenticated`] plus a renewal of the session if its bearer is
    /// about to expire. Every path that is about to *spend* the token uses this
    /// one; the sync reader below is for the paths that only need the account
    /// identity and make no request.
    async fn authenticated_fresh(
        &self,
    ) -> Result<(AuthConfig, Zeroizing<String>, String, ValidatedAccountId), ChatServiceError> {
        // The outcome is deliberately not branched on: `NotRenewable` and
        // `Unavailable` both leave whatever session exists in place, and
        // `ReauthenticationRequired` has already cleared it — so the read below
        // reports the state that actually holds, in its own terms.
        let _ = ensure_fresh_session(&self.sessions).await;
        self.authenticated()
    }

    /// The current session, read without touching the network. Callers about to
    /// make a request want [`Self::authenticated_fresh`] instead — a bearer read
    /// here may be seconds from expiry.
    fn authenticated(
        &self,
    ) -> Result<(AuthConfig, Zeroizing<String>, String, ValidatedAccountId), ChatServiceError> {
        #[cfg(test)]
        let config_override = self
            .auth_config_override
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        #[cfg(test)]
        let config = match config_override {
            Some(config) => config,
            None => AuthConfig::from_env().map_err(|_| ChatServiceError::NotConfigured)?,
        };
        #[cfg(not(test))]
        let config = AuthConfig::from_env().map_err(|_| ChatServiceError::NotConfigured)?;
        let snapshot = self
            .sessions
            .authenticated_snapshot()
            .ok_or(ChatServiceError::AuthRequired)?;
        let identity = snapshot.identity;
        let account_id =
            canonical_account_id(&identity).map_err(|_| ChatServiceError::AuthRequired)?;
        let account = self
            .accounts
            .current()
            .map_err(|_| ChatServiceError::Unavailable)?
            .ok_or(ChatServiceError::NoActiveAccount)?;
        if account.as_str() != account_id {
            return Err(ChatServiceError::AuthRequired);
        }
        let account =
            ValidatedAccountId::parse(&account_id).map_err(|_| ChatServiceError::Unavailable)?;
        Ok((config, snapshot.access_token, identity.tenant_id, account))
    }

    fn da_session(
        &self,
        config: &AuthConfig,
        token: &str,
    ) -> Result<DaChatSession, ChatServiceError> {
        #[cfg(test)]
        if config.da_base_url.scheme() == "http"
            && config
                .da_base_url
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())
        {
            return DaChatSession::for_test_with_client(
                config.da_base_url.as_str(),
                token,
                self.da_client.clone(),
            )
            .map_err(|_| ChatServiceError::Unavailable);
        }
        DaChatSession::with_client(config.da_base_url.as_str(), token, self.da_client.clone())
            .map_err(|_| ChatServiceError::Unavailable)
    }

    #[cfg(test)]
    pub(super) fn session_generation(&self) -> u64 {
        self.sessions.generation()
    }

    fn ensure_captured_auth_context(
        &self,
        expected: &CapturedChatAuth<'_>,
    ) -> Result<(), ChatServiceError> {
        if self.sessions.generation() != expected.generation {
            return Err(ChatServiceError::Message(
                "The active account changed. Try again.".to_owned(),
            ));
        }
        let (config, token, _, account) = self.authenticated()?;
        if &config != expected.config
            || token.as_str() != expected.token
            || &account != expected.account
            || self.sessions.generation() != expected.generation
        {
            return Err(ChatServiceError::Message(
                "The active account changed. Try again.".to_owned(),
            ));
        }
        Ok(())
    }

    fn set_active_conversation(&self, account: &ValidatedAccountId, id: ConversationId) {
        self.active_conversations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(account.as_str().to_owned(), id);
    }

    async fn sync_service(
        &self,
        config: &AuthConfig,
        token: &str,
        tenant_id: &str,
        account: ValidatedAccountId,
    ) -> Result<ChatSyncService<DaChatRepository, EncryptedChatProjection>, ChatServiceError> {
        // A test build reaches a loopback DA through `da_session`, as every
        // other tenant session does.
        #[cfg(test)]
        let session = self.da_session(config, token)?;
        #[cfg(not(test))]
        let session =
            DaChatSession::with_client(config.da_base_url.as_str(), token, self.da_client.clone())
                .map_err(|error| {
                    tracing::error!(%error, "sync service: DA session construction failed");
                    ChatServiceError::Unavailable
                })?;
        let cloud = Arc::new(DaChatRepository::new(session).with_expected_tenant(tenant_id));
        let path = projection_db_path(&self.app_local_data, &account);
        let local = EncryptedChatProjection::open(path, self.secrets.clone(), account.clone())
            .map_err(|error| {
                tracing::error!(%error, "sync service: local projection store failed to open");
                ChatServiceError::Unavailable
            })?;
        Ok(ChatSyncService::new(cloud, local, account))
    }

    /// Path A — streams the run directly from an allowlisted provider using the
    /// account's DA-resolved credential, mirroring every delta to DA as it
    /// arrives.
    ///
    /// Two shapes, chosen by whether the account has a trusted project open:
    /// with one, the run is tool-capable and goes through
    /// [`run_with_tool_loop`] driving a [`LocalTurnDriver`]; without one there
    /// are no tools to offer (every internal tool requires a project), so it
    /// takes the cheaper single-turn [`run_local`] path. See
    /// [`crate::agent::tool_session`] for why that is the routing rule.
    fn spawn_title_upgrade(&self, request: TitleUpgradeRequest) {
        let TitleUpgradeRequest {
            app,
            config,
            bearer,
            da_client,
            tenant_id,
            account,
            conversation_id,
            model,
            provider,
            credential,
            opening_message,
        } = request;
        let app_local_data = self.app_local_data.clone();
        let secrets = self.secrets.clone();
        let title_writes = self.title_writes.clone();
        let plan = self.plan_providers();
        tokio::spawn(async move {
            tracing::info!(
                target: "xfalcon::audit",
                action = "model.call",
                purpose = "conversation_title",
                conversation = %conversation_id.as_str(),
                model = %model.model_id,
                provider = ?provider,
                credential = %credential.credential_id,
                "conversation title: model call started"
            );
            let cancel = CancellationToken::new();
            let adapter = match adapter_for_selected_model(
                provider,
                model.custom.as_ref(),
                &app_local_data,
                &account,
                plan.as_deref(),
                None,
            ) {
                Ok(adapter) => adapter,
                Err(error) => {
                    tracing::warn!(%error, "conversation title: private model route refused");
                    return;
                }
            };
            let title = match tokio::time::timeout(
                Duration::from_secs(30),
                generate_model_title(
                    adapter.as_ref(),
                    &model.model_id,
                    &opening_message,
                    credential.secret.as_str(),
                    &cancel,
                ),
            )
            .await
            {
                Ok(Ok(title)) => title,
                Ok(Err(error)) => {
                    tracing::warn!(
                        conversation = %conversation_id.as_str(),
                        %error,
                        "conversation title: model upgrade failed"
                    );
                    return;
                }
                Err(_) => {
                    cancel.cancel();
                    tracing::warn!(
                        conversation = %conversation_id.as_str(),
                        "conversation title: model upgrade timed out"
                    );
                    return;
                }
            };

            let title_write = title_writes.lock().await;
            let projection = match EncryptedChatProjection::open(
                projection_db_path(&app_local_data, &account),
                secrets,
                account.clone(),
            ) {
                Ok(projection) => projection,
                Err(error) => {
                    tracing::warn!(
                        conversation = %conversation_id.as_str(),
                        %error,
                        "conversation title: source store unavailable"
                    );
                    return;
                }
            };
            if !matches!(
                projection.title_source(&conversation_id),
                Ok(TitleSource::Derived)
            ) {
                return;
            }
            let da = match DaChatSession::with_client(
                config.da_base_url.as_str(),
                bearer.as_str(),
                da_client,
            ) {
                Ok(da) => da,
                Err(error) => {
                    tracing::warn!(
                        conversation = %conversation_id.as_str(),
                        %error,
                        "conversation title: DA session unavailable"
                    );
                    return;
                }
            };
            let conversation_uuid = match parse_conversation_uuid(&conversation_id) {
                Ok(id) => id,
                Err(error) => {
                    tracing::warn!(
                        conversation = %conversation_id.as_str(),
                        %error,
                        "conversation title: invalid conversation id"
                    );
                    return;
                }
            };
            if let Err(error) = da.set_conversation_title(conversation_uuid, &title).await {
                tracing::warn!(
                    conversation = %conversation_id.as_str(),
                    %error,
                    "conversation title: DA update failed"
                );
                return;
            }
            if !matches!(
                projection.advance_title_source(&conversation_id, TitleSource::Llm),
                Ok(true)
            ) {
                tracing::warn!(
                    conversation = %conversation_id.as_str(),
                    "conversation title: upgraded title source could not be stored"
                );
                return;
            }
            tracing::info!(
                conversation = %conversation_id.as_str(),
                model = %model.model_id,
                provider = ?provider,
                credential = %credential.credential_id,
                "conversation title: model upgrade applied"
            );
            drop(title_write);

            let Some(app) = app else {
                return;
            };
            let cloud = Arc::new(DaChatRepository::new(da).with_expected_tenant(tenant_id));
            let sync = ChatSyncService::new(cloud, projection, account);
            if let Ok(reconciled) = sync.reconcile_conversation(conversation_id).await {
                let view = conversation_view_from(reconciled.snapshot, reconciled.availability);
                let _ = emit_chat_projection_event(&app, &projection_snapshot_replaced(view));
            }
        });
    }

    fn spawn_local_run(&self, request: LocalRunRequest) {
        let LocalRunRequest {
            app,
            config,
            bearer,
            da_client,
            tenant_id,
            account,
            conversation_id,
            da_run_id,
            run_id_string,
            model_id,
            custom,
            context_length,
            max_output_tokens,
            execution_mode,
            provider,
            credential,
            cancel,
            attachments_required,
            mcp_scope,
            auth_generation,
        } = request;
        let run_cancels = self.run_cancels.clone();
        let app_local_data = self.app_local_data.clone();
        let secrets = self.secrets.clone();
        let projects = self.projects.clone();
        let approvals = self.approvals.clone();
        let standing_approvals = self.standing_approvals.clone();
        let audit = self.audit.get().cloned();
        let parked_runs = self.parked_runs.clone();
        let mcp = self.mcp.clone();
        let sessions = self.sessions.clone();
        let plan = self.plan_providers();
        tokio::spawn(async move {
            tracing::info!(run = %run_id_string, model = %model_id, "local run: starting");
            let cancelled_teardown = || RunTeardown {
                app: &app,
                config: &config,
                bearer: &bearer,
                da_client: &da_client,
                tenant_id: &tenant_id,
                account: account.clone(),
                app_local_data: app_local_data.clone(),
                secrets: secrets.clone(),
                conversation_id: conversation_id.clone(),
                run_id: RunId::from(run_id_string.clone()),
                run_id_string: run_id_string.clone(),
                run_state: RunState::Cancelled,
                run_cancels: run_cancels.clone(),
            };
            let Ok(session) = DaChatSession::with_client(
                config.da_base_url.as_str(),
                bearer.as_str(),
                da_client.clone(),
            ) else {
                tracing::error!(run = %run_id_string, "local run: DA session construction failed; run abandoned");
                let _ = emit_chat_inference_event(
                    &app,
                    &InferenceEvent::failed(
                        RunId::from(run_id_string.clone()),
                        InferenceErrorCode::Internal,
                        "The conversation could not be reached. You can retry.",
                        true,
                    ),
                );
                run_cancels.lock().await.remove(&run_id_string);
                return;
            };
            // Parsed before any read: a malformed id fails the run without an
            // MCP discovery call it could not use. That call also refreshes the
            // MCP session, which this skips. A durable conversation always has
            // a valid id, so neither change is reachable for one.
            let conversation_uuid = match Uuid::parse_str(conversation_id.as_str()) {
                Ok(uuid) => uuid,
                Err(_) => {
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The conversation identity is invalid.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };
            // Inference uses only DA's frozen context snapshot. The UI's
            // 200-row history endpoint is not an inference fallback: running
            // from it would make restart behavior depend on which rows happened
            // to fit one page.
            let selected_mcp_runtime = mcp_scope.is_some();
            let mcp_servers = async {
                match mcp_scope.as_ref() {
                    Some(scope) => Arc::clone(&scope.runtimes),
                    None => mcp.runtime_set(&config).await,
                }
            };
            let Some((mcp_servers, context_result)) = prepare_run_step(
                read_run_prerequisites(mcp_servers, &session, conversation_uuid),
                &cancel,
                &session,
                cancelled_teardown(),
            )
            .await
            else {
                return;
            };
            if sessions.generation() != auth_generation
                || !sessions.authenticated_snapshot().is_some_and(|snapshot| {
                    canonical_account_id(&snapshot.identity).is_ok_and(|id| id == account.as_str())
                })
            {
                fail_run_before_execution(
                    &app,
                    &RunId::from(run_id_string.clone()),
                    &session,
                    da_run_id,
                    &model_id,
                    "The active account changed. Try again.",
                )
                .await;
                run_cancels.lock().await.remove(&run_id_string);
                return;
            }
            let context_snapshot = match context_result {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::error!(run = %run_id_string, %error, "local run: authoritative context load failed");
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The conversation context could not be loaded safely.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };
            let seed_messages = match context_from_snapshot(&context_snapshot.messages) {
                Ok(messages) => messages,
                Err(error) => {
                    tracing::error!(run = %run_id_string, %error, "local run: authoritative context is inconsistent");
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The conversation context is incomplete or inconsistent.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };
            let adapter = match adapter_for_selected_model(
                provider,
                custom.as_ref(),
                &app_local_data,
                &account,
                plan.as_deref(),
                Some(&conversation_id),
            ) {
                Ok(adapter) => adapter,
                Err(error) => {
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        &crate::agent::local_executor::provider_failure_message(provider, error),
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };
            let run_id = RunId::from(run_id_string.clone());
            let capacity = match crate::agent::context_budget::ContextCapacity::new(
                context_length,
                max_output_tokens,
            ) {
                Ok(capacity) => capacity,
                Err(error) => {
                    tracing::error!(run = %run_id_string, %error, "local run: invalid model capacity contract");
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The selected model has invalid context capacity metadata.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };
            let result_budget =
                crate::tools::ToolResultBudget::for_context_tokens(capacity.context_tokens());
            // DA's snapshot carries each turn's text only; the files the person
            // attached are put back on the turns they came with.
            let mut seed_messages = seed_messages;
            let Some(attached) = prepare_run_step(
                crate::chat::attachments::attach_files(
                    &session,
                    conversation_uuid,
                    &mut seed_messages,
                    crate::chat::attachments::AttachmentBudget::for_result_budget(result_budget),
                    attachments_required,
                ),
                &cancel,
                &session,
                cancelled_teardown(),
            )
            .await
            else {
                return;
            };
            match attached {
                Ok(()) => {}
                // Answering a message without the files it was sent with would
                // be answering a different question.
                Err(error) if attachments_required => {
                    tracing::error!(run = %run_id_string, %error, "local run: attached files could not be listed");
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The files attached to this message could not be loaded. Try again.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
                // A message sent without files is answered even when the list
                // is unreadable: one auxiliary endpoint must not stop every
                // chat. Files from earlier turns are missing from this answer,
                // and the log says so.
                Err(error) => {
                    tracing::warn!(run = %run_id_string, %error, "local run: earlier attachments could not be listed; answering without them");
                }
            }
            let requested_max_output_tokens = match u32::try_from(capacity.max_output_tokens()) {
                Ok(value) => value,
                Err(_) => {
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The selected model output limit is too large for this device.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };
            let hard_input_bytes = match usize::try_from(capacity.hard_input_tokens()) {
                Ok(value) => value,
                Err(_) => {
                    fail_run_before_execution(
                        &app,
                        &RunId::from(run_id_string.clone()),
                        &session,
                        da_run_id,
                        &model_id,
                        "The selected model context limit is too large for this device.",
                    )
                    .await;
                    run_cancels.lock().await.remove(&run_id_string);
                    return;
                }
            };

            // The company gateway queues requests and thinks for minutes, so
            // its runs get limits that outlast one queued request.
            let on_gateway = custom.as_ref().is_some_and(|route| {
                gateway::is_gateway_model(&route.base_url, &route.upstream_model_id)
            });
            let run = Run::new(
                run_id.clone(),
                conversation_id.clone(),
                xfalcon_contracts::credentials::ModelId::from(model_id.clone()),
                RunBudgets {
                    event_budget: 4_096,
                    output_byte_budget: MAX_MESSAGE_TEXT_BYTES,
                    wall_clock: if on_gateway {
                        gateway::RUN_WALL_CLOCK
                    } else {
                        RUN_WALL_CLOCK
                    },
                },
            );
            let journal = PersistingEmittingRunJournal::new(
                app.clone(),
                RunJournalStore::new(app_local_data.clone(), account.clone()),
                run_id.clone(),
            );

            // Read at run assembly, not at boot: a skill imported, enabled,
            // or removed between messages takes effect on the next message.
            // A library that cannot be read costs the run its skills, which is
            // a degraded answer, not a reason to refuse the question.
            // The run's own account's library, never another account's.
            let skills = Arc::new(SkillStore::open(&app_local_data, &account));
            let advertised: Vec<PromptSkill> = skills
                .enabled()
                .unwrap_or_else(|error| {
                    tracing::warn!(run = %run_id_string, kind = error.kind_label(), "local run: skill library unreadable; running without skills");
                    Vec::new()
                })
                .iter()
                .map(|skill| PromptSkill {
                    name: skill.name().to_owned(),
                    description: skill.description().to_owned(),
                    source: skill.discovered.source().to_string(),
                })
                .collect();

            let admitted_payloads = seed_messages
                .iter()
                .flat_map(|message| message.durable_tool_result_payloads.iter().cloned())
                .collect::<Vec<_>>();
            let exact_result = (!admitted_payloads.is_empty()).then(|| {
                crate::tools::exact_result::ExactResultToolExecutor::new(
                    session.clone(),
                    conversation_uuid,
                    admitted_payloads,
                    result_budget,
                )
            });

            let Some(tool_session) = prepare_run_step(
                build_tool_session(ToolSessionRequest {
                    projects: projects.clone(),
                    approvals,
                    mcp: Some(mcp_servers),
                    account: account.clone(),
                    conversation_id: conversation_id.clone(),
                    run_id: run_id.clone(),
                    result_budget,
                    // Skill-declared tool narrowing needs a per-conversation skill
                    // *selection* surface, which does not exist yet; every enabled
                    // skill is advertised to every run, as in xfalcon-web.
                    skill_scope: None,
                    standing_approvals: Some(standing_approvals),
                    skills: skills.clone(),
                    // A refused MCP grant is the user's to fix, and only this
                    // window can ask them. Without it the run still reports the
                    // refusal correctly — it just asks nobody.
                    reauth: Some(Arc::new(WindowReauthNotifier::new(app.clone()))),
                    exact_result,
                    audit,
                }),
                &cancel,
                &session,
                cancelled_teardown(),
            )
            .await
            else {
                return;
            };
            if selected_mcp_runtime && !tool_session.unavailable_mcp_servers.is_empty() {
                fail_run_before_execution(
                    &app,
                    &RunId::from(run_id_string.clone()),
                    &session,
                    da_run_id,
                    &model_id,
                    "The selected MCP server could not be reached. Try again or start a new chat.",
                )
                .await;
                run_cancels.lock().await.remove(&run_id_string);
                return;
            }
            let prompt_project = tool_session
                .has_project_tools
                .then_some(())
                .and_then(|_| {
                    projects
                        .project_for_conversation(&account, &conversation_id)
                        .ok()
                        .flatten()
                })
                .and_then(|project_id| {
                    projects
                        .project_display_name(&account, project_id)
                        .ok()
                        .map(|display_name| PromptProject { display_name })
                });

            let app_for_inference = app.clone();
            let on_inference: Arc<dyn Fn(&InferenceEvent) + Send + Sync> = Arc::new(move |event| {
                let _ = emit_chat_inference_event(&app_for_inference, event);
            });
            let driver = LocalTurnDriver::new(
                session,
                adapter,
                credential.secret,
                on_inference.clone(),
                LocalTurnDriverConfig {
                    run_id: run_id.clone(),
                    da_run_id,
                    model_id: model_id.clone(),
                    route_identity: credential.credential_id.clone(),
                    max_output_tokens: Some(requested_max_output_tokens),
                    execution_mode,
                    tools: tool_session.tools.clone(),
                    tool_ids: tool_session.tool_ids.clone(),
                    seed_messages: with_system_prompt(
                        seed_messages,
                        prompt_project.as_ref(),
                        &tool_session.tools,
                        &tool_session.tool_ids,
                        &advertised,
                        &tool_session.unavailable_mcp_servers,
                    ),
                    conversation_id: conversation_uuid,
                    max_context_bytes: hard_input_bytes,
                    result_budget,
                    mcp_scope,
                    active_compaction: context_snapshot.active_compaction.clone(),
                    history_revision: context_snapshot.history_revision,
                    snapshot_head_sequence_no: context_snapshot.snapshot_head_sequence_no,
                    boundary_sequence_no: context_snapshot.boundary_sequence_no,
                    compactable_through_sequence_no: context_snapshot
                        .compactable_through_sequence_no,
                    compactable_source_message_count: context_snapshot
                        .compactable_source_message_count,
                    source_message_ids_sha256: context_snapshot.source_message_ids_sha256.clone(),
                },
            );
            let mut active_tool_run = ToolRun::new(
                run,
                journal,
                EmittingInvocationJournal::new(app.clone()),
                tool_session,
                driver,
                on_inference,
                cancel,
            );
            if on_gateway {
                active_tool_run.set_watchdog(WatchdogLimits {
                    stall: gateway::RUN_STALL_LIMIT,
                    hard: gateway::RUN_HARD_LIMIT,
                    ..WatchdogLimits::default()
                });
            }
            let local_result = active_tool_run.drive(Vec::new()).await;
            let run_state = active_tool_run.state();
            let mut tool_run = Some(active_tool_run);

            // A paused run is not finished: nothing terminal has been
            // written to DA, its journal must survive, and it has to stay
            // resumable. Park it and stop here — the reconcile/cleanup tail
            // below runs when the decision arrives, not now.
            if matches!(local_result, Ok(None)) {
                if let Some(tool_run) = tool_run.take() {
                    tracing::info!(run = %run_id_string, "local run: parked awaiting a tool approval");
                    park_tool_run(
                        &parked_runs,
                        &run_cancels,
                        &app_local_data,
                        &account,
                        run_id_string.clone(),
                        tool_run,
                    )
                    .await;
                    return;
                }
            }

            match &local_result {
                Ok(Some(outcome)) => {
                    tracing::info!(run = %run_id_string, finish = ?outcome.finish_reason, state = ?run_state, "local run: finished");
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(run = %run_id_string, ?error, state = ?run_state, "local run: failed");
                }
            }
            emit_run_conclusion(&app, &run_id, local_result);

            finish_run(RunTeardown {
                app: &app,
                config: &config,
                bearer: &bearer,
                da_client: &da_client,
                tenant_id: &tenant_id,
                account,
                app_local_data,
                secrets,
                conversation_id,
                run_id,
                run_id_string,
                run_state,
                run_cancels,
            })
            .await;
        });
    }

    async fn run_owner_conversation(
        &self,
        account: &ValidatedAccountId,
        run_id: &str,
    ) -> Result<ConversationId, ChatServiceError> {
        self.run_cancels
            .lock()
            .await
            .get(run_id)
            .filter(|run| run.account == *account)
            .map(|run| run.conversation_id.clone())
            .ok_or_else(|| {
                ChatServiceError::Message("No active run found for this account.".to_owned())
            })
    }

    /// Resolves one tool call a run paused on, then drives that run onward.
    ///
    /// The run has been sitting in [`ParkedToolRuns`] holding its own
    /// `ToolLoop`, turn driver, journals, and `Run` — see that module's doc
    /// comment for why none of those can be rebuilt here.
    pub async fn resolve_tool_approval(
        &self,
        app: tauri::AppHandle,
        run_id: String,
        invocation_id: String,
        approve: bool,
    ) -> Result<(), ChatServiceError> {
        let _project_cleanup = self.deleted_project_cleanup.lock().await;
        let (config, token, tenant_id, account) = self.authenticated_fresh().await?;
        let conversation_id = self.run_owner_conversation(&account, &run_id).await?;
        let Some(mut tool_run) = self.parked_runs.take(&run_id).await else {
            return Err(ChatServiceError::Message(
                "that request is no longer waiting for a decision".to_owned(),
            ));
        };

        if self
            .run_cancels
            .lock()
            .await
            .get(&run_id)
            .is_some_and(|run| run.cancel.is_cancelled())
        {
            abandon_parked_run(
                &self.run_cancels,
                &self.app_local_data,
                &account,
                run_id,
                tool_run,
            );
            return Err(ChatServiceError::Message(
                "This run has been cancelled.".to_owned(),
            ));
        }

        // The run remains registered while this call owns it. Cleanup can
        // cancel and wait for it without holding the gate over tool execution.
        drop(_project_cleanup);

        let invocation_id = InvocationId::from(invocation_id);
        if tool_run
            .resolve_approval(&invocation_id, approve)
            .await
            .is_err()
        {
            // Keep ownership of every remaining invocation. If Stop arrived
            // during resolution, the shared park path completes its teardown.
            park_tool_run(
                &self.parked_runs,
                &self.run_cancels,
                &self.app_local_data,
                &account,
                run_id,
                tool_run,
            )
            .await;
            return Err(ChatServiceError::Message(
                "that request could not be resolved".to_owned(),
            ));
        }

        // Still waiting on a sibling invocation from the same round — resume
        // only once every one of them has a decision, or the next model turn
        // would see a call it never got an answer for.
        if !tool_run.pending_invocation_ids().is_empty() {
            park_tool_run(
                &self.parked_runs,
                &self.run_cancels,
                &self.app_local_data,
                &account,
                run_id,
                tool_run,
            )
            .await;
            return Ok(());
        }

        let run_cancels = self.run_cancels.clone();
        let app_local_data = self.app_local_data.clone();
        let secrets = self.secrets.clone();
        let parked_runs = self.parked_runs.clone();
        let da_client = self.da_client.clone();
        tokio::spawn(async move {
            let outcomes = tool_run.take_resolved_outcomes();
            let result = tool_run.drive(outcomes).await;
            let run_state = tool_run.state();
            let run_id_value = tool_run.run_id().clone();

            if matches!(result, Ok(None)) {
                park_tool_run(
                    &parked_runs,
                    &run_cancels,
                    &app_local_data,
                    &account,
                    run_id,
                    tool_run,
                )
                .await;
                return;
            }

            emit_run_conclusion(&app, &run_id_value, result);

            finish_run(RunTeardown {
                app: &app,
                config: &config,
                bearer: &token,
                da_client: &da_client,
                tenant_id: &tenant_id,
                account,
                app_local_data,
                secrets,
                conversation_id,
                run_id: run_id_value,
                run_id_string: run_id,
                run_state,
                run_cancels,
            })
            .await;
        });
        Ok(())
    }
}

/// Closing the cancellation check after publishing the parked run prevents a
/// Stop that arrived while approval owned it from leaving it parked forever.
async fn park_tool_run(
    parked_runs: &Arc<ParkedToolRuns>,
    runs: &RunCancelRegistry,
    app_local_data: &Path,
    account: &ValidatedAccountId,
    run_id: String,
    tool_run: ChatToolRun,
) {
    parked_runs.park(run_id.clone(), tool_run).await;
    let cancelled = runs
        .lock()
        .await
        .get(&run_id)
        .is_some_and(|run| run.account == *account && run.cancel.is_cancelled());
    if cancelled {
        if let Some(parked) = parked_runs.take(&run_id).await {
            abandon_parked_run(runs, app_local_data, account, run_id, parked);
        }
    }
}

fn abandon_parked_run(
    runs: &RunCancelRegistry,
    app_local_data: &Path,
    account: &ValidatedAccountId,
    run_id: String,
    parked: ChatToolRun,
) {
    let teardown = parked.abandon_in_background("run cancelled");
    let runs = runs.clone();
    let journals = RunJournalStore::new(app_local_data.to_path_buf(), account.clone());
    tokio::spawn(async move {
        match teardown.await {
            Ok(()) => {
                journals.remove_run(&RunId::from(run_id.clone()));
                runs.lock().await.remove(&run_id);
            }
            Err(error) => {
                tracing::error!(%error, "parked run teardown failed");
                if let Some(run) = runs.lock().await.get_mut(&run_id) {
                    run.teardown_failed = true;
                }
            }
        }
    });
}

pub(crate) fn inference_credential_error(error: CredentialServiceError) -> String {
    match error {
        CredentialServiceError::NoUsableCredential
        | CredentialServiceError::NotFound
        | CredentialServiceError::Conflict
        | CredentialServiceError::InvalidRequest
        | CredentialServiceError::StorageUnavailable => MISSING_INFERENCE_KEY.to_owned(),
        CredentialServiceError::Unauthorized => "Your session expired. Sign in again.".to_owned(),
        CredentialServiceError::RateLimited => {
            "Provider key lookup is rate limited. Wait a moment and try again.".to_owned()
        }
        CredentialServiceError::ServiceUnavailable
        | CredentialServiceError::ProviderUnavailable
        | CredentialServiceError::ReconciliationRequired { .. } => {
            "Provider keys are unavailable right now. Try again in a moment.".to_owned()
        }
        CredentialServiceError::InvalidCredential
        | CredentialServiceError::InsufficientPermissions => {
            "The provider key is no longer usable. Re-enter it in Settings → Providers.".to_owned()
        }
    }
}

fn custom_credential_matches(credential: &ResolvedCredential, route: &CustomModelRoute) -> bool {
    credential.credential_id == route.provider_key_id.to_string()
        && credential.base_url.as_deref() == Some(route.base_url.as_str())
}

async fn resolve_custom_model_binding<Load, LoadFuture, Resolve, ResolveFuture>(
    mut load_model: Load,
    mut resolve_key: Resolve,
) -> Result<(ModelSelection, ResolvedCredential), String>
where
    Load: FnMut() -> LoadFuture,
    LoadFuture: Future<Output = Result<ModelSelection, String>>,
    Resolve: FnMut(Uuid) -> ResolveFuture,
    ResolveFuture: Future<Output = Result<ResolvedCredential, CredentialServiceError>>,
{
    // The first read is always fresh; on a mismatch, reload the model and
    // resolved key once more. Never use a provider-wide credential fallback.
    for attempt in 0..2 {
        let model = load_model().await?;
        let route = model.custom.as_ref().ok_or_else(|| {
            "This private model is incomplete. Edit or remove it in Settings → Models.".to_owned()
        })?;
        match resolve_key(route.provider_key_id).await {
            Ok(credential) if custom_credential_matches(&credential, route) => {
                return Ok((model, credential));
            }
            Ok(_) if attempt == 0 => continue,
            Ok(_) => {
                return Err("This private model changed while loading. Try again.".to_owned());
            }
            Err(
                CredentialServiceError::NoUsableCredential
                | CredentialServiceError::ServiceUnavailable,
            ) if attempt == 0 => continue,
            Err(error) => return Err(custom_inference_credential_error(error)),
        }
    }
    Err("This private model changed while loading. Try again.".to_owned())
}

/// Whether a provider's calls spend a plan's quota. The title upgrade and the
/// follow-up suggestions are calls the person did not ask for, so both are
/// skipped for these providers.
pub(super) fn is_plan_provider(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::ClaudeCode | ProviderId::ChatgptPlan)
}

pub(super) fn adapter_for_selected_model(
    provider: ProviderId,
    custom: Option<&CustomModelRoute>,
    app_local_data: &Path,
    account: &ValidatedAccountId,
    plan: Option<&PlanProviders>,
    conversation: Option<&ConversationId>,
) -> Result<Arc<dyn ProviderAdapter>, ProviderError> {
    match (provider, custom) {
        (ProviderId::CustomOpenAi, Some(route)) => {
            let sampling = custom_model_sampling(app_local_data, account, route)?;
            custom_openai_adapter_with_sampling(&route.base_url, &route.upstream_model_id, sampling)
        }
        (ProviderId::CustomOpenAi, None) | (_, Some(_)) => Err(ProviderError::InvalidResponse),
        (ProviderId::ClaudeCode | ProviderId::ChatgptPlan, None) => plan
            .ok_or(ProviderError::RuntimeUnavailable)?
            .adapter(provider, account, conversation.map(ConversationId::as_str)),
        (_, None) => Ok(adapter_for(provider)),
    }
}

fn custom_model_sampling(
    app_local_data: &Path,
    account: &ValidatedAccountId,
    route: &CustomModelRoute,
) -> Result<CustomModelSampling, ProviderError> {
    CustomModelSettingsStore::new(app_local_data, account)
        .sampling(route.model_id)
        .map_err(|_| ProviderError::InvalidResponse)
}

fn custom_inference_credential_error(error: CredentialServiceError) -> String {
    match error {
        CredentialServiceError::Unauthorized => "Your session expired. Sign in again.".to_owned(),
        CredentialServiceError::RateLimited => {
            "Private model key lookup is rate limited. Wait a moment and try again.".to_owned()
        }
        CredentialServiceError::ServiceUnavailable
        | CredentialServiceError::ProviderUnavailable
        | CredentialServiceError::ReconciliationRequired { .. } => {
            "The private model key is unavailable right now. Try again shortly.".to_owned()
        }
        _ => "This private model's key is unavailable. Update it in Settings → Models.".to_owned(),
    }
}

#[cfg(test)]
mod custom_binding_tests {
    use std::collections::VecDeque;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const ENDPOINT: &str = "https://models.example.com/v1";

    fn model(key_id: Uuid) -> ModelSelection {
        ModelSelection {
            model_id: format!("custom/{}", Uuid::from_u128(1)),
            provider: "custom_openai".to_owned(),
            context_length: 131_072,
            max_output_tokens: 16_384,
            execution_mode: ModelExecutionMode::BoundedContext,
            custom: Some(CustomModelRoute {
                model_id: Uuid::from_u128(1),
                base_url: ENDPOINT.to_owned(),
                upstream_model_id: "organization/private-model".to_owned(),
                provider_key_id: key_id,
            }),
        }
    }

    fn credential(key_id: Uuid, url: &str) -> ResolvedCredential {
        ResolvedCredential {
            credential_id: key_id.to_string(),
            secret: Zeroizing::new("test-only-key".to_owned()),
            base_url: Some(url.to_owned()),
        }
    }

    fn managed_custom_model(is_enabled: bool) -> crate::clients::da::generated::ModelEntity {
        let id = Uuid::from_u128(1);
        crate::clients::da::generated::ModelEntity {
            id,
            model_id: format!("custom/{id}"),
            provider: "custom_openai".to_owned(),
            provider_parent: "custom_openai".to_owned(),
            provider_sub: None,
            display_name: "Private model".to_owned(),
            context_length: 32_768,
            max_output_tokens: Some(4_096),
            input_price_per_m: 0.0,
            output_price_per_m: 0.0,
            released_at: None,
            is_chat: true,
            is_enabled,
            supports_streaming: true,
            supports_tools: true,
            supports_vision: false,
            metadata: None,
            created_at: None,
            updated_at: None,
            is_active: true,
            tenant_id: "tenant".to_owned(),
            supports_reasoning: false,
            supports_extended_thinking: false,
            reasoning_param_style: "none".to_owned(),
            default_thinking_budget_tokens: None,
            owner_user_id: Some("user".to_owned()),
            base_url: Some(ENDPOINT.to_owned()),
            upstream_model_id: Some("organization/private-model".to_owned()),
            provider_key_id: Some(Uuid::from_u128(2)),
        }
    }

    fn sparse_custom_model_update() -> CustomModelUpdateInput {
        CustomModelUpdateInput {
            base_url: None,
            secret: None,
            is_enabled: None,
            is_active: None,
            upstream_model_id: None,
            display_name: None,
            context_length: None,
            max_output_tokens: None,
            supports_tools: None,
            supports_vision: None,
            sampling: None,
        }
    }

    fn mutation_test_service(root: &Path, da: &MockServer) -> (ChatService, ValidatedAccountId) {
        let sessions = Arc::new(AuthSessionStore::new());
        sessions.set_authenticated(
            xfalcon_contracts::auth::SessionProjection::authenticated(
                xfalcon_contracts::auth::IdentityProjection::new(
                    "user-1", "tenant-1", "User", None,
                ),
                chrono::Utc::now().timestamp_millis() + 3_600_000,
            ),
            "test-bearer",
            None,
        );
        let accounts = Arc::new(CurrentAccountScope::default());
        accounts
            .bind_native("tenant-1:user-1")
            .expect("bind account");
        let account = ValidatedAccountId::parse("tenant-1:user-1").expect("account");
        let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());
        let service = ChatService::new(
            sessions.clone(),
            accounts.clone(),
            root.to_owned(),
            secrets.clone(),
            Arc::new(CredentialRuntime::new(
                sessions,
                Arc::new(crate::persistence::InMemorySecretStore::default()),
            )),
            Arc::new(ModelCatalogCache::default()),
            Arc::new(ProjectsService::new(
                root.to_owned(),
                accounts.clone(),
                secrets.clone(),
            )),
            Arc::new(ApprovalsService::new(root.to_owned(), accounts, secrets)),
        );
        service.set_custom_model_test_overrides(
            AuthConfig {
                da_base_url: da.uri().parse().expect("DA URL"),
                casdoor_issuer: "https://auth.test.invalid".parse().expect("auth URL"),
                client_id: "test-client".to_owned(),
                organization: "test-org".to_owned(),
                loopback_port: 0,
                mcp_client_id: None,
                mcp_client_secret: None,
            },
            CustomModelConnectionError::ModelRejected,
        );
        (service, account)
    }

    fn mutation_test_model_row(id: Uuid, key_id: Uuid) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "model_id": format!("custom/{id}"),
            "provider": "custom_openai",
            "provider_parent": "custom_openai",
            "provider_sub": null,
            "display_name": "Private model",
            "context_length": 32_768,
            "max_output_tokens": 4_096,
            "input_price_per_m": 0.0,
            "output_price_per_m": 0.0,
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
            "owner_user_id": "user-1",
            "base_url": ENDPOINT,
            "upstream_model_id": "organization/private-model",
            "provider_key_id": key_id,
        })
    }

    #[tokio::test]
    async fn failed_connection_gate_prevents_create_and_update_mutations() {
        let root = tempfile::tempdir().expect("tempdir");
        let da = MockServer::start().await;
        let (service, account) = mutation_test_service(root.path(), &da);
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [], "next_cursor": null, "has_more": false, "limit": 200
            })))
            .expect(4)
            .mount(&da)
            .await;
        let settings_path = root
            .path()
            .join("accounts")
            .join(account_directory_name(account.as_str()))
            .join("custom-model-settings.json");
        let create = CustomModelCreateInput {
            base_url: ENDPOINT.to_owned(),
            secret: "create-test-secret".to_owned(),
            upstream_model_id: "organization/private-model".to_owned(),
            display_name: "Private model".to_owned(),
            context_length: 32_768,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            sampling: CustomModelSampling::default(),
        };

        let create_error = service
            .custom_model_create(create)
            .await
            .expect_err("failed connection test must stop create");
        assert_eq!(
            create_error.to_string(),
            "The provider could not use this model configuration. Check the model ID, API base URL, and model settings."
        );
        let reads = da.received_requests().await.expect("DA requests");
        assert_eq!(reads.len(), 4);
        assert!(reads.iter().all(|request| request.method.as_str() == "GET"));
        assert!(!settings_path.exists());
        da.reset().await;

        let model_id = Uuid::from_u128(1);
        let key_id = Uuid::from_u128(2);
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [mutation_test_model_row(model_id, key_id)],
                "next_cursor": null,
                "has_more": false,
                "limit": 200
            })))
            .expect(8)
            .mount(&da)
            .await;
        let original_sampling = CustomModelSampling {
            temperature: Some(0.4),
            top_p: Some(0.8),
        };
        let settings = CustomModelSettingsStore::new(root.path(), &account);
        settings
            .set_sampling(model_id, original_sampling.clone())
            .expect("seed local settings");
        let mut update = sparse_custom_model_update();
        update.secret = Some("replacement-test-secret".to_owned());
        update.sampling = Some(CustomModelSampling {
            temperature: Some(0.2),
            top_p: Some(0.7),
        });

        let update_error = service
            .custom_model_update(model_id.to_string(), update)
            .await
            .expect_err("failed connection test must stop update");
        assert_eq!(
            update_error.to_string(),
            "The provider could not use this model configuration. Check the model ID, API base URL, and model settings."
        );
        assert_eq!(
            settings.sampling(model_id).expect("reload settings"),
            original_sampling
        );
        let requests = da.received_requests().await.expect("DA requests");
        assert_eq!(requests.len(), 8);
        assert!(requests
            .iter()
            .all(|request| request.method.as_str() == "GET"));

        da.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&da)
            .await;
        let error = service
            .custom_model_update(model_id.to_string(), sparse_custom_model_update())
            .await
            .expect_err("catalog failure cannot mean an uncertain update");
        assert_eq!(error.code(), ChatErrorCode::Failed);
        assert_eq!(
            error.to_string(),
            "Could not check for existing private models. Try again."
        );
        assert_eq!(
            settings.sampling(model_id).expect("reload settings"),
            original_sampling
        );
        let requests = da.received_requests().await.expect("DA requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method.as_str(), "GET");
    }

    #[tokio::test]
    async fn oversized_private_model_key_is_rejected_before_any_da_mutation() {
        let root = tempfile::tempdir().expect("tempdir");
        let da = MockServer::start().await;
        let (service, account) = mutation_test_service(root.path(), &da);
        let input = CustomModelCreateInput {
            base_url: ENDPOINT.to_owned(),
            secret: "x".repeat(MAX_PROVIDER_KEY_BYTES + 1),
            upstream_model_id: "organization/private-model".to_owned(),
            display_name: "Private model".to_owned(),
            context_length: 32_768,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            sampling: CustomModelSampling::default(),
        };

        let error = service
            .custom_model_create(input)
            .await
            .expect_err("oversized keys must fail before save");
        assert_eq!(error.to_string(), "Enter a valid API key.");
        assert!(da
            .received_requests()
            .await
            .expect("DA requests")
            .is_empty());
        assert!(!root
            .path()
            .join("accounts")
            .join(account_directory_name(account.as_str()))
            .join("custom-model-settings.json")
            .exists());
    }

    #[tokio::test]
    async fn duplicate_connections_are_rejected_before_test_or_mutation_regardless_of_label() {
        let root = tempfile::tempdir().expect("tempdir");
        let da = MockServer::start().await;
        let (service, account) = mutation_test_service(root.path(), &da);
        let model_id = Uuid::from_u128(10);
        let key_id = Uuid::from_u128(11);
        let mut row = mutation_test_model_row(model_id, key_id);
        row["display_name"] = serde_json::json!("Company coding model");
        let editable_id = Uuid::from_u128(12);
        let mut editable_row = mutation_test_model_row(editable_id, Uuid::from_u128(13));
        editable_row["upstream_model_id"] = serde_json::json!("organization/another-model");
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [row, editable_row], "next_cursor": null, "has_more": false, "limit": 200
            })))
            .mount(&da)
            .await;
        Mock::given(method("POST")).and(path(format!("/api/v1/chat/provider-keys/{key_id}:resolve")))
            .respond_with(ResponseTemplate::new(200).insert_header("cache-control", "no-store")
                .set_body_json(serde_json::json!({
                    "id": key_id, "provider": "custom_openai", "base_url": ENDPOINT, "secret": "same-key"
                }))).mount(&da).await;
        let create = CustomModelCreateInput {
            base_url: "https://MODELS.example.com:443/v1/".to_owned(),
            secret: "same-key".to_owned(),
            upstream_model_id: "organization/private-model".to_owned(),
            display_name: "A different display name".to_owned(),
            context_length: 32_768,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            sampling: CustomModelSampling::default(),
        };
        let error = service
            .custom_model_create(create)
            .await
            .expect_err("same connection is blocked");
        assert_eq!(
            error.to_string(),
            "This model is already added as “Company coding model”."
        );
        let settings = root
            .path()
            .join("accounts")
            .join(account_directory_name(account.as_str()))
            .join("custom-model-settings.json");
        assert!(!settings.exists());

        let mut update = sparse_custom_model_update();
        update.secret = Some("same-key".to_owned());
        let error = service
            .custom_model_update(model_id.to_string(), update)
            .await
            .expect_err("editing the same row reaches the connection gate");
        assert!(error
            .to_string()
            .starts_with("The provider could not use this model configuration."));

        let mut colliding_update = sparse_custom_model_update();
        colliding_update.secret = Some("same-key".to_owned());
        colliding_update.upstream_model_id = Some("organization/private-model".to_owned());
        let error = service
            .custom_model_update(editable_id.to_string(), colliding_update)
            .await
            .expect_err("updating another row to this connection is blocked");
        assert_eq!(
            error.to_string(),
            "This model is already added as “Company coding model”."
        );
        let requests = da.received_requests().await.expect("requests");
        assert!(requests
            .iter()
            .all(|request| request.method.as_str() == "GET"
                || (request.method.as_str() == "POST"
                    && request.url.path().ends_with(":resolve"))));
    }

    #[tokio::test]
    async fn duplicate_connection_is_blocked_even_when_existing_capacity_is_incomplete() {
        let root = tempfile::tempdir().expect("tempdir");
        let da = MockServer::start().await;
        let (service, _) = mutation_test_service(root.path(), &da);
        let model_id = Uuid::from_u128(30);
        let key_id = Uuid::from_u128(31);
        let mut row = mutation_test_model_row(model_id, key_id);
        row["display_name"] = serde_json::json!(
            "Incomplete\nprivate\u{0007} model with a label that must remain bounded even when the data analyst service returns far too much display text for a notification or modal"
        );
        row["max_output_tokens"] = serde_json::Value::Null;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [row], "next_cursor": null, "has_more": false, "limit": 200
            })))
            .expect(8)
            .mount(&da)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/chat/provider-keys/{key_id}:resolve")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("cache-control", "no-store")
                    .set_body_json(serde_json::json!({
                        "id": key_id,
                        "provider": "custom_openai",
                        "base_url": ENDPOINT,
                        "secret": "same-key"
                    })),
            )
            .expect(1)
            .mount(&da)
            .await;
        let listed = service
            .custom_model_list()
            .await
            .expect("incomplete private model remains manageable");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, model_id.to_string());
        assert_eq!(listed[0].max_output_tokens, None);

        let input = CustomModelCreateInput {
            base_url: ENDPOINT.to_owned(),
            secret: "same-key".to_owned(),
            upstream_model_id: "organization/private-model".to_owned(),
            display_name: "Different label".to_owned(),
            context_length: 32_768,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            sampling: CustomModelSampling::default(),
        };

        let error = service
            .custom_model_create(input)
            .await
            .expect_err("an incomplete existing row is still the same connection");
        assert_eq!(
            error.to_string(),
            "This model is already added as “Incomplete private model with a label that must remain bounded even when the data analyst service returns far too much display t”."
        );
        let requests = da.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 9);
        assert!(requests.iter().all(|request| {
            request.method.as_str() == "GET"
                || (request.method.as_str() == "POST" && request.url.path().ends_with(":resolve"))
        }));
    }

    #[tokio::test]
    async fn existing_connection_check_uses_exact_url_key_and_upstream_id_and_fails_closed() {
        let root = tempfile::tempdir().expect("tempdir");
        let da = MockServer::start().await;
        let (service, account) = mutation_test_service(root.path(), &da);
        let (config, token, _, _) = service.authenticated_fresh().await.expect("auth");
        let model_id = Uuid::from_u128(20);
        let key_id = Uuid::from_u128(21);
        let mut row = mutation_test_model_row(model_id, key_id);
        row["is_enabled"] = serde_json::json!(false);
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [row], "next_cursor": null, "has_more": false, "limit": 200
            })))
            .mount(&da)
            .await;
        Mock::given(method("POST")).and(path(format!("/api/v1/chat/provider-keys/{key_id}:resolve")))
            .respond_with(ResponseTemplate::new(200).insert_header("cache-control", "no-store")
                .set_body_json(serde_json::json!({
                    "id": key_id, "provider": "custom_openai", "base_url": ENDPOINT, "secret": "same-key"
                }))).expect(2).mount(&da).await;
        for (url, key, id, expected) in [
            (ENDPOINT, "same-key", None, 1),
            (ENDPOINT, "different-key", None, 0),
            ("https://other.example.com/v1", "same-key", None, 0),
            (ENDPOINT, "same-key", Some("another-model"), 0),
        ] {
            let matches = service
                .existing_custom_models_for_connection(
                    &config,
                    &token,
                    &account,
                    CustomModelConnectionCandidate {
                        base_url: url,
                        secret: key,
                        upstream_model_id: id,
                        exclude_id: None,
                    },
                )
                .await
                .expect("check");
            assert_eq!(matches.len(), expected);
        }
        da.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&da)
            .await;
        let error = service
            .existing_custom_models_for_connection(
                &config,
                &token,
                &account,
                CustomModelConnectionCandidate {
                    base_url: ENDPOINT,
                    secret: "same-key",
                    upstream_model_id: None,
                    exclude_id: None,
                },
            )
            .await
            .expect_err("unavailable catalog must not mean no duplicate");
        assert_eq!(error.code(), ChatErrorCode::Failed);
        assert_eq!(
            error.to_string(),
            "Could not check for existing private models. Try again."
        );
        da.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/models"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&da)
            .await;
        let error = service
            .existing_custom_models_for_connection(
                &config,
                &token,
                &account,
                CustomModelConnectionCandidate {
                    base_url: ENDPOINT,
                    secret: "same-key",
                    upstream_model_id: None,
                    exclude_id: None,
                },
            )
            .await
            .expect_err("an expired session must retain sign-in guidance");
        assert_eq!(error.code(), ChatErrorCode::SessionExpired);
    }

    #[test]
    fn connection_retest_is_limited_to_wire_binding_changes_and_reenable() {
        let enabled = managed_custom_model(true);
        let mut harmless = sparse_custom_model_update();
        harmless.display_name = Some("Renamed model".to_owned());
        harmless.context_length = Some(64_000);
        harmless.supports_tools = Some(false);
        harmless.supports_vision = Some(true);
        harmless.is_enabled = Some(false);
        harmless.is_active = Some(false);
        assert!(!custom_model_update_requires_connection_test(
            &enabled, &harmless
        ));

        for changed in [
            CustomModelUpdateInput {
                base_url: Some("https://other.example.com/v1".to_owned()),
                ..sparse_custom_model_update()
            },
            CustomModelUpdateInput {
                secret: Some("replacement-key".to_owned()),
                ..sparse_custom_model_update()
            },
            CustomModelUpdateInput {
                upstream_model_id: Some("organization/other-model".to_owned()),
                ..sparse_custom_model_update()
            },
            CustomModelUpdateInput {
                max_output_tokens: Some(8_000),
                ..sparse_custom_model_update()
            },
            CustomModelUpdateInput {
                sampling: Some(CustomModelSampling {
                    temperature: Some(0.2),
                    top_p: None,
                }),
                ..sparse_custom_model_update()
            },
        ] {
            assert!(custom_model_update_requires_connection_test(
                &enabled, &changed
            ));
        }

        let disabled = managed_custom_model(false);
        let mut reenable = sparse_custom_model_update();
        reenable.is_enabled = Some(true);
        assert!(custom_model_update_requires_connection_test(
            &disabled, &reenable
        ));

        let mut inactive = managed_custom_model(true);
        inactive.is_active = false;
        let mut reactivate = sparse_custom_model_update();
        reactivate.is_active = Some(true);
        assert!(custom_model_update_requires_connection_test(
            &inactive,
            &reactivate
        ));
    }

    #[test]
    fn endpoint_change_requires_an_explicit_replacement_key_before_any_probe() {
        let current = managed_custom_model(true);
        let mut changed = sparse_custom_model_update();
        changed.base_url = Some("https://other.example.com/v1".to_owned());
        let error = validate_custom_model_update_connection_security(&current, &changed)
            .expect_err("the old bound key must never be sent to a new endpoint");
        assert_eq!(
            error.to_string(),
            "Enter a new API key when changing the endpoint URL."
        );

        changed.secret = Some("replacement-key".to_owned());
        validate_custom_model_update_connection_security(&current, &changed)
            .expect("an explicit replacement key authorizes testing the new endpoint");
    }

    #[test]
    fn incomplete_capacity_can_be_repaired_with_an_output_limit() {
        let mut current = managed_custom_model(true);
        current.max_output_tokens = None;
        let mut repair = sparse_custom_model_update();
        repair.max_output_tokens = Some(4_096);

        validate_custom_model_update(&current, &repair)
            .expect("an explicit valid output limit repairs the incomplete row");
        assert!(custom_model_update_requires_connection_test(
            &current, &repair
        ));
    }

    #[test]
    fn duplicate_display_names_cannot_emit_controls_secrets_or_unbounded_text() {
        assert_eq!(
            safe_existing_custom_model_name("\n\t", "private-key"),
            "an existing private model"
        );
        assert_eq!(
            safe_existing_custom_model_name("Model private-key", "private-key"),
            "an existing private model"
        );
        let safe =
            safe_existing_custom_model_name(&format!("Model\n{}", "x".repeat(200)), "private-key");
        assert_eq!(safe.chars().count(), 128);
        assert!(!safe.chars().any(char::is_control));
    }

    #[test]
    fn stored_connection_secret_must_match_the_exact_key_and_current_route() {
        let key_id = Uuid::from_u128(2);
        assert!(custom_model_connection_credential_matches(
            key_id,
            ENDPOINT,
            &credential(key_id, ENDPOINT)
        ));
        assert!(!custom_model_connection_credential_matches(
            key_id,
            ENDPOINT,
            &credential(Uuid::from_u128(3), ENDPOINT)
        ));
        assert!(!custom_model_connection_credential_matches(
            key_id,
            ENDPOINT,
            &credential(key_id, "https://other.example.com/v1")
        ));
    }

    #[test]
    fn persisted_sampling_is_reloaded_by_central_provider_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("tenant:user").expect("account");
        let route = CustomModelRoute {
            model_id: Uuid::from_u128(1),
            base_url: ENDPOINT.to_owned(),
            upstream_model_id: "organization/private-model".to_owned(),
            provider_key_id: Uuid::from_u128(2),
        };
        CustomModelSettingsStore::new(root.path(), &account)
            .set_sampling(
                route.model_id,
                CustomModelSampling {
                    temperature: Some(0.25),
                    top_p: Some(0.75),
                },
            )
            .expect("persist sampling");

        let reloaded = custom_model_sampling(root.path(), &account, &route)
            .expect("central resolution reloads settings");
        let request = crate::providers::ProviderChatRequest {
            model_id: format!("custom/{}", route.model_id),
            messages: vec![ContextMessage::new(ContextRole::User, "hello")],
            max_output_tokens: Some(64),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        };
        let body = crate::providers::custom_openai_request_body_for_test(
            &route.base_url,
            &route.upstream_model_id,
            reloaded,
            &request,
        )
        .expect("provider request body");

        assert_eq!(body["model"], route.upstream_model_id);
        assert_eq!(body["temperature"], 0.25);
        assert_eq!(body["top_p"], 0.75);
    }

    async fn resolve_sequence(
        keys: Vec<ResolvedCredential>,
    ) -> (
        Result<(ModelSelection, ResolvedCredential), String>,
        usize,
        Vec<Uuid>,
    ) {
        let key_id = Uuid::from_u128(2);
        let loads = Arc::new(Mutex::new(0));
        let resolved_ids = Arc::new(Mutex::new(Vec::new()));
        let keys = Arc::new(Mutex::new(VecDeque::from(keys)));
        let outcome = resolve_custom_model_binding(
            {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        *loads.lock().expect("load count") += 1;
                        Ok(model(key_id))
                    }
                }
            },
            {
                let resolved_ids = Arc::clone(&resolved_ids);
                let keys = Arc::clone(&keys);
                move |requested_id| {
                    let resolved_ids = Arc::clone(&resolved_ids);
                    let keys = Arc::clone(&keys);
                    async move {
                        resolved_ids
                            .lock()
                            .expect("resolved ids")
                            .push(requested_id);
                        Ok(keys.lock().expect("keys").pop_front().expect("next key"))
                    }
                }
            },
        )
        .await;
        let count = *loads.lock().expect("load count");
        let ids = resolved_ids.lock().expect("resolved ids").clone();
        (outcome, count, ids)
    }

    #[tokio::test]
    async fn a_mismatched_endpoint_reloads_the_model_and_exact_key_once() {
        let key_id = Uuid::from_u128(2);
        let (outcome, loads, ids) = resolve_sequence(vec![
            credential(key_id, "https://old.example.com/v1"),
            credential(key_id, ENDPOINT),
        ])
        .await;
        let (selected, resolved) = outcome.expect("second binding matches");
        assert_eq!(
            selected.custom.expect("private route").provider_key_id,
            key_id
        );
        assert_eq!(resolved.base_url.as_deref(), Some(ENDPOINT));
        assert_eq!(loads, 2);
        assert_eq!(ids, vec![key_id, key_id]);
    }

    #[tokio::test]
    async fn a_second_mismatch_fails_without_another_key_or_endpoint() {
        let key_id = Uuid::from_u128(2);
        let (outcome, loads, ids) = resolve_sequence(vec![
            credential(key_id, "https://old.example.com/v1"),
            credential(key_id, "https://other.example.com/v1"),
        ])
        .await;
        let error = outcome.expect_err("second mismatch refuses inference");
        assert!(error.contains("changed while loading"));
        assert_eq!(loads, 2);
        assert_eq!(ids, vec![key_id, key_id]);
    }
}

/// Emits the one terminal inference event a finished run owes the UI, if any.
///
/// Several failure paths already emitted their own `Failed` through the
/// presentation sink with a more accurate error code than this layer could
/// reconstruct; emitting again would render the run as failing twice.
fn emit_run_conclusion<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    run_id: &RunId,
    result: Result<Option<RunEngineOutcome>, RunEngineError>,
) {
    match result {
        Ok(Some(outcome)) => {
            let _ = emit_chat_inference_event(
                app,
                &InferenceEvent::completed(run_id.clone(), outcome.finish_reason),
            );
        }
        // Paused — handled by the caller, which parks rather than concluding.
        Ok(None) => {}
        Err(RunEngineError::Cancelled) => {
            let _ = emit_chat_inference_event(
                app,
                &InferenceEvent::completed(run_id.clone(), FinishReason::Cancelled),
            );
        }
        Err(RunEngineError::ToolsUnavailable) | Err(RunEngineError::Turn(_)) => {}
        Err(error) => {
            let (code, message) = run_failure_report(&error);
            let _ = emit_chat_inference_event(
                app,
                &InferenceEvent::failed(run_id.clone(), code, &message, true),
            );
        }
    }
}

/// What the reader is told when a run dies, and under which code.
///
/// Every arm here is reachable through ordinary use — a model over-running
/// its tool budget, a slow tool, an oversized answer, a provider that stops
/// answering. One catch-all ("the agent run did not finish cleanly") gave all
/// of them the same sentence, which names no cause and suggests no move; the
/// reader's only option was to retry blind into the same wall. Each message
/// names what ran out and what to do differently.
///
/// Every arm is retryable: none of these are permanent states of the world,
/// and the run's own inputs are still on screen.
fn run_failure_report(error: &RunEngineError) -> (InferenceErrorCode, String) {
    match error {
        RunEngineError::ToolLoop(ToolLoopError::MaxRoundsExceeded(rounds)) => (
            InferenceErrorCode::Internal,
            format!(
                "This answer used all {rounds} tool steps without reaching a result. Ask for a narrower slice, then retry."
            ),
        ),
        RunEngineError::ToolLoop(ToolLoopError::RoundTimeout) => (
            InferenceErrorCode::Internal,
            "A tool took too long to answer, so the step was abandoned. You can retry.".to_owned(),
        ),
        RunEngineError::ToolLoop(ToolLoopError::AuthorizationRefused) => (
            InferenceErrorCode::Internal,
            "MCP server sign-in expired. Reconnect it, then retry.".to_owned(),
        ),
        RunEngineError::ToolLoop(_) => (
            InferenceErrorCode::Internal,
            "A tool call could not be completed. You can retry.".to_owned(),
        ),
        RunEngineError::Run(_) => (
            InferenceErrorCode::Internal,
            "This answer ran past the time budget for a single turn. Ask for a narrower slice, then retry."
                .to_owned(),
        ),
        RunEngineError::Output(_) => (
            InferenceErrorCode::Internal,
            "This answer grew past the size one turn can hold. Ask for a narrower slice, then retry."
                .to_owned(),
        ),
        RunEngineError::Presentation(_) => (
            InferenceErrorCode::Internal,
            "The response stream could not be assembled. You can retry.".to_owned(),
        ),
        RunEngineError::Executor(_) => (
            InferenceErrorCode::ProviderUnavailable,
            "The model provider stopped answering. Check the provider key in Settings, then retry."
                .to_owned(),
        ),
        // Handled by their own arms above; matched for exhaustiveness so a
        // new variant cannot fall through to a message that does not fit it.
        RunEngineError::Cancelled
        | RunEngineError::ToolsUnavailable
        | RunEngineError::Turn(_) => (
            InferenceErrorCode::Internal,
            "The agent run did not finish. You can retry.".to_owned(),
        ),
    }
}

struct RunTeardown<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
    config: &'a AuthConfig,
    bearer: &'a str,
    da_client: &'a reqwest::Client,
    tenant_id: &'a str,
    account: ValidatedAccountId,
    app_local_data: PathBuf,
    secrets: Arc<dyn SecretStore>,
    conversation_id: ConversationId,
    run_id: RunId,
    run_id_string: String,
    run_state: RunState,
    run_cancels: RunCancelRegistry,
}

/// The names of the files a message was sent with, for a title: "q3.xlsx",
/// "q3.xlsx, notes.md and 2 more". `None` when they cannot be read, which
/// leaves the conversation untitled rather than titled "Attached files".
async fn attachment_title_basis(
    da: &DaChatSession,
    conversation_id: Uuid,
    attachment_ids: &[Uuid],
) -> Option<String> {
    let attachments = match da.list_conversation_attachments(conversation_id).await {
        Ok(attachments) => attachments,
        Err(error) => {
            tracing::warn!(%error, "conversation title: attachment names could not be read");
            return None;
        }
    };
    let names: Vec<&str> = attachment_ids
        .iter()
        .filter_map(|id| {
            attachments
                .iter()
                .find(|attachment| attachment.id == *id)
                .map(|attachment| attachment.file_name.as_str())
        })
        .collect();
    title_basis_from_names(&names)
}

pub(super) fn title_basis_from_names(names: &[&str]) -> Option<String> {
    match names.len() {
        0 => None,
        count if count <= MAX_TITLED_ATTACHMENTS => Some(names.join(", ")),
        count => Some(format!(
            "{} and {} more",
            names[..MAX_TITLED_ATTACHMENTS].join(", "),
            count - MAX_TITLED_ATTACHMENTS
        )),
    }
}

/// MCP discovery and DA's frozen context share no data, so a run issues both
/// together. The caller races the pair with one `prepare_run_step`, so Stop
/// drops both reads and takes the same cancelled path as either alone.
async fn read_run_prerequisites<M: std::future::Future>(
    mcp_servers: M,
    session: &DaChatSession,
    conversation_uuid: Uuid,
) -> (M::Output, Result<ContextSnapshot, DaChatApiError>) {
    tokio::join!(
        mcp_servers,
        session.read_context_snapshot(conversation_uuid)
    )
}

/// Preparation has no executing tools or pending run writes to drain. Stop can
/// drop these reads/discovery waits, then use the same terminal UI and cleanup
/// path as an executing run. Do not wrap the driver or durable mutations here.
async fn prepare_run_step<T, R: tauri::Runtime>(
    step: impl std::future::Future<Output = T>,
    cancel: &CancellationToken,
    session: &DaChatSession,
    teardown: RunTeardown<'_, R>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => {
            emit_run_conclusion(teardown.app, &teardown.run_id, Err(RunEngineError::Cancelled));
            let result = match Uuid::parse_str(teardown.run_id.as_str()) {
                Ok(run_id) => session.cancel_run(run_id).await,
                Err(_) => Err(DaChatApiError::RequestFailed),
            };
            match result {
                Ok(()) => finish_run(teardown).await,
                Err(error) => {
                    // Do not reconcile a still-queued DA run or erase the only
                    // recovery evidence when its terminal write did not land.
                    tracing::error!(run = %teardown.run_id.as_str(), %error, "cancelled preparation could not be synchronized");
                    let store = RunJournalStore::new(teardown.app_local_data.clone(), teardown.account.clone());
                    let journal_result = store.load_entries(&teardown.run_id).and_then(|entries| {
                        let sequence = entries.last().map_or(1, |entry| entry.sequence.saturating_add(1));
                        let from = entries.last().map_or(RunState::Queued, |entry| entry.to.clone());
                        store.append_transition(&teardown.run_id, sequence, from, RunState::Interrupted)
                    });
                    if let Err(error) = journal_result {
                        tracing::error!(run = %teardown.run_id.as_str(), %error, "cancelled preparation recovery journal could not be saved");
                    }
                    let _ = emit_chat_inference_event(teardown.app, &InferenceEvent::failed(
                        teardown.run_id.clone(), InferenceErrorCode::Internal,
                        "The response stopped, but its status could not be synchronized. Reopen the conversation to refresh its status.",
                        true,
                    ));
                    teardown.run_cancels.lock().await.remove(&teardown.run_id_string);
                }
            }
            None
        }
        result = step => Some(result),
    }
}

/// Reconciles the conversation, drops the run's on-disk journal if it reached
/// a terminal state, and releases its cancellation token.
///
/// Shared by every path that ends a run so none of them can forget one of the
/// three — a leaked cancel token in particular counts against
/// `MAX_IN_FLIGHT_RUNS` forever.
async fn finish_run<R: tauri::Runtime>(teardown: RunTeardown<'_, R>) {
    let RunTeardown {
        app,
        config,
        bearer,
        da_client,
        tenant_id,
        account,
        app_local_data,
        secrets,
        conversation_id,
        run_id,
        run_id_string,
        run_state,
        run_cancels,
    } = teardown;

    if let Ok(sync) = build_sync(
        config,
        bearer,
        da_client,
        tenant_id,
        account.clone(),
        &app_local_data,
        secrets,
    ) {
        reconcile_conversation_with_retry(app, &sync, conversation_id).await;
    }
    if should_remove_run_journal(&run_state) {
        RunJournalStore::new(app_local_data, account).remove_run(&run_id);
    }
    run_cancels.lock().await.remove(&run_id_string);
}

async fn fail_run_before_execution<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    ui_run_id: &RunId,
    session: &DaChatSession,
    run_id: Uuid,
    model_id: &str,
    message: &str,
) {
    // The reader is told first. This used to finalize the DA run and stop, which
    // left the chat showing "thinking" for a run that had already ended.
    let _ = emit_chat_inference_event(
        app,
        &InferenceEvent::failed(
            ui_run_id.clone(),
            InferenceErrorCode::Internal,
            message,
            true,
        ),
    );
    let result = session
        .finalize_run(
            run_id,
            RunFinalize {
                status: "error".to_owned(),
                expected_from: Some(
                    crate::clients::da::generated::RunFinalizeExpectedFrom::Array(vec![
                        "queued".to_owned(),
                        "running".to_owned(),
                        "streaming".to_owned(),
                        "awaiting_approval".to_owned(),
                    ]),
                ),
                model: model_id.to_owned(),
                error: Some(message.to_owned()),
                completed_at: Some(chrono::Utc::now()),
                ..Default::default()
            },
        )
        .await;
    if let Err(error) = result {
        tracing::error!(run = %run_id, %error, "local run: pre-execution failure could not be finalized");
    }
}

/// Puts the run's system prompt in front of its history.
///
/// Desktop sent none at all before this: every `system_prompt` on the wire was
/// `None` and nothing was seeded, so the model ran with no persona, no SQL
/// contract, and no output contract — which is why "build a dashboard" came
/// back as SQL. The prompt is assembled the way xfalcon-web assembles it
/// (`crate::agent::prompt`), and it has to be built HERE rather than where the
/// history is read, because its tool inventory is a function of the tool
/// session this run actually resolved.
fn with_system_prompt(
    history: Vec<ContextMessage>,
    project: Option<&PromptProject>,
    tools: &[ProviderToolSchema],
    tool_ids: &HashMap<String, ToolId>,
    skills: &[PromptSkill],
    unavailable_servers: &[String],
) -> Vec<ContextMessage> {
    let system = build_system_prompt(&SystemPromptInputs {
        conversation_system_prompt: None,
        project,
        skills,
        tools,
        tool_ids: Some(tool_ids),
        unavailable_servers,
    });
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.push(ContextMessage::new(ContextRole::System, system));
    messages.extend(history);
    messages
}

pub(crate) fn should_remove_run_journal(state: &RunState) -> bool {
    !is_in_flight_run_state(state.clone())
}

/// Whether this device's run journal claims a run, given the entries read for
/// it.
///
/// The discriminator boot recovery turns on. This device journals every
/// transition of every run it executes, so an in-flight DA row with no local
/// transition log was started somewhere else — xFalcon Web executes its own
/// runs against these same conversations. `chat-run-recovery` scopes boot
/// recovery to a "journal-tail run — one whose local transition log ends
/// non-terminal", and this is that scope as a predicate.
///
/// Callers pass `unwrap_or_default()` of the journal read, so a journal that
/// cannot be read counts the same as one that is absent: the run is treated as
/// another client's and left alone. That is the safe direction to fail. The
/// cost is a stale in-flight row on DA, which is the source of truth's to
/// reap; the alternative cost is cancelling an answer streaming in another
/// client.
#[must_use]
pub(crate) fn device_journaled_run(entries: &[JournalEntry]) -> bool {
    !entries.is_empty()
}

async fn reconcile_conversation_with_retry<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    sync: &ChatSyncService<DaChatRepository, EncryptedChatProjection>,
    conversation_id: ConversationId,
) {
    for delay in [
        Duration::ZERO,
        Duration::from_millis(200),
        Duration::from_millis(500),
    ] {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if let Ok(reconciled) = sync.reconcile_conversation(conversation_id.clone()).await {
            let view = conversation_view_from(reconciled.snapshot, reconciled.availability);
            let _ = emit_chat_projection_event(app, &projection_snapshot_replaced(view));
            return;
        }
    }
}

fn build_sync(
    config: &AuthConfig,
    token: &str,
    da_client: &reqwest::Client,
    tenant_id: &str,
    account: ValidatedAccountId,
    app_local_data: &Path,
    secrets: Arc<dyn SecretStore>,
) -> Result<ChatSyncService<DaChatRepository, EncryptedChatProjection>, ChatServiceError> {
    let session = DaChatSession::with_client(config.da_base_url.as_str(), token, da_client.clone())
        .map_err(|_| ChatServiceError::Unavailable)?;
    let cloud = Arc::new(DaChatRepository::new(session).with_expected_tenant(tenant_id));
    let path = projection_db_path(app_local_data, &account);
    let local = EncryptedChatProjection::open(path, secrets, account.clone())
        .map_err(|_| ChatServiceError::Unavailable)?;
    Ok(ChatSyncService::new(cloud, local, account))
}

fn projection_db_path(app_root: &Path, account: &ValidatedAccountId) -> PathBuf {
    app_root
        .join("accounts")
        .join(account_directory_name(account.as_str()))
        .join(LOCAL_PROJECTIONS_DIR)
        .join(CHAT_PROJECTION_DB_FILE)
}

pub(super) fn model_catalog_view(
    models: &[crate::clients::da::generated::ModelEntity],
    selected: &ModelSelection,
    selection_locked: bool,
) -> ChatModelCatalogView {
    let mut listed = supported_models(models);
    // A conversation locked to a model no longer offered — the retired Luna id —
    // still names it, so the composer can show what the conversation runs.
    if !listed
        .iter()
        .any(|model| model.model_id.trim() == selected.model_id)
    {
        listed.extend(
            models
                .iter()
                .find(|model| model.model_id.trim() == selected.model_id),
        );
    }
    let projected = listed
        .into_iter()
        .map(|model| {
            ModelView::new(
                model.model_id.trim(),
                match model.provider.as_str() {
                    "custom_openai" => ProviderId::CustomOpenAi,
                    "claude_code" => ProviderId::ClaudeCode,
                    "chatgpt_plan" => ProviderId::ChatgptPlan,
                    _ => ProviderId::OpenRouter,
                },
                model.display_name.trim(),
                ModelCapabilitiesView::new(
                    model.supports_tools,
                    model.supports_vision,
                    model.supports_reasoning || model.supports_extended_thinking,
                ),
            )
        })
        .collect();
    ChatModelCatalogView::new(projected, selected.model_id.clone(), selection_locked)
}

struct CustomModelConnectionCandidate<'a> {
    base_url: &'a str,
    secret: &'a str,
    upstream_model_id: Option<&'a str>,
    exclude_id: Option<Uuid>,
}

async fn existing_custom_model_connections(
    models: &[crate::clients::da::generated::ModelEntity],
    account: &str,
    candidate: CustomModelConnectionCandidate<'_>,
    repository: &dyn CredentialRepository,
) -> Result<Vec<ExistingCustomModel>, ChatServiceError> {
    use subtle::ConstantTimeEq;

    let Some((tenant_id, user_id)) = account.split_once(':') else {
        return Err(ChatServiceError::Unavailable);
    };
    let mut existing = Vec::new();
    for model in models {
        if candidate.exclude_id == Some(model.id) {
            continue;
        }
        if model.provider != "custom_openai"
            || model.provider_parent != "custom_openai"
            || model.tenant_id != tenant_id
            || model.owner_user_id.as_deref() != Some(user_id)
            || model.model_id != format!("custom/{}", model.id)
        {
            continue;
        }
        let Some(base_url) = model.base_url.as_deref() else {
            continue;
        };
        let Some(upstream_model_id) = model.upstream_model_id.as_deref() else {
            continue;
        };
        let Ok(normalized_base_url) = crate::providers::normalize_base_url_input(base_url) else {
            continue;
        };
        if custom_openai_adapter_with_sampling(
            &normalized_base_url,
            upstream_model_id,
            CustomModelSampling::default(),
        )
        .is_err()
        {
            continue;
        }
        if candidate
            .upstream_model_id
            .is_some_and(|id| id != upstream_model_id)
            || normalized_base_url != candidate.base_url
        {
            continue;
        }
        let key_id = model.provider_key_id.ok_or(ChatServiceError::Unavailable)?;
        let credential = repository
            .resolve_secret(key_id, ProviderId::CustomOpenAi)
            .await
            .map_err(|_| {
                ChatServiceError::Message(
                    "Could not check for existing private models. Try again.".to_owned(),
                )
            })?;
        if !custom_model_connection_credential_matches(key_id, base_url, &credential) {
            return Err(ChatServiceError::Message(
                "Could not check for existing private models. Try again.".to_owned(),
            ));
        }
        if bool::from(
            candidate
                .secret
                .as_bytes()
                .ct_eq(credential.secret.as_bytes()),
        ) {
            // A model name must never reflect the submitted secret back to IPC.
            let name = safe_existing_custom_model_name(&model.display_name, candidate.secret);
            existing.push(ExistingCustomModel {
                upstream_model_id: upstream_model_id.to_owned(),
                display_name: name,
            });
        }
    }
    Ok(existing)
}

fn safe_existing_custom_model_name(display_name: &str, secret: &str) -> String {
    const FALLBACK: &str = "an existing private model";
    if display_name.contains(secret) {
        return FALLBACK.to_owned();
    }
    let replaced: String = display_name
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let normalized = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let bounded: String = normalized.trim().chars().take(128).collect();
    if bounded.is_empty() || bounded.contains(secret) {
        FALLBACK.to_owned()
    } else {
        bounded
    }
}

fn custom_model_duplicate_error(duplicate: &ExistingCustomModel, secret: &str) -> ChatServiceError {
    if duplicate.display_name.contains(secret) {
        ChatServiceError::Message(
            "This model is already added. Review your private models.".to_owned(),
        )
    } else {
        ChatServiceError::Message(format!(
            "This model is already added as “{}”.",
            duplicate.display_name
        ))
    }
}

fn custom_model_views(
    models: &[crate::clients::da::generated::ModelEntity],
    account: &str,
    settings: &CustomModelSettingsStore,
) -> Result<Vec<CustomModelView>, ChatServiceError> {
    let sampling = settings
        .sampling_snapshot()
        .map_err(|_| ChatServiceError::TemporarilyUnavailable)?;
    let mut views = Vec::new();
    for model in models {
        let model_sampling = sampling.get(&model.id).cloned().unwrap_or_default();
        if let Some(view) = custom_model_view(model, account, model_sampling) {
            views.push(view);
        }
    }
    Ok(views)
}

fn custom_model_view(
    model: &crate::clients::da::generated::ModelEntity,
    account: &str,
    sampling: CustomModelSampling,
) -> Option<CustomModelView> {
    let (tenant_id, user_id) = account.split_once(':')?;
    if model.provider != "custom_openai"
        || model.provider_parent != "custom_openai"
        || model.tenant_id != tenant_id
        || model.owner_user_id.as_deref() != Some(user_id)
        || model.provider_key_id.is_none()
        || model.model_id != format!("custom/{}", model.id)
    {
        return None;
    }
    let base_url = model.base_url.as_deref()?;
    let upstream_model_id = model.upstream_model_id.as_deref()?;
    if custom_openai_adapter_with_sampling(base_url, upstream_model_id, sampling.clone()).is_err() {
        return None;
    }
    Some(CustomModelView {
        version: xfalcon_contracts::CONTRACT_VERSION,
        id: model.id.to_string(),
        model_id: model.model_id.clone(),
        display_name: model.display_name.clone(),
        base_url: base_url.to_owned(),
        upstream_model_id: upstream_model_id.to_owned(),
        context_length: model.context_length,
        max_output_tokens: model.max_output_tokens,
        supports_tools: model.supports_tools,
        supports_vision: model.supports_vision,
        sampling,
        is_enabled: model.is_enabled,
        is_active: model.is_active,
    })
}

fn validate_custom_model_update(
    current: &crate::clients::da::generated::ModelEntity,
    input: &CustomModelUpdateInput,
) -> Result<(), ChatServiceError> {
    let base_url = input
        .base_url
        .as_deref()
        .or(current.base_url.as_deref())
        .ok_or_else(|| ChatServiceError::Message("Enter an API base URL.".to_owned()))?;
    let upstream_model_id = input
        .upstream_model_id
        .as_deref()
        .or(current.upstream_model_id.as_deref())
        .ok_or_else(|| ChatServiceError::Message("Enter a model ID.".to_owned()))?;
    let display_name = input
        .display_name
        .as_deref()
        .unwrap_or(&current.display_name);
    let context_length = input.context_length.unwrap_or(current.context_length);
    let max_output_tokens = input
        .max_output_tokens
        .or(current.max_output_tokens)
        .ok_or_else(|| {
            ChatServiceError::Message("Enter a maximum output token limit.".to_owned())
        })?;
    if let Some(secret) = input.secret.as_deref() {
        validate_custom_model_secret(secret)?;
    }
    let sampling = input.sampling.as_ref().cloned().unwrap_or_default();
    validate_custom_model_configuration(
        base_url,
        upstream_model_id,
        display_name,
        context_length,
        max_output_tokens,
        &sampling,
    )
}

fn custom_model_update_requires_connection_test(
    current: &crate::clients::da::generated::ModelEntity,
    input: &CustomModelUpdateInput,
) -> bool {
    input.secret.is_some()
        || input
            .base_url
            .as_deref()
            .is_some_and(|base_url| Some(base_url) != current.base_url.as_deref())
        || input
            .upstream_model_id
            .as_deref()
            .is_some_and(|model_id| Some(model_id) != current.upstream_model_id.as_deref())
        || input
            .max_output_tokens
            .is_some_and(|tokens| Some(tokens) != current.max_output_tokens)
        || input.sampling.is_some()
        || (input.is_enabled == Some(true) && !current.is_enabled)
        || (input.is_active == Some(true) && !current.is_active)
}

fn validate_custom_model_update_connection_security(
    current: &crate::clients::da::generated::ModelEntity,
    input: &CustomModelUpdateInput,
) -> Result<(), ChatServiceError> {
    let changes_base_url = input
        .base_url
        .as_deref()
        .is_some_and(|base_url| Some(base_url) != current.base_url.as_deref());
    if changes_base_url && input.secret.is_none() {
        return Err(ChatServiceError::Message(
            "Enter a new API key when changing the endpoint URL.".to_owned(),
        ));
    }
    Ok(())
}

fn custom_model_connection_credential_matches(
    provider_key_id: Uuid,
    current_base_url: &str,
    credential: &ResolvedCredential,
) -> bool {
    credential.credential_id == provider_key_id.to_string()
        && credential.base_url.as_deref() == Some(current_base_url)
}

fn map_custom_model_connection_credential_error(error: CredentialServiceError) -> ChatServiceError {
    let message = match error {
        CredentialServiceError::Unauthorized => "Sign in again to test this private model.",
        CredentialServiceError::RateLimited
        | CredentialServiceError::ProviderUnavailable
        | CredentialServiceError::StorageUnavailable
        | CredentialServiceError::ServiceUnavailable
        | CredentialServiceError::ReconciliationRequired { .. } => {
            "Could not load this private model's saved API key. Try again."
        }
        CredentialServiceError::InvalidRequest
        | CredentialServiceError::InvalidCredential
        | CredentialServiceError::InsufficientPermissions
        | CredentialServiceError::Conflict
        | CredentialServiceError::NoUsableCredential
        | CredentialServiceError::NotFound => "Enter the API key again to test this private model.",
    };
    ChatServiceError::Message(message.to_owned())
}

fn validate_custom_model_secret(secret: &str) -> Result<(), ChatServiceError> {
    if secret.is_empty()
        || secret.len() > MAX_PROVIDER_KEY_BYTES
        || secret.trim() != secret
        || secret.bytes().any(|byte| byte < 33 || byte == 127)
    {
        Err(ChatServiceError::Message(
            "Enter a valid API key.".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn validate_custom_model_configuration(
    base_url: &str,
    upstream_model_id: &str,
    display_name: &str,
    context_length: i64,
    max_output_tokens: i64,
    sampling: &CustomModelSampling,
) -> Result<(), ChatServiceError> {
    if sampling
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err(ChatServiceError::Message(
            "sampling.temperature must be between 0 and 2.".to_owned(),
        ));
    }
    if sampling
        .top_p
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(ChatServiceError::Message(
            "sampling.top_p must be between 0 and 1.".to_owned(),
        ));
    }
    custom_openai_adapter_with_sampling(base_url, upstream_model_id, sampling.clone()).map_err(
        |_| {
            ChatServiceError::Message(
                "Enter a valid API base URL, such as http://localhost:8000/v1.".to_owned(),
            )
        },
    )?;
    if display_name.trim() != display_name
        || display_name.is_empty()
        || display_name.chars().count() > 128
        || display_name.bytes().any(|byte| byte < 32 || byte == 127)
    {
        return Err(ChatServiceError::Message(
            "Enter a model name with 1 to 128 visible characters.".to_owned(),
        ));
    }
    if context_length <= 0
        || context_length > i64::from(i32::MAX)
        || max_output_tokens <= 0
        || max_output_tokens > i64::from(i32::MAX)
        || max_output_tokens >= context_length
    {
        return Err(ChatServiceError::Message(
            "context_length and max_output_tokens must be positive whole numbers, with output smaller than context.".to_owned(),
        ));
    }
    crate::agent::context_budget::ContextCapacity::new(context_length, max_output_tokens)
        .map_err(|_| {
            ChatServiceError::Message(format!(
                "context_length must leave room for model output and {} reserved tokens for xFalcon's instructions and tools.",
                crate::agent::context_budget::PROTOCOL_SAFETY_TOKENS,
            ))
        })?;
    Ok(())
}

#[cfg(test)]
impl ChatService {
    pub(crate) async fn set_send_in_flight_for_tests(&self, in_flight: bool) {
        *self.send_in_flight.lock().await = in_flight;
    }
}

fn project_run_stop_timeout() -> ChatServiceError {
    ChatServiceError::Message(
        "Project conversations are still stopping. Try again shortly.".to_owned(),
    )
}

fn parse_conversation_uuid(id: &ConversationId) -> Result<Uuid, ChatServiceError> {
    Uuid::parse_str(id.as_str())
        .map_err(|_| ChatServiceError::Message("Conversation id is invalid.".to_owned()))
}

pub(crate) fn map_chat_error(error: crate::chat::domain::ChatError) -> ChatServiceError {
    match error {
        crate::chat::domain::ChatError::Unauthorized => ChatServiceError::SessionExpired,
        crate::chat::domain::ChatError::NotFound => ChatServiceError::NotFound,
        crate::chat::domain::ChatError::Unavailable => ChatServiceError::Unavailable,
        // The remaining variants each carry a precise Display string; with no
        // log stream on this app, the surfaced message is the only diagnostic.
        other => ChatServiceError::Message(other.to_string()),
    }
}

fn map_revision_error(error: DaChatApiError) -> ChatServiceError {
    match error {
        DaChatApiError::NotFound => {
            ChatServiceError::Message("Message not found in conversation.".to_owned())
        }
        error => map_da_error(error),
    }
}

/// Separates a workspace failure that passes on its own from one that does not.
///
/// `RequestFailed` is both a transport failure and DA's own 5xx (see
/// `map_error_response`), and a redeploy produces one or the other for as long
/// as it runs. Folding those in with a contract violation is what made a
/// two-minute outage read the same as a permanent fault, and left the reader
/// with no reason to simply wait.
fn map_da_error(error: DaChatApiError) -> ChatServiceError {
    match error {
        DaChatApiError::Unauthorized => ChatServiceError::SessionExpired,
        DaChatApiError::CustomModelUnavailable => ChatServiceError::Message(
            "This private model was removed or disabled. Select another model in Settings → Models."
                .to_owned(),
        ),
        DaChatApiError::CustomModelCapacityUnsupported => ChatServiceError::Message(
            "This private model does not have enough context room to be used. Remove it in Settings → Models and add the company model."
                .to_owned(),
        ),
        DaChatApiError::NotFound => {
            ChatServiceError::Message("Conversation was not found.".to_owned())
        }
        DaChatApiError::RequestFailed | DaChatApiError::Timeout | DaChatApiError::RateLimited => {
            ChatServiceError::TemporarilyUnavailable
        }
        _ => ChatServiceError::Unavailable,
    }
}

fn binding_from_metadata(
    metadata: Option<indexmap::IndexMap<String, serde_json::Value>>,
) -> Result<McpConversationBinding, ChatServiceError> {
    let metadata = metadata
        .unwrap_or_default()
        .into_iter()
        .collect::<serde_json::Map<_, _>>();
    McpConversationBinding::from_metadata(&metadata).map_err(map_mcp_binding_error)
}

fn map_mcp_binding_error(_error: BindingError) -> ChatServiceError {
    invalid_mcp_binding_error()
}

fn invalid_mcp_binding_error() -> ChatServiceError {
    ChatServiceError::Message(
        "This chat's MCP server selection is invalid. Start a new chat.".to_owned(),
    )
}

fn map_mcp_selection_error(
    error: xfalcon_contracts::mcp::McpServerOperationError,
) -> ChatServiceError {
    tracing::warn!(code = ?error.code, "chat MCP selection is unavailable");
    ChatServiceError::Message(
        "The selected MCP server changed or is unavailable. Start a new chat.".to_owned(),
    )
}

fn validate_requested_selection(
    binding: &McpConversationBinding,
    selected: Option<&McpServerId>,
) -> Result<(), ChatServiceError> {
    if binding.matches_selection(selected) {
        Ok(())
    } else {
        Err(ChatServiceError::Message(
            "This chat is bound to a different MCP server. Start a new chat to change it."
                .to_owned(),
        ))
    }
}

fn conversation_owned_by(
    conversation: &crate::clients::da::generated::Conversation,
    tenant_id: &str,
    account: &ValidatedAccountId,
) -> bool {
    let Some((_, user_id)) = account.as_str().rsplit_once(':') else {
        return false;
    };
    conversation.tenant_id == tenant_id && conversation.user_id == user_id
}

fn custom_model_preflight_error(error: ChatServiceError) -> ChatServiceError {
    match error {
        ChatServiceError::AuthRequired | ChatServiceError::SessionExpired => error,
        _ => ChatServiceError::Message(
            "Could not check for existing private models. Try again.".to_owned(),
        ),
    }
}

fn map_custom_model_error(error: CustomModelApiError) -> ChatServiceError {
    match error {
        CustomModelApiError::Unauthorized => ChatServiceError::SessionExpired,
        CustomModelApiError::Unavailable | CustomModelApiError::RateLimited => {
            ChatServiceError::TemporarilyUnavailable
        }
        error => ChatServiceError::Message(error.to_string()),
    }
}

#[cfg(test)]
mod active_run_tests {
    use super::*;

    #[test]
    fn active_run_admission_is_scoped_to_one_conversation() {
        let occupied = ConversationId::from(Uuid::new_v4().to_string());
        let other = ConversationId::from(Uuid::new_v4().to_string());
        let runs = HashMap::from([(
            "run-1".to_owned(),
            ActiveLocalRun {
                account: ValidatedAccountId::parse("account-1").expect("account"),
                conversation_id: occupied.clone(),
                cancel: CancellationToken::new(),
                teardown_failed: false,
            },
        )]);

        assert!(has_active_run_for_conversation(&runs, &occupied));
        assert!(!has_active_run_for_conversation(&runs, &other));
    }
}

#[cfg(test)]
#[path = "tests/preparation_cancel.rs"]
mod preparation_cancel_tests;
