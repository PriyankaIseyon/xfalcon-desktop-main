//! The `mcp_server_*` command surface, and the runtime it wraps.
//!
//! Same idiom as `crate::credentials::commands`: a managed-state runtime type
//! holds the long-lived collaborators, each `#[tauri::command]` is a thin
//! wrapper over one of its methods, and every failure crosses the boundary as a
//! [`McpServerOperationError`] code the UI can branch on — never a DA message,
//! a server message, or a transport string.
//!
//! # What this surface may and may not write
//!
//! Server *definitions* live in DA so desktop and xFalcon Web share one
//! registry. Server *credentials* never leave the device
//! (`super::credential_store`). So a create is two writes that must agree: the
//! DA row first, then the credential sealed under the row id DA assigned. If
//! sealing fails the row is deleted again — a registered server nobody can
//! authenticate to is worse than no row at all.
//!
//! Connection status is never written back. DA's `status` columns are one
//! prober's view, and a laptop behind a captive portal must not mark a healthy
//! server broken for the whole tenant; [`mcp_server_test`] reports what *this*
//! device just observed and stores nothing.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::mcp::{
    McpAuthType, McpConnectionReport, McpConnectionState, McpPermissionMode, McpServerErrorCode,
    McpServerId, McpServerInput, McpServerListResult, McpServerOperationError,
    McpServerUnavailableReason, McpServerView, McpSignInStatus, McpToolView, BUNDLED_SERVER_ID,
};
use zeroize::Zeroizing;

use crate::audit::{emit, AuditAction, AuditOutcome, AuditResourceType};
use crate::auth::{
    canonical_account_id, ensure_fresh_session, AuthConfig, AuthConfigError, AuthSessionStore,
};
use crate::clients::da::mcp_servers::{
    DaMcpServerRepository, McpServerPatch, McpServerRecord, McpServerRepositoryError, NewMcpServer,
};
use crate::credentials::vault::CredentialVault;
use crate::persistence::SecretStore;

use super::catalog::McpToolCatalog;
use super::client::McpClientError;
use super::credential_store::{
    credential_db_path, CredentialOwner, DeviceCredentials, MAX_CREDENTIAL_BYTES,
};
use super::dispatch::{notify_connection_refusal, ReauthNotifier, WindowReauthNotifier};
use super::oauth::{
    self,
    envelope::GrantKey,
    flow::{BrowserOpener, SignIn, SignInError},
    registration::OauthClient,
    store::{DeviceGrants, DeviceOauthClients, DevicePreferences, StoredOauthClientSummary},
};
use super::registry::{
    bundled_view, classify, is_bundled_alias, name_is_registrable, validate_endpoint,
    DeviceOauthClient, DialableServer, EndpointError, ServerVerdict,
};
use super::runtime::{
    namespaced_tool_id, McpRuntime, McpRuntimeError, McpRuntimeSet, SharedGrantBinding,
    ToolAdmission,
};
use super::McpServerResolver;
use crate::clients::da::mapping::build_hardened_client;
use crate::clients::da::mcp_oauth_grants::DaMcpGrantRepository;
use std::time::Duration;
use xfalcon_contracts::mcp::{McpReauthReason, McpReauthRequired};

/// How long a user-initiated connectivity check waits before reporting the
/// server unreachable.
///
/// Shorter than the run-time registration deadline on purpose: someone is
/// watching a spinner. The check is repeatable at no cost, so waiting out
/// `connect_with_reconnect`'s full backoff budget buys nothing.
const TEST_DEADLINE: Duration = Duration::from_secs(12);

/// Per-request budget for the loopback DA client tests use. Short enough that a
/// mock that never answers fails the test rather than hanging it.
#[cfg(test)]
const TEST_DA_TIMEOUT: Duration = Duration::from_millis(750);

/// OAuth client details are credentials, not documents. Bound both before any
/// registry or discovery call so an IPC caller cannot retain arbitrary input
/// through the browser flow.
const MAX_OAUTH_CLIENT_ID_BYTES: usize = 4 * 1024;
const MAX_OAUTH_CLIENT_SECRET_BYTES: usize = 4 * 1024;

/// Where this runtime's DA coordinates come from.
enum ConfigSource {
    /// The process environment, exactly as every other DA caller resolves it.
    Environment,
    /// A fixed origin, and the loopback-tolerant DA client that goes with it.
    ///
    /// Test-only: `AuthConfig::from_env` cannot be pointed at a `wiremock`
    /// server, and mutating the process environment instead would leak between
    /// concurrently-running tests. Boxed so this arm does not widen the enum
    /// for the production one.
    #[cfg(test)]
    Fixed(Box<AuthConfig>),
}

/// Everything the `mcp_server_*` commands and a run's MCP registry need.
pub struct McpServerRuntime {
    sessions: Arc<AuthSessionStore>,
    app_local_data: PathBuf,
    vault: Arc<CredentialVault>,
    config: ConfigSource,
    #[cfg(test)]
    test_grant_key: Option<Arc<GrantKey>>,
    /// The last-built run set, reused while the account and the dialable
    /// registry it was built from are unchanged.
    ///
    /// Rebuilding per run would re-handshake every server on every message;
    /// never rebuilding would ignore a server the user just registered. Keying
    /// on the classified [`DialableServer`] list makes "the registry changed"
    /// exactly as sensitive as it needs to be: an endpoint, auth requirement,
    /// permission mode, or credential-presence change rebuilds, and nothing
    /// else does.
    servers: AsyncMutex<Option<CachedRuntimeSet>>,
    /// Shared by every DA repository this runtime builds, so its calls reuse
    /// one tenant connection instead of handshaking per call.
    da_client: reqwest::Client,
}

/// Which servers are signed in, split by how far each sign-in reaches.
#[derive(Default)]
struct SignedInServers {
    /// Grants sealed on this device (`super::oauth::store`).
    device: Vec<String>,
    /// Grants DA holds for the whole account (`super::oauth::shared_grant`).
    account: Vec<String>,
}

/// Everything this device holds about the registry, as `super::registry::classify`
/// asks it.
///
/// Gathered in one place because every command that returns a view needs all
/// three, and because a lookup added to one call site and forgotten at another
/// is how a field ends up populated in `list` and empty in `update` — which is
/// exactly what a user reads as "it did not save".
#[derive(Default)]
struct DeviceFacts {
    credentials: Vec<String>,
    signed_in: SignedInServers,
    oauth_clients: Vec<StoredOauthClientSummary>,
}

impl DeviceFacts {
    /// Every registered row, classified against these facts.
    fn classify(
        &self,
        records: &[McpServerRecord],
        bundled_endpoint: &reqwest::Url,
    ) -> Vec<(McpServerView, ServerVerdict)> {
        let has_credential = credential_predicate(&self.credentials);
        let has_grant = sign_in_state(&self.signed_in.device, &self.signed_in.account);
        let oauth_client = oauth_client_lookup(&self.oauth_clients);
        classify(
            records,
            bundled_endpoint,
            &has_credential,
            &has_grant,
            &oauth_client,
        )
    }

    /// The classified row for `id`, or `None` when the registry has no such row.
    fn view_of(
        &self,
        records: &[McpServerRecord],
        bundled_endpoint: &reqwest::Url,
        id: &str,
    ) -> Option<(McpServerView, ServerVerdict)> {
        self.classify(records, bundled_endpoint)
            .into_iter()
            .find(|(view, _)| view.id.as_str() == id)
    }
}

/// The dialable servers, and the rows they were classified from.
///
/// The rows travel with them because a shared-grant binding is built from the
/// row's `auth_config`, which `DialableServer` deliberately does not carry (it is
/// a cache key, and a credential — even encrypted — does not belong in one).
#[derive(Default)]
struct DialableSet {
    dialable: Vec<DialableServer>,
    records: Vec<McpServerRecord>,
    device_grants: Vec<String>,
}

struct CachedRuntimeSet {
    account_id: String,
    dialable: Vec<DialableServer>,
    device_grants: Vec<String>,
    bundled_enabled: bool,
    /// Kept beside the set so a registry change reuses the bundled server's
    /// warm session instead of re-handshaking the tenant workspace.
    bundled: Option<Arc<McpRuntime>>,
    set: Arc<McpRuntimeSet>,
}

/// The signed-in identity, resolved once per operation.
struct SessionContext {
    identity: IdentityProjection,
    access_token: Zeroizing<String>,
    owner: CredentialOwner,
}

impl McpServerRuntime {
    /// Mirrors `CredentialRuntime::new`, and must be handed the *same* secret
    /// store: registered MCP credentials and legacy provider envelopes use the
    /// same device key. New provider credentials are encrypted by DA.
    #[must_use]
    pub fn new(
        sessions: Arc<AuthSessionStore>,
        app_local_data: PathBuf,
        credential_secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self::with_vault(
            sessions,
            app_local_data,
            Arc::new(CredentialVault::new(credential_secrets)),
        )
    }

    /// As [`Self::new`], for a caller that already holds the vault.
    #[must_use]
    pub fn with_vault(
        sessions: Arc<AuthSessionStore>,
        app_local_data: PathBuf,
        vault: Arc<CredentialVault>,
    ) -> Self {
        Self {
            sessions,
            app_local_data,
            vault,
            config: ConfigSource::Environment,
            #[cfg(test)]
            test_grant_key: None,
            servers: AsyncMutex::new(None),
            da_client: build_hardened_client()
                .expect("hardened DA MCP client must be constructible"),
        }
    }

