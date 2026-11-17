//! The live MCP sessions a run draws tools from, and how one advertised tool
//! becomes a registry tool.
//!
//! [`super::client::connect`] establishes a session and [`super::catalog`]
//! caches what it advertises, but nothing owned either across a run. This
//! module does: one lazily-established session per server per app, shared by
//! every run, reconnected on demand when the transport dies, held in a bounded
//! [`McpRuntimeSet`] alongside the bundled tenant server.
//!
//! # One session per server, not one per run
//!
//! An MCP session is a real network connection with a handshake and a
//! server-side session id. Opening one per run would pay that cost on every
//! message and multiply each server's session count by the user's chat
//! activity. Sharing is safe because `rmcp` multiplexes concurrent requests
//! over a single peer, and the only per-run state — policy, budgets, the
//! journal — lives in `crate::agent::tool_loop::ToolLoop`, not here.
//!
//! # Whose risk hints, and how much they buy
//!
//! MCP tools carry no risk classification this app defines, only the protocol's
//! optional annotation block ([`McpToolHints`]). Those annotations alone never
//! decide anything: [`ToolAdmission`] combines them with the server's
//! DA-registered `permission_mode`, and the default in every direction is the
//! one that requires human approval — including for a tool that claims to be
//! both read-only and destructive. A server's self-declaration is not authority
//! to mutate unreviewed
//! (`docs/decisions.md`,
//! D6).

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::mcp::{McpPermissionMode, BUNDLED_SERVER_ID};
use xfalcon_contracts::tools::{ToolRisk, ToolSource};
use zeroize::Zeroizing;

use crate::auth::{AuthConfig, AuthSessionStore};
use crate::tools::registry::{ToolDescriptor, ToolDescriptorError, ToolRegistry};

use super::auth::{McpAuth, McpAuthError, McpAuthProvider, RegisteredAuth};
use super::catalog::{McpToolCatalog, McpToolCatalogCache, McpToolHints};
use super::client::{connect_with_reconnect, McpClientError, McpSession, ReconnectPolicy};
use super::credential_store::DeviceCredentials;
use super::oauth::{
    self,
    envelope::GrantKey,
    exchange::ExchangeError,
    registration::OauthClient,
    shared_grant::SharedGrantError,
    store::{DeviceGrants, StoredGrant},
};
use super::registry::{DialableServer, ServerAuthRequirement};
use super::server_resolver::{McpServerDescriptor, McpServerResolver};
use crate::clients::da::mcp_oauth_grants::DaMcpGrantRepository;

/// Prefix every MCP-sourced tool id carries, ahead of its server's slug.
///
/// Namespacing is not cosmetic: the effective registry mixes this app's own
/// `project.*` tools with whatever every connected server publishes, and a
/// server that advertised a tool literally named `project.read_file` would
/// otherwise shadow the internal one — routing a read of the user's trusted
/// project to a remote server.
pub(crate) const MCP_TOOL_ID_PREFIX: &str = "mcp.";

/// How many servers may hold a session at once, the bundled one included.
///
/// The binding constraint is not sockets, it is the model's attention: every
/// held session contributes its whole catalog (up to
/// `super::catalog`'s 512-tool ceiling, each with a compiled JSON Schema
/// validator) to one declaration list. Past a handful of servers a run is
/// paying context and memory for tools it will never call. Registrations beyond
/// this cap are still listed in Settings — they are recorded as
/// [`ServerExclusion::SessionLimit`] rather than silently ignored.
pub(crate) const MAX_CONCURRENT_SERVER_SESSIONS: usize = 32;

