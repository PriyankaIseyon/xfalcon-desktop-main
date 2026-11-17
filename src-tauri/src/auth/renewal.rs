//! Keeping a signed-in session signed in.
//!
//! Casdoor mints access tokens with a lifetime measured in an hour, and this app
//! is used for longer than that. Without renewal every session ends the same
//! way: a chat message, an upload, or a tool call fails, and the user is told to
//! sign in again for no reason they can see. The refresh token needed to avoid
//! that was already being stored — it simply had no caller.
//!
//! # Where this runs
//!
//! Every entry point that is about to spend the bearer calls
//! [`ensure_fresh_session`] first. That is deliberately not "a background timer":
//! a timer that has not fired yet leaves a stale token attached to the very next
//! request, and one that fires while the app is idle spends a network call to
//! renew a session nobody is using.
//!
//! # What it does not do
//!
//! It does not persist anything. The refresh token stays in memory for the
//! lifetime of the process, so closing the app still ends the session — writing a
//! long-lived credential to disk is a deliberate decision about credentials at
//! rest, not a detail to slip in behind a defect fix. (The device-sealed store
//! under `crate::credentials` is where such a decision would be implemented.)
//!
//! # Concurrency
//!
//! One process holds one session, and several tasks can notice its expiry at the
//! same moment. Renewal is serialized on a process-wide lock and re-checks the
//! session after acquiring it, so a burst of concurrent requests produces exactly
//! one refresh — the same discipline `crate::mcp::oauth::shared_grant` applies to
//! the MCP grant it shares with xfalcon-web.

use std::sync::OnceLock;

use reqwest::Client;
use tokio::sync::Mutex;
use xfalcon_contracts::auth::SessionProjection;

use super::config::AuthConfig;
use super::discovery::{build_discovery_client, discover_auth};
use super::oauth::{exchange_refresh_token, OAuthError, TokenBundle};
use super::session::{AuthSessionStore, RenewableSession};
use super::token::{ensure_identity_matches_token, verify_access_token, VerifiedAccessToken};

/// Renew this far before the token actually expires.
///
/// Sixty seconds covers the round trip plus any clock skew between this device
/// and the authorization server. The same leeway xfalcon-web applies to the MCP
/// grant it shares with this app (`REFRESH_LEEWAY`), for the same reason: a token
/// that expires *in flight* fails a request that had every right to succeed.
const RENEWAL_LEEWAY_MS: i64 = 60_000;

/// What a renewal attempt concluded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RenewalOutcome {
    /// The session is usable as it stands — either not near expiry, or renewed
    /// by this call or by a task that got there first.
    Fresh,
    /// There is no session to renew, or none with a refresh token. Callers treat
    /// this the same as `Fresh`: the request proceeds and whatever gate needs a
    /// session reports its absence in its own terms.
    NotRenewable,
    /// The authorization server refused the refresh token. The session cannot be
    /// recovered without the user, and has been cleared.
    ReauthenticationRequired,
    /// Renewal could not be attempted or completed — offline, the token endpoint
    /// unreachable, a malformed response. The existing token is left in place: it
    /// may still have seconds of life, and destroying a session over a transient
    /// failure is worse than one failed request.
    Unavailable,
}

