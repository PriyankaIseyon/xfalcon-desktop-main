//! Signs the desktop in to the user's ChatGPT plan, keeps its access token
//! fresh, and reports what state the sign-in is in. The protocol steps live in
//! `oauth`, `callback` and `identity`; this file decides their order and what
//! each failure means to the rest of the app.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use reqwest::{Client, Url};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::auth::pkce::{generate_oauth_nonce, generate_oauth_state, generate_pkce};
use crate::providers::adapter::ProviderError;

use super::callback::{CallbackError, CallbackListener};
use super::identity::{fetch_jwks, verify_id_token, IdentityError};
use super::models::{self, ModelsError, OfferedModel};
use super::oauth::{
    build_authorize_url, build_http_client, exchange_code, fetch_discovery, refresh_tokens,
    revoke_refresh_token, AuthorizeRequest, Discovery, OAuthConfig, OAuthError, Registration,
    RevokeOutcome, PLAN_USAGE_SCOPE,
};
use super::store::{Meta, PlanStore, Profile, SessionRecord, StoreError};

const MODELS_ENDPOINT: &str = "https://api.openai.com/v1/models";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// A token with less life than this is refreshed before it is handed out.
const REFRESH_WITHIN_SECONDS: i64 = 60;
/// A token may be served before its `earliest_refresh_at` only while it has at
/// least this much life. Below it, a refresh happens whatever the server asked.
const DEFERRAL_MIN_LIFE_SECONDS: i64 = 20;
/// A refresh that failed for a reason that may pass keeps the current token
/// only while it has more life than this.
const FALLBACK_TOKEN_SECONDS: i64 = 5;
const REVOKE_DELAYS: [Duration; 2] = [Duration::from_millis(500), Duration::from_secs(1)];

/// Receives the sign-in URL and opens it in the system browser.
pub(crate) trait BrowserOpener: Send + Sync {
    fn open(&self, url: &Url) -> Result<(), BrowserOpenError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BrowserOpenError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionState {
    SignedOut,
    Ready,
    PlanUsageDisabled,
    NeedsSignIn,
    /// The stored sign-in exists but cannot be read now, for example while the
    /// keychain is locked. It may still be good, so the caller must not ask for a
    /// new sign-in.
    CheckFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionStatus {
    pub(crate) state: SessionState,
    pub(crate) welcome_pending: bool,
    pub(crate) account_label: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum SessionError {
    #[error("sign-in was cancelled")]
    Cancelled,
    #[error("sign-in timed out")]
    TimedOut,
    #[error("sign-in was declined")]
    Declined,
    #[error("the sign-in response was not valid")]
    Rejected,
    #[error("OpenAI did not accept the sign-in registration")]
    RegistrationRejected,
    #[error("could not reach OpenAI")]
    Network,
    #[error("the browser could not be opened")]
    Browser,
    #[error("the plan store failed")]
    Store,
}

/// Long-lived: its owner keeps one behind an `Arc`. Refreshes and session
/// writes are serialized on `refresh_lock`. A sign-out waits for a refresh in
/// flight, so no rotated token is written back after its session was deleted.
pub(crate) struct ChatgptPlanSession {
    store: PlanStore,
    config: OAuthConfig,
    http: Client,
    models_endpoint: Url,
    refresh_lock: AsyncMutex<()>,
    /// Makes each `meta.json` read-modify-write atomic against the others. It
    /// is never held across an await.
    meta_lock: Mutex<()>,
    /// A rotated record whose save failed. Its refresh token is the only valid one,
    /// so it is served, and saved again, before the sealed record is read. Written
    /// only under `refresh_lock`, and never held across an await.
    unsaved: Mutex<Option<SessionRecord>>,
}

/// The registration a sign-in asks OpenAI to recognize as an account this device
/// already has.
struct ReturningAttempt {
    client_id: String,
    /// The subject the saved profile records. A verified ID token must match it.
    subject: Option<String>,
    login_hint: Option<String>,
    id_token_hint: Option<String>,
    force_consent: bool,
    /// Set for a profile that stands in for a sign-in. A failure that a stale
    /// registration explains is then retried once as a new registration.
    may_be_stale: bool,
}

/// How one authorization round ended when it did not succeed.
enum AttemptError {
    /// The profile's registration is stale. The caller drops that profile and
    /// retries once as a new registration.
    Stale(String),
    Failed(SessionError),
}

impl From<SessionError> for AttemptError {
    fn from(error: SessionError) -> Self {
        Self::Failed(error)
    }
}

impl ChatgptPlanSession {
    pub(crate) fn new(
        store: PlanStore,
        config: OAuthConfig,
        models_endpoint: Url,
    ) -> Result<Self, SessionError> {
        let http = build_http_client(&config).map_err(|_| SessionError::Network)?;
        Ok(Self {
            store,
            config,
            http,
            models_endpoint,
            refresh_lock: AsyncMutex::new(()),
            meta_lock: Mutex::new(()),
            unsaved: Mutex::new(None),
        })
    }

    /// `None` only when a built-in URL stops parsing or the HTTP client cannot be
    /// built. Both are defects the caller must surface.
    pub(crate) fn production(store: PlanStore) -> Option<Self> {
        let config = OAuthConfig::production()?;
        let models_endpoint = Url::parse(MODELS_ENDPOINT).ok()?;
        Self::new(store, config, models_endpoint).ok()
    }

    /// Reads local state only. It never touches the network. Without an active
    /// client id the device is signed out, even when profiles remain.
    pub(crate) fn status(&self) -> SessionStatus {
        // Without a readable meta.json the active sign-in is unknown. It must not
        // read as SignedOut, because the caller would then skip the sign-out that
        // removes the stored tokens.
        let meta = match self.store.load_meta() {
            Ok(meta) => meta,
            Err(error) => return status_of_store_error(error),
        };
        if meta.active_client_id.is_none() {
            return SessionStatus {
                state: SessionState::SignedOut,
                welcome_pending: false,
                account_label: None,
            };
        }
        match self.active_record(&meta) {
            Ok(Some(record)) => {
                let state = if grants_plan_usage(&record.scopes) {
                    SessionState::Ready
                } else {
                    SessionState::PlanUsageDisabled
                };
                SessionStatus {
                    state,
                    welcome_pending: state == SessionState::Ready && !meta.welcome_acknowledged,
                    account_label: record.email,
                }
            }
            Ok(None) => needs_sign_in(),
            Err(error) => status_of_store_error(error),
        }
    }

    pub(crate) async fn sign_in(
        &self,
        opener: &dyn BrowserOpener,
        cancel: &CancellationToken,
        now_unix: i64,
    ) -> Result<SessionStatus, SessionError> {
        let discovery =
            fetch_discovery(&self.http, &self.config)
                .await
                .map_err(|error| match error {
                    OAuthError::DiscoveryRejected => SessionError::Rejected,
                    _ => SessionError::Network,
                })?;
        // A sign-in cancelled before it writes anything stops here.
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled);
        }
        let host_id = {
            let _guard = self.meta_guard();
            self.store.host_id().map_err(|_| SessionError::Store)?
        };
        let meta = self.store.load_meta().map_err(|_| SessionError::Store)?;
        let returning = self.returning_attempt(&meta);
        let first = self
            .attempt(
                opener,
                cancel,
                now_unix,
                &discovery,
                &host_id,
                returning.as_ref(),
            )
            .await;
        match first {
            Err(AttemptError::Stale(client_id)) => {
                {
                    let _refresh = self.refresh_lock.lock().await;
                    self.discard_profile(&client_id)?;
                }
                finish(
                    self.attempt(opener, cancel, now_unix, &discovery, &host_id, None)
                        .await,
                )
            }
            other => finish(other),
        }
    }

    /// A token good for at least a minute, refreshed first when it is not.
    pub(crate) async fn access_token(
        &self,
        now_unix: i64,
    ) -> Result<Zeroizing<String>, ProviderError> {
        if self.unsaved().is_none() {
            let record = self.usable_record()?;
            if token_is_good(&record, now_unix) {
                return Ok(Zeroizing::new(record.access_token));
            }
        }
        let _refresh = self.refresh_lock.lock().await;
        // A rotated record that never reached disk is saved first. Another caller may
        // have refreshed while this one waited for the lock, so the record is read again.
        let saved = self.save_unsaved();
        let record = self.usable_record()?;
        if token_is_good(&record, now_unix) {
            // A good token is served even when its save failed. The save is tried
            // again by the next caller.
            return Ok(Zeroizing::new(record.access_token));
        }
        saved?;
        self.refresh(&record, now_unix).await
    }

    pub(crate) async fn offered_models(
        &self,
        now_unix: i64,
    ) -> Result<Vec<OfferedModel>, ProviderError> {
        let token = self.access_token(now_unix).await?;
        let listed = models::list_models(&self.http, &self.models_endpoint, token.as_str())
            .await
            .map_err(|error| match error {
                ModelsError::Unauthorized => ProviderError::PlanSignInRequired,
                ModelsError::Forbidden => ProviderError::PlanNotEligible,
                ModelsError::Unavailable => ProviderError::Unavailable,
                ModelsError::Malformed => ProviderError::InvalidResponse,
            })?;
        Ok(models::offered_models(&listed))
    }

    /// Removes every sealed session on this device, then revokes the active
    /// sign-in's refresh token at OpenAI when it can. Other sealed sessions are
    /// removed without a revocation.
    ///
    /// Runs under `refresh_lock`, so no refresh or sign-in save can write a record
    /// back after it removes one. The revocation runs without the lock. A sign-in
    /// that is still in its browser step is stopped by the caller's cancel token,
    /// which the service sends before it calls this. A sign-in that is past its
    /// callback checks that token under this same lock before it saves, so it cannot
    /// make itself active after this removes the stored sign-ins.
    pub(crate) async fn sign_out(&self) -> RevokeOutcome {
        let (record, deleted) = {
            let _refresh = self.refresh_lock.lock().await;
            // Read before anything is removed. An unreadable meta.json names no
            // active sign-in, so there is nothing that can be revoked.
            let record = match self.store.load_meta() {
                Ok(meta) => match meta.active_client_id {
                    Some(client_id) => self.record_for(&client_id),
                    None => Ok(None),
                },
                Err(error) => Err(error),
            };
            *self.unsaved() = None;
            let deleted = self.store.delete_all_sessions().is_ok();
            {
                let _guard = self.meta_guard();
                // Written only when meta.json reads back. Overwriting an unreadable file
                // would wipe the profiles it still holds.
                if let Ok(mut meta) = self.store.load_meta() {
                    meta.active_client_id = None;
                    // The sealed records are gone either way. If this write fails, the
                    // active id names nothing, and status() reports NeedsSignIn.
                    let _ = self.store.save_meta(&meta);
                }
            }
            (record, deleted)
        };
        let outcome = match record {
            // No sealed record for the active client, so nothing was left to revoke.
            Ok(None) => RevokeOutcome::Confirmed,
            Ok(Some(SessionRecord {
                client_id,
                refresh_token: Some(refresh_token),
                ..
            })) => match fetch_discovery(&self.http, &self.config).await {
                Ok(discovery) => {
                    revoke_refresh_token(
                        &self.http,
                        &discovery,
                        &client_id,
                        &refresh_token,
                        &REVOKE_DELAYS,
                    )
                    .await
                }
                Err(_) => RevokeOutcome::Unconfirmed,
            },
            // A record without a refresh token, or one that cannot be read, is not revoked.
            Ok(Some(_)) | Err(_) => RevokeOutcome::Unconfirmed,
        };
        // A delete that failed leaves the tokens on disk, so the user must also
        // disconnect at OpenAI.
        if deleted {
            outcome
        } else {
            RevokeOutcome::Unconfirmed
        }
    }

    pub(crate) fn acknowledge_welcome(&self) -> Result<(), SessionError> {
        let _guard = self.meta_guard();
        let mut meta = self.store.load_meta().map_err(|_| SessionError::Store)?;
        meta.welcome_acknowledged = true;
        self.store.save_meta(&meta).map_err(|_| SessionError::Store)
    }

    /// The first registration to ask for: the active sign-in when its record loads,
    /// else the last profile, else none.
    fn returning_attempt(&self, meta: &Meta) -> Option<ReturningAttempt> {
        // A record that cannot be read here falls back to the saved profile, as a
        // missing record does.
        if let Some(record) = self.active_record(meta).ok().flatten() {
            return Some(ReturningAttempt {
                client_id: record.client_id.clone(),
                subject: subject_of(meta, &record.client_id),
                login_hint: record.email.clone(),
                id_token_hint: record.id_token.clone(),
                force_consent: !grants_plan_usage(&record.scopes),
                may_be_stale: false,
            });
        }
        meta.profiles.last().map(|profile| ReturningAttempt {
            client_id: profile.client_id.clone(),
            subject: Some(profile.subject.clone()),
            login_hint: profile.email.clone(),
            id_token_hint: None,
            force_consent: false,
            may_be_stale: true,
        })
    }

    /// One authorization round, with a fresh state, nonce, PKCE and listener. It
    /// ends in `Stale` only when `returning` is a profile and the failure is one
    /// that a stale registration produces.
    async fn attempt(
        &self,
        opener: &dyn BrowserOpener,
        cancel: &CancellationToken,
        now_unix: i64,
        discovery: &Discovery,
        host_id: &str,
        returning: Option<&ReturningAttempt>,
    ) -> Result<SessionStatus, AttemptError> {
        let registration = match returning {
            Some(returning) => Registration::Returning {
                client_id: returning.client_id.clone(),
                login_hint: returning.login_hint.clone(),
                id_token_hint: returning.id_token_hint.clone(),
                force_consent: returning.force_consent,
            },
            None => Registration::New,
        };
        let failed = |error: SessionError| match returning {
            Some(returning) if returning.may_be_stale => {
                AttemptError::Stale(returning.client_id.clone())
            }
            _ => AttemptError::Failed(error),
        };

        let pkce = generate_pkce();
        let state = generate_oauth_state();
        let nonce = generate_oauth_nonce();
        let listener = CallbackListener::bind(&state.value)
            .await
            .map_err(|_| SessionError::Browser)?;
        let redirect_uri = listener.redirect_uri();
        let authorize = build_authorize_url(
            discovery,
            &AuthorizeRequest {
                registration,
                host_id: host_id.to_owned(),
                redirect_uri: redirect_uri.clone(),
                state: state.value.clone(),
                nonce: nonce.value.clone(),
                code_challenge: pkce.challenge,
            },
        )
        .map_err(|_| SessionError::Rejected)?;
        opener.open(&authorize).map_err(|_| SessionError::Browser)?;
        let callback =
            listener
                .wait(cancel, SIGN_IN_TIMEOUT)
                .await
                .map_err(|error| match error {
                    CallbackError::Cancelled => SessionError::Cancelled,
                    CallbackError::TimedOut => SessionError::TimedOut,
                    CallbackError::StateMismatch => SessionError::Rejected,
                    CallbackError::BindFailed | CallbackError::Io => SessionError::Browser,
                })?;
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled.into());
        }
        if let Some(error) = callback.error {
            // A refusal at the consent screen says nothing about the saved
            // registration, so it neither retries nor drops the profile.
            if error == "access_denied" {
                return Err(AttemptError::Failed(SessionError::Declined));
            }
            // Only a registration OpenAI no longer recognizes is stale. Any other
            // error leaves the saved registration in place.
            if matches!(error.as_str(), "invalid_client" | "invalid_grant") {
                return Err(failed(SessionError::Rejected));
            }
            return Err(AttemptError::Failed(SessionError::Rejected));
        }
        let code = callback.code.ok_or(SessionError::Rejected)?;
        let client_id = match (returning, callback.client_id) {
            (Some(returning), Some(issued)) if issued != returning.client_id => {
                return Err(failed(SessionError::Rejected));
            }
            (Some(returning), _) => returning.client_id.clone(),
            (None, Some(issued)) if issued.starts_with("oaiapp_") => issued,
            (None, _) => return Err(AttemptError::Failed(SessionError::Rejected)),
        };