/// How long one server gets to produce a usable catalog before a run gives up
/// on it.
///
/// [`super::client::connect_with_reconnect`]'s own budget (four attempts with
/// backoff, each with its own transport timeouts) is sized for a session that
/// must eventually succeed. A run is different: the user asked a question, and
/// a server that cannot answer "what tools do you have" promptly must not hold
/// the whole turn. The excluded server is retried on the next run.
const REGISTRATION_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpRuntimeError {
    #[error(transparent)]
    Auth(#[from] McpAuthError),
    #[error(transparent)]
    Connect(#[from] McpClientError),
    #[error("the server's tool catalog could not be read")]
    Catalog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionInvalidation {
    LatestGeneration,
    StaleGeneration,
}

/// Binds the signed-in session's bearer to the bundled server.
///
/// `McpAuth::bind` requires proof of the audience the bearer was minted for,
/// which the OAuth flow records on the session store at login
/// (`AuthSessionStore::set_authorized_resource`). Re-running discovery here
/// instead would be a second, racy source of truth for the same fact.
pub(crate) struct SessionMcpAuthProvider {
    sessions: Arc<AuthSessionStore>,
    expected_account_id: Option<String>,
}

impl SessionMcpAuthProvider {
    #[must_use]
    pub(crate) fn new(sessions: Arc<AuthSessionStore>) -> Self {
        Self {
            sessions,
            expected_account_id: None,
        }
    }

    #[must_use]
    pub(crate) fn for_account(
        sessions: Arc<AuthSessionStore>,
        expected_account_id: String,
    ) -> Self {
        Self {
            sessions,
            expected_account_id: Some(expected_account_id),
        }
    }
}

#[async_trait::async_trait]
impl McpAuthProvider for SessionMcpAuthProvider {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        let snapshot = self
            .sessions
            .authenticated_snapshot()
            .ok_or(McpAuthError::MissingBearer)?;
        if self.expected_account_id.as_ref().is_some_and(|expected| {
            crate::auth::canonical_account_id(&snapshot.identity)
                .map_or(true, |actual| actual != *expected)
        }) {
            return Err(McpAuthError::MissingBearer);
        }
        let resource = snapshot
            .authorized_resource
            .as_deref()
            .ok_or(McpAuthError::AudienceMismatch)?;
        McpAuth::bind(snapshot.access_token, resource, server)
    }
}

/// Binds a registered server's device-held credential to its requests.
///
/// Reads from the store on every bind rather than holding the value: the store
/// is the only place it lives, and a credential rotated in Settings has to take
/// effect on the next reconnect without rebuilding the runtime. The signed-in
/// session bearer is structurally out of reach here — this provider can only
/// ever return what was sealed under this server's own DA row id.
pub(crate) struct DeviceCredentialAuthProvider {
    credentials: DeviceCredentials,
    server_id: String,
    requirement: RegisteredAuth,
}

impl DeviceCredentialAuthProvider {
    #[must_use]
    pub(crate) fn new(
        credentials: DeviceCredentials,
        server_id: impl Into<String>,
        requirement: RegisteredAuth,
    ) -> Self {
        Self {
            credentials,
            server_id: server_id.into(),
            requirement,
        }
    }
}

#[async_trait::async_trait]
impl McpAuthProvider for DeviceCredentialAuthProvider {
    async fn current_auth(&self, _server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        if matches!(self.requirement, RegisteredAuth::None) {
            // An open server takes nothing, so it must not be blocked on a
            // credential lookup that would find nothing either.
            return McpAuth::bind_registered(Zeroizing::new(String::new()), &self.requirement);
        }
        let credential = self
            .credentials
            .get(&self.server_id)
            .map_err(|_| McpAuthError::CredentialUnavailable)?
            .ok_or(McpAuthError::MissingBearer)?;
        McpAuth::bind_registered(credential, &self.requirement)
    }
}

/// Binds the access token of a grant this device obtained through
/// `super::oauth`, renewing it first when it is close enough to expiry to be
/// refused mid-session.
///
/// Renewal happens here — on the connect path — rather than on a timer, because
/// the timer version is wrong in both directions: it renews grants for servers
/// nothing is using, and it attaches an expired token whenever a connect happens
/// to precede the next tick. Doing it at bind time means a token is renewed
/// exactly when one is about to be used, and never otherwise.
///
/// A rejected refresh deletes the unusable grant and returns `MissingBearer`,
/// which requested tool calls and explicit connection checks surface as a
/// sign-in prompt. Transient failures retain the stored grant; they do not
/// force a new authorization.
pub(crate) struct OauthGrantAuthProvider {
    grants: DeviceGrants,
    server_id: String,
    /// Serializes renewal so two concurrent connects to the same server do not
    /// both spend the refresh token. An authorization server that rotates refresh
    /// tokens invalidates the first response when the second request arrives, so
    /// the loser of that race would poison a grant that was working.
    renewal: Arc<AsyncMutex<()>>,
}

impl OauthGrantAuthProvider {
    #[must_use]
    pub(crate) fn new(grants: DeviceGrants, server_id: impl Into<String>) -> Self {
        Self {
            grants,
            server_id: server_id.into(),
            renewal: Arc::new(AsyncMutex::new(())),
        }
    }

    /// The grant to use, renewed if due.
    async fn current_grant(
        &self,
        server: &McpServerDescriptor,
    ) -> Result<StoredGrant, McpAuthError> {
        let stored = self
            .grants
            .get(&self.server_id)
            .map_err(|_| McpAuthError::CredentialUnavailable)?
            .ok_or(McpAuthError::MissingBearer)?;
        // Registry URLs can change independently of the sealed grant. Check
        // before both token use and renewal, including the re-read under lock.
        if !super::auth::resource_matches_server(&stored.resource, server) {
            return Err(McpAuthError::AudienceMismatch);
        }
        if !stored.needs_renewal() {
            return Ok(stored);
        }

        let _guard = self.renewal.lock().await;
        // Re-read under the lock: the connect that held it before this one may
        // have already renewed, and spending the refresh token twice is exactly
        // what the lock exists to prevent.
        let stored = self
            .grants
            .get(&self.server_id)
            .map_err(|_| McpAuthError::CredentialUnavailable)?
            .ok_or(McpAuthError::MissingBearer)?;
        // Registry URLs can change independently of the sealed grant. Check
        // before both token use and renewal, including the re-read under lock.
        if !super::auth::resource_matches_server(&stored.resource, server) {
            return Err(McpAuthError::AudienceMismatch);
        }
        if !stored.needs_renewal() {
            return Ok(stored);
        }
        if !stored.is_renewable() {
            return Ok(stored);
        }

        let Ok(http) = oauth::build_client() else {
            return Ok(stored);
        };
        match oauth::flow::renew(&http, &stored).await {
            Ok(renewed) => {
                // A renewal that cannot be persisted is still usable for this
                // connection; the next one renews again rather than failing.
                let _ = self.grants.put(&self.server_id, &renewed);
                Ok(renewed)
            }
            Err(ExchangeError::Refused) => {
                // The refresh token is dead. Dropping the whole grant is what
                // turns "every reconnect quietly fails" into "this server asks
                // you to sign in again".
                let _ = self.grants.delete(&self.server_id);
                Err(McpAuthError::MissingBearer)
            }
            // Unreachable or malformed: keep what is stored and let the server
            // decide whether it still accepts it.
            Err(ExchangeError::RequestFailed | ExchangeError::Invalid) => Ok(stored),
        }
    }
}

/// What one row needs to authenticate from the account-wide grant, assembled by
/// `super::commands` because it needs the signed-in session to reach DA.
pub(crate) struct SharedGrantBinding {
    pub(crate) grants: Arc<DaMcpGrantRepository>,
    pub(crate) key: Arc<GrantKey>,
    /// The OAuth client from the server row's `auth_config` — the one the web app
    /// registered.
    pub(crate) client: OauthClient,
    pub(crate) scope: Option<String>,
    /// `auth_config.resource_metadata_url`, when the operator set one on the
    /// shared row. Carried so a sign-in honours it exactly as web does; a
    /// renewal never needs it, because the coordinates it uses were stored when
    /// the grant was established.
    pub(crate) resource_metadata_url: Option<String>,
}

/// Binds the access token of the **shared** grant DA holds for this server,
/// renewing it through the same lease protocol xfalcon-web uses.
///
/// This is the provider that makes one sign-in serve the whole account: the row
/// it reads is the row web reads (`crate::clients::da::mcp_oauth_grants`), the
/// ciphertext is the format web writes (`super::oauth::envelope`), and the
/// refresh discipline is web's, so the two can interleave against one credential
/// without destroying it.
///
/// Selected over [`OauthGrantAuthProvider`] only when a shared key-encryption key
/// is configured *and* the server row carries an OAuth client this build could
/// decrypt. Without both, the device-sealed grant is used instead and the two
/// products simply do not share a sign-in.
pub(crate) struct SharedGrantAuthProvider {
    grants: Arc<DaMcpGrantRepository>,
    key: Arc<GrantKey>,
    server_id: String,
    client: OauthClient,
    scope: Option<String>,
}

impl SharedGrantAuthProvider {
    #[must_use]
    pub(crate) fn new(
        grants: Arc<DaMcpGrantRepository>,
        key: Arc<GrantKey>,
        server_id: impl Into<String>,
        client: OauthClient,
        scope: Option<String>,
    ) -> Self {
        Self {
            grants,
            key,
            server_id: server_id.into(),
            client,
            scope,
        }
    }
}

#[async_trait::async_trait]
impl McpAuthProvider for SharedGrantAuthProvider {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        let http = oauth::build_client().map_err(|_| McpAuthError::CredentialUnavailable)?;
        let bearer = oauth::shared_grant::resolve_bearer(
            &http,
            &self.grants,
            &self.key,
            &self.server_id,
            server,
            &oauth::shared_grant::SharedGrantClient {
                client: &self.client,
                scope: self.scope.as_deref(),
            },
        )
        .await
        .map_err(|error| match error {
            // "Sign in again" and "this device cannot read what is stored" are
            // different problems with different fixes, and the connect-failure
            // mapping in `super::commands` distinguishes them for the user.
            SharedGrantError::ReauthorizationRequired => McpAuthError::MissingBearer,
            SharedGrantError::NotConfigured
            | SharedGrantError::Unavailable
            | SharedGrantError::Transient
            | SharedGrantError::RenewalRefused => McpAuthError::CredentialUnavailable,
        })?;
        McpAuth::bind_registered(bearer, &RegisteredAuth::Oauth)
    }
}

#[async_trait::async_trait]
impl McpAuthProvider for OauthGrantAuthProvider {
    async fn current_auth(&self, server: &McpServerDescriptor) -> Result<McpAuth, McpAuthError> {
        let grant = self.current_grant(server).await?;
        McpAuth::bind(
            Zeroizing::new(grant.access_token.clone()),
            &grant.resource,
            server,
        )
    }

    /// Drops the grant the server just refused, so the next attempt runs the
    /// authorization flow instead of attaching it again.
    ///
    /// The same reasoning as [`Self::current_grant`]'s dead-refresh-token branch,
    /// which already deletes for exactly this reason: keeping a credential the
    /// server has rejected turns "sign in again and it works" into "every
    /// reconnect quietly fails".
    fn forget(&self) {
        let _ = self.grants.delete(&self.server_id);
    }
}

/// Owns one MCP server's session for the app's lifetime.
pub(crate) struct McpRuntime {
    slug: String,
    /// The DA row this runtime dials, when it has one.
    ///
    /// The slug namespaces tool ids; this is the id `mcp_server_sign_in` takes,
    /// and a reconnect prompt that cannot supply it cannot be acted on. `None`
    /// for the bundled server, which has no row and whose credential is the
    /// app's own session — a refusal there is an app sign-in problem, not a
    /// per-server one, and offering to reconnect a server would send the user
    /// somewhere that cannot fix it.
    identity: Option<McpServerIdentity>,
    permission_mode: McpPermissionMode,
    server: McpServerDescriptor,
    auth: Arc<dyn McpAuthProvider>,
    session: AsyncMutex<McpSessionSlot>,
    catalog: McpToolCatalogCache,
}

#[derive(Default)]
struct McpSessionSlot {
    current: Option<Arc<McpSession>>,
    /// Generation of the most recent successful handshake. Kept when
    /// `current` is cleared so another failure from that same session remains
    /// authoritative until a replacement handshake succeeds.
    latest_generation: Option<Arc<()>>,
}

/// How a reconnect prompt names and reaches one server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpServerIdentity {
    pub(crate) id: String,
    /// What to call it: the row's display name when set, else its name.
    pub(crate) label: String,
}