    /// A runtime pointed at a loopback DA origin. See [`ConfigSource::Fixed`].
    #[cfg(test)]
    pub(crate) fn for_test(
        sessions: Arc<AuthSessionStore>,
        app_local_data: PathBuf,
        vault: Arc<CredentialVault>,
        config: AuthConfig,
    ) -> Self {
        Self {
            sessions,
            app_local_data,
            vault,
            config: ConfigSource::Fixed(Box::new(config)),
            test_grant_key: None,
            servers: AsyncMutex::new(None),
            da_client: build_hardened_client()
                .expect("hardened DA MCP client must be constructible"),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_grant_key(mut self, key: Arc<GrantKey>) -> Self {
        self.test_grant_key = Some(key);
        self
    }

    /// The bundled entry first, then every DA row classified against what this
    /// build and this device can do.
    ///
    /// The bundled server is built rather than classified: it has no DA row, so
    /// nothing can register or delete it.
    ///
    /// Four things must succeed, and a failure in any one of them empties the
    /// Settings panel identically. They are logged apart because the panel
    /// cannot say which: a macOS install showing no servers while Windows shows
    /// them is answerable from these lines and from nothing else in the app.
    /// Ids and counts only — no tokens, no URLs beyond the tenant's own origin.
    pub(crate) async fn list(&self) -> Result<McpServerListResult, McpServerOperationError> {
        let (context, config) = self.session_and_config(|| self.load_config()).inspect_err(
            |error| tracing::warn!(code = ?error.code, "mcp list: no session or no tenant config"),
        )?;
        let records = self.records(&config, &context).await.inspect_err(|error| {
            tracing::warn!(
                code = ?error.code,
                da_base_url = %config.da_base_url,
                "mcp list: the registry could not be read from DA"
            );
        })?;
        let facts = self
            .device_facts(&config, &context)
            .await
            .inspect_err(|error| {
                tracing::warn!(code = ?error.code, "mcp list: a device store could not be read");
            })?;
        let bundled_enabled = self.bundled_enabled(&context).inspect_err(|error| {
            tracing::warn!(code = ?error.code, "mcp list: the device preference store could not be read");
        })?;
        let bundled = McpServerResolver::resolve(&config);
        let bundled_aliases = records
            .iter()
            .filter(|record| is_bundled_alias(record, bundled.endpoint()))
            .count();
        // The counts are the facts that separate "DA returned nothing for this
        // identity" from "the rows arrived and the panel dropped them" — the
        // two shapes a reader reports the same way.
        tracing::info!(
            registered = records.len(),
            bundled_aliases,
            credentials = facts.credentials.len(),
            device_grants = facts.signed_in.device.len(),
            account_grants = facts.signed_in.account.len(),
            oauth_clients = facts.oauth_clients.len(),
            organization = %config.organization,
            "mcp list: registry read"
        );
        let mut servers =
            Vec::with_capacity(records.len().saturating_add(usize::from(bundled_enabled)));
        if bundled_enabled {
            servers.push(bundled_view(bundled.endpoint().as_str()));
        }
        servers.extend(
            facts
                .classify(&records, bundled.endpoint())
                .into_iter()
                .map(|(view, _)| view),
        );
        Ok(McpServerListResult::new(servers))
    }

    /// Registers a server, then seals its device-held material under the row id
    /// DA assigned.
    ///
    /// Every input rule is checked before the DA write, so a rejected
    /// registration leaves no row behind; and material that cannot be sealed
    /// rolls the row back, so the registry never holds a server this device
    /// could not authenticate to.
    ///
    /// `oauth_client` is stored here rather than only handed to a sign-in,
    /// which is the whole point of taking it at registration: a browser flow
    /// that is declined, times out, or is refused must not discard the pair the
    /// operator just pasted.
    pub(crate) async fn create(
        &self,
        input: McpServerInput,
        credential: Option<String>,
        oauth_client: OauthClientInput,
    ) -> Result<McpServerView, McpServerOperationError> {
        let (context, config) = self.session_and_config(|| self.load_config())?;
        let bundled = McpServerResolver::resolve(&config);
        let credential = validated_credential(&input, credential, false, bundled.endpoint())?;
        let oauth_client = oauth_client.validated(input.auth_type)?;
        let repository = self.repository(&config, &context)?;
        let record = repository
            .create(NewMcpServer {
                name: input.name.trim().to_owned(),
                display_name: input.display_name,
                description: input.description,
                url: input.url,
                auth_type: input.auth_type,
                header_name: input.header_name,
                permission_mode: input.permission_mode,
                enabled: input.enabled,
            })
            .await
            .map_err(map_repository_error)?;

        // Sealing is the last step precisely so this rollback is possible: the
        // alternative is a listed server that reports "needs a credential on
        // this device" forever, with no record of why. Both stores roll the row
        // back the same way, because a half-configured registration is the
        // failure this ordering exists to prevent.
        if let Some(credential) = credential {
            if self
                .credentials(&context)
                .put(&record.id, credential)
                .is_err()
            {
                let _ = repository.delete(&record.id).await;
                return Err(error_for(McpServerErrorCode::StorageUnavailable));
            }
        }
        if let ValidatedOauthClient::Set { client_id, secret } = oauth_client {
            if self
                .oauth_clients(&context)
                .put(&record.id, &client_id, secret)
                .is_err()
            {
                let _ = self.credentials(&context).delete(&record.id);
                let _ = repository.delete(&record.id).await;
                return Err(error_for(McpServerErrorCode::StorageUnavailable));
            }
        }

        self.forget_cached_set().await;
        // Re-listed rather than projected from the create response: a server's
        // slug is unique across the whole account, so only the full registry can
        // say which namespace this row actually took.
        let records = self.records(&config, &context).await?;
        let facts = self.device_facts(&config, &context).await?;
        facts
            .view_of(&records, bundled.endpoint(), &record.id)
            .map(|(view, _)| view)
            .ok_or_else(|| error_for(McpServerErrorCode::ServiceUnavailable))
    }

    /// Updates a registered server, and replaces its device-held material when
    /// new material is supplied.
    ///
    /// `credential` of `None` keeps whatever the device already holds — editing
    /// a description must not force re-entering an API key.
    ///
    /// `oauth_client` is declarative rather than a patch: the edit sheet is
    /// seeded with the stored client id, so what it sends back *is* the desired
    /// state, and a blanked field removes the stored client instead of being
    /// indistinguishable from "leave it alone". Its confidential half is the one
    /// exception, for the reason every secret field is: it cannot be seeded, so
    /// blank means keep — and it is dropped only when the id it belongs to
    /// changes.
    pub(crate) async fn update(
        &self,
        id: String,
        input: McpServerInput,
        credential: Option<String>,
        oauth_client: OauthClientInput,
    ) -> Result<McpServerView, McpServerOperationError> {
        if id == BUNDLED_SERVER_ID {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        let (context, config) = self.session_and_config(|| self.load_config())?;
        let bundled = McpServerResolver::resolve(&config);
        let mut records = self.records(&config, &context).await?;
        let index = records
            .iter()
            .position(|record| record.id == id)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
        if is_bundled_alias(&records[index], bundled.endpoint()) {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        let credentials = self.credentials(&context);
        let already_stored = self.stored_credential_ids(&context)?.contains(&id);
        let credential =
            validated_credential(&input, credential, already_stored, bundled.endpoint())?;
        let oauth_clients = self.oauth_clients(&context);
        let stored_client = oauth_clients
            .summary(&id)
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        let oauth_client = oauth_client
            .validated(input.auth_type)?
            .resolved_against(stored_client.as_ref());

        if records[index].name != input.name.trim() {
            // DA's patch surface carries no name, and uniqueness is enforced on
            // it, so a rename here would silently do nothing. Refusing beats
            // accepting an edit that is not applied.
            return Err(error_for(McpServerErrorCode::InvalidRequest));
        }
        let new_endpoint = validate_endpoint(&input.url).map_err(map_endpoint_error)?;
        let previous_endpoint = records[index]
            .url
            .as_deref()
            .and_then(|url| validate_endpoint(url).ok());
        let invalidates_device_grant = records[index].auth_type.needs_sign_in()
            && (!input.auth_type.needs_sign_in()
                || previous_endpoint.as_ref() != Some(&new_endpoint));

        let repository = self.repository(&config, &context)?;
        let updated = repository
            .update(
                &id,
                McpServerPatch {
                    display_name: input.display_name,
                    description: input.description,
                    url: Some(input.url),
                    auth_type: Some(input.auth_type),
                    header_name: input.header_name,
                    permission_mode: Some(input.permission_mode),
                    enabled: Some(input.enabled),
                },
            )
            .await
            .map_err(map_repository_error)?;

        if let Some(credential) = credential {
            credentials
                .put(&id, credential)
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        } else if !input.auth_type.needs_device_credential() && already_stored {
            // The server no longer authenticates, so the sealed value is dead
            // weight. Leaving it would keep a secret nobody can use and nothing
            // will ever remove.
            credentials
                .delete(&id)
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        }

        // Same rule, one layer up: a row that no longer signs in through the
        // browser has no use for a client, and `validated` has already turned a
        // non-`oauth` method into `Clear`.
        match oauth_client {
            ResolvedOauthClient::Keep => {}
            ResolvedOauthClient::Clear => oauth_clients
                .delete(&id)
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?,
            ResolvedOauthClient::Set { client_id, secret } => oauth_clients
                .put(&id, &client_id, secret)
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?,
        }

        if invalidates_device_grant {
            // A grant is resource-bound. After an OAuth URL change it must not
            // make the replacement look signed in, and after an auth-method
            // change it can never be used again. Local deletion failures are
            // reported instead of leaving live material behind silently.
            oauth::flow::sign_out(&self.oauth_http()?, &self.grants(&context), &id)
                .await
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        }

        self.forget_cached_set().await;
        // Substituted in place rather than re-listed: an update cannot change a
        // name, so the registry's order — and therefore every slug — is exactly
        // what the list above already established.
        records[index] = updated;
        let facts = self.device_facts(&config, &context).await?;
        facts
            .view_of(&records, bundled.endpoint(), &id)
            .map(|(view, _)| view)
            .ok_or_else(|| error_for(McpServerErrorCode::ServiceUnavailable))
    }

    /// Unregisters a server and forgets its device credential, or disables the
    /// configured tenant server for this account on this device.
    ///
    /// DA first: if the credential removal then fails, the definition is gone
    /// and only an unopenable ciphertext remains, which the reported
    /// `StorageUnavailable` tells the user to retry. The reverse order would
    /// leave a live registration nobody can authenticate to.
    pub(crate) async fn delete(&self, id: String) -> Result<(), McpServerOperationError> {
        if id == BUNDLED_SERVER_ID {
            let context = self.session()?;
            self.preferences(&context)
                .set_bundled_enabled(false)
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
            self.forget_cached_set().await;
            return Ok(());
        }
        let (context, config) = self.session_and_config(|| self.load_config())?;
        let records = self.records(&config, &context).await?;
        let record = records
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
        if is_bundled_alias(record, McpServerResolver::resolve(&config).endpoint()) {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        self.repository(&config, &context)?
            .delete(&id)
            .await
            .map_err(map_repository_error)?;
        self.forget_cached_set().await;
        // Revoked upstream where the authorization server supports it, and
        // deleted locally regardless: unregistering a server must not leave a
        // usable grant for it on the device.
        oauth::flow::sign_out(&self.oauth_http()?, &self.grants(&context), &id)
            .await
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        // The operator-issued client outlives a grant on purpose (it survives a
        // sign-out), so unregistering the server is the one moment it must be
        // forgotten too.
        self.oauth_clients(&context)
            .delete(&id)
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        self.credentials(&context)
            .delete(&id)
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))
    }

    /// Restores the configured tenant server for this account on this device.
    pub(crate) async fn restore_bundled(&self) -> Result<McpServerView, McpServerOperationError> {
        let (context, config) = self.session_and_config(|| self.load_config())?;
        self.preferences(&context)
            .set_bundled_enabled(true)
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        self.forget_cached_set().await;
        let bundled = McpServerResolver::resolve(&config);
        Ok(bundled_view(bundled.endpoint().as_str()))
    }

    /// Dials one server now and reports what it advertises.
    ///
    /// A disabled row is still dialable here: checking a URL before turning the
    /// server on is exactly what this is for. What is *not* dialed is a row this
    /// build cannot use at all — that returns
    /// [`McpConnectionState::NotAttempted`] with the classification's reason,
    /// rather than a failure that looks like the server's fault.
    pub(crate) async fn test(
        &self,
        id: String,
        reauth: Option<&dyn ReauthNotifier>,
    ) -> Result<McpConnectionReport, McpServerOperationError> {
        let (context, config) = self.session_and_config(|| self.load_config())?;
        if id == BUNDLED_SERVER_ID {
            if !self.bundled_enabled(&context)? {
                return Err(error_for(McpServerErrorCode::NotFound));
            }
            let bundled = McpRuntime::bundled(&config, self.sessions.clone());
            return Ok(dial(&bundled, McpServerId::from(BUNDLED_SERVER_ID), reauth).await);
        }
        let records = self.records(&config, &context).await?;
        let bundled = McpServerResolver::resolve(&config);
        let record = records
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
        if is_bundled_alias(record, bundled.endpoint()) {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        let facts = self.device_facts(&config, &context).await?;
        let (view, verdict) = facts
            .view_of(&records, bundled.endpoint(), &id)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
        match verdict {
            ServerVerdict::Unavailable(reason) => {
                if reason == McpServerUnavailableReason::NeedsSignIn {
                    notify_missing_sign_in(&view, reauth);
                }
                Ok(McpConnectionReport::failed(
                    view.id,
                    McpConnectionState::NotAttempted,
                    unavailable_message(reason),
                ))
            }
            ServerVerdict::Dialable(server) => {
                let shared = (!facts.signed_in.device.iter().any(|stored| stored == &id))
                    .then(|| {
                        records
                            .iter()
                            .find(|record| record.id == id)
                            .and_then(|record| self.shared_binding(&config, &context, record))
                    })
                    .flatten();
                let runtime = McpRuntime::for_dialable(
                    &server,
                    self.credentials(&context),
                    self.grants(&context),
                    shared,
                );
                Ok(dial(&runtime, view.id, reauth).await)
            }
        }
    }

    /// Signs in to one registered `oauth` server, in the system browser, and
    /// seals the resulting grant to this device.
    ///
    /// `client_id` / `client_secret` are the operator-issued credentials for an
    /// authorization server that will not register a client dynamically. Both are
    /// optional and both are ignored when the server does register one, so the
    /// common case needs no input at all — see `super::oauth::flow`.
    ///
    /// A pair supplied here is sealed to this device *before* the browser flow
    /// starts, not after it succeeds. The retry dialog is the only place the
    /// user can answer `client_registration_unavailable`, and a flow they then
    /// decline or let time out must not make them type the same pair again.
    ///
    /// Returns the row as it now stands, so the surface reflects the new state
    /// without a second round trip.
    pub(crate) async fn sign_in(
        &self,
        id: String,
        client_id: Option<String>,
        client_secret: Option<String>,
        browser: &dyn BrowserOpener,
    ) -> Result<McpServerView, McpServerOperationError> {
        let (client_id, client_secret) = validated_oauth_client(client_id, client_secret)?;
        if id == BUNDLED_SERVER_ID {
            // The bundled server rides the session bearer; there is no separate
            // sign-in to run, and pretending otherwise would mint a second grant
            // for a resource that already has one.
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        let (context, config) = self.session_and_config(|| self.load_config())?;
        let records = self.records(&config, &context).await?;
        let index = records
            .iter()
            .position(|record| record.id == id)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
        let record = &records[index];
        let bundled = McpServerResolver::resolve(&config);
        if is_bundled_alias(record, bundled.endpoint()) {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        if !record.auth_type.needs_sign_in() {
            return Err(error_for(McpServerErrorCode::InvalidRequest));
        }
        let raw_url = record
            .url
            .as_deref()
            .ok_or_else(|| error_for(McpServerErrorCode::InvalidRequest))?;
        // The same gate every dial goes through: a sign-in is an authenticated
        // outbound request to this URL, so it may not reach a host a dial could
        // not.
        let endpoint = validate_endpoint(raw_url).map_err(map_endpoint_error)?;

        let oauth_clients = self.oauth_clients(&context);
        // Sealed before discovery: from here on the pair survives every way this
        // flow can fail, including the ones that never reach a token endpoint.
        if let Some(pasted_id) = client_id.as_deref() {
            oauth_clients
                .put(&id, pasted_id, client_secret.clone().map(Zeroizing::new))
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        }
        let stored_client = oauth_clients
            .get(&id)
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;

        let device_grants = self.grants(&context);
        let device_grant_present = device_grants
            .stored_ids()
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?
            .iter()
            .any(|stored| stored == &id);
        // A client this device holds is this device's answer, whether it was
        // pasted a moment ago or on a previous attempt. Both keep the sign-in on
        // the per-device path; only a server with neither that nor a device
        // grant uses the account-wide binding.
        let explicit_client = stored_client.is_some();
        let shared = (!explicit_client && !device_grant_present)
            .then(|| self.shared_binding(&config, &context, record))
            .flatten();
        // Only the configured fallback belongs to the tenant's issuer. A
        // registered resource chooses its own issuer, and must not be able to
        // receive that pair merely by asking this device to sign in.
        let client_issuer =
            (shared.is_none() && !explicit_client && config.mcp_client_id.is_some())
                .then_some(&config.casdoor_issuer);
        let (client_id, client_secret) = match &shared {
            Some(shared) => (
                Some(shared.client.client_id.clone()),
                shared
                    .client
                    .client_secret
                    .as_ref()
                    .map(|secret| secret.to_string()),
            ),
            None => device_path_client(
                stored_client,
                config.mcp_client_id.as_deref(),
                config.mcp_client_secret.as_deref(),
            ),
        };
        let destination = match &shared {
            Some(shared) => oauth::flow::GrantDestination::Shared {
                grants: shared.grants.as_ref(),
                key: shared.key.as_ref(),
            },
            None => oauth::flow::GrantDestination::Device(&device_grants),
        };
        // Only the shared row carries one: it is a field of the `auth_config`
        // the web app wrote, and web honours it, so a sign-in performed here
        // against that row must resolve the same document. A rejected URL is
        // dropped rather than failing the sign-in, because discovery's own
        // routes are still available and this is a hint, not a requirement.
        let resource_metadata_url = shared
            .as_ref()
            .and_then(|shared| shared.resource_metadata_url.as_deref())
            .and_then(|raw| reqwest::Url::parse(raw.trim()).ok())
            .filter(|url| url.origin() == endpoint.origin() && oauth::endpoint_is_usable(url));

        oauth::flow::sign_in(
            &self.oauth_http()?,
            browser,
            SignIn {
                server_id: &id,
                endpoint: &endpoint,
                client_id,
                client_secret: client_secret.map(Zeroizing::new),
                client_issuer,
                resource_metadata_url,
                loopback_port: oauth::flow::sign_in_loopback_port(config.loopback_port),
                destination,
            },
        )
        .await
        .map_err(map_sign_in_error)?;

        self.forget_cached_set().await;
        // The DA row is untouched by a sign-in — only this device's stores
        // changed — so the list already read above is still current, and its
        // order is what decides every slug.
        let facts = self.device_facts(&config, &context).await?;
        facts
            .view_of(&records, bundled.endpoint(), &id)
            .map(|(view, _)| view)
            .ok_or_else(|| error_for(McpServerErrorCode::ServiceUnavailable))
    }

    /// Signs out of one registered `oauth` server: revoked upstream where the
    /// authorization server supports it, and forgotten on this device either way.
    ///
    /// The operator-issued client is deliberately *not* removed. It says which
    /// application this device authenticates as, not that it is authenticated;
    /// discarding it here would make every sign-out cost another paste of a
    /// value that has not changed.
    ///
    /// Idempotent — signing out of a server that was never signed in to is
    /// success, so the surface never has to guess whether the action applies.
    pub(crate) async fn sign_out(
        &self,
        id: String,
    ) -> Result<McpServerView, McpServerOperationError> {
        if id == BUNDLED_SERVER_ID {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        let (context, config) = self.session_and_config(|| self.load_config())?;
        let records = self.records(&config, &context).await?;
        let record = records
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
        let bundled = McpServerResolver::resolve(&config);
        if is_bundled_alias(record, bundled.endpoint()) {
            return Err(error_for(McpServerErrorCode::NotEditable));
        }
        let grants = self.grants(&context);
        let has_device_grant = grants
            .stored_ids()
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?
            .iter()
            .any(|stored| stored == &id);
        if has_device_grant {
            // A device-scoped sign-out must not evict an account-wide grant the
            // UI did not ask the user to confirm.
            oauth::flow::sign_out(&self.oauth_http()?, &grants, &id)
                .await
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        } else if self.grant_key().is_some() {
            let shared = self
                .shared_grants(&config, &context)
                .ok_or_else(|| error_for(McpServerErrorCode::ServiceUnavailable))?;
            oauth::flow::sign_out_shared(shared.as_ref(), &id)
                .await
                .map_err(|()| error_for(McpServerErrorCode::ServiceUnavailable))?;
        }
        self.forget_cached_set().await;

        let facts = self.device_facts(&config, &context).await?;
        facts
            .view_of(&records, bundled.endpoint(), &id)
            .map(|(view, _)| view)
            .ok_or_else(|| error_for(McpServerErrorCode::NotFound))
    }

    /// The MCP servers a run may draw tools from.
    ///
    /// Infallible by design: a registry this device cannot read right now
    /// degrades to the bundled tenant server when it is enabled. A preference
    /// read failure omits it so a stored removal is never silently reversed.
    pub(crate) async fn runtime_set(&self, config: &AuthConfig) -> Arc<McpRuntimeSet> {
        // Every server in the returned set authenticates with this session's
        // bearer (the bundled one directly, a registered one through its own
        // grant). Renewing here means a warm run does not start dialing with a
        // token that expires mid-conversation.
        let _ = ensure_fresh_session(&self.sessions).await;
        let Ok(context) = self.session() else {
            return Arc::new(McpRuntimeSet::new(None, Vec::new()));
        };
        let bundled_enabled = match self.bundled_enabled(&context) {
            Ok(enabled) => enabled,
            Err(error) => {
                tracing::warn!(code = ?error.code, "mcp runtime: the device preference store could not be read");
                false
            }
        };
        let DialableSet {
            dialable,
            records,
            device_grants,
        } = self.dialable_servers(config, &context).await;

        let mut guard = self.servers.lock().await;
        let reusable_bundled = match guard.as_ref() {
            Some(cached) if cached.account_id == context.owner.account_id => {
                if cached.dialable == dialable
                    && cached.device_grants == device_grants
                    && cached.bundled_enabled == bundled_enabled
                {
                    return Arc::clone(&cached.set);
                }
                // The registry changed and the tenant workspace did not, so its
                // session is still good.
                cached.bundled.clone()
            }
            // Another account's set (or none at all) shares nothing with this
            // one — not even the bundled session, whose bearer is that
            // account's.
            _ => None,
        };

        let bundled = bundled_enabled.then(|| {
            reusable_bundled.unwrap_or_else(|| {
                Arc::new(McpRuntime::bundled_for_account(
                    config,
                    self.sessions.clone(),
                    context.owner.account_id.as_str().to_owned(),
                ))
            })
        });
        let credentials = self.credentials(&context);
        let grants = self.grants(&context);
        let registered = dialable
            .iter()
            .map(|server| {
                let shared = (!device_grants.iter().any(|id| id == &server.id))
                    .then(|| {
                        records
                            .iter()
                            .find(|record| record.id == server.id)
                            .and_then(|record| self.shared_binding(config, &context, record))
                    })
                    .flatten();
                Arc::new(McpRuntime::for_dialable(
                    server,
                    credentials.clone(),
                    grants.clone(),
                    shared,
                ))
            })
            .collect();
        let set = Arc::new(McpRuntimeSet::new(bundled.clone(), registered));
        *guard = Some(CachedRuntimeSet {
            account_id: context.owner.account_id.clone(),
            dialable,
            device_grants,
            bundled_enabled,
            bundled,
            set: Arc::clone(&set),
        });
        set
    }

    /// Builds the runtime for one chat-selected server without consulting the
    /// process-wide `All` set. The selected endpoint is always resolved from
    /// trusted configuration or the account-scoped DA registry.
    pub(crate) async fn runtime_set_for_server(
        &self,
        config: &AuthConfig,
        server_id: &str,
        expected_endpoint: Option<&str>,
    ) -> Result<(Arc<McpRuntimeSet>, String), McpServerOperationError> {
        if server_id.is_empty() || server_id.len() > 256 {
            return Err(error_for(McpServerErrorCode::InvalidRequest));
        }
        let _ = ensure_fresh_session(&self.sessions).await;
        let context = self.session()?;
        let session_generation = self.sessions.generation();
        let bundled = McpServerResolver::resolve(config);

        let (runtime, endpoint) = if server_id == BUNDLED_SERVER_ID {
            if !self.bundled_enabled(&context)? {
                return Err(error_for(McpServerErrorCode::NotFound));
            }
            (
                Arc::new(McpRuntime::selected_bundled_for_account(
                    config,
                    self.sessions.clone(),
                    context.owner.account_id.as_str().to_owned(),
                )),
                bundled.endpoint().as_str().to_owned(),
            )
        } else {
            // Read the complete registry so slug allocation and alias checks
            // use the same stable classification as the ordinary server list.
            let records = self.records(config, &context).await?;
            let record = records
                .iter()
                .find(|record| record.id == server_id)
                .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
            if is_bundled_alias(record, bundled.endpoint()) {
                return Err(error_for(McpServerErrorCode::NotEditable));
            }
            let facts = self.device_facts(config, &context).await?;
            let (view, verdict) = facts
                .view_of(&records, bundled.endpoint(), server_id)
                .ok_or_else(|| error_for(McpServerErrorCode::NotFound))?;
            if !view.enabled {
                return Err(error_for(McpServerErrorCode::NotFound));
            }
            let server = match verdict {
                ServerVerdict::Dialable(server) => server,
                ServerVerdict::Unavailable(_) => {
                    return Err(error_for(McpServerErrorCode::ServiceUnavailable));
                }
            };
            if matches!(server.permission_mode, McpPermissionMode::Deny) {
                return Err(error_for(McpServerErrorCode::NotFound));
            }
            let shared = (!facts.signed_in.device.iter().any(|id| id == server_id))
                .then(|| self.shared_binding(config, &context, record))
                .flatten();
            let endpoint = server.descriptor.endpoint().as_str().to_owned();
            let runtime = Arc::new(McpRuntime::for_selected_dialable(
                &server,
                self.credentials(&context),
                self.grants(&context),
                shared,
            ));
            (runtime, endpoint)
        };

        if expected_endpoint.is_some_and(|expected| expected != endpoint) {
            return Err(error_for(McpServerErrorCode::NotFound));
        }
        if self.sessions.generation() != session_generation
            || !matches!(
                self.session(),
                Ok(current) if current.owner.account_id == context.owner.account_id
            )
        {
            return Err(error_for(McpServerErrorCode::Unauthorized));
        }
        Ok((Arc::new(McpRuntimeSet::new(None, vec![runtime])), endpoint))
    }

    /// Every registered server a run should hold a session for.
    ///
    /// Empty on any failure to read the registry: see [`Self::runtime_set`].
    async fn dialable_servers(&self, config: &AuthConfig, context: &SessionContext) -> DialableSet {
        let Ok(repository) = self.repository(config, context) else {
            return DialableSet::default();
        };
        let Ok(records) = repository.list().await else {
            return DialableSet::default();
        };
        let Ok(facts) = self.device_facts(config, context).await else {
            return DialableSet::default();
        };
        let device_grants = facts.signed_in.device.clone();
        let bundled = McpServerResolver::resolve(config);
        let dialable = facts
            .classify(&records, bundled.endpoint())
            .into_iter()
            .filter_map(|(view, verdict)| match verdict {
                // A disabled row is listed so it can be re-enabled, and never
                // dialed — that is the whole point of the flag.
                ServerVerdict::Dialable(server) if view.enabled => Some(*server),
                // Registry eligibility is not a request to use this server.
                // A missing grant can mean it was never configured on this
                // device; unrelated queries must not prompt for its sign-in.
                _ => None,
            })
            .collect();
        DialableSet {
            dialable,
            records,
            device_grants,
        }
    }

    /// Drops the cached set so the next run rebuilds it.
    ///
    /// Called after every mutation: the alternative is a run still holding a
    /// session to a server the user just deleted.
    async fn forget_cached_set(&self) {
        *self.servers.lock().await = None;
    }

    fn session(&self) -> Result<SessionContext, McpServerOperationError> {
        let snapshot = self
            .sessions
            .authenticated_snapshot()
            .ok_or_else(|| error_for(McpServerErrorCode::Unauthorized))?;
        let account_id = canonical_account_id(&snapshot.identity)
            .map_err(|_| error_for(McpServerErrorCode::Unauthorized))?;
        Ok(SessionContext {
            owner: CredentialOwner {
                account_id,
                tenant_id: snapshot.identity.tenant_id.clone(),
                user_id: snapshot.identity.user_id.clone(),
            },
            identity: snapshot.identity,
            access_token: snapshot.access_token,
        })
    }

    /// The session and the DA coordinates, in that order.
    ///
    /// Order is the point: a signed-out caller is refused before configuration
    /// is even read, so "not configured" can never be reported to someone whose
    /// real problem is that they are not signed in. The loader is a parameter so
    /// that ordering is provable without an environment.
    fn session_and_config(
        &self,
        load_config: impl FnOnce() -> Result<AuthConfig, AuthConfigError>,
    ) -> Result<(SessionContext, AuthConfig), McpServerOperationError> {
        let context = self.session()?;
        let config =
            load_config().map_err(|_| error_for(McpServerErrorCode::ServiceUnavailable))?;
        Ok((context, config))
    }

    fn load_config(&self) -> Result<AuthConfig, AuthConfigError> {
        match &self.config {
            ConfigSource::Environment => AuthConfig::from_env(),
            #[cfg(test)]
            ConfigSource::Fixed(config) => Ok((**config).clone()),
        }
    }

    fn credentials(&self, context: &SessionContext) -> DeviceCredentials {
        DeviceCredentials::new(
            credential_db_path(&self.app_local_data, &context.owner.account_id),
            self.vault.clone(),
            context.owner.clone(),
        )
    }

    /// The OAuth grant store, alongside the credential store and sharing its
    /// database file (`super::oauth::store`).
    fn grants(&self, context: &SessionContext) -> DeviceGrants {
        DeviceGrants::new(
            credential_db_path(&self.app_local_data, &context.owner.account_id),
            self.vault.clone(),
            context.owner.clone(),
        )
    }

    /// The operator-issued OAuth client store, alongside the grant store and
    /// sharing its database file (`super::oauth::store`).
    fn oauth_clients(&self, context: &SessionContext) -> DeviceOauthClients {
        DeviceOauthClients::new(
            credential_db_path(&self.app_local_data, &context.owner.account_id),
            self.vault.clone(),
            context.owner.clone(),
        )
    }

    fn preferences(&self, context: &SessionContext) -> DevicePreferences {
        DevicePreferences::new(
            credential_db_path(&self.app_local_data, &context.owner.account_id),
            context.owner.account_id.clone(),
        )
    }

    fn bundled_enabled(&self, context: &SessionContext) -> Result<bool, McpServerOperationError> {
        self.preferences(context)
            .bundled_enabled()
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))
    }

    fn stored_credential_ids(
        &self,
        context: &SessionContext,
    ) -> Result<Vec<String>, McpServerOperationError> {
        self.credentials(context)
            .stored_ids()
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))
    }

    /// Everything the device stores about the registry, read in one place.
    ///
    /// A single method because every caller needs all three parts and none may
    /// skip one: a view built without the OAuth client lookup reports no stored
    /// client, which the edit sheet renders as an empty field over a saved
    /// value.
    async fn device_facts(
        &self,
        config: &AuthConfig,
        context: &SessionContext,
    ) -> Result<DeviceFacts, McpServerOperationError> {
        Ok(DeviceFacts {
            credentials: self.stored_credential_ids(context)?,
            signed_in: self.stored_grant_ids(config, context).await?,
            oauth_clients: self
                .oauth_clients(context)
                .summaries()
                .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?,
        })
    }

    /// Every server this device can authenticate as signed-in: the grants sealed
    /// here, plus — when a shared key is configured — the account-wide grants DA
    /// holds.
    ///
    /// The union is what makes a server signed in on the web app show as signed in
    /// here. A DA read that fails contributes nothing rather than failing the
    /// list: the device's own grants are still a truthful answer, and a data plane
    /// that is briefly unreachable must not make Settings unreadable.
    async fn stored_grant_ids(
        &self,
        config: &AuthConfig,
        context: &SessionContext,
    ) -> Result<SignedInServers, McpServerOperationError> {
        let device = self
            .grants(context)
            .stored_ids()
            .map_err(|_| error_for(McpServerErrorCode::StorageUnavailable))?;
        let mut account = Vec::new();
        if self.grant_key().is_some() {
            if let Some(shared) = self.shared_grants(config, context) {
                // A data plane that is briefly unreachable contributes nothing
                // rather than failing the list: this device's own grants are still
                // a truthful answer, and Settings must stay readable.
                if let Ok(ids) = shared.signed_in_server_ids().await {
                    account = ids;
                }
            }
        }
        Ok(SignedInServers { device, account })
    }

    /// The shared key-encryption key, when this deployment configured one.
    ///
    /// Read per call rather than cached at construction: it is an operator
    /// setting, and a build that reads it once at boot would need a restart to
    /// pick up a rotation. The read is an environment lookup and a 32-byte decode.
    fn grant_key(&self) -> Option<Arc<GrantKey>> {
        #[cfg(test)]
        if matches!(self.config, ConfigSource::Fixed(_)) {
            return self.test_grant_key.clone();
        }
        GrantKey::from_env().ok().map(Arc::new)
    }

    /// The account-wide grant repository for this session, when a shared key
    /// exists to make it useful.
    fn shared_grants(
        &self,
        config: &AuthConfig,
        context: &SessionContext,
    ) -> Option<Arc<DaMcpGrantRepository>> {
        #[cfg(test)]
        if matches!(self.config, ConfigSource::Fixed(_))
            || is_loopback_test_origin(&config.da_base_url)
        {
            return DaMcpGrantRepository::for_test(
                config.da_base_url.as_str(),
                context.access_token.as_str(),
                &context.identity,
                TEST_DA_TIMEOUT,
            )
            .ok()
            .map(Arc::new);
        }
        DaMcpGrantRepository::new(
            config.da_base_url.as_str(),
            context.access_token.as_str(),
            &context.identity,
            self.da_client.clone(),
        )
        .ok()
        .map(Arc::new)
    }

    /// The binding that makes one row authenticate from the account-wide grant,
    /// or `None` to leave it on the device-held one.
    ///
    /// `None` for three distinct reasons, all of which are correct outcomes rather
    /// than failures: no shared key is configured, the row carries no OAuth client
    /// this key can open (the operator registered it on a different key, or never
    /// registered one), or DA is unreachable. In each case the device-sealed grant
    /// is used and the two products keep separate sign-ins.
    fn shared_binding(
        &self,
        config: &AuthConfig,
        context: &SessionContext,
        record: &McpServerRecord,
    ) -> Option<SharedGrantBinding> {
        if !record.auth_type.needs_sign_in() {
            return None;
        }
        let key = self.grant_key()?;
        let stored = record.auth_config.as_ref()?;
        let decrypted = {
            let key = Arc::clone(&key);
            stored.decrypt_oauth(&move |ciphertext| key.open(ciphertext).ok())?
        };
        Some(SharedGrantBinding {
            grants: self.shared_grants(config, context)?,
            client: OauthClient {
                client_id: decrypted.client_id.clone(),
                client_secret: Some(decrypted.client_secret.clone()),
            },
            scope: decrypted.scope.clone(),
            resource_metadata_url: decrypted.resource_metadata_url.clone(),
            key,
        })
    }

    /// How many registered servers this device holds an operator-issued OAuth
    /// client for, for the diagnostics bundle.
    ///
    /// Infallible and local: no registry request, and no session or store
    /// failure is reported. `0` is the honest answer to "does this device hold
    /// one" in every case that is not "yes, this many" — and together with
    /// whether the build packages a client, it is what separates "sign-in keeps
    /// asking for a client id" from "sign-in has one and the server refuses it".
    #[must_use]
    pub fn device_oauth_client_count(&self) -> usize {
        self.session()
            .ok()
            .and_then(|context| self.oauth_clients(&context).summaries().ok())
            .map_or(0, |clients| clients.len())
    }

    fn oauth_http(&self) -> Result<reqwest::Client, McpServerOperationError> {
        oauth::build_client().map_err(|_| error_for(McpServerErrorCode::ServiceUnavailable))
    }

    fn repository(
        &self,
        config: &AuthConfig,
        context: &SessionContext,
    ) -> Result<DaMcpServerRepository, McpServerOperationError> {
        #[cfg(test)]
        if matches!(self.config, ConfigSource::Fixed(_))
            || is_loopback_test_origin(&config.da_base_url)
        {
            return DaMcpServerRepository::for_test(
                config.da_base_url.as_str(),
                context.access_token.as_str(),
                &context.identity,
                TEST_DA_TIMEOUT,
            )
            .map_err(map_repository_error);
        }
        DaMcpServerRepository::new(
            config.da_base_url.as_str(),
            context.access_token.as_str(),
            &context.identity,
            self.da_client.clone(),
        )
        .map_err(map_repository_error)
    }

    async fn records(
        &self,
        config: &AuthConfig,
        context: &SessionContext,
    ) -> Result<Vec<McpServerRecord>, McpServerOperationError> {
        self.repository(config, context)?
            .list()
            .await
            .map_err(map_repository_error)
    }
}

#[cfg(test)]
fn is_loopback_test_origin(url: &reqwest::Url) -> bool {
    url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback())
}

#[tauri::command]
pub async fn mcp_server_list(
    app: tauri::AppHandle,
) -> Result<McpServerListResult, McpServerOperationError> {
    app.state::<McpServerRuntime>().list().await
}

#[tauri::command]
pub async fn mcp_server_create(
    app: tauri::AppHandle,
    input: McpServerInput,
    credential: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
) -> Result<McpServerView, McpServerOperationError> {
    let result = app
        .state::<McpServerRuntime>()
        .create(
            input,
            credential,
            OauthClientInput {
                client_id: oauth_client_id,
                client_secret: oauth_client_secret,
            },
        )
        .await;
    let server_id = result.as_ref().map_or("new", |view| view.id.as_str());
    record_mcp_server(
        &app,
        AuditAction::McpServerCreate,
        server_id,
        emit::outcome_for(&result),
    );
    result
}

#[tauri::command]
pub async fn mcp_server_update(
    app: tauri::AppHandle,
    id: String,
    input: McpServerInput,
    credential: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
) -> Result<McpServerView, McpServerOperationError> {
    let result = app
        .state::<McpServerRuntime>()
        .update(
            id.clone(),
            input,
            credential,
            OauthClientInput {
                client_id: oauth_client_id,
                client_secret: oauth_client_secret,
            },
        )
        .await;
    record_mcp_server(
        &app,
        AuditAction::McpServerUpdate,
        &id,
        emit::outcome_for(&result),
    );
    result
}

#[tauri::command]
pub async fn mcp_server_delete(
    app: tauri::AppHandle,
    id: String,
) -> Result<(), McpServerOperationError> {
    let result = app.state::<McpServerRuntime>().delete(id.clone()).await;
    record_mcp_server(
        &app,
        AuditAction::McpServerDelete,
        &id,
        emit::outcome_for(&result),
    );
    result
}

#[tauri::command]
pub async fn mcp_server_restore_bundled(
    app: tauri::AppHandle,
) -> Result<McpServerView, McpServerOperationError> {
    let result = app.state::<McpServerRuntime>().restore_bundled().await;
    record_mcp_server(
        &app,
        AuditAction::McpServerUpdate,
        BUNDLED_SERVER_ID,
        emit::outcome_for(&result),
    );
    result
}

/// Records one MCP server action by the server's id. Names, URLs and
/// credentials never reach the audit log: the helper takes only the id.
fn record_mcp_server<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: AuditAction,
    server_id: &str,
    outcome: AuditOutcome,
) {
    emit::record(
        app,
        action,
        AuditResourceType::McpServer,
        server_id,
        outcome,
    );
}

#[tauri::command]
pub async fn mcp_server_test(
    app: tauri::AppHandle,
    id: String,
) -> Result<McpConnectionReport, McpServerOperationError> {
    app.state::<McpServerRuntime>()
        .test(id, Some(&WindowReauthNotifier::new(app.clone())))
        .await
}

/// Opens a URL with the operating system's default browser.
///
/// The one place an authorization URL leaves this process. A system browser and
/// not an embedded webview, deliberately: the user has to be able to see the
/// authorization server's real address and certificate, and this app must not be
/// in a position to observe what they type into it (RFC 8252 §8.12).
struct SystemBrowser {
    app: tauri::AppHandle,
}

impl BrowserOpener for SystemBrowser {
    fn open(&self, url: &reqwest::Url) -> bool {
        use tauri_plugin_opener::OpenerExt as _;

        self.app
            .opener()
            .open_url(url.as_str(), None::<&str>)
            .is_ok()
    }
}

/// The client an MCP sign-in authenticates with on the per-device path.
///
/// A client this device holds wins, because it is the operator's answer to the
/// question the surface asked — whether it was pasted a moment ago (the caller
/// seals it before reaching here) or on an earlier attempt. A packaged one
/// spares them the question entirely. Neither is consulted on the shared path,
/// which takes its client from the DA row. With none of the three the flow
/// attempts dynamic registration, and only asks if that is refused.
///
/// The id and its secret resolve together, and never interleave: a secret
/// belongs to the client it was issued for, so a stored id carries only its
/// stored secret and a packaged id only a packaged one. Taking the id from one
/// source and the secret from the other would send a credential the
/// authorization server never issued for that client, which it rejects as
/// `invalid_client` — a failure that reads like a wrong secret rather than a
/// mixed pair.
fn device_path_client(
    stored: Option<OauthClient>,
    packaged_id: Option<&str>,
    packaged_secret: Option<&str>,
) -> (Option<String>, Option<String>) {
    match stored {
        Some(client) => (
            Some(client.client_id),
            client.client_secret.map(|secret| secret.to_string()),
        ),
        None => (
            packaged_id.map(str::to_owned),
            // Only alongside a packaged id: a secret with nothing to identify
            // is not a client, and would be sent against a dynamically
            // registered one it does not belong to.
            packaged_id.and(packaged_secret).map(str::to_owned),
        ),
    }
}

/// The OAuth client details a create or update carries, straight off the wire.
///
/// A pair of plain arguments rather than a field on [`McpServerInput`], for the
/// same reason `credential` is: the input struct is the definition written to
/// the shared registry, and confidential material must not be able to ride along
/// inside it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OauthClientInput {
    pub(crate) client_id: Option<String>,
    pub(crate) client_secret: Option<String>,
}