/// Renews the session when its bearer is within [`RENEWAL_LEEWAY_MS`] of expiry.
///
/// Cheap and safe to call before every authenticated request: with a healthy
/// session it reads one timestamp and returns.
pub(crate) async fn ensure_fresh_session(sessions: &AuthSessionStore) -> RenewalOutcome {
    let Some(session) = sessions.renewable() else {
        return RenewalOutcome::NotRenewable;
    };
    if !is_near_expiry(session.expires_at_ms) {
        return RenewalOutcome::Fresh;
    }

    let _guard = renewal_lock().lock().await;
    // Re-read under the lock: while this task waited, another may have renewed
    // the session — in which case there is nothing left to do, and refreshing
    // again would spend a rotated token the first renewal already replaced.
    let Some(session) = sessions.renewable() else {
        return RenewalOutcome::NotRenewable;
    };
    if !is_near_expiry(session.expires_at_ms) {
        return RenewalOutcome::Fresh;
    }

    let Ok(config) = AuthConfig::from_env() else {
        return RenewalOutcome::Unavailable;
    };
    let Some(client) = renewal_client() else {
        return RenewalOutcome::Unavailable;
    };
    let Ok(discovered) = discover_auth(client, &config).await else {
        return RenewalOutcome::Unavailable;
    };

    let tokens =
        match exchange_refresh_token(client, &discovered, &config, session.refresh_token.as_str())
            .await
        {
            Ok(tokens) => tokens,
            // `invalid_grant`: revoked, already used, or past its own lifetime.
            // Nothing this device holds can recover the session.
            Err(OAuthError::RedirectRejected) => {
                sessions.clear();
                return RenewalOutcome::ReauthenticationRequired;
            }
            Err(_) => return RenewalOutcome::Unavailable,
        };

    // Verified, not merely accepted: the renewed token's audience, issuer, and
    // organization are re-checked exactly as they were at sign-in. An
    // authorization server that returned a token for a different resource would
    // otherwise be trusted purely because the refresh call returned 200.
    let Ok(verified) = verify_access_token(
        client,
        &config.casdoor_issuer,
        &discovered.jwks_uri,
        &discovered.resource,
        &config.organization,
        &tokens.access_token,
    )
    .await
    else {
        return RenewalOutcome::Unavailable;
    };

    commit_renewal(sessions, session, &discovered.resource, &verified, tokens)
}

/// The write half of a renewal, once both calls have returned. Refuses a
/// renewed token for another person, and writes only while the session is still
/// the one that was read: a sign-out or a new sign-in during the awaits must not
/// be undone by a refresh that started before it.
fn commit_renewal(
    sessions: &AuthSessionStore,
    session: RenewableSession,
    discovered_resource: &str,
    verified: &VerifiedAccessToken,
    tokens: TokenBundle,
) -> RenewalOutcome {
    // The names are the person's, so only the disagreement is logged.
    if ensure_identity_matches_token(verified, &session.identity).is_err() {
        tracing::warn!("renewed access token names a different user than the session");
        sessions.clear();
        return RenewalOutcome::ReauthenticationRequired;
    }

    // The identity is carried over rather than re-fetched: this is the same
    // subject it was a moment ago, and a DA round trip here would put the chat
    // data plane on the critical path of every renewal.
    let projection = SessionProjection::authenticated(session.identity, verified.expires_at_ms);
    let written = sessions.set_authenticated_if_generation(
        session.generation,
        projection,
        tokens.access_token,
        // An authorization server that does not rotate refresh tokens omits the
        // field; keeping the one already held is what lets the session renew a
        // second time.
        tokens
            .refresh_token
            .or_else(|| Some(session.refresh_token.to_string())),
    );
    if !written {
        tracing::debug!("session changed during renewal; the renewed tokens were dropped");
        return RenewalOutcome::NotRenewable;
    }
    // `set_authenticated_if_generation` drops the previous secrets, including the
    // audience the MCP client checks the bearer against. Restoring it from this
    // renewal's own discovery keeps `McpAuth::bind` able to attach the new token
    // — without it, renewing a session would silently disable every MCP tool.
    sessions.set_authorized_resource(
        session
            .authorized_resource
            .unwrap_or_else(|| discovered_resource.to_owned()),
    );
    RenewalOutcome::Fresh
}

fn is_near_expiry(expires_at_ms: i64) -> bool {
    let now = chrono::Utc::now().timestamp_millis();
    expires_at_ms.saturating_sub(now) <= RENEWAL_LEEWAY_MS
}