impl McpRuntime {
    /// The bundled tenant server: endpoint derived from trusted configuration,
    /// authenticated with the signed-in session's audience-bound bearer, and
    /// always reviewed — it holds the reserved [`BUNDLED_SERVER_ID`] slug so no
    /// DA row can shadow its tool namespace.
    #[must_use]
    pub(crate) fn bundled(config: &AuthConfig, sessions: Arc<AuthSessionStore>) -> Self {
        Self::new(
            BUNDLED_SERVER_ID.to_owned(),
            McpPermissionMode::Ask,
            McpServerResolver::resolve(config),
            Arc::new(SessionMcpAuthProvider::new(sessions)),
        )
    }

    #[must_use]
    pub(crate) fn bundled_for_account(
        config: &AuthConfig,
        sessions: Arc<AuthSessionStore>,
        expected_account_id: String,
    ) -> Self {
        Self::new(
            BUNDLED_SERVER_ID.to_owned(),
            McpPermissionMode::Ask,
            McpServerResolver::resolve(config),
            Arc::new(SessionMcpAuthProvider::for_account(
                sessions,
                expected_account_id,
            )),
        )
    }

    #[must_use]
    pub(crate) fn selected_bundled_for_account(
        config: &AuthConfig,
        sessions: Arc<AuthSessionStore>,
        expected_account_id: String,
    ) -> Self {
        Self::new(
            format!("selected_{BUNDLED_SERVER_ID}"),
            McpPermissionMode::Ask,
            McpServerResolver::resolve(config),
            Arc::new(SessionMcpAuthProvider::for_account(
                sessions,
                expected_account_id,
            )),
        )
    }

