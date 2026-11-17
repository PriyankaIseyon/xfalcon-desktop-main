use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;
#[cfg(debug_assertions)]
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::auth::SessionProjection;

use super::canonical_account_id;
use super::config::AuthConfig;
use super::config::AuthConfigError;
use super::discovery::{build_discovery_client, discover_auth, DiscoveryError};
use super::loopback::{LoopbackError, LoopbackWaiter};
use super::oauth::{build_authorization_url, exchange_authorization_code, OAuthError};
use super::pkce::{generate_oauth_nonce, generate_oauth_state, generate_pkce, nonce_matches};
use super::session::AuthSessionStore;
use super::token::{ensure_identity_matches_token, verify_access_token, TokenValidationError};
use crate::clients::da::identity::{fetch_chat_identity, DaIdentityError};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Debug-build-only, opt-in escape hatch that skips the real Casdoor/DA
/// sign-in flow entirely and synthesizes an already-authenticated session,
/// so the rest of the app (anything not itself DA-dependent, e.g. trusted
/// projects) can be exercised without a reachable tenant. `cfg(debug_assertions)`
/// means this branch does not exist in a release build regardless of the
/// env var below; the env var means it is never active in a normal debug
/// run either, unless a developer explicitly opts in. Remove once Task 7/8
/// land and there is a real end-to-end path to test against instead.
#[cfg(debug_assertions)]
const DEV_SKIP_AUTH_ENV: &str = "XFALCON_DEV_SKIP_AUTH";

#[derive(Debug, thiserror::Error)]
pub enum AuthServiceError {
    #[error("sign-in is already in progress")]
    AlreadyAuthenticating,
    #[error("{0}")]
    NotConfigured(#[from] AuthConfigError),
    #[error("could not open the system browser")]
    BrowserUnavailable,
    #[error("could not reach the tenant authorization service")]
    DiscoveryFailed,
    #[error(
        "tenant authorization server does not match XFALCON_CASDOOR_ISSUER; check .env against the tenant dashboard"
    )]
    IssuerMismatch,
    #[error("tenant authorization metadata was invalid or unsupported")]
    DiscoveryMetadataInvalid,
    #[error(
        "loopback sign-in port {0} is unavailable; close other desktop sessions or set XFALCON_OAUTH_LOOPBACK_PORT"
    )]
    LoopbackUnavailable(u16),
    #[error("authorization callback was invalid")]
    CallbackInvalid,
    #[error("sign-in timed out waiting for the browser callback")]
    CallbackTimedOut,
    #[error(
        "Casdoor rejected the redirect URI. Add {0} to this application's redirect URLs in Casdoor (public PKCE client)."
    )]
    RedirectNotRegistered(String),
    #[error("token exchange failed")]
    TokenExchangeFailed,
    #[error("signed in but tenant identity could not be verified")]
    IdentityFailed,
    #[error("this device's clock disagrees with the tenant's by more than a minute")]
    DeviceClockSkew,
    #[error("sign-in was cancelled")]
    Cancelled,
}

pub struct AuthService {
    sessions: Arc<AuthSessionStore>,
    client: Client,
    flow_cancel: Mutex<Option<CancellationToken>>,
}

impl AuthService {
    pub fn new(sessions: Arc<AuthSessionStore>) -> Self {
        let client =
            build_discovery_client().expect("hardened authentication client must be constructible");
        Self {
            sessions,
            client,
            flow_cancel: Mutex::new(None),
        }
    }

    pub fn status(&self) -> SessionProjection {
        self.sessions.projection()
    }