/// An [`OauthClientInput`] that has passed every boundary rule.
#[derive(Debug)]
enum ValidatedOauthClient {
    /// Store this pair. `secret` of `None` means a public client, or — on an
    /// update — "whatever is already sealed", which
    /// [`ValidatedOauthClient::resolved_against`] settles.
    Set {
        client_id: String,
        secret: Option<Zeroizing<String>>,
    },
    /// Hold no client for this server.
    Clear,
}

/// What an update must actually do to the store.
#[derive(Debug)]
enum ResolvedOauthClient {
    /// The stored row already says this. Writing it again would re-seal an
    /// unchanged value under a fresh envelope for nothing.
    Keep,
    Clear,
    Set {
        client_id: String,
        secret: Option<Zeroizing<String>>,
    },
}

impl OauthClientInput {
    /// Checks the pair against the method it was submitted with.
    ///
    /// A method that does not sign in through the browser has nowhere to keep a
    /// client, so anything supplied alongside one is refused rather than
    /// silently dropped — the same rule `validated_credential` applies to a
    /// credential, and for the same reason: accepting and discarding a value
    /// leaves the user believing it was stored.
    fn validated(
        self,
        auth_type: McpAuthType,
    ) -> Result<ValidatedOauthClient, McpServerOperationError> {
        let (client_id, client_secret) =
            validated_oauth_client(self.client_id, self.client_secret)?;
        if !auth_type.needs_sign_in() {
            if client_id.is_some() || client_secret.is_some() {
                return Err(error_for(McpServerErrorCode::InvalidRequest));
            }
            return Ok(ValidatedOauthClient::Clear);
        }
        Ok(match client_id {
            None => ValidatedOauthClient::Clear,
            Some(client_id) => ValidatedOauthClient::Set {
                client_id,
                secret: client_secret.map(Zeroizing::new),
            },
        })
    }
}