    /// A registered server this build and this device can dial, authenticating
    /// with whichever material its row calls for: a value the user pasted, a grant
    /// this device holds, or the account-wide grant DA holds.
    ///
    /// Every store is passed because the choice is per server and
    /// `super::registry` has already made it — reading the requirement here keeps
    /// the mapping in one place instead of having the caller pre-select a store.
    /// `shared` is `Some` only when a shared key-encryption key is configured and
    /// this row's OAuth client could be decrypted with it; that is what selects
    /// the account-wide grant over the device-held one.
    #[must_use]
    pub(crate) fn for_dialable(
        server: &DialableServer,
        credentials: DeviceCredentials,
        grants: DeviceGrants,
        shared: Option<SharedGrantBinding>,
    ) -> Self {
        Self::for_dialable_with_slug(server, server.slug.clone(), credentials, grants, shared)
    }

    /// A selected chat uses an id-derived namespace so a server rename or a
    /// later slug reassignment cannot authorize tool calls from another row's
    /// history.
    #[must_use]
    pub(crate) fn for_selected_dialable(
        server: &DialableServer,
        credentials: DeviceCredentials,
        grants: DeviceGrants,
        shared: Option<SharedGrantBinding>,
    ) -> Self {
        Self::for_dialable_with_slug(
            server,
            format!("selected_{}", server.id),
            credentials,
            grants,
            shared,
        )
    }

