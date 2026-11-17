use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

use xfalcon_contracts::auth::{IdentityProjection, SessionProjection};
use zeroize::{Zeroize, Zeroizing};

use super::canonical_account_id;
use super::token::CLOCK_TOLERANCE_SECONDS;
use crate::security::account_cleanup::CurrentAccountScope;

#[derive(Default)]
struct SessionSecrets {
    access_token: Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
    /// The `resource` value DA's Protected Resource Metadata returned at
    /// discovery time — what this bearer was actually minted for.
    ///
    /// Lives inside `SessionSecrets` rather than beside it so it cannot
    /// outlive the token it describes: every path that drops the secrets
    /// (`set`, `clear`) drops this with them. `crate::mcp::auth::McpAuth::bind`
    /// refuses to attach a bearer to a server without it, so a stale value
    /// here would be worse than none.
    authorized_resource: Option<String>,
}

impl Drop for SessionSecrets {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(ref mut refresh) = self.refresh_token {
            refresh.zeroize();
        }
    }
}

struct SessionState {
    projection: SessionProjection,
    secrets: Option<SessionSecrets>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            projection: SessionProjection::unauthenticated(),
            secrets: None,
        }
    }
}

pub(crate) struct AuthenticatedSessionSnapshot {
    pub(crate) identity: IdentityProjection,
    pub(crate) access_token: Zeroizing<String>,
    pub(crate) authorized_resource: Option<String>,
}

/// An authenticated session that can be renewed without the user, and the two
/// facts renewal needs: when the current bearer stops being accepted, and what
/// audience it was minted for.
pub(crate) struct RenewableSession {
    pub(crate) identity: IdentityProjection,
    pub(crate) expires_at_ms: i64,
    pub(crate) refresh_token: Zeroizing<String>,
    pub(crate) authorized_resource: Option<String>,
    /// The session's generation when it was read; see [`AuthSessionStore::generation`].
    pub(crate) generation: u64,
}