impl ValidatedOauthClient {
    /// Settles the one field an edit sheet cannot seed.
    ///
    /// A blank confidential field means "keep what is sealed" only while the id
    /// it belongs to is unchanged. Once the id changes the sealed value was
    /// issued for a different client, so it is dropped rather than sent with an
    /// identifier it does not match.
    fn resolved_against(self, stored: Option<&StoredOauthClientSummary>) -> ResolvedOauthClient {
        match self {
            Self::Clear if stored.is_none() => ResolvedOauthClient::Keep,
            Self::Clear => ResolvedOauthClient::Clear,
            Self::Set { client_id, secret } => {
                let unchanged_id = stored.is_some_and(|stored| stored.client_id == client_id);
                match secret {
                    Some(secret) => ResolvedOauthClient::Set {
                        client_id,
                        secret: Some(secret),
                    },
                    None if unchanged_id => ResolvedOauthClient::Keep,
                    None => ResolvedOauthClient::Set {
                        client_id,
                        secret: None,
                    },
                }
            }
        }
    }
}

/// Looks one server's stored client up by id, for `super::registry::classify`.
fn oauth_client_lookup(
    stored: &[StoredOauthClientSummary],
) -> impl Fn(&str) -> Option<DeviceOauthClient> + '_ {
    move |server_id| {
        stored
            .iter()
            .find(|client| client.server_id == server_id)
            .map(|client| DeviceOauthClient {
                client_id: client.client_id.clone(),
                confidential: client.confidential,
            })
    }
}