    fn for_dialable_with_slug(
        server: &DialableServer,
        slug: String,
        credentials: DeviceCredentials,
        grants: DeviceGrants,
        shared: Option<SharedGrantBinding>,
    ) -> Self {
        let auth: Arc<dyn McpAuthProvider> = match &server.auth {
            ServerAuthRequirement::Oauth => match shared {
                Some(shared) => Arc::new(SharedGrantAuthProvider::new(
                    shared.grants,
                    shared.key,
                    server.id.clone(),
                    shared.client,
                    shared.scope,
                )),
                None => Arc::new(OauthGrantAuthProvider::new(grants, server.id.clone())),
            },
            ServerAuthRequirement::None
            | ServerAuthRequirement::Bearer
            | ServerAuthRequirement::Header { .. } => Arc::new(DeviceCredentialAuthProvider::new(
                credentials,
                server.id.clone(),
                RegisteredAuth::from(&server.auth),
            )),
        };
        let runtime = Self::new(
            slug,
            server.permission_mode,
            server.descriptor.clone(),
            auth,
        );
        if matches!(server.auth, ServerAuthRequirement::Oauth) {
            runtime.with_identity(McpServerIdentity {
                id: server.id.clone(),
                label: server.label.clone(),
            })
        } else {
            runtime
        }
    }

    /// A runtime over any already-validated descriptor and auth provider. Kept
    /// crate-visible so tests can drive a real fixture server without a DA row
    /// or a device credential; production goes through [`Self::bundled`] /
    /// [`Self::for_dialable`], which are the only paths that can name an
    /// endpoint.
    #[must_use]
    pub(crate) fn new(
        slug: String,
        permission_mode: McpPermissionMode,
        server: McpServerDescriptor,
        auth: Arc<dyn McpAuthProvider>,
    ) -> Self {
        Self {
            slug,
            identity: None,
            permission_mode,
            server,
            auth,
            session: AsyncMutex::new(McpSessionSlot::default()),
            catalog: McpToolCatalogCache::new(),
        }
    }

    /// Names the OAuth row a reconnect can authorize. Bundled and non-OAuth
    /// servers deliberately keep `None`: browser sign-in cannot repair them.
    #[must_use]
    pub(crate) fn with_identity(mut self, identity: McpServerIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    #[must_use]
    pub(crate) fn slug(&self) -> &str {
        &self.slug
    }

    #[must_use]
    pub(crate) fn identity(&self) -> Option<&McpServerIdentity> {
        self.identity.as_ref()
    }

    /// Forgets the credential this server refused, so the next attempt
    /// authorizes afresh rather than replaying it.
    pub(crate) fn forget_credential(&self) {
        self.auth.forget();
    }

    /// The provider this runtime authenticates with.
    ///
    /// Test-only, and the point of it is the mapping rather than the provider: it
    /// is what lets a test prove an `oauth` row reads the grant store and a
    /// `bearer` row reads the credential store, without standing up a server to
    /// observe the header.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn auth_for_test(&self) -> &dyn McpAuthProvider {
        self.auth.as_ref()
    }

    #[must_use]
    pub(crate) fn permission_mode(&self) -> McpPermissionMode {
        self.permission_mode
    }

    /// The live session and a catalog fresh enough for it, connecting first if
    /// needed.
    ///
    /// A session whose transport has closed is discarded and replaced rather
    /// than returned: every call through it would fail, and the caller has no
    /// better way to notice.
    pub(crate) async fn connected(
        &self,
        cancel: &CancellationToken,
    ) -> Result<(Arc<McpSession>, Arc<McpToolCatalog>), McpRuntimeError> {
        self.connected_with_failure_action(cancel, |_| {}).await
    }