    pub async fn begin_login(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<SessionProjection, AuthServiceError> {
        #[cfg(debug_assertions)]
        if let Some(session) = self.dev_skip_auth() {
            return Ok(session);
        }

        let config = AuthConfig::from_env()?;
        let current = self.sessions.projection();
        if matches!(current, SessionProjection::Authenticating { .. }) {
            return Err(AuthServiceError::AlreadyAuthenticating);
        }

        self.sessions.set(SessionProjection::authenticating());

        match self.run_login(app, &config).await {
            Ok(projection) => {
                self.clear_flow_cancel();
                Ok(projection)
            }
            Err(error) => {
                self.clear_flow_cancel();
                self.sessions.clear();
                Err(error)
            }
        }
    }

    async fn run_login(
        &self,
        app: &tauri::AppHandle,
        config: &AuthConfig,
    ) -> Result<SessionProjection, AuthServiceError> {
        // Taken after `begin_login` set the authenticating state: any change
        // from here on (a cancel, or a superseding sign-in) moves it.
        let generation = self.sessions.generation();
        let discovered = discover_auth(&self.client, config)
            .await
            .map_err(map_discovery_error)?;

        let (waiter, redirect_uri) = LoopbackWaiter::bind_on_port(config.loopback_port)
            .await
            .map_err(|_| AuthServiceError::LoopbackUnavailable(config.loopback_port))?;
        self.set_flow_cancel(waiter.cancel_token());
        // A cancel that landed during discovery found no flow to stop. Without this
        // check the browser would still open and the loopback port stay held until
        // the sign-in times out, which also blocks the next attempt.
        if self.sessions.generation() != generation {
            return Err(AuthServiceError::Cancelled);
        }

        let pkce = generate_pkce();
        let state = generate_oauth_state();
        let nonce = generate_oauth_nonce();

        let authorize_url = build_authorization_url(
            &discovered,
            config,
            &pkce,
            &state.value,
            &nonce.value,
            &redirect_uri,
        )
        .map_err(|_| AuthServiceError::TokenExchangeFailed)?;

        app.opener()
            .open_url(authorize_url.as_str(), None::<&str>)
            .map_err(|_| AuthServiceError::BrowserUnavailable)?;

        let callback = waiter
            .wait_for_callback(&state.value, LOGIN_TIMEOUT)
            .await
            .map_err(map_loopback_error)?;

        let tokens = exchange_authorization_code(
            &self.client,
            &discovered,
            config,
            &callback.code,
            &redirect_uri,
            &pkce,
        )
        .await
        .map_err(|error| map_oauth_error(error, &redirect_uri.value))?;

        let verified_token = verify_access_token(
            &self.client,
            &config.casdoor_issuer,
            &discovered.jwks_uri,
            &discovered.resource,
            &config.organization,
            &tokens.access_token,
        )
        .await
        .map_err(|error| {
            // Every way this can fail reaches the surface as one code, because
            // the remedy is the same for all of them: an operator has to look
            // at the tenant. So the step and the values it compared are logged
            // here, where an operator can read them. The audience and the
            // organization are the tenant's own coordinates, already in `.env`
            // and in the tenant's own metadata — no part of the token is
            // logged.
            let (claimed_issuer, claimed_audience) =
                crate::auth::token::unverified_issuer_and_audience(&tokens.access_token);
            tracing::warn!(
                %error,
                expected_audience = %discovered.resource,
                expected_issuer = %config.casdoor_issuer,
                expected_organization = %config.organization,
                claimed_issuer,
                claimed_audience,
                absent_claims = %crate::auth::token::unverified_missing_claims(&tokens.access_token),
                "access token did not verify against the tenant"
            );
            if matches!(error, TokenValidationError::ClockSkew) {
                return AuthServiceError::DeviceClockSkew;
            }
            AuthServiceError::IdentityFailed
        })?;

        if verified_token.nonce.is_none() {
            // The check passes with nothing to compare when the tenant omits the
            // claim, so the log records that it was skipped.
            tracing::info!("access token carries no nonce claim; the nonce check was skipped");
        }
        // Refused before any identity is fetched or session written: a token
        // minted for a different sign-in request must not be accepted here.
        if !nonce_matches(verified_token.nonce.as_deref(), &nonce.value) {
            tracing::warn!("access token nonce does not match the sign-in request");
            return Err(AuthServiceError::IdentityFailed);
        }

        let identity = fetch_chat_identity(
            &self.client,
            &config.da_base_url,
            &tokens.access_token,
            &verified_token.owner,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "the workspace did not confirm the signed-in identity");
            map_identity_error(error)
        })?;
        ensure_identity_matches_token(&verified_token, &identity).map_err(|error| {
            // The names themselves are the person's, so only the disagreement
            // is recorded: which of the two sources to look at is the part an
            // operator cannot work out from the surface.
            tracing::warn!(
                %error,
                "the workspace named a different user than the access token"
            );
            AuthServiceError::IdentityFailed
        })?;
        canonical_account_id(&identity).map_err(|_| {
            // The identity's own ids are the person's, and the refusal carries
            // nothing else: the step is the whole finding.
            tracing::warn!("the signed-in identity is not usable as an account scope");
            AuthServiceError::IdentityFailed
        })?;