        let tokens = exchange_code(
            &self.http,
            discovery,
            &client_id,
            &code,
            &pkce.verifier,
            &redirect_uri,
        )
        .await
        .map_err(|error| match error {
            OAuthError::InvalidGrant => failed(SessionError::Rejected),
            OAuthError::InvalidClient => failed(SessionError::RegistrationRejected),
            _ => AttemptError::Failed(SessionError::Network),
        })?;
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled.into());
        }
        let id_token = tokens.id_token.ok_or(SessionError::Rejected)?;
        let jwks =
            fetch_jwks(&self.http, &discovery.jwks_uri)
                .await
                .map_err(|error| match error {
                    IdentityError::Network => SessionError::Network,
                    _ => SessionError::Rejected,
                })?;
        let identity = verify_id_token(
            &id_token,
            &jwks,
            &discovery.issuer,
            &client_id,
            &nonce.value,
            now_unix,
        )
        .map_err(|_| SessionError::Rejected)?;
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled.into());
        }
        if let Some(returning) = returning {
            if returning.subject.as_deref() != Some(identity.subject.as_str()) {
                return Err(SessionError::Rejected.into());
            }
        }

        let expires_in = i64::try_from(tokens.expires_in).map_err(|_| SessionError::Rejected)?;
        let record = SessionRecord {
            client_id: client_id.clone(),
            subject: identity.subject,
            email: identity.email,
            id_token: Some(id_token),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: now_unix.saturating_add(expires_in),
            earliest_refresh_at: tokens.earliest_refresh_at,
            scopes: tokens.scope.split_whitespace().map(str::to_owned).collect(),
            saved_at: now_unix,
        };
        // The save and the change of active sign-in happen under `refresh_lock`, which
        // sign-out also takes. Sign-out either ran first, so the cancel check below
        // sees the cancel the service sent before it. Or it waits for this block and
        // then removes the sign-in saved here.
        let _refresh = self.refresh_lock.lock().await;
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled.into());
        }
        self.store
            .save_session(&record)
            .map_err(|_| SessionError::Store)?;
        if cancel.is_cancelled() {
            // Cancelled after the save and before the account became active. The
            // record is removed again, so the cancelled sign-in leaves nothing behind.
            self.store
                .delete_session(&record.client_id)
                .map_err(|_| SessionError::Store)?;
            return Err(SessionError::Cancelled.into());
        }
        {
            // The record is saved first, so a profile never names a sign-in that
            // is not on disk.
            let _guard = self.meta_guard();
            let mut meta = self.store.load_meta().map_err(|_| SessionError::Store)?;
            let profile = Profile {
                client_id: record.client_id.clone(),
                subject: record.subject.clone(),
                email: record.email.clone(),
            };
            match meta
                .profiles
                .iter_mut()
                .find(|known| known.client_id == profile.client_id)
            {
                Some(known) => *known = profile,
                None => meta.profiles.push(profile),
            }
            meta.active_client_id = Some(record.client_id.clone());
            self.store
                .save_meta(&meta)
                .map_err(|_| SessionError::Store)?;
        }
        // The new sign-in replaces any rotated record that was held in memory.
        *self.unsaved() = None;
        Ok(self.status())
    }

    /// Drops a profile whose registration OpenAI no longer accepts, with its sealed
    /// record. An active sign-in on that client goes too, because nothing can resume
    /// it. The caller holds `refresh_lock`, so no refresh writes the record back.
    fn discard_profile(&self, client_id: &str) -> Result<(), SessionError> {
        {
            let _guard = self.meta_guard();
            let mut meta = self.store.load_meta().map_err(|_| SessionError::Store)?;
            meta.profiles
                .retain(|profile| profile.client_id != client_id);
            if meta.active_client_id.as_deref() == Some(client_id) {
                meta.active_client_id = None;
            }
            self.store
                .save_meta(&meta)
                .map_err(|_| SessionError::Store)?;
        }
        self.forget_unsaved(client_id);
        self.store
            .delete_session(client_id)
            .map_err(|_| SessionError::Store)
    }

    /// Ends the active sign-in after OpenAI rejects its token with 401. The sealed
    /// record goes and the profile stays, so `status()` reports NeedsSignIn. The
    /// adapter calls it when a turn fails with `PlanSignInRequired`.
    pub(crate) async fn mark_sign_in_required(&self) -> Result<(), SessionError> {
        let _refresh = self.refresh_lock.lock().await;
        let meta = self.store.load_meta().map_err(|_| SessionError::Store)?;
        let Some(client_id) = meta.active_client_id else {
            return Ok(());
        };
        self.forget_unsaved(&client_id);
        self.store
            .delete_session(&client_id)
            .map_err(|_| SessionError::Store)
    }

    /// Exchanges the stored refresh token for a new pair. Runs under `refresh_lock`.
    async fn refresh(
        &self,
        record: &SessionRecord,
        now_unix: i64,
    ) -> Result<Zeroizing<String>, ProviderError> {
        let Some(refresh_token) = record.refresh_token.as_deref() else {
            self.drop_session(&record.client_id)?;
            return Err(ProviderError::PlanSignInRequired);
        };
        let discovery = match fetch_discovery(&self.http, &self.config).await {
            Ok(discovery) => discovery,
            Err(_) => return fallback(record, now_unix),
        };
        let tokens =
            match refresh_tokens(&self.http, &discovery, &record.client_id, refresh_token).await {
                Ok(tokens) => tokens,
                Err(OAuthError::RefreshTerminal) => {
                    self.drop_session(&record.client_id)?;
                    return Err(ProviderError::PlanSignInRequired);
                }
                // OpenAI no longer recognizes the registration, so its profile goes
                // too and the next sign-in registers anew.
                Err(OAuthError::InvalidClient) => {
                    self.discard_profile(&record.client_id)
                        .map_err(|_| ProviderError::Unavailable)?;
                    return Err(ProviderError::PlanSignInRequired);
                }
                Err(_) => return fallback(record, now_unix),
            };
        // Each refresh rotates the refresh token, so an answer without one cannot
        // be stored in a usable way.
        let expires_in = i64::try_from(tokens.expires_in);
        let (Some(rotated), Ok(expires_in)) = (tokens.refresh_token, expires_in) else {
            return fallback(record, now_unix);
        };
        let scopes = if tokens.scope.trim().is_empty() {
            record.scopes.clone()
        } else {
            tokens.scope.split_whitespace().map(str::to_owned).collect()
        };
        let updated = SessionRecord {
            client_id: record.client_id.clone(),
            subject: record.subject.clone(),
            email: record.email.clone(),
            id_token: tokens.id_token.or_else(|| record.id_token.clone()),
            access_token: tokens.access_token,
            refresh_token: Some(rotated),
            expires_at: now_unix.saturating_add(expires_in),
            earliest_refresh_at: tokens.earliest_refresh_at,
            scopes,
            saved_at: now_unix,
        };
        // The rotated pair is the only valid one, so a failed save keeps it in memory.
        // It is served, and saved again, before anything reads the sealed record. The
        // failed call reports Unavailable; its retry is served from memory, so no
        // refresh runs with the rotated-away token.
        if self.store.save_session(&updated).is_err() {
            tracing::warn!("could not save a refreshed ChatGPT plan sign-in; it is kept in memory");
            *self.unsaved() = Some(updated);
            return Err(ProviderError::Unavailable);
        }
        Ok(Zeroizing::new(updated.access_token.clone()))
    }

    /// The active sign-in's record, when it can be used at all. A missing or
    /// damaged record needs a new sign-in, and a plan that is not granted cannot be
    /// used. A record or meta.json that cannot be read now is a store failure, not
    /// a sign-in that has ended.
    fn usable_record(&self) -> Result<SessionRecord, ProviderError> {
        let meta = self
            .store
            .load_meta()
            .map_err(|_| ProviderError::Unavailable)?;
        let client_id = meta
            .active_client_id
            .as_deref()
            .ok_or(ProviderError::PlanSignInRequired)?;
        let record = match self.record_for(client_id) {
            Ok(Some(record)) => record,
            Ok(None) | Err(StoreError::Corrupt) => return Err(ProviderError::PlanSignInRequired),
            Err(StoreError::Unreadable | StoreError::Unavailable) => {
                return Err(ProviderError::Unavailable)
            }
        };
        if !grants_plan_usage(&record.scopes) {
            return Err(ProviderError::PlanNotEligible);
        }
        Ok(record)
    }

    /// The active sign-in's record. `Ok(None)` when there is no active sign-in or its
    /// record is gone. A record that cannot be read or opened is an error, so it is
    /// not taken for a sign-in that ended.
    fn active_record(&self, meta: &Meta) -> Result<Option<SessionRecord>, StoreError> {
        match meta.active_client_id.as_deref() {
            Some(client_id) => self.record_for(client_id),
            None => Ok(None),
        }
    }

    /// The record for `client_id`: the rotated record held in memory when there is
    /// one, otherwise the sealed record.
    fn record_for(&self, client_id: &str) -> Result<Option<SessionRecord>, StoreError> {
        let unsaved = self.unsaved().clone();
        if let Some(record) = unsaved.filter(|record| record.client_id == client_id) {
            return Ok(Some(record));
        }
        self.store.load_session(client_id)
    }

    /// Writes the rotated record that a failed save left in memory. The caller holds
    /// `refresh_lock`.
    fn save_unsaved(&self) -> Result<(), ProviderError> {
        let unsaved = self.unsaved().clone();
        let Some(record) = unsaved else {
            return Ok(());
        };
        if self.store.save_session(&record).is_err() {
            tracing::warn!("could not save a refreshed ChatGPT plan sign-in; it is kept in memory");
            return Err(ProviderError::Unavailable);
        }
        *self.unsaved() = None;
        Ok(())
    }

    /// Clears the rotated record held in memory when it belongs to `client_id`.
    fn forget_unsaved(&self, client_id: &str) {
        let mut unsaved = self.unsaved();
        if unsaved
            .as_ref()
            .is_some_and(|record| record.client_id == client_id)
        {
            *unsaved = None;
        }
    }

    /// Removes a dead sign-in's tokens. The profile and the active client id stay,
    /// so `status()` reports NeedsSignIn until the user signs in again. The caller
    /// holds `refresh_lock`.
    fn drop_session(&self, client_id: &str) -> Result<(), ProviderError> {
        self.forget_unsaved(client_id);
        self.store
            .delete_session(client_id)
            .map_err(|_| ProviderError::Unavailable)
    }

    /// The rotated record held in memory, if any. Never held across an await.
    fn unsaved(&self) -> MutexGuard<'_, Option<SessionRecord>> {
        self.unsaved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Held only around a `meta.json` read-modify-write, never across an await.
    fn meta_guard(&self) -> MutexGuard<'_, ()> {
        self.meta_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn grants_plan_usage(scopes: &[String]) -> bool {
    scopes.iter().any(|scope| scope == PLAN_USAGE_SCOPE)
}