/// A field left blank on the sign-in sheet is an absent one, not an empty
/// credential.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn validated_oauth_client(
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<(Option<String>, Option<String>), McpServerOperationError> {
    let client_id = non_blank(client_id);
    let client_secret = non_blank(client_secret);
    if client_id
        .as_ref()
        .is_some_and(|value| value.len() > MAX_OAUTH_CLIENT_ID_BYTES)
        || client_secret
            .as_ref()
            .is_some_and(|value| value.len() > MAX_OAUTH_CLIENT_SECRET_BYTES)
        || (client_id.is_none() && client_secret.is_some())
    {
        return Err(error_for(McpServerErrorCode::InvalidRequest));
    }
    Ok((client_id, client_secret))
}

#[tauri::command]
pub async fn mcp_server_sign_in(
    app: tauri::AppHandle,
    id: String,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<McpServerView, McpServerOperationError> {
    let browser = SystemBrowser { app: app.clone() };
    let result = app
        .state::<McpServerRuntime>()
        .sign_in(id.clone(), client_id, client_secret, &browser)
        .await;
    record_mcp_server(
        &app,
        AuditAction::McpSignIn,
        &id,
        emit::outcome_for(&result),
    );
    result
}

#[tauri::command]
pub async fn mcp_server_sign_out(
    app: tauri::AppHandle,
    id: String,
) -> Result<McpServerView, McpServerOperationError> {
    let result = app.state::<McpServerRuntime>().sign_out(id.clone()).await;
    record_mcp_server(
        &app,
        AuditAction::McpSignOut,
        &id,
        emit::outcome_for(&result),
    );
    result
}

/// Checks every registration rule and returns the credential to seal, if any.
///
/// `already_stored` is what makes an edit different from a registration: a
/// server that needs a credential is satisfied by one the device already holds.
fn validated_credential(
    input: &McpServerInput,
    credential: Option<String>,
    already_stored: bool,
    bundled_endpoint: &reqwest::Url,
) -> Result<Option<Zeroizing<String>>, McpServerOperationError> {
    if !name_is_registrable(&input.name) {
        return Err(error_for(McpServerErrorCode::InvalidRequest));
    }
    if !input.auth_type.is_supported() {
        // `other` is DA's name for a method this build has no implementation of,
        // and no wire spelling to register one under (plan D4).
        return Err(error_for(McpServerErrorCode::InvalidRequest));
    }
    let endpoint = validate_endpoint(&input.url).map_err(map_endpoint_error)?;
    if endpoint == *bundled_endpoint {
        return Err(error_for(McpServerErrorCode::NotEditable));
    }
    let header_name_missing = input
        .header_name
        .as_deref()
        .is_none_or(|name| name.trim().is_empty());
    if matches!(input.auth_type, McpAuthType::Header) && header_name_missing {
        return Err(error_for(McpServerErrorCode::InvalidRequest));
    }

    let credential = credential
        .map(Zeroizing::new)
        .filter(|value| !value.trim().is_empty());
    if credential
        .as_ref()
        .is_some_and(|value| value.len() > MAX_CREDENTIAL_BYTES)
    {
        // Refused here rather than at the store, so an over-long value is a bad
        // request instead of a storage failure that rolls a registration back.
        return Err(error_for(McpServerErrorCode::InvalidRequest));
    }
    if input.auth_type.needs_device_credential() {
        if credential.is_none() && !already_stored {
            return Err(error_for(McpServerErrorCode::InvalidRequest));
        }
    } else if credential.is_some() {
        // A server that authenticates with nothing has nowhere to put a
        // credential, and an `oauth` server's material is obtained by
        // `mcp_server_sign_in` rather than pasted. Accepting and dropping the
        // value would leave the user believing a secret is stored.
        return Err(error_for(McpServerErrorCode::InvalidRequest));
    }
    Ok(credential)
}

fn notify_missing_sign_in(view: &McpServerView, reauth: Option<&dyn ReauthNotifier>) {
    if let Some(notifier) = reauth {
        notifier.reauth_required(McpReauthRequired::new(
            view.id.clone(),
            view.display_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&view.name),
            McpReauthReason::CredentialRefused,
        ));
    }
}