    /// [`Self::connected`] with a synchronous action for the terminal failure.
    /// The action runs while the session-generation lock is still held, so a
    /// refused credential can be removed before another caller renews it and
    /// installs a replacement session.
    pub(crate) async fn connected_with_failure_action<F>(
        &self,
        cancel: &CancellationToken,
        on_failure: F,
    ) -> Result<(Arc<McpSession>, Arc<McpToolCatalog>), McpRuntimeError>
    where
        F: Fn(&McpRuntimeError),
    {
        loop {
            let session = {
                let mut guard = self.session.lock().await;
                if guard
                    .current
                    .as_ref()
                    .is_some_and(|session| session.is_closed())
                {
                    guard.current = None;
                }
                match guard.current.as_ref() {
                    Some(session) => Arc::clone(session),
                    None => {
                        let fresh = match connect_with_reconnect(
                            &self.server,
                            self.auth.as_ref(),
                            &ReconnectPolicy::default(),
                            cancel,
                        )
                        .await
                        {
                            Ok(fresh) => Arc::new(fresh),
                            Err(error) => {
                                let error = McpRuntimeError::Connect(error);
                                on_failure(&error);
                                return Err(error);
                            }
                        };
                        guard.latest_generation = Some(Arc::clone(fresh.cache_generation()));
                        guard.current = Some(Arc::clone(&fresh));
                        fresh
                    }
                }
            };

            match self.catalog.get_or_refresh(session.as_ref()).await {
                Ok(catalog) => return Ok((session, catalog)),
                Err(_) => {
                    let error = session
                        .take_refusal()
                        .map(|refusal| McpRuntimeError::Connect(refusal.into()))
                        .unwrap_or(McpRuntimeError::Catalog);
                    if self.invalidate_with(&session, || on_failure(&error)).await
                        == SessionInvalidation::LatestGeneration
                    {
                        return Err(error);
                    }
                    // Another failure already replaced this session. Retry
                    // against that replacement instead of surfacing a stale
                    // catalog/refusal or clearing its catalog.
                }
            }
        }
    }

    /// Drops `session` if it is still current, so the next call reconnects.
    ///
    /// Several calls can fail together after one transport dies. The first
    /// failure may already have installed a replacement by the time a later
    /// failure arrives, so invalidation must compare the exact captured
    /// session while holding the runtime lock. Catalog reuse checks the
    /// generation independently, so clearing a session never needs to wait
    /// for a catalog refresh holding its own lock across network I/O.
    pub(crate) async fn invalidate(&self, session: &Arc<McpSession>) -> SessionInvalidation {
        self.invalidate_with(session, || {}).await
    }

    pub(crate) async fn invalidate_with<F>(
        &self,
        session: &Arc<McpSession>,
        on_latest: F,
    ) -> SessionInvalidation
    where
        F: FnOnce(),
    {
        let mut guard = self.session.lock().await;
        if !guard
            .latest_generation
            .as_ref()
            .is_some_and(|generation| Arc::ptr_eq(generation, session.cache_generation()))
        {
            return SessionInvalidation::StaleGeneration;
        }
        if guard
            .current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            guard.current = None;
        }
        on_latest();
        SessionInvalidation::LatestGeneration
    }
}

/// Why a server this device could dial is nonetheless absent from a
/// [`McpRuntimeSet`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerExclusion {
    /// `permission_mode = deny`. Its tools are withheld from the model, so
    /// holding a session for it would buy nothing.
    ToolsWithheld,
    /// More dialable servers than [`MAX_CONCURRENT_SERVER_SESSIONS`].
    SessionLimit,
}

/// One server left out of a set, and why. Recorded rather than dropped: a
/// caller asked to explain a missing tool needs the reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExcludedServer {
    pub(crate) slug: String,
    pub(crate) reason: ServerExclusion,
}

/// What one server contributed to a run's registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerContribution {
    /// Registered this many tools.
    Served(usize),
    /// Could not connect, or could not read a usable catalog, inside
    /// [`REGISTRATION_DEADLINE`]. The run proceeds without it.
    Unreachable,
    /// Connected, and contributed nothing: an empty catalog, or every
    /// descriptor rejected or colliding with one already registered.
    NothingRegistrable,
}

/// Every MCP server one run may draw tools from.
///
/// Bounded by construction ([`MAX_CONCURRENT_SERVER_SESSIONS`]) and ordered:
/// the bundled server first, then registered servers in DA's order, so the
/// provider-facing wire names a run declares are stable across runs.
pub(crate) struct McpRuntimeSet {
    /// A `Vec` rather than a map: it is capped at
    /// [`MAX_CONCURRENT_SERVER_SESSIONS`] entries, and every
    /// access is either a full iteration (registration) or one slug lookup
    /// (dispatch), for which a linear scan beats hashing.
    runtimes: Vec<Arc<McpRuntime>>,
    excluded: Vec<ExcludedServer>,
}

impl McpRuntimeSet {
    /// Builds the set, applying the two rules that keep it bounded and honest:
    /// a `deny` server is not held at all, and the cap truncates the rest.
    ///
    /// The optional bundled server is inserted first when this account has it
    /// enabled. It is never excluded by permission mode or by the session cap.
    #[must_use]
    pub(crate) fn new(bundled: Option<Arc<McpRuntime>>, registered: Vec<Arc<McpRuntime>>) -> Self {
        let mut runtimes = Vec::with_capacity(
            registered
                .len()
                .saturating_add(usize::from(bundled.is_some())),
        );
        let mut excluded = Vec::new();
        runtimes.extend(bundled);
        for runtime in registered {
            if matches!(runtime.permission_mode(), McpPermissionMode::Deny) {
                excluded.push(ExcludedServer {
                    slug: runtime.slug().to_owned(),
                    reason: ServerExclusion::ToolsWithheld,
                });
                continue;
            }
            if runtimes.len() >= MAX_CONCURRENT_SERVER_SESSIONS {
                excluded.push(ExcludedServer {
                    slug: runtime.slug().to_owned(),
                    reason: ServerExclusion::SessionLimit,
                });
                continue;
            }
            runtimes.push(runtime);
        }
        Self { runtimes, excluded }
    }