/// The subject a saved profile records for `client_id`.
fn subject_of(meta: &Meta, client_id: &str) -> Option<String> {
    meta.profiles
        .iter()
        .find(|profile| profile.client_id == client_id)
        .map(|profile| profile.subject.clone())
}

/// Turns one attempt's outcome into the caller's result. A new registration is
/// never stale, so `Stale` is reported as a failure rather than retried.
fn finish(result: Result<SessionStatus, AttemptError>) -> Result<SessionStatus, SessionError> {
    match result {
        Ok(status) => Ok(status),
        Err(AttemptError::Failed(error)) => Err(error),
        Err(AttemptError::Stale(_)) => Err(SessionError::Rejected),
    }
}

fn needs_sign_in() -> SessionStatus {
    SessionStatus {
        state: SessionState::NeedsSignIn,
        welcome_pending: false,
        account_label: None,
    }
}

fn check_failed() -> SessionStatus {
    SessionStatus {
        state: SessionState::CheckFailed,
        welcome_pending: false,
        account_label: None,
    }
}

/// The status for a store read that failed. A file or key that cannot be read is
/// a failed check, because the sign-in may still be good. Damaged bytes cannot be
/// used, so they need a new sign-in.
fn status_of_store_error(error: StoreError) -> SessionStatus {
    match error {
        StoreError::Unreadable | StoreError::Unavailable => check_failed(),
        StoreError::Corrupt => needs_sign_in(),
    }
}