/// Connects, lists tools, and reports what happened — storing nothing.
///
/// A one-off runtime rather than the run set's: a connectivity check must
/// observe this moment instead of reusing a session established minutes ago,
/// and must not invalidate a session a run is mid-way through using.
async fn dial(
    runtime: &McpRuntime,
    id: McpServerId,
    reauth: Option<&dyn ReauthNotifier>,
) -> McpConnectionReport {
    let cancel = CancellationToken::new();
    match tokio::time::timeout(
        TEST_DEADLINE,
        runtime.connected_with_failure_action(&cancel, |error| {
            notify_connection_refusal(runtime, error, reauth);
        }),
    )
    .await
    {
        Ok(Ok((_session, catalog))) => McpConnectionReport::connected(
            id,
            tool_views(runtime.slug(), runtime.permission_mode(), &catalog),
        ),
        Ok(Err(error)) => {
            let (state, message) = connection_failure(&error);
            McpConnectionReport::failed(id, state, message)
        }
        Err(_elapsed) => {
            // Nothing else holds the token, so cancelling it here is what stops
            // an abandoned handshake from outliving the check.
            cancel.cancel();
            McpConnectionReport::failed(
                id,
                McpConnectionState::Unreachable,
                "This server did not answer in time. Try again in a moment.",
            )
        }
    }
}