    #[must_use]
    pub(crate) fn find(&self, slug: &str) -> Option<&Arc<McpRuntime>> {
        self.runtimes.iter().find(|runtime| runtime.slug() == slug)
    }

    /// Whether one persisted namespaced tool id belongs to a runtime this set
    /// can dispatch. Selected chats use this to reject foreign tool history.
    #[must_use]
    pub(crate) fn contains_tool_id(&self, tool_id: &str) -> bool {
        mcp_tool_route(tool_id).is_some_and(|(slug, _)| self.find(slug).is_some())
    }

    #[must_use]
    pub(crate) fn excluded(&self) -> &[ExcludedServer] {
        &self.excluded
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.runtimes.len()
    }

    /// Registers every held server's advertised tools into `registry`, and
    /// reports what each one contributed.
    ///
    /// Failure isolation is the point: each server is connected under its own
    /// cancellation token and its own deadline, and a server that cannot
    /// connect or list tools is reported [`ServerContribution::Unreachable`]
    /// while the others still register. No server's failure can fail a run —
    /// the user asked a question, not for an MCP session.
    ///
    /// Servers are connected concurrently but registered in set order, so one
    /// slow server delays the run by its own deadline rather than by the sum of
    /// every server's, and the resulting registry order stays deterministic.
    pub(crate) async fn register_tools(
        &self,
        registry: &mut ToolRegistry,
    ) -> Vec<(String, ServerContribution)> {
        let catalogs = join_all(
            self.runtimes
                .iter()
                .map(|runtime| catalog_within_deadline(runtime)),
        )
        .await;

        let mut contributions = Vec::with_capacity(self.runtimes.len());
        for (runtime, catalog) in self.runtimes.iter().zip(catalogs) {
            let Some(catalog) = catalog else {
                contributions.push((runtime.slug().to_owned(), ServerContribution::Unreachable));
                continue;
            };
            let mut registered = 0usize;
            for advertised in catalog.iter() {
                let built = descriptor_for(
                    runtime.slug(),
                    advertised.name(),
                    advertised.description(),
                    advertised.input_schema(),
                    advertised.hints(),
                    runtime.permission_mode(),
                );
                // A descriptor this build refuses (over-long name, oversized
                // schema) and a tool withheld by policy both contribute
                // nothing, and neither is a reason to abandon the rest of the
                // catalog. Both are logged: a tool that silently vanishes from
                // the model's view is indistinguishable from a server that
                // never advertised it, which makes the difference undiagnosable
                // from the app.
                let descriptor = match built {
                    Ok(Some(descriptor)) => descriptor,
                    Ok(None) => {
                        tracing::debug!("tool withheld by permission mode");
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "advertised tool refused; it will not be offered to the model"
                        );
                        continue;
                    }
                };
                // A collision means two servers published names that collapse
                // to the same namespaced id — impossible while slugs are
                // unique, and the reason they are. Keeping the first is what
                // preserves the namespacing guarantee.
                if registry.register(descriptor).is_ok() {
                    registered += 1;
                }
            }
            contributions.push((
                runtime.slug().to_owned(),
                if registered == 0 {
                    ServerContribution::NothingRegistrable
                } else {
                    ServerContribution::Served(registered)
                },
            ));
        }
        contributions
    }
}

/// One server's catalog, or `None` if it could not produce one in time.
///
/// The token is created here and cancelled on timeout so an abandoned
/// handshake does not outlive the attempt: nothing else holds it, and no
/// unrelated run can cancel this connect.
async fn catalog_within_deadline(runtime: &McpRuntime) -> Option<Arc<McpToolCatalog>> {
    let cancel = CancellationToken::new();
    let outcome = tokio::time::timeout(
        REGISTRATION_DEADLINE,
        runtime.connected_with_failure_action(&cancel, |error| {
            super::dispatch::notify_connection_refusal(runtime, error, None);
        }),
    )
    .await;
    match outcome {
        Ok(Ok((_session, catalog))) => Some(catalog),
        Ok(Err(error)) => {
            // These logs are exported in local diagnostics. Registered names
            // and transport errors (which may contain URLs or server text)
            // must not cross that boundary.
            let failure = match error {
                McpRuntimeError::Auth(_) => "credential_unavailable",
                McpRuntimeError::Connect(_) => "connection_failed",
                McpRuntimeError::Catalog => "catalog_invalid",
            };
            tracing::warn!(failure, "background MCP discovery failed");
            None
        }
        Err(_elapsed) => {
            cancel.cancel();
            None
        }
    }
}