pub struct AuthSessionStore {
    state: RwLock<SessionState>,
    /// Bumped under the `state` write lock by every change of who is signed in,
    /// so a write made after an await can tell that the session moved.
    generation: AtomicU64,
    /// Runs when an authenticated session ends. See
    /// [`Self::set_session_end_hook`].
    session_end_hook: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl AuthSessionStore {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(SessionState::default()),
            generation: AtomicU64::new(0),
            session_end_hook: OnceLock::new(),
        }
    }

    /// Counts every change of who is signed in: `set`, `clear` and the
    /// `set_authenticated` family. A caller that reads the session, awaits, and
    /// then writes captures this first and writes only if it has not moved.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Registers the hook that runs when an authenticated session ends: when
    /// it is cleared, or when a new sign-in drops it. Set once at startup; a
    /// second registration is refused and logged.
    ///
    /// The hook runs after the session lock is released, so it may call into
    /// other state, but must not call back into this store.
    pub fn set_session_end_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        if self.session_end_hook.set(Box::new(hook)).is_err() {
            tracing::warn!("auth session end hook was already set");
        }
    }

    fn notify_session_ended(&self) {
        if let Some(hook) = self.session_end_hook.get() {
            hook();
        }
    }

    pub fn projection(&self) -> SessionProjection {
        self.state
            .read()
            .map(|state| state.projection.clone())
            .unwrap_or_else(|_| SessionProjection::unauthenticated())
    }

    /// Replaces the session with `session` and drops its secrets. Leaving an
    /// authenticated session fires the session-end hook.
    pub fn set(&self, session: SessionProjection) {
        let mut ended = false;
        if let Ok(mut state) = self.state.write() {
            ended = matches!(state.projection, SessionProjection::Authenticated { .. });
            state.projection = session;
            state.secrets = None;
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        if ended {
            self.notify_session_ended();
        }
    }

    pub fn set_authenticated(
        &self,
        session: SessionProjection,
        access_token: impl Into<String>,
        refresh_token: Option<String>,
    ) {
        let secrets = authenticated_secrets(access_token, refresh_token);
        if let Ok(mut state) = self.state.write() {
            state.projection = session;
            state.secrets = Some(secrets);
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Like [`Self::set_authenticated`], but writes only while the session is
    /// still at generation `expected`. Returns whether it wrote. The check and
    /// the write happen under one lock, so a change between them is impossible.
    pub(crate) fn set_authenticated_if_generation(
        &self,
        expected: u64,
        session: SessionProjection,
        access_token: impl Into<String>,
        refresh_token: Option<String>,
    ) -> bool {
        let secrets = authenticated_secrets(access_token, refresh_token);
        let Ok(mut state) = self.state.write() else {
            return false;
        };
        if self.generation() != expected {
            return false;
        }
        state.projection = session;
        state.secrets = Some(secrets);
        self.generation.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// Forgets the session and its secrets. Fires the session-end hook.
    pub fn clear(&self) {
        if let Ok(mut state) = self.state.write() {
            *state = SessionState::default();
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        self.notify_session_ended();
    }

    /// Records what the current session's bearer was minted for.
    ///
    /// A no-op when there is no authenticated session: a resource with no
    /// token to describe is meaningless, and storing one would let a later
    /// `set_authenticated` inherit a resource from a *different* login.
    ///
    /// Separate from [`Self::set_authenticated`] rather than a parameter on
    /// it because not every authenticated session has a discovered resource —
    /// only the OAuth flow does.
    pub fn set_authorized_resource(&self, resource: impl Into<String>) {
        if let Ok(mut state) = self.state.write() {
            if let Some(secrets) = state.secrets.as_mut() {
                secrets.authorized_resource = Some(resource.into());
            }
        }
    }

    /// What the current bearer was minted for, if the session was established
    /// through a flow that discovered it.
    pub fn authorized_resource(&self) -> Option<String> {
        self.state.read().ok().and_then(|state| {
            state
                .secrets
                .as_ref()
                .and_then(|secrets| secrets.authorized_resource.clone())
        })
    }

    pub fn access_token(&self) -> Option<Zeroizing<String>> {
        self.state.read().ok().and_then(|state| {
            state
                .secrets
                .as_ref()
                .map(|secrets| secrets.access_token.clone())
        })
    }

    /// What [`crate::auth::renewal`] needs to decide whether this session is
    /// about to expire, and to renew it if so.
    ///
    /// Returns `None` unless the session is authenticated *and* holds a refresh
    /// token: a session with no way to renew has nothing to decide.
    pub(crate) fn renewable(&self) -> Option<RenewableSession> {
        self.state.read().ok().and_then(|state| {
            let SessionProjection::Authenticated {
                identity,
                expires_at_ms,
                ..
            } = &state.projection
            else {
                return None;
            };
            let secrets = state.secrets.as_ref()?;
            Some(RenewableSession {
                identity: identity.clone(),
                expires_at_ms: *expires_at_ms,
                refresh_token: secrets.refresh_token.as_ref()?.clone(),
                authorized_resource: secrets.authorized_resource.clone(),
                generation: self.generation(),
            })
        })
    }

    /// Whether the account bound in `accounts` belongs to this session, and the
    /// session can still act for it.
    ///
    /// For commands that serve what this device holds for the account and reach
    /// no server that would check the bearer: those have nothing else between
    /// them and a webview that calls them directly. The account scope alone is
    /// not enough — it outlives the session whenever the session ends without a
    /// sign-out: a refused renewal, a cancelled or failed sign-in.
    ///
    /// Refused: no session, a session for another account, and a dead session —
    /// expired past [`CLOCK_TOLERANCE_SECONDS`] with no refresh token to renew
    /// it. An expired session that still holds a refresh token is accepted: it
    /// renews on its next request, and refusing it would turn every stretch
    /// offline into a sign-out.
    pub(crate) fn owns_bound_account(&self, accounts: &CurrentAccountScope) -> bool {
        let Ok(Some(account)) = accounts.current() else {
            return false;
        };
        let Ok(state) = self.state.read() else {
            return false;
        };
        let SessionProjection::Authenticated {
            identity,
            expires_at_ms,
            ..
        } = &state.projection
        else {
            return false;
        };
        let Some(secrets) = state.secrets.as_ref() else {
            return false;
        };
        if secrets.refresh_token.is_none() && is_past_expiry(*expires_at_ms) {
            return false;
        }
        canonical_account_id(identity).is_ok_and(|id| id == account.as_str())
    }

    pub(crate) fn authenticated_snapshot(&self) -> Option<AuthenticatedSessionSnapshot> {
        self.state.read().ok().and_then(|state| {
            let SessionProjection::Authenticated { identity, .. } = &state.projection else {
                return None;
            };
            let secrets = state.secrets.as_ref()?;
            Some(AuthenticatedSessionSnapshot {
                identity: identity.clone(),
                access_token: secrets.access_token.clone(),
                authorized_resource: secrets.authorized_resource.clone(),
            })
        })
    }
}

fn authenticated_secrets(
    access_token: impl Into<String>,
    refresh_token: Option<String>,
) -> SessionSecrets {
    SessionSecrets {
        access_token: Zeroizing::new(access_token.into()),
        refresh_token: refresh_token.map(Zeroizing::new),
        authorized_resource: None,
    }
}

/// Whether a bearer expiring at `expires_at_ms` is past the tolerance token
/// verification allows, so no server would still accept it.
fn is_past_expiry(expires_at_ms: i64) -> bool {
    let tolerance_ms = i64::try_from(CLOCK_TOLERANCE_SECONDS * 1_000).unwrap_or(i64::MAX);
    chrono::Utc::now().timestamp_millis() > expires_at_ms.saturating_add(tolerance_ms)
}

impl Default for AuthSessionStore {
    fn default() -> Self {
        Self::new()
    }
}