/// Projects a connected server's catalog for Settings.
///
/// `requires_approval` comes from [`ToolAdmission`], the same function the run's
/// registry uses, so what a user is shown here is what a call will actually do.
fn tool_views(slug: &str, mode: McpPermissionMode, catalog: &McpToolCatalog) -> Vec<McpToolView> {
    catalog
        .iter()
        .map(|tool| {
            McpToolView::new(
                namespaced_tool_id(slug, tool.name()),
                tool.name(),
                tool.description().map(str::to_owned),
                ToolAdmission::of(mode, tool.hints()).requires_approval(),
            )
        })
        .collect()
}

/// One fixed, already-redacted explanation per failure shape.
///
/// Fixed strings rather than `error.to_string()`: a transport or server message
/// is attacker-influenced text about someone else's infrastructure, and the
/// three states below are all a user can act on.
///
/// Known limit: an HTTP 401 with no `WWW-Authenticate` header is indistinguishable
/// from any other failed handshake at this layer (`rmcp` only raises its typed
/// auth error when the challenge header is present), so a refused credential is
/// reported `Unreachable`. Narrowing that needs the transport to carry the
/// status, which is a change to `super::client`, not to this mapping.
fn connection_failure(error: &McpRuntimeError) -> (McpConnectionState, &'static str) {
    match error {
        // Never dialed: the value this build would have attached is absent or
        // unusable, which is this device's problem and not the server's.
        McpRuntimeError::Auth(_) => (
            McpConnectionState::NotAttempted,
            "This device holds no usable credential for this server.",
        ),
        // Connected, and then advertised a catalog this build refuses —
        // duplicate tool names, a malformed schema, or more tools than it will
        // hold.
        McpRuntimeError::Catalog => (
            McpConnectionState::Rejected,
            "This server's tool list could not be used. Contact the server administrator.",
        ),
        McpRuntimeError::Connect(error) => connect_failure(error),
    }
}

fn connect_failure(error: &McpClientError) -> (McpConnectionState, &'static str) {
    match error {
        McpClientError::Cancelled => (
            McpConnectionState::NotAttempted,
            "The connection check was cancelled.",
        ),
        McpClientError::Auth(_) => (
            McpConnectionState::NotAttempted,
            "This device holds no usable credential for this server.",
        ),
        // The exchange completed and produced something this build will not
        // speak — the server is there, and refuses this client on MCP terms.
        McpClientError::UnsupportedProtocolVersion(_) | McpClientError::MissingServerInfo => (
            McpConnectionState::Rejected,
            "This server does not speak the MCP revision this app requires.",
        ),
        McpClientError::Initialize(_) | McpClientError::Transport(_) => (
            McpConnectionState::Unreachable,
            "This server could not be reached. It may be restarting — try again in a moment.",
        ),
        // The server answered — and refused the credential. Reporting this as
        // unreachable would send the user to check their network for a problem
        // one sign-in fixes.
        McpClientError::AuthorizationRequired => (
            McpConnectionState::Rejected,
            "This server rejected the saved sign-in. Sign in again to reconnect it.",
        ),
        // The grant is real and too narrow. The scope the server asked for is in
        // the error itself; this surface keeps its fixed-string discipline, and
        // signing in again is the action either way.
        McpClientError::InsufficientScope { .. } => (
            McpConnectionState::Rejected,
            "The saved sign-in does not cover what this server requires. Sign in again.",
        ),
        // The server answered and asked this account to slow down. The sign-in
        // is fine and the network is reachable, so neither is the fix; waiting is.
        McpClientError::RateLimited(_) => (
            McpConnectionState::Rejected,
            "This server is rate limiting requests. Try again shortly.",
        ),
        // Describes the attempt sequence, not a failure of its own: the last
        // attempt's error is the one that says what happened.
        McpClientError::ReconnectExhausted { last_error, .. } => connect_failure(last_error),
    }
}

/// Why a classified row was never dialed, in the same fixed-string discipline.
const fn unavailable_message(reason: McpServerUnavailableReason) -> &'static str {
    match reason {
        McpServerUnavailableReason::UnsupportedTransport => {
            "This app connects MCP servers over Streamable HTTP only."
        }
        McpServerUnavailableReason::UnsupportedAuthMethod => {
            "This app does not implement this server's sign-in method."
        }
        McpServerUnavailableReason::NeedsSignIn => {
            "Sign in to this server on this device to connect it."
        }
        McpServerUnavailableReason::MissingDeviceCredential => {
            "Add this server's credential on this device to connect it."
        }
        McpServerUnavailableReason::InvalidUrl => {
            "This server's address is not one this app will connect to."
        }
    }
}

/// Answers `classify`'s "does this device hold a credential?" from one query's
/// result, rather than one query per row.
fn credential_predicate(stored: &[String]) -> impl Fn(&str) -> bool + '_ {
    move |id| stored.iter().any(|stored| stored == id)
}

/// Answers `classify`'s "is this server signed in, and how far does it reach?".
///
/// The device set wins where a server appears in both. An explicit per-device
/// sign-in must remain device-scoped even when the account also has a shared
/// grant, and the runtime follows this same preference.
fn sign_in_state<'a>(
    device: &'a [String],
    account: &'a [String],
) -> impl Fn(&str) -> McpSignInStatus + 'a {
    move |id| {
        if device.iter().any(|stored| stored == id) {
            McpSignInStatus::SignedIn
        } else if account.iter().any(|stored| stored == id) {
            McpSignInStatus::SignedInForAccount
        } else {
            McpSignInStatus::NotSignedIn
        }
    }
}

fn map_endpoint_error(error: EndpointError) -> McpServerOperationError {
    error_for(match error {
        // Well-formed, and refused: the distinction the UI needs is "fix the
        // URL" versus "this app will not connect there at all".
        EndpointError::HostRejected => McpServerErrorCode::UrlRejected,
        EndpointError::NotHttps
        | EndpointError::EmbeddedUserinfo
        | EndpointError::QueryOrFragment
        | EndpointError::MissingHost => McpServerErrorCode::InvalidRequest,
    })
}