        let session = SessionProjection::authenticated(identity, verified_token.expires_at_ms);
        // The callback finished, but the person may have cancelled since: the
        // waiter's token cannot say so, because the waiter cancels it on drop.
        // `cancel_login` empties the flow slot and clears the session, and the
        // write is guarded by the generation, so a cancel that lands after this
        // check still leaves the session cleared.
        let may_complete = login_may_complete(
            self.flow_cancel_taken(),
            generation,
            self.sessions.generation(),
        );
        if !may_complete
            || !self.sessions.set_authenticated_if_generation(
                generation,
                session.clone(),
                tokens.access_token,
                tokens.refresh_token,
            )
        {
            return Err(AuthServiceError::Cancelled);
        }
        // Recorded after the session exists, from the same discovery that
        // authenticated it: `crate::mcp::auth::McpAuth::bind` refuses to
        // attach this bearer to the bundled MCP server without proof of the
        // audience it was minted for, and re-running discovery at connect
        // time would be a second, racy source of truth.
        self.sessions
            .set_authorized_resource(discovered.resource.clone());
        Ok(session)
    }

    pub fn cancel_login(&self) -> SessionProjection {
        if let Ok(mut guard) = self.flow_cancel.lock() {
            if let Some(token) = guard.take() {
                token.cancel();
            }
        }
        self.sessions.clear();
        self.sessions.projection()
    }

    pub fn logout(&self) -> SessionProjection {
        self.cancel_login()
    }

    fn set_flow_cancel(&self, token: CancellationToken) {
        if let Ok(mut guard) = self.flow_cancel.lock() {
            *guard = Some(token);
        }
    }

    fn clear_flow_cancel(&self) {
        if let Ok(mut guard) = self.flow_cancel.lock() {
            guard.take();
        }
    }

    /// Whether `cancel_login` has taken the flow's cancellation handle. Only it
    /// takes the handle during a flow, so an empty slot means the person
    /// cancelled. A poisoned lock counts as cancelled.
    fn flow_cancel_taken(&self) -> bool {
        !self.flow_cancel.lock().is_ok_and(|guard| guard.is_some())
    }

    /// See [`DEV_SKIP_AUTH_ENV`]'s doc comment. Returns `None` (falling
    /// through to the real flow) unless the env var is set to exactly `"1"`;
    /// never touches `AuthConfig`/network/browser/loopback, and produces a
    /// synthetic identity whose `tenant_id`/`user_id` satisfy
    /// `canonical_account_id`'s own validation the same way a real one
    /// would, so downstream account-scoped storage binds normally.
    #[cfg(debug_assertions)]
    fn dev_skip_auth(&self) -> Option<SessionProjection> {
        if std::env::var(DEV_SKIP_AUTH_ENV).ok().as_deref() != Some("1") {
            return None;
        }
        let identity = IdentityProjection::new(
            "dev-user",
            "dev-tenant",
            "Dev User (XFALCON_DEV_SKIP_AUTH)",
            None,
        );
        let far_future_expiry_ms = i64::MAX;
        let session = SessionProjection::authenticated(identity, far_future_expiry_ms);
        self.sessions
            .set_authenticated(session.clone(), "dev-skip-auth-not-a-real-token", None);
        Some(session)
    }
}

/// Whether a sign-in may write its session: not when the person cancelled it,
/// and not when the session has moved since the sign-in began.
pub(crate) fn login_may_complete(cancelled: bool, expected: u64, current: u64) -> bool {
    !cancelled && expected == current
}

fn map_discovery_error(error: DiscoveryError) -> AuthServiceError {
    match error {
        DiscoveryError::IssuerNotAllowlisted => AuthServiceError::IssuerMismatch,
        DiscoveryError::ProtectedResourceUnavailable | DiscoveryError::RequestFailed => {
            AuthServiceError::DiscoveryFailed
        }
        DiscoveryError::InvalidMetadata => AuthServiceError::DiscoveryMetadataInvalid,
    }
}

fn map_loopback_error(error: LoopbackError) -> AuthServiceError {
    match error {
        LoopbackError::Cancelled => AuthServiceError::Cancelled,
        LoopbackError::TimedOut => AuthServiceError::CallbackTimedOut,
        LoopbackError::InvalidCallback => AuthServiceError::CallbackInvalid,
        LoopbackError::BindFailed | LoopbackError::Io(_) => AuthServiceError::CallbackInvalid,
    }
}

fn map_oauth_error(error: OAuthError, redirect_uri: &str) -> AuthServiceError {
    match error {
        OAuthError::RedirectRejected => {
            AuthServiceError::RedirectNotRegistered(redirect_uri.to_owned())
        }
        OAuthError::ExchangeFailed | OAuthError::InvalidResponse => {
            AuthServiceError::TokenExchangeFailed
        }
    }
}

/// One outward code for every way the identity step can fail: the person can
/// do nothing about any of them, and telling them apart on screen would only
/// describe the tenant's configuration to whoever is reading it. The log has
/// already recorded which one it was.
fn map_identity_error(error: DaIdentityError) -> AuthServiceError {
    match error {
        DaIdentityError::Unauthorized
        | DaIdentityError::RequestFailed
        | DaIdentityError::UnexpectedStatus(_)
        | DaIdentityError::InvalidResponse
        | DaIdentityError::IncompleteIdentity
        | DaIdentityError::TenantMismatch { .. } => AuthServiceError::IdentityFailed,
    }
}