/// The stored token needs no refresh yet: it has at least `REFRESH_WITHIN_SECONDS`
/// of life, or it has at least `DEFERRAL_MIN_LIFE_SECONDS` of life and the server
/// asked that no refresh happen before `earliest_refresh_at`. Below that floor the
/// token is refreshed whatever `earliest_refresh_at` says.
fn token_is_good(record: &SessionRecord, now_unix: i64) -> bool {
    let remaining = record.expires_at.saturating_sub(now_unix);
    remaining >= REFRESH_WITHIN_SECONDS
        || (remaining >= DEFERRAL_MIN_LIFE_SECONDS
            && record.earliest_refresh_at.is_some_and(|at| at > now_unix))
}

/// A refresh that failed for a reason that may pass keeps the sign-in. The
/// current token is used only while it still has a few seconds of life.
fn fallback(record: &SessionRecord, now_unix: i64) -> Result<Zeroizing<String>, ProviderError> {
    if record.expires_at.saturating_sub(now_unix) > FALLBACK_TOKEN_SECONDS {
        return Ok(Zeroizing::new(record.access_token.clone()));
    }
    Err(ProviderError::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::path::Path;
    use std::sync::Arc;

    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use crate::persistence::{InMemorySecretStore, SecretStore};
    use crate::providers::chatgpt_plan::store::FlakySecrets;
    use crate::security::account_cleanup::ValidatedAccountId;

    use super::*;

    const NOW: i64 = 1_800_000_000;
    const CLIENT: &str = "oaiapp_issued_client";
    const HOST_ID: &str = "urn:uuid:00000000-0000-4000-8000-000000000001";
    const SUBJECT: &str = "user-123";
    const EMAIL: &str = "person@example.com";
    const PLAN_SCOPE: &str =
        "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
    const NO_PLAN_SCOPE: &str = "openid profile email offline_access resource.invoke";
    const RSA_KID: &str = "rsa-test";
    const RSA_PRIVATE_PEM: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAoSm0iPW8QHXs1t6cVRmec32HpZKecwoUXhXFXSqCOj5tc3JK
C5b7rPAKFvdRIvn3z8eTwF2GzWxhJUU3bYj84DyGoNEysV5Z1C9SCmIeo79EO0BX
uE1uCwyWPmYAgfUXA+WJv0b+zdEZCROH6RYXiGfkV1negHdhUKV6jtdytlU8Oemq
UFTZO19wKMeKrcr5Ry2UjJPjmyAGGkfkVRWP2Ga6uDxvjnMDlzjKjO8Y5/qDrlrQ
ADg6cG8u+uetbTnf4yOaj/2wrNExNP1r8mSdkbx9ut3mLPN639I7wPyPWGa4ifNc
CUUk9xaJiKA58ZafB5Kalcgqikk+WBD4hvzcJQIDAQABAoIBAAnW60ISaL4xxPZA
bKCIeiNYdmvuNfTVmHU/FIfx5LYMIjASegFXpEGEfUU2L9K4GsBpkbSEsUqA87Rs
E6yBGNlS8LBIXXU1Srw48GQYZgD+2Dpa3RAwUAfmKpUgMqc8kL1o6f5O5hcoTUQ/
2EQrmF+4YhlQMBS9VShwlfk7H7KQjEsfHi9fW6hilz7y2siOZwLwVD2AFYXmXCnx
e5XmxV9X/nJAYUARm03+shqgK6QaXiWQaa6bonl5AXhXmv0kBvZ2c7+TcKmu5ke9
ihlCbwQhxDgjqacqa00EL2aq3wuavlyIuHsSt9uq8RGCIdWN6WrxXM6kH0Q2KEUs
o16NkOECgYEA2UhDWd3NZ+Q0xZujpv1PBwdl690DApz6ZoX2UDIHL5iCpIDvJaW9
40I4g1ooy5xWJUwySaBn8JImbcCprq/vn+hQS8mySbVwHo4VIeYK7bAcnTDdYfPq
fWIJnpxQzpd+T81C7T6LYe2ckw7EOFvss4ZN4Kca2ap1zPrsaoKCqUUCgYEAveFz
0HzsTibk0tsweocoPXfBsoSExOgU0xRx+5FqM6Xufzk3aE2EZcq1UbfucsUwAUwA
XJgQU8dArlTF5kBWCBgJqUXDPMutRn+y/Psvhcc/Jht77mSBQj3y0F5hI34Dppqd
VXP41aWBJTuLyQvd+C6rGCSFTWXmo1O2K6LQ5WECgYBmnmJwxviD4Jur2F3qOChi
wXO5eaA6U7lPQmWaaoZKTFUnLF3GAgk0k8ns/1/VCfkFlH8Llt6AGAZeP2fkQOOD
Okzx/khd13skcyY35FW0/EAHXVSxNmnA3dR9PIZNi+7z8ZVyRt+9WGNicM7rbz7T
atLNDx73PjJ0pO2TQCBFoQKBgAuwDvLsXZdJ6ZfZChjgrOdrsa6NXabJKk772TBE
+SaIti96lCCFdPrXCWYoTjMeVCAgT31jls2xXBfe+/d6SlFhOWcD9kqNtvK59YiA
5aH0a7yqwGwiH5q2OXX9+Ti4Dv14Uurpoo10DIkAK1CKvHUEWl+4D8R92z5+bp4s
/XLBAoGASce8fdfKPzyvhYljCsBTmTQjjtY7dQh0MrBZynwiaoYQ0CCCcoZ3ftIf
S/hjoedDMjrV/6w6egWFRYhvP+ofiu6jRzvE3rTN9kFLDqKAF5RanYODA0PgLN+H
i1yytNYlOMX+CYCaLrGFGrUXj9XB4fMVgm2n+Nz6tVTOhASPTuU=
-----END RSA PRIVATE KEY-----"#;
    const RSA_MODULUS: &str = "oSm0iPW8QHXs1t6cVRmec32HpZKecwoUXhXFXSqCOj5tc3JKC5b7rPAKFvdRIvn3z8eTwF2GzWxhJUU3bYj84DyGoNEysV5Z1C9SCmIeo79EO0BXuE1uCwyWPmYAgfUXA-WJv0b-zdEZCROH6RYXiGfkV1negHdhUKV6jtdytlU8OemqUFTZO19wKMeKrcr5Ry2UjJPjmyAGGkfkVRWP2Ga6uDxvjnMDlzjKjO8Y5_qDrlrQADg6cG8u-uetbTnf4yOaj_2wrNExNP1r8mSdkbx9ut3mLPN639I7wPyPWGa4ifNcCUUk9xaJiKA58ZafB5Kalcgqikk-WBD4hvzcJQ";

    fn secrets() -> Arc<InMemorySecretStore> {
        Arc::new(InMemorySecretStore::default())
    }

    fn plan_store(root: &Path, secrets: &Arc<InMemorySecretStore>) -> PlanStore {
        let account = ValidatedAccountId::parse("person:tenant").expect("account");
        PlanStore::new(root, &account, secrets.clone())
    }

    fn new_session(
        root: &Path,
        secrets: &Arc<InMemorySecretStore>,
        issuer: &str,
    ) -> ChatgptPlanSession {
        let config = OAuthConfig::for_test(Url::parse(issuer).expect("issuer"));
        let models = Url::parse(&format!("{issuer}/v1/models")).expect("models url");
        ChatgptPlanSession::new(plan_store(root, secrets), config, models).expect("session")
    }

    fn session_record(scopes: &str, expires_at: i64) -> SessionRecord {
        SessionRecord {
            client_id: CLIENT.to_owned(),
            subject: SUBJECT.to_owned(),
            email: Some(EMAIL.to_owned()),
            id_token: Some("id-token-old".to_owned()),
            access_token: "access-old".to_owned(),
            refresh_token: Some("refresh-old".to_owned()),
            expires_at,
            earliest_refresh_at: None,
            scopes: scopes.split_whitespace().map(str::to_owned).collect(),
            saved_at: NOW - 3_000,
        }
    }

    fn saved_profile() -> Profile {
        Profile {
            client_id: CLIENT.to_owned(),
            subject: SUBJECT.to_owned(),
            email: Some(EMAIL.to_owned()),
        }
    }

    /// An active sign-in with its profile, as an earlier run would have left it.
    fn seed(
        root: &Path,
        secrets: &Arc<InMemorySecretStore>,
        record: &SessionRecord,
        welcome_acknowledged: bool,
    ) {
        let store = plan_store(root, secrets);
        store.save_session(record).expect("seed session");
        store
            .save_meta(&Meta {
                host_id: Some(HOST_ID.to_owned()),
                active_client_id: Some(CLIENT.to_owned()),
                welcome_acknowledged,
                profiles: vec![saved_profile()],
            })
            .expect("seed meta");
    }

    /// A profile with no active sign-in, as a sign-out leaves one.
    fn seed_profile(root: &Path, secrets: &Arc<InMemorySecretStore>, profile: Profile) {
        plan_store(root, secrets)
            .save_meta(&Meta {
                profiles: vec![profile],
                ..Meta::default()
            })
            .expect("seed profile");
    }

    fn id_token(issuer: &str, audience: &str, nonce: &str) -> String {
        let claims = json!({
            "iss": issuer,
            "aud": audience,
            "exp": NOW + 600,
            "sub": SUBJECT,
            "nonce": nonce,
            "email": EMAIL,
        });
        let mut jwt_header = Header::new(Algorithm::RS256);
        jwt_header.kid = Some(RSA_KID.to_owned());
        let key = EncodingKey::from_rsa_pem(RSA_PRIVATE_PEM.as_bytes()).expect("test RSA key");
        encode(&jwt_header, &claims, &key).expect("signed ID token")
    }

    async fn mount_discovery(server: &MockServer) {
        mount_discovery_document(server).await;
        mount_jwks(
            server,
            ResponseTemplate::new(200).set_body_json(json!({
                "keys": [{
                    "kty": "RSA",
                    "kid": RSA_KID,
                    "use": "sig",
                    "alg": "RS256",
                    "n": RSA_MODULUS,
                    "e": "AQAB",
                }]
            })),
        )
        .await;
    }

    /// The discovery document alone, for a test that answers the key set itself.
    async fn mount_discovery_document(server: &MockServer) {
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/api/accounts/authorize"),
                "token_endpoint": format!("{base}/api/accounts/oauth/token"),
                "jwks_uri": format!("{base}/.well-known/jwks.json"),
                "revocation_endpoint": format!("{base}/api/accounts/oauth/revoke"),
            })))
            .mount(server)
            .await;
    }

    async fn mount_jwks(server: &MockServer, reply: impl Respond + 'static) {
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(reply)
            .mount(server)
            .await;
    }

    /// Answers the code exchange with an ID token that echoes the nonce of the
    /// attempt, so the verification in `sign_in` has to pass for real.
    struct SignInTokens {
        nonce: Arc<Mutex<String>>,
        issuer: String,
        scope: &'static str,
    }

    impl Respond for SignInTokens {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let nonce = self.nonce.lock().expect("nonce").clone();
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-1",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": self.scope,
                "refresh_token": "refresh-1",
                "id_token": id_token(&self.issuer, CLIENT, &nonce),
            }))
        }
    }

    async fn mount_sign_in_tokens(
        server: &MockServer,
        nonce: &Arc<Mutex<String>>,
        scope: &'static str,
    ) {
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .respond_with(SignInTokens {
                nonce: nonce.clone(),
                issuer: server.uri(),
                scope,
            })
            .expect(1)
            .mount(server)
            .await;
    }

    /// Only a refresh request carrying the seeded refresh token gets `reply`.
    async fn mount_refresh(server: &MockServer, reply: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .and(body_string_contains("refresh_token=refresh-old"))
            .respond_with(reply)
            .expect(1)
            .mount(server)
            .await;
    }

    /// Stands in for the system browser. Each `open` consumes the next reply in
    /// `callbacks`, records the authorize URL and the nonce in it, then sends the
    /// redirect itself, as the browser would.
    struct FakeBrowser {
        nonce: Arc<Mutex<String>>,
        callbacks: Mutex<Vec<Vec<(&'static str, &'static str)>>>,
        urls: Mutex<Vec<String>>,
    }

    impl FakeBrowser {
        /// One reply per expected browser visit, in order.
        fn new(
            nonce: &Arc<Mutex<String>>,
            callbacks: Vec<Vec<(&'static str, &'static str)>>,
        ) -> Self {
            Self {
                nonce: nonce.clone(),
                callbacks: Mutex::new(callbacks),
                urls: Mutex::new(Vec::new()),
            }
        }

        fn opened_urls(&self) -> Vec<String> {
            self.urls.lock().expect("urls").clone()
        }

        fn authorize_url(&self) -> String {
            self.opened_urls()
                .pop()
                .expect("an authorize URL was opened")
        }
    }

    impl BrowserOpener for FakeBrowser {
        fn open(&self, url: &Url) -> Result<(), BrowserOpenError> {
            let query = |key: &str| {
                url.query_pairs()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.into_owned())
            };
            let redirect = query("redirect_uri")
                .and_then(|raw| Url::parse(&raw).ok())
                .ok_or(BrowserOpenError)?;
            let state = query("state").ok_or(BrowserOpenError)?;
            let reply = {
                let mut callbacks = self.callbacks.lock().expect("callbacks");
                if callbacks.is_empty() {
                    return Err(BrowserOpenError);
                }
                callbacks.remove(0)
            };
            *self.nonce.lock().expect("nonce") = query("nonce").ok_or(BrowserOpenError)?;
            self.urls.lock().expect("urls").push(url.to_string());

            let mut callback = redirect.clone();
            callback
                .query_pairs_mut()
                .append_pair("state", &state)
                .extend_pairs(reply.iter());
            let port = redirect.port().ok_or(BrowserOpenError)?;
            let target = format!(
                "{}?{}",
                callback.path(),
                callback.query().unwrap_or_default()
            );
            tokio::spawn(send_redirect(port, target));
            Ok(())
        }
    }

    /// Sends the browser's GET with the loopback host the listener requires.
    async fn send_redirect(port: u16, target: String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect to the listener");
        let request =
            format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write the redirect");
        let mut reply = Vec::new();
        stream
            .read_to_end(&mut reply)
            .await
            .expect("read the reply");
    }

    async fn sign_in(
        session: &ChatgptPlanSession,
        browser: &FakeBrowser,
    ) -> Result<SessionStatus, SessionError> {
        sign_in_with(session, browser, &CancellationToken::new()).await
    }

    async fn sign_in_with(
        session: &ChatgptPlanSession,
        browser: &dyn BrowserOpener,
        cancel: &CancellationToken,
    ) -> Result<SessionStatus, SessionError> {
        tokio::time::timeout(
            Duration::from_secs(30),
            session.sign_in(browser, cancel, NOW),
        )
        .await
        .expect("sign-in finished in time")
    }

    /// A browser that cancels the sign-in as it opens, before any callback arrives.
    struct CancelOnOpen(CancellationToken);

    impl BrowserOpener for CancelOnOpen {
        fn open(&self, _url: &Url) -> Result<(), BrowserOpenError> {
            self.0.cancel();
            Ok(())
        }
    }

    /// Answers the token exchange, cancelling the sign-in first. The exchange has
    /// completed by the time the answer is read, so the cancel lands after it.
    struct CancelWithTokens {
        cancel: CancellationToken,
        tokens: SignInTokens,
    }

    impl Respond for CancelWithTokens {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            self.cancel.cancel();
            self.tokens.respond(request)
        }
    }

    /// Answers the key set request, cancelling the sign-in first.
    struct CancelWithJwks {
        cancel: CancellationToken,
        jwks: serde_json::Value,
    }

    impl Respond for CancelWithJwks {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            self.cancel.cancel();
            ResponseTemplate::new(200).set_body_json(self.jwks.clone())
        }
    }

    fn jwks_body() -> serde_json::Value {
        json!({
            "keys": [{
                "kty": "RSA",
                "kid": RSA_KID,
                "use": "sig",
                "alg": "RS256",
                "n": RSA_MODULUS,
                "e": "AQAB",
            }]
        })
    }

    /// A session whose store fails the key creations and key reads a test arms.
    fn flaky_session(root: &Path, flaky: &Arc<FlakySecrets>, issuer: &str) -> ChatgptPlanSession {
        let account = ValidatedAccountId::parse("person:tenant").expect("account");
        let store = PlanStore::new(root, &account, flaky.clone());
        let config = OAuthConfig::for_test(Url::parse(issuer).expect("issuer"));
        let models = Url::parse(&format!("{issuer}/v1/models")).expect("models url");
        ChatgptPlanSession::new(store, config, models).expect("session")
    }

    #[tokio::test]
    async fn new_sign_in_stores_the_issued_client_and_returns_ready() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", CLIENT), ("code", "auth-code")]],
        );

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(
            status,
            SessionStatus {
                state: SessionState::Ready,
                welcome_pending: true,
                account_label: Some(EMAIL.to_owned()),
            }
        );
        let url = browser.authorize_url();
        assert!(url.contains("client_id=dynamic_agent_client"));
        assert!(url.contains("agent_name_hint="));
        let store = plan_store(dir.path(), &secrets);
        let meta = store.load_meta().expect("meta");
        assert_eq!(meta.active_client_id.as_deref(), Some(CLIENT));
        assert_eq!(meta.profiles, vec![saved_profile()]);
        assert!(meta.host_id.is_some_and(|id| id.starts_with("urn:uuid:")));
        let record = store
            .load_session(CLIENT)
            .expect("load")
            .expect("saved record");
        assert_eq!(record.access_token, "access-1");
        assert_eq!(record.refresh_token.as_deref(), Some("refresh-1"));
        assert_eq!(record.expires_at, NOW + 3600);
        assert!(record.scopes.iter().any(|scope| scope == PLAN_USAGE_SCOPE));
        server.verify().await;
    }

    #[tokio::test]
    async fn sign_in_without_the_plan_scope_is_kept_with_plan_usage_disabled() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, NO_PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", CLIENT), ("code", "auth-code")]],
        );

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(status.state, SessionState::PlanUsageDisabled);
        assert!(!status.welcome_pending);
        assert_eq!(
            session
                .access_token(NOW)
                .await
                .expect_err("plan not granted"),
            ProviderError::PlanNotEligible
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn declined_callback_gives_declined_and_stores_nothing() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(&nonce, vec![vec![("error", "access_denied")]]);

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Declined)
        );
        assert!(plan_store(dir.path(), &secrets)
            .load_meta()
            .expect("meta")
            .active_client_id
            .is_none());
    }

    #[tokio::test]
    async fn returning_sign_in_rejects_a_callback_naming_another_client() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            false,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", "oaiapp_other"), ("code", "auth-code")]],
        );

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Rejected)
        );
        assert_eq!(browser.opened_urls().len(), 1);
        let url = browser.authorize_url();
        assert!(url.contains("client_id=oaiapp_issued_client"));
        assert!(url.contains("id_token_hint=id-token-old"));
        assert!(url.contains("login_hint="));
        assert!(!url.contains("prompt=consent"));
        assert!(!url.contains("agent_name_hint"));
        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .profiles,
            vec![saved_profile()]
        );
    }

    #[tokio::test]
    async fn profile_only_sign_in_reuses_the_saved_client_id_and_login_hint() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(&nonce, vec![vec![("code", "auth-code")]]);

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(status.state, SessionState::Ready);
        let url = browser.authorize_url();
        assert!(url.contains("client_id=oaiapp_issued_client"));
        assert!(url.contains("login_hint=person%40example.com"));
        assert!(!url.contains("id_token_hint"));
        assert!(!url.contains("prompt=consent"));
        assert!(!url.contains("agent_name_hint"));
        let meta = plan_store(dir.path(), &secrets).load_meta().expect("meta");
        assert_eq!(meta.profiles, vec![saved_profile()]);
        assert_eq!(meta.active_client_id.as_deref(), Some(CLIENT));
        server.verify().await;
    }

    #[tokio::test]
    async fn sign_in_after_sign_out_reuses_the_saved_client_id() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/revoke"))
            .and(body_string_contains("token=refresh-old"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());
        assert_eq!(session.sign_out().await, RevokeOutcome::Confirmed);
        let browser = FakeBrowser::new(&nonce, vec![vec![("code", "auth-code")]]);

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(status.state, SessionState::Ready);
        let url = browser.authorize_url();
        assert!(url.contains("client_id=oaiapp_issued_client"));
        assert!(url.contains("login_hint="));
        assert!(!url.contains("id_token_hint"));
        server.verify().await;
    }

    #[tokio::test]
    async fn stale_profile_error_is_retried_once_as_a_new_registration() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![
                vec![("error", "invalid_client")],
                vec![("client_id", CLIENT), ("code", "auth-code")],
            ],
        );

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(status.state, SessionState::Ready);
        let urls = browser.opened_urls();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("client_id=oaiapp_issued_client"));
        assert!(urls[0].contains("login_hint="));
        assert!(urls[1].contains("client_id=dynamic_agent_client"));
        assert!(urls[1].contains("agent_name_hint="));
        let meta = plan_store(dir.path(), &secrets).load_meta().expect("meta");
        assert_eq!(meta.profiles, vec![saved_profile()]);
        assert_eq!(meta.active_client_id.as_deref(), Some(CLIENT));
        server.verify().await;
    }

    #[tokio::test]
    async fn invalid_grant_on_a_profile_sign_in_is_retried_once_as_a_new_registration() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![
                vec![("code", "stale-code")],
                vec![("client_id", CLIENT), ("code", "auth-code")],
            ],
        );

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(status.state, SessionState::Ready);
        let urls = browser.opened_urls();
        assert_eq!(urls.len(), 2);
        assert!(urls[1].contains("client_id=dynamic_agent_client"));
        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .profiles,
            vec![saved_profile()]
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn a_callback_naming_another_client_on_a_profile_sign_in_retries_as_new() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![
                vec![("client_id", "oaiapp_other"), ("code", "auth-code")],
                vec![("client_id", CLIENT), ("code", "auth-code")],
            ],
        );

        let status = sign_in(&session, &browser).await.expect("signed in");

        assert_eq!(status.state, SessionState::Ready);
        assert_eq!(browser.opened_urls().len(), 2);
        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .profiles,
            vec![saved_profile()]
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn second_failure_returns_the_error_and_drops_the_stale_profile() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![
                vec![("error", "invalid_client")],
                vec![("error", "access_denied")],
            ],
        );

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Declined)
        );
        assert_eq!(browser.opened_urls().len(), 2);
        let meta = plan_store(dir.path(), &secrets).load_meta().expect("meta");
        assert!(meta.profiles.is_empty());
        assert!(meta.active_client_id.is_none());
        assert_eq!(session.status().state, SessionState::SignedOut);
    }

    #[tokio::test]
    async fn access_denied_on_a_profile_sign_in_is_declined_without_a_retry() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        // One reply only: a retry would find no browser visit left and fail as
        // Browser, so the Declined result below rules the retry out too.
        let browser = FakeBrowser::new(&nonce, vec![vec![("error", "access_denied")]]);

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Declined)
        );
        assert_eq!(browser.opened_urls().len(), 1);
        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .profiles,
            vec![saved_profile()]
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn profile_sign_in_with_another_subject_is_rejected_and_keeps_the_profile() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let other = Profile {
            client_id: CLIENT.to_owned(),
            subject: "someone-else".to_owned(),
            email: Some(EMAIL.to_owned()),
        };
        seed_profile(dir.path(), &secrets, other.clone());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(&nonce, vec![vec![("code", "auth-code")]]);

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Rejected)
        );
        assert_eq!(browser.opened_urls().len(), 1);
        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .profiles,
            vec![other]
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn fresh_token_is_returned_without_a_network_call() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        let token = session.access_token(NOW).await.expect("token");

        assert_eq!(token.as_str(), "access-old");
        assert!(server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());
    }

    #[tokio::test]
    async fn expiring_token_refreshes_once_for_concurrent_callers() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        mount_refresh(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-new",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": PLAN_SCOPE,
                "refresh_token": "refresh-new",
            })),
        )
        .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 30),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        let (first, second) = tokio::join!(session.access_token(NOW), session.access_token(NOW));

        assert_eq!(first.expect("first caller").as_str(), "access-new");
        assert_eq!(second.expect("second caller").as_str(), "access-new");
        server.verify().await;
        let record = plan_store(dir.path(), &secrets)
            .load_session(CLIENT)
            .expect("load")
            .expect("record");
        assert_eq!(record.refresh_token.as_deref(), Some("refresh-new"));
        assert_eq!(record.access_token, "access-new");
        assert_eq!(record.id_token.as_deref(), Some("id-token-old"));
        assert_eq!(record.expires_at, NOW + 3600);
    }

    #[tokio::test]
    async fn terminal_refresh_error_deletes_the_session_and_asks_for_sign_in() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        mount_refresh(
            &server,
            ResponseTemplate::new(400).set_body_json(json!({"error": "refresh_token_reused"})),
        )
        .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 30),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        assert_eq!(
            session.access_token(NOW).await.expect_err("sign-in needed"),
            ProviderError::PlanSignInRequired
        );
        assert!(plan_store(dir.path(), &secrets)
            .load_session(CLIENT)
            .expect("load")
            .is_none());
        // The active client id stays, so the device reports a sign-in that is needed.
        assert_eq!(session.status().state, SessionState::NeedsSignIn);
        server.verify().await;
    }

    #[tokio::test]
    async fn temporary_refresh_failure_keeps_a_token_that_still_has_life() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        mount_refresh(&server, ResponseTemplate::new(503)).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 30),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        let token = session.access_token(NOW).await.expect("still usable");

        assert_eq!(token.as_str(), "access-old");
        server.verify().await;
        let record = plan_store(dir.path(), &secrets)
            .load_session(CLIENT)
            .expect("load")
            .expect("record kept");
        assert_eq!(record.refresh_token.as_deref(), Some("refresh-old"));
    }

    #[tokio::test]
    async fn sign_out_revokes_deletes_the_session_and_clears_the_active_client() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/revoke"))
            .and(body_string_contains("token=refresh-old"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        assert_eq!(session.sign_out().await, RevokeOutcome::Confirmed);

        server.verify().await;
        let store = plan_store(dir.path(), &secrets);
        assert!(store.load_session(CLIENT).expect("load").is_none());
        let meta = store.load_meta().expect("meta");
        assert_eq!(meta.active_client_id, None);
        assert_eq!(meta.profiles, vec![saved_profile()]);
        assert_eq!(session.status().state, SessionState::SignedOut);
        assert_eq!(
            session.access_token(NOW).await.expect_err("signed out"),
            ProviderError::PlanSignInRequired
        );
    }

    #[test]
    fn status_reports_each_state_from_local_records_only() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, "http://127.0.0.1:9");
        assert_eq!(
            session.status(),
            SessionStatus {
                state: SessionState::SignedOut,
                welcome_pending: false,
                account_label: None,
            }
        );

        // A profile without an active client id is still signed out.
        seed_profile(dir.path(), &secrets, saved_profile());
        assert_eq!(session.status().state, SessionState::SignedOut);

        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            false,
        );
        assert_eq!(
            session.status(),
            SessionStatus {
                state: SessionState::Ready,
                welcome_pending: true,
                account_label: Some(EMAIL.to_owned()),
            }
        );

        seed(
            dir.path(),
            &secrets,
            &session_record(NO_PLAN_SCOPE, NOW + 3600),
            false,
        );
        assert_eq!(
            session.status(),
            SessionStatus {
                state: SessionState::PlanUsageDisabled,
                welcome_pending: false,
                account_label: Some(EMAIL.to_owned()),
            }
        );

        // Without its key the sealed record cannot be opened. The keyring cannot say
        // whether the key is gone or only locked, so this is a failed check, and the
        // sign-in is not taken for a sign-in that ended.
        secrets
            .delete_account_keys("person:tenant")
            .expect("delete keys");
        assert_eq!(session.status().state, SessionState::CheckFailed);
    }

    #[test]
    fn acknowledging_the_welcome_clears_welcome_pending() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            false,
        );
        let session = new_session(dir.path(), &secrets, "http://127.0.0.1:9");
        assert!(session.status().welcome_pending);

        session.acknowledge_welcome().expect("acknowledged");

        assert_eq!(
            session.status(),
            SessionStatus {
                state: SessionState::Ready,
                welcome_pending: false,
                account_label: Some(EMAIL.to_owned()),
            }
        );
    }

    #[tokio::test]
    async fn offered_models_lists_only_the_models_with_documented_limits() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer access-old"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [
                    {"slug": "gpt-6-nova", "display_name": "GPT-6 Nova", "visibility": "list"},
                    {"slug": "unlisted-model", "display_name": "Unlisted", "visibility": "list"},
                    {"slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list"},
                    {"slug": "gpt-6-luna", "display_name": "GPT-6 Luna", "visibility": "hide"},
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        let offered = session.offered_models(NOW).await.expect("models");

        assert_eq!(
            offered,
            vec![OfferedModel {
                slug: "gpt-6.1-sol".to_owned(),
                display_name: "Sol".to_owned(),
                context_length: 1_050_000,
                max_output_tokens: 128_000,
            }]
        );
        server.verify().await;
    }

    /// The app's commands run these futures on its multi-threaded runtime, so
    /// each must be `Send`. The test only has to compile.
    #[test]
    fn session_futures_are_send() {
        fn assert_send<F: Future + Send>(_: &F) {}
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, "http://127.0.0.1:9");
        let browser = FakeBrowser::new(&Arc::new(Mutex::new(String::new())), Vec::new());
        let cancel = CancellationToken::new();
        assert_send(&session.sign_in(&browser, &cancel, NOW));
        assert_send(&session.access_token(NOW));
        assert_send(&session.offered_models(NOW));
        assert_send(&session.sign_out());
    }

    #[test]
    fn debug_output_of_the_session_types_shows_no_token_or_identity() {
        let record = session_record(PLAN_SCOPE, NOW + 3600);
        let status = SessionStatus {
            state: SessionState::Ready,
            welcome_pending: true,
            account_label: None,
        };
        let debug = format!("{record:?} {status:?} {:?}", saved_profile());
        for secret in ["access-old", "refresh-old", "id-token-old", SUBJECT, EMAIL] {
            assert!(!debug.contains(secret), "{secret} appears in {debug}");
        }
    }

    /// Nothing of the sign-in is stored: no active client, no profile, no sealed record.
    /// The host id made before the browser opened is not part of the sign-in.
    fn assert_no_sign_in_stored(root: &Path, secrets: &Arc<InMemorySecretStore>) {
        let store = plan_store(root, secrets);
        let meta = store.load_meta().expect("meta");
        assert_eq!(meta.active_client_id, None);
        assert!(meta.profiles.is_empty());
        assert!(store.load_session(CLIENT).expect("load").is_none());
    }

    #[tokio::test]
    async fn a_cancel_during_the_browser_visit_writes_nothing() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let cancel = CancellationToken::new();

        let result = sign_in_with(&session, &CancelOnOpen(cancel.clone()), &cancel).await;

        assert_eq!(result, Err(SessionError::Cancelled));
        assert_no_sign_in_stored(dir.path(), &secrets);
    }

    #[tokio::test]
    async fn a_cancel_after_the_token_exchange_writes_nothing() {
        let server = MockServer::start().await;
        mount_discovery_document(&server).await;
        mount_jwks(
            &server,
            ResponseTemplate::new(200).set_body_json(jwks_body()),
        )
        .await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let cancel = CancellationToken::new();
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .respond_with(CancelWithTokens {
                cancel: cancel.clone(),
                tokens: SignInTokens {
                    nonce: nonce.clone(),
                    issuer: server.uri(),
                    scope: PLAN_SCOPE,
                },
            })
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", CLIENT), ("code", "auth-code")]],
        );

        let result = sign_in_with(&session, &browser, &cancel).await;

        assert_eq!(result, Err(SessionError::Cancelled));
        server.verify().await;
        assert_no_sign_in_stored(dir.path(), &secrets);
    }

    #[tokio::test]
    async fn a_cancel_after_the_key_set_is_fetched_writes_nothing() {
        let server = MockServer::start().await;
        mount_discovery_document(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let cancel = CancellationToken::new();
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        mount_jwks(
            &server,
            CancelWithJwks {
                cancel: cancel.clone(),
                jwks: jwks_body(),
            },
        )
        .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", CLIENT), ("code", "auth-code")]],
        );

        let result = sign_in_with(&session, &browser, &cancel).await;

        assert_eq!(result, Err(SessionError::Cancelled));
        server.verify().await;
        assert_no_sign_in_stored(dir.path(), &secrets);
    }

    #[tokio::test]
    async fn a_callback_error_other_than_a_stale_registration_keeps_the_profile() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(&nonce, vec![vec![("error", "invalid_request")]]);

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Rejected)
        );
        assert_eq!(browser.opened_urls().len(), 1, "no retry");
        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .profiles,
            vec![saved_profile()]
        );
    }

    #[tokio::test]
    async fn a_profile_discarded_as_stale_takes_its_sealed_session_with_it() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed_profile(dir.path(), &secrets, saved_profile());
        plan_store(dir.path(), &secrets)
            .save_session(&session_record(PLAN_SCOPE, NOW + 3600))
            .expect("seed session");
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![
                vec![("error", "invalid_client")],
                vec![("error", "access_denied")],
            ],
        );

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Declined)
        );
        let store = plan_store(dir.path(), &secrets);
        assert!(store.load_session(CLIENT).expect("load").is_none());
        assert!(store.load_meta().expect("meta").profiles.is_empty());
    }

    #[tokio::test]
    async fn a_code_exchange_refused_as_an_invalid_client_is_a_refused_registration() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_client"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let nonce = Arc::new(Mutex::new(String::new()));
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", CLIENT), ("code", "auth-code")]],
        );

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::RegistrationRejected)
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn a_key_set_that_cannot_be_fetched_is_a_network_failure() {
        let server = MockServer::start().await;
        mount_discovery_document(&server).await;
        mount_jwks(&server, ResponseTemplate::new(503)).await;
        let nonce = Arc::new(Mutex::new(String::new()));
        mount_sign_in_tokens(&server, &nonce, PLAN_SCOPE).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let session = new_session(dir.path(), &secrets, &server.uri());
        let browser = FakeBrowser::new(
            &nonce,
            vec![vec![("client_id", CLIENT), ("code", "auth-code")]],
        );

        assert_eq!(
            sign_in(&session, &browser).await,
            Err(SessionError::Network)
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn sign_out_removes_every_sealed_session_not_only_the_active_one() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/revoke"))
            .and(body_string_contains("token=refresh-old"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let orphan = SessionRecord {
            client_id: "oaiapp_orphan".to_owned(),
            ..session_record(PLAN_SCOPE, NOW + 3600)
        };
        plan_store(dir.path(), &secrets)
            .save_session(&orphan)
            .expect("orphan session");
        let session = new_session(dir.path(), &secrets, &server.uri());

        assert_eq!(session.sign_out().await, RevokeOutcome::Confirmed);

        server.verify().await;
        let store = plan_store(dir.path(), &secrets);
        assert!(store.load_session("oaiapp_orphan").expect("load").is_none());
        assert!(store.load_session(CLIENT).expect("load").is_none());
    }

    #[tokio::test]
    async fn sign_out_with_an_unreadable_meta_removes_the_sealed_sessions_and_keeps_meta() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let store = plan_store(dir.path(), &secrets);
        store
            .save_session(&session_record(PLAN_SCOPE, NOW + 3600))
            .expect("save");
        std::fs::create_dir_all(store.meta_path()).expect("meta as a directory");
        let session = new_session(dir.path(), &secrets, &server.uri());

        // The active sign-in is unknown, so nothing can be revoked.
        assert_eq!(session.sign_out().await, RevokeOutcome::Unconfirmed);

        assert!(store.load_session(CLIENT).expect("load").is_none());
        assert!(store.meta_path().is_dir(), "meta was replaced");
    }

    #[tokio::test]
    async fn sign_out_with_no_sealed_record_reports_confirmed() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        plan_store(dir.path(), &secrets)
            .save_meta(&Meta {
                host_id: Some(HOST_ID.to_owned()),
                active_client_id: Some(CLIENT.to_owned()),
                welcome_acknowledged: true,
                profiles: vec![saved_profile()],
            })
            .expect("meta");
        let session = new_session(dir.path(), &secrets, &server.uri());

        assert_eq!(session.sign_out().await, RevokeOutcome::Confirmed);

        assert_eq!(
            plan_store(dir.path(), &secrets)
                .load_meta()
                .expect("meta")
                .active_client_id,
            None
        );
        assert!(server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());
    }

    #[tokio::test]
    async fn mark_sign_in_required_drops_the_sealed_record_and_keeps_the_profile() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let session = new_session(dir.path(), &secrets, "http://127.0.0.1:9");
        assert_eq!(session.status().state, SessionState::Ready);

        session.mark_sign_in_required().await.expect("marked");

        assert_eq!(session.status().state, SessionState::NeedsSignIn);
        let store = plan_store(dir.path(), &secrets);
        assert!(store.load_session(CLIENT).expect("load").is_none());
        let meta = store.load_meta().expect("meta");
        assert_eq!(meta.active_client_id.as_deref(), Some(CLIENT));
        assert_eq!(meta.profiles, vec![saved_profile()]);
    }

    #[tokio::test]
    async fn a_refresh_refused_as_an_invalid_client_drops_the_profile() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        mount_refresh(
            &server,
            ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_client"})),
        )
        .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 30),
            true,
        );
        let session = new_session(dir.path(), &secrets, &server.uri());

        assert_eq!(
            session.access_token(NOW).await.expect_err("sign-in needed"),
            ProviderError::PlanSignInRequired
        );

        server.verify().await;
        let store = plan_store(dir.path(), &secrets);
        let meta = store.load_meta().expect("meta");
        assert!(meta.profiles.is_empty());
        assert_eq!(meta.active_client_id, None);
        assert!(store.load_session(CLIENT).expect("load").is_none());
    }

    #[tokio::test]
    async fn a_rotated_token_whose_save_failed_is_served_and_saved_again() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        mount_refresh(&server, rotated_reply()).await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 30),
            true,
        );
        let flaky = FlakySecrets::new(&secrets);
        flaky
            .failing_creates
            .store(1, std::sync::atomic::Ordering::SeqCst);
        let session = flaky_session(dir.path(), &flaky, &server.uri());

        // The rotated pair could not be saved, so this call fails without a retry.
        assert_eq!(
            session.access_token(NOW).await.expect_err("save failed"),
            ProviderError::Unavailable
        );
        // The next call serves the rotated token from memory and saves it. No second
        // refresh runs with the rotated-away token.
        assert_eq!(
            session.access_token(NOW).await.expect("served").as_str(),
            "access-new"
        );

        server.verify().await;
        let record = plan_store(dir.path(), &secrets)
            .load_session(CLIENT)
            .expect("load")
            .expect("record");
        assert_eq!(record.refresh_token.as_deref(), Some("refresh-new"));
        assert_eq!(record.access_token, "access-new");
    }

    #[tokio::test]
    async fn sign_out_revokes_the_rotated_token_and_forgets_the_unsaved_record() {
        let server = MockServer::start().await;
        mount_discovery(&server).await;
        mount_refresh(&server, rotated_reply()).await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/revoke"))
            .and(body_string_contains("token=refresh-new"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 30),
            true,
        );
        let flaky = FlakySecrets::new(&secrets);
        flaky
            .failing_creates
            .store(1, std::sync::atomic::Ordering::SeqCst);
        let session = flaky_session(dir.path(), &flaky, &server.uri());
        session
            .access_token(NOW)
            .await
            .expect_err("the rotated pair is not saved");

        assert_eq!(session.sign_out().await, RevokeOutcome::Confirmed);

        assert_eq!(
            session.access_token(NOW).await.expect_err("signed out"),
            ProviderError::PlanSignInRequired
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn a_locked_keychain_fails_the_run_as_unavailable_and_keeps_the_record() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let flaky = FlakySecrets::new(&secrets);
        flaky
            .failing_reads
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let session = flaky_session(dir.path(), &flaky, &server.uri());

        assert_eq!(
            session.access_token(NOW).await.expect_err("locked"),
            ProviderError::Unavailable
        );

        // Once the keychain answers again, the record that the failed run left alone
        // is still there to be used.
        flaky
            .failing_reads
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            session.access_token(NOW).await.expect("unlocked").as_str(),
            "access-old"
        );
        assert!(server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());
    }

    #[test]
    fn an_unreadable_meta_reads_as_check_failed_not_signed_out() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        std::fs::create_dir_all(plan_store(dir.path(), &secrets).meta_path())
            .expect("meta as a directory");
        let session = new_session(dir.path(), &secrets, "http://127.0.0.1:9");

        assert_eq!(session.status().state, SessionState::CheckFailed);
    }

    #[test]
    fn a_damaged_meta_reads_as_needs_sign_in() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        let meta_path = plan_store(dir.path(), &secrets).meta_path();
        std::fs::create_dir_all(meta_path.parent().expect("meta has a parent"))
            .expect("store directory");
        std::fs::write(&meta_path, b"not json").expect("damaged meta");
        let session = new_session(dir.path(), &secrets, "http://127.0.0.1:9");

        assert_eq!(session.status().state, SessionState::NeedsSignIn);
    }

    #[test]
    fn a_locked_key_reads_as_check_failed_and_the_sign_in_is_kept() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = secrets();
        seed(
            dir.path(),
            &secrets,
            &session_record(PLAN_SCOPE, NOW + 3600),
            true,
        );
        let flaky = FlakySecrets::new(&secrets);
        flaky
            .failing_reads
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let session = flaky_session(dir.path(), &flaky, "http://127.0.0.1:9");

        assert_eq!(session.status().state, SessionState::CheckFailed);
        assert!(plan_store(dir.path(), &secrets)
            .load_session(CLIENT)
            .expect("the sealed record is still there")
            .is_some());
    }

    #[test]
    fn a_deferred_token_needs_twenty_seconds_of_life() {
        let mut deferred = session_record(PLAN_SCOPE, NOW + 19);
        deferred.earliest_refresh_at = Some(NOW + 1000);
        assert!(
            !token_is_good(&deferred, NOW),
            "19 s left: refresh whatever the server asked"
        );

        let mut held = session_record(PLAN_SCOPE, NOW + 21);
        held.earliest_refresh_at = Some(NOW + 1000);
        assert!(token_is_good(&held, NOW), "21 s left: the deferral holds");

        let mut past = session_record(PLAN_SCOPE, NOW + 21);
        past.earliest_refresh_at = Some(NOW - 1);
        assert!(!token_is_good(&past, NOW), "a deferral that has passed");

        let mut plenty = session_record(PLAN_SCOPE, NOW + 61);
        plenty.earliest_refresh_at = None;
        assert!(token_is_good(&plenty, NOW), "61 s left needs no deferral");
    }

    /// A refresh answer that rotates the refresh token.
    fn rotated_reply() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-new",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": PLAN_SCOPE,
            "refresh_token": "refresh-new",
        }))
    }

    #[test]
    fn plan_usage_needs_the_exact_scope_name() {
        assert!(grants_plan_usage(&[PLAN_USAGE_SCOPE.to_owned()]));
        assert!(grants_plan_usage(&[
            "openid".to_owned(),
            "profile".to_owned(),
            PLAN_USAGE_SCOPE.to_owned(),
        ]));
        assert!(!grants_plan_usage(&[]));
        for near_miss in [
            "chatgpt.tokens.use.directx",
            "chatgpt.tokens.use",
            "chatgpt.tokens.use.direct ",
            "CHATGPT.TOKENS.USE.DIRECT",
            "openid chatgpt.tokens.use.direct",
        ] {
            assert!(
                !grants_plan_usage(&[near_miss.to_owned()]),
                "{near_miss:?} must not grant plan usage"
            );
        }
    }
}