/// One code per outcome the surface acts on differently.
///
/// `ClientRegistrationUnavailable` is the one that carries product weight: it is
/// not a failure but a request for the one field an authorization server with
/// dynamic registration disabled needs (`super::oauth::registration`).
fn map_sign_in_error(error: SignInError) -> McpServerOperationError {
    error_for(match error {
        SignInError::Discovery(_) => McpServerErrorCode::DiscoveryFailed,
        SignInError::ClientRegistrationUnavailable => {
            McpServerErrorCode::ClientRegistrationUnavailable
        }
        SignInError::Declined => McpServerErrorCode::SignInDeclined,
        SignInError::TimedOut => McpServerErrorCode::SignInTimedOut,
        // A browser that will not open, a loopback port already taken, a
        // registration that broke mid-request, and a refused exchange are all
        // "the sign-in did not produce a grant, try again": distinguishing them
        // further would name this device's local problems in a message about
        // someone else's server.
        SignInError::ClientRegistrationFailed
        | SignInError::RedirectUnavailable
        | SignInError::BrowserUnavailable
        | SignInError::Refused => McpServerErrorCode::SignInFailed,
        SignInError::Storage => McpServerErrorCode::StorageUnavailable,
    })
}

fn map_repository_error(error: McpServerRepositoryError) -> McpServerOperationError {
    error_for(match error {
        McpServerRepositoryError::Unauthorized => McpServerErrorCode::Unauthorized,
        McpServerRepositoryError::NotFound => McpServerErrorCode::NotFound,
        McpServerRepositoryError::Conflict => McpServerErrorCode::Conflict,
        // A row outside DA's own contract, an unreachable registry, and a
        // mutation that needs reconciliation all leave the caller in one place:
        // list again. That is exactly what `ServiceUnavailable` asks for, and
        // inventing a distinction the UI cannot act on would not help.
        McpServerRepositoryError::InvalidRow
        | McpServerRepositoryError::Unavailable
        | McpServerRepositoryError::ReconciliationRequired => {
            McpServerErrorCode::ServiceUnavailable
        }
    })
}

fn error_for(code: McpServerErrorCode) -> McpServerOperationError {
    McpServerOperationError::new(code)
}

/// Covers the one invariant that lives inside this module's private helpers.
/// Everything reachable through the runtime's own surface is exercised against a
/// real `wiremock` DA and a real MCP server in `super::tests::commands`.
#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::persistence::InMemorySecretStore;

    fn stored(client_id: &str, client_secret: Option<&str>) -> Option<OauthClient> {
        Some(OauthClient {
            client_id: client_id.to_owned(),
            client_secret: client_secret.map(|secret| Zeroizing::new(secret.to_owned())),
        })
    }

    /// The surface asks for a client id only when it has to. A build that
    /// packages one must not ask at all, and a device that already holds one
    /// must use it over the packaged value — including on the retry after a
    /// sign-in that never completed.
    #[test]
    fn a_packaged_client_spares_the_paste_without_overriding_a_stored_one() {
        assert_eq!(
            device_path_client(None, Some("packaged"), None),
            (Some("packaged".to_owned()), None)
        );
        assert_eq!(
            device_path_client(stored("issued", None), Some("packaged"), None),
            (Some("issued".to_owned()), None)
        );
        // Nothing packaged and nothing stored: the flow tries dynamic
        // registration, which is what asks the question in the first place.
        assert_eq!(device_path_client(None, None, None), (None, None));
    }

    /// A secret authenticates one specific client. Carrying it across to a
    /// different id sends a credential the authorization server never issued for
    /// that client, and it answers `invalid_client` — which reads as a wrong
    /// secret rather than a mixed pair.
    #[test]
    fn an_id_and_its_secret_never_come_from_different_sources() {
        // Packaged pair, used whole.
        assert_eq!(
            device_path_client(None, Some("packaged"), Some("packaged-secret")),
            (
                Some("packaged".to_owned()),
                Some("packaged-secret".to_owned())
            )
        );
        // A stored id takes its own secret, and leaves the packaged one behind
        // even though one is available.
        assert_eq!(
            device_path_client(
                stored("issued", Some("issued-secret")),
                Some("packaged"),
                Some("packaged-secret"),
            ),
            (Some("issued".to_owned()), Some("issued-secret".to_owned()))
        );
        // A stored id with no secret is a public client, not an invitation to
        // attach the packaged secret.
        assert_eq!(
            device_path_client(
                stored("issued", None),
                Some("packaged"),
                Some("packaged-secret"),
            ),
            (Some("issued".to_owned()), None)
        );
        // A secret with no id to identify belongs to no client, so it is not
        // sent against a dynamically registered one.
        assert_eq!(
            device_path_client(None, None, Some("orphan-secret")),
            (None, None)
        );
    }

    /// A client is only storable for the method that uses one. Accepting it for
    /// any other method would report a value as saved that nothing can read.
    #[test]
    fn a_client_is_refused_for_a_method_that_never_signs_in() {
        for auth_type in [McpAuthType::None, McpAuthType::Bearer, McpAuthType::Header] {
            assert_eq!(
                OauthClientInput {
                    client_id: Some("issued".to_owned()),
                    client_secret: None,
                }
                .validated(auth_type)
                .unwrap_err()
                .code,
                McpServerErrorCode::InvalidRequest
            );
            assert!(matches!(
                OauthClientInput::default().validated(auth_type).unwrap(),
                ValidatedOauthClient::Clear
            ));
        }
    }

    /// The edit sheet is seeded with the stored id, so what it returns is the
    /// desired state: a blanked field removes the client rather than reading as
    /// "leave it alone". The confidential half is the one field it cannot seed,
    /// so blank keeps it — but only for the id it was issued against.
    #[test]
    fn an_edit_keeps_a_sealed_value_only_while_its_client_id_is_unchanged() {
        let held = StoredOauthClientSummary {
            server_id: "srv-1".to_owned(),
            client_id: "issued".to_owned(),
            confidential: true,
        };
        let input = |client_id: Option<&str>, client_secret: Option<&str>| OauthClientInput {
            client_id: client_id.map(str::to_owned),
            client_secret: client_secret.map(str::to_owned),
        };

        // Same id, nothing retyped: the sealed value stays exactly as it is.
        assert!(matches!(
            input(Some("issued"), None)
                .validated(McpAuthType::Oauth)
                .unwrap()
                .resolved_against(Some(&held)),
            ResolvedOauthClient::Keep
        ));
        // A new id with no secret drops the one sealed for the old id.
        assert!(matches!(
            input(Some("reissued"), None)
                .validated(McpAuthType::Oauth)
                .unwrap()
                .resolved_against(Some(&held)),
            ResolvedOauthClient::Set { secret: None, .. }
        ));
        // A retyped secret replaces it under the same id.
        assert!(matches!(
            input(Some("issued"), Some("rotated"))
                .validated(McpAuthType::Oauth)
                .unwrap()
                .resolved_against(Some(&held)),
            ResolvedOauthClient::Set {
                secret: Some(_),
                ..
            }
        ));
        // Blanking the id removes the client and its sealed value together.
        assert!(matches!(
            input(None, None)
                .validated(McpAuthType::Oauth)
                .unwrap()
                .resolved_against(Some(&held)),
            ResolvedOauthClient::Clear
        ));
        // Blank over nothing writes nothing, rather than deleting a row that is
        // not there on every save of every OAuth server.
        assert!(matches!(
            input(None, None)
                .validated(McpAuthType::Oauth)
                .unwrap()
                .resolved_against(None),
            ResolvedOauthClient::Keep
        ));
        // A first client on a row that held none is stored whole.
        assert!(matches!(
            input(Some("issued"), Some("issued-secret"))
                .validated(McpAuthType::Oauth)
                .unwrap()
                .resolved_against(None),
            ResolvedOauthClient::Set {
                secret: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn oauth_client_input_rejects_orphan_and_oversized_values() {
        assert_eq!(
            validated_oauth_client(None, Some("secret".to_owned()))
                .unwrap_err()
                .code,
            McpServerErrorCode::InvalidRequest
        );
        assert_eq!(
            validated_oauth_client(Some("x".repeat(MAX_OAUTH_CLIENT_ID_BYTES + 1)), None)
                .unwrap_err()
                .code,
            McpServerErrorCode::InvalidRequest
        );
        assert_eq!(
            validated_oauth_client(
                Some("client".to_owned()),
                Some("x".repeat(MAX_OAUTH_CLIENT_SECRET_BYTES + 1)),
            )
            .unwrap_err()
            .code,
            McpServerErrorCode::InvalidRequest
        );
        assert_eq!(
            validated_oauth_client(Some("  client  ".to_owned()), Some("  secret  ".to_owned()),)
                .unwrap(),
            (Some("client".to_owned()), Some("secret".to_owned()))
        );
    }

    #[test]
    fn a_signed_out_runtime_is_refused_before_configuration_is_even_read() {
        let runtime = McpServerRuntime::new(
            Arc::new(AuthSessionStore::default()),
            PathBuf::from("unused"),
            Arc::new(InMemorySecretStore::default()),
        );
        let configuration_read = Cell::new(false);

        let outcome = runtime.session_and_config(|| {
            configuration_read.set(true);
            Err(AuthConfigError::NotConfigured)
        });

        assert_eq!(
            outcome.err().map(|error| error.code),
            Some(McpServerErrorCode::Unauthorized)
        );
        assert!(
            !configuration_read.get(),
            "a signed-out caller must not be told the app is misconfigured"
        );
    }
}