/// What one advertised tool is admitted to do, given its server's
/// `permission_mode` and its own annotations.
///
/// The single place that rule lives (plan D6). Both the run's registry and the
/// `mcp_server_test` projection derive from it, so a tool cannot be presented
/// as unattended in Settings and registered as reviewed in a run, or the
/// reverse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolAdmission {
    /// Never registered — the model does not learn the tool exists.
    Withheld,
    /// Registered in the tier every call of which pauses for human approval.
    Reviewed,
    /// Registered in the tier that may run without one.
    Unattended,
}

impl ToolAdmission {
    #[must_use]
    pub(crate) fn of(mode: McpPermissionMode, hints: McpToolHints) -> Self {
        match mode {
            McpPermissionMode::Deny => Self::Withheld,
            // `allow` buys exactly one thing: skipping review for a tool the
            // server annotated read-only and not destructive. An unannotated or
            // mutating tool still pauses, and a server that says nothing about a
            // tool must not get the lenient reading by omission —
            // `McpPermissionMode`'s own default is `Ask` for the same reason.
            // See `McpToolHints::destructive` for why a tool claiming both wins
            // no leniency from the read-only half.
            McpPermissionMode::Allow if hints.unattendable() => Self::Unattended,
            McpPermissionMode::Allow | McpPermissionMode::Ask => Self::Reviewed,
        }
    }

    /// The registry risk tier this admission registers under, or `None` when
    /// the tool is not registered at all. `Mutating` is the tier
    /// `crate::tools::policy::PolicyEngine` demands an approval token for.
    #[must_use]
    pub(crate) fn risk(self) -> Option<ToolRisk> {
        match self {
            Self::Withheld => None,
            Self::Reviewed => Some(ToolRisk::Mutating),
            Self::Unattended => Some(ToolRisk::ReadOnly),
        }
    }

    /// Whether a call pauses for review. A withheld tool never runs at all,
    /// which is not the same as running unattended.
    #[must_use]
    pub(crate) fn requires_approval(self) -> bool {
        !matches!(self, Self::Unattended)
    }
}

/// Turns one advertised MCP tool into a registry descriptor under `slug`'s
/// namespace.
///
/// `Ok(None)` is a policy outcome, not a failure: the server's
/// `permission_mode` withholds the tool from the model entirely. `Err` means
/// the advertised metadata itself is unusable.
pub(crate) fn descriptor_for(
    slug: &str,
    name: &str,
    description: Option<&str>,
    input_schema: &Value,
    hints: McpToolHints,
    mode: McpPermissionMode,
) -> Result<Option<ToolDescriptor>, ToolDescriptorError> {
    let Some(risk) = ToolAdmission::of(mode, hints).risk() else {
        return Ok(None);
    };
    ToolDescriptor::new(
        namespaced_tool_id(slug, name),
        ToolSource::Mcp,
        name,
        description.unwrap_or("A tool published by a connected MCP server."),
        input_schema.clone(),
        risk,
        // No capability tags: MCP tools act on tenant- or vendor-side data,
        // never on the user's local trusted project, so neither
        // `CAPABILITY_REQUIRES_PROJECT` nor `CAPABILITY_REQUIRES_TRUSTED_PATH`
        // describes them. Tagging them anyway would make policy demand a
        // local path they have no concept of.
        Vec::new(),
        // Idempotency tracks the annotations, not the review tier: they are the
        // only evidence the protocol offers about replay safety, and an operator
        // dialing review up or down says nothing about it.
        hints.idempotent,
    )
    .map(Some)
}

/// The registry id one server's tool takes.
#[must_use]
pub(crate) fn namespaced_tool_id(slug: &str, tool_name: &str) -> String {
    format!("{MCP_TOOL_ID_PREFIX}{slug}.{tool_name}")
}

/// Recovers `(server slug, server-side tool name)` from a namespaced id.
///
/// Splits at the first `.` after the prefix, because a slug never contains one
/// (`super::registry::slug_for`) while a server-side tool name routinely does
/// (`warehouse.query`) — splitting at the last would put most of the tool name
/// in the slug and route the call nowhere.
#[must_use]
pub(crate) fn mcp_tool_route(tool_id: &str) -> Option<(&str, &str)> {
    let (slug, name) = tool_id.strip_prefix(MCP_TOOL_ID_PREFIX)?.split_once('.')?;
    if slug.is_empty() || name.is_empty() {
        return None;
    }
    Some((slug, name))
}