fn renewal_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// The same hardened client the discovery and sign-in paths use — TLS-only, no
/// redirects, bounded timeouts. Built once: a client per renewal would discard
/// the connection pool that makes the refresh fast enough to sit in front of a
/// request.
fn renewal_client() -> Option<&'static Client> {
    static CLIENT: OnceLock<Option<Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| build_discovery_client().ok())
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_expiring_inside_the_leeway_is_renewed_early() {
        let now = chrono::Utc::now().timestamp_millis();

        assert!(is_near_expiry(now + RENEWAL_LEEWAY_MS - 1_000));
        assert!(
            is_near_expiry(now - 5_000),
            "an already-expired token is certainly near expiry"
        );
    }

    #[test]
    fn a_healthy_token_is_left_alone() {
        let now = chrono::Utc::now().timestamp_millis();

        assert!(!is_near_expiry(now + RENEWAL_LEEWAY_MS + 60_000));
    }

    fn signed_in_store(subject: &str) -> AuthSessionStore {
        let sessions = AuthSessionStore::new();
        let identity =
            xfalcon_contracts::auth::IdentityProjection::new(subject, "tenant", "User", None);
        let expires_at_ms = chrono::Utc::now().timestamp_millis() + 3_600_000;
        sessions.set_authenticated(
            SessionProjection::authenticated(identity, expires_at_ms),
            "old-access-token",
            Some("held-refresh-token".to_owned()),
        );
        sessions
    }

    fn verified_for(subject: &str) -> VerifiedAccessToken {
        VerifiedAccessToken {
            owner: "tenant".to_owned(),
            preferred_username: subject.to_owned(),
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 3_600_000,
            nonce: None,
        }
    }

    fn renewed_tokens() -> TokenBundle {
        TokenBundle {
            access_token: "renewed-access-token".to_owned(),
            refresh_token: Some("rotated-refresh-token".to_owned()),
        }
    }

    #[test]
    fn a_renewal_that_finishes_after_sign_out_writes_nothing() {
        let sessions = signed_in_store("user");
        let session = sessions.renewable().expect("renewable");
        // The refresh is in flight when the person signs out.
        sessions.clear();

        let outcome = commit_renewal(
            &sessions,
            session,
            "https://tenant.example/mcp",
            &verified_for("user"),
            renewed_tokens(),
        );

        assert_eq!(outcome, RenewalOutcome::NotRenewable);
        assert!(sessions.authenticated_snapshot().is_none());
        assert!(sessions.access_token().is_none());
        assert!(matches!(
            sessions.projection(),
            SessionProjection::Unauthenticated { .. }
        ));
    }

    #[test]
    fn a_renewal_for_the_same_session_is_written() {
        let sessions = signed_in_store("user");
        let session = sessions.renewable().expect("renewable");

        let outcome = commit_renewal(
            &sessions,
            session,
            "https://tenant.example/mcp",
            &verified_for("user"),
            renewed_tokens(),
        );

        assert_eq!(outcome, RenewalOutcome::Fresh);
        assert_eq!(
            sessions.access_token().expect("token").as_str(),
            "renewed-access-token"
        );
        assert_eq!(
            sessions.authorized_resource().as_deref(),
            Some("https://tenant.example/mcp")
        );
    }

    #[test]
    fn a_renewed_token_for_another_subject_is_refused_and_clears_the_session() {
        let sessions = signed_in_store("user");
        let session = sessions.renewable().expect("renewable");

        let outcome = commit_renewal(
            &sessions,
            session,
            "https://tenant.example/mcp",
            &verified_for("someone-else"),
            renewed_tokens(),
        );

        assert_eq!(outcome, RenewalOutcome::ReauthenticationRequired);
        assert!(sessions.access_token().is_none());
        assert!(matches!(
            sessions.projection(),
            SessionProjection::Unauthenticated { .. }
        ));
    }

    #[tokio::test]
    async fn a_session_with_no_refresh_token_is_not_renewable() {
        // Reported distinctly from a failure: the caller must not treat "signed
        // out" as "renewal is broken", and must not clear a session it never had.
        let sessions = AuthSessionStore::new();

        assert_eq!(
            ensure_fresh_session(&sessions).await,
            RenewalOutcome::NotRenewable
        );
    }

    #[tokio::test]
    async fn a_healthy_session_is_reported_fresh_without_touching_the_network() {
        let sessions = AuthSessionStore::new();
        let identity =
            xfalcon_contracts::auth::IdentityProjection::new("user", "tenant", "User", None);
        let expires_at_ms = chrono::Utc::now().timestamp_millis() + 3_600_000;
        sessions.set_authenticated(
            SessionProjection::authenticated(identity, expires_at_ms),
            "access-token",
            Some("refresh-token".to_owned()),
        );

        // No wiremock server is configured, so any network attempt would report
        // `Unavailable` — `Fresh` here proves none was made.
        assert_eq!(ensure_fresh_session(&sessions).await, RenewalOutcome::Fresh);
    }
}
