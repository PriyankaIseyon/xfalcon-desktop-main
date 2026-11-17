//! Resolving a bearer from the **shared** DA grant, with the same concurrency
//! protocol xfalcon-web uses.
//!
//! A port of `resolveBearerFromGrant` in web's
//! `src/lib/chat-api/mcp-oauth-grants.ts`. The constants and the protocol are
//! deliberately identical, because desktop and web operate on the *same row*:
//! two clients running different refresh disciplines against one shared
//! credential is precisely how a working grant gets destroyed — both call the
//! authorization server, and the loser's refresh token is invalidated by the
//! winner's rotation.
//!
//! The protocol, per attempt (max [`MAX_REFRESH_ATTEMPTS`]):
//!
//! 1. **Load** the grant. No refresh token ⇒ reauthorization required.
//! 2. **Fast path.** Access token still outside [`REFRESH_LEEWAY`] ⇒ decrypt and
//!    return it. This is also what picks up a peer's refresh that landed between
//!    two of this caller's attempts.
//! 3. **Version-pinned lease.** `:acquire-refresh-lease` with the version just
//!    read. Not acquired ⇒ sleep [`lease_poll_delay`] and re-loop, so the next
//!    pass fast-paths the winner's token.
//! 4. **Refresh** at the authorization server, then `:commit-refresh` under the
//!    same version CAS, which clears the lease atomically on success.
//! 5. **CAS lost while holding the lease** ⇒ re-read to disambiguate: same
//!    version means a permission wall (bail loudly rather than spin), advanced
//!    version means a peer committed (re-loop and fast-path).
//! 6. **Release** the lease unless the commit already cleared it.
//!
//! A definitive 4xx from the token endpoint is a dead grant and surfaces as
//! [`SharedGrantError::ReauthorizationRequired`]; a 5xx, timeout, or network
//! failure stays transient and propagates for the caller's own retry.

use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rand::Rng;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::clients::da::mcp_oauth_grants::{
    DaMcpGrantRepository, GrantRecord, GrantRepositoryError, RefreshCommit,
};

use super::envelope::GrantKey;
use super::exchange::{self, ExchangeError, TokenRequest};
use super::registration::OauthClient;

/// Leeway against the authorization server's `expires_in`. Web's
/// `REFRESH_LEEWAY_MS`.
pub(crate) const REFRESH_LEEWAY: Duration = Duration::from_secs(60);

/// How long a claimed lease is valid. Web's `REFRESH_LEASE_MS`: long enough for a
/// realistic authorization-server round trip plus the commit, short enough that a
/// crashed holder does not block peers for long.
pub(crate) const REFRESH_LEASE: Duration = Duration::from_secs(10);

/// Web's `MAX_REFRESH_ATTEMPTS`.
pub(crate) const MAX_REFRESH_ATTEMPTS: u32 = 3;

const LEASE_POLL_BASE: Duration = Duration::from_millis(200);
const LEASE_POLL_MAX: Duration = Duration::from_millis(1500);

/// Web's `leasePollDelayMs`: exponential from 200 ms, capped at 1500 ms, jittered
/// 0.5×–1.5× so concurrent waiters do not re-hit the row in lockstep.
#[must_use]
pub(crate) fn lease_poll_delay(attempt: u32) -> Duration {
    let exponential = LEASE_POLL_BASE
        .saturating_mul(1u32 << attempt.min(8))
        .min(LEASE_POLL_MAX);
    let jitter = rand::rng().random_range(0.5..1.5);
    exponential.mul_f64(jitter)
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum SharedGrantError {
    /// The grant is absent, has no refresh token, or the authorization server has
    /// definitively refused it. The user must sign in again. Web's
    /// `MCPOAuthGrantMissingError` / `OAUTH_REAUTH_REQUIRED`.
    #[error("this server needs to be signed in to again")]
    ReauthorizationRequired,
    /// No shared key is configured, so this build cannot read the tenant's grant
    /// even though one may exist. Distinct from every other arm because the fix
    /// is a deployment setting, not a user action.
    #[error("this build cannot read the shared grant: no shared grant key is configured")]
    NotConfigured,
    #[error("the tenant's grant store is unavailable")]
    Unavailable,
    /// The refresh could not be completed for a reason that may pass: a 5xx from
    /// the authorization server, a timeout, contention that outlasted the retry
    /// budget.
    #[error("the grant could not be renewed right now")]
    Transient,
    /// The commit was refused while this caller held the lease and the version had
    /// not moved — a permission wall rather than a race. Reported rather than
    /// retried, because looping cannot clear it.
    #[error("the tenant's grant store refused the renewal")]
    RenewalRefused,
}

impl From<GrantRepositoryError> for SharedGrantError {
    fn from(error: GrantRepositoryError) -> Self {
        match error {
            GrantRepositoryError::NotFound => Self::ReauthorizationRequired,
            GrantRepositoryError::Unauthorized
            | GrantRepositoryError::Unavailable
            | GrantRepositoryError::InvalidRow => Self::Unavailable,
        }
    }
}

/// The client identity used to authenticate the refresh, and the scope to
/// request with it. Web reads both from the server row's decrypted `auth_config`;
/// desktop cannot decrypt that column, so the same values are supplied by
/// whoever established the grant (`super::flow`) and re-read from the device.
pub(crate) struct SharedGrantClient<'a> {
    pub(crate) client: &'a OauthClient,
    /// Fallback scope when the stored grant carries none, matching web's
    /// `grant.scope ?? oauthConfig.scope`.
    pub(crate) scope: Option<&'a str>,
}

/// The OAuth client an operator registered on the web app, read back out of the
/// server row.
///
/// This is the piece that makes a server configured once on web work on desktop
/// with no per-device input: web wrote `client_id` + `client_secret` into
/// `mcp_servers.auth_config`, and a desktop configured with the same
/// key-encryption key decrypts exactly what web decrypts.
#[must_use]
pub(crate) fn client_from_row(
    config: &crate::clients::da::mcp_servers::DecryptedOauthConfig,
) -> OauthClient {
    OauthClient {
        client_id: config.client_id.clone(),
        client_secret: Some(config.client_secret.clone()),
    }
}

/// Returns a bearer for `server_id`, refreshing the shared grant if it is due.
pub(crate) async fn resolve_bearer(
    http: &reqwest::Client,
    grants: &DaMcpGrantRepository,
    key: &GrantKey,
    server_id: &str,
    server: &crate::mcp::server_resolver::McpServerDescriptor,
    identity: &SharedGrantClient<'_>,
) -> Result<Zeroizing<String>, SharedGrantError> {
    let mut attempt: u32 = 0;
    while attempt < MAX_REFRESH_ATTEMPTS {
        attempt += 1;
        let grant = grants.load(server_id).await?;
        // A shared registry edit must not move an existing bearer (or its
        // renewal) to another resource. Re-check every lease retry as well.
        if !crate::mcp::auth::resource_matches_server(&grant.resource, server) {
            return Err(SharedGrantError::ReauthorizationRequired);
        }
        let Some(refresh_enc) = grant
            .refresh_token_enc
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            // Web: a grant with no refresh token cannot be renewed, so it is
            // treated as absent rather than used until it silently expires.
            return Err(SharedGrantError::ReauthorizationRequired);
        };

        if let Some(access) = usable_access_token(&grant, key) {
            return Ok(access);
        }

        let holder = Uuid::new_v4().to_string();
        let acquired = grants
            .acquire_refresh_lease(&grant.id, grant.refresh_version, REFRESH_LEASE, &holder)
            .await?;
        if !acquired.acquired {
            tokio::time::sleep(lease_poll_delay(attempt - 1)).await;
            continue;
        }

        let outcome = renew_under_lease(&Renewal {
            http,
            grants,
            key,
            server_id,
            grant: &grant,
            refresh_enc,
            identity,
            holder: &holder,
        })
        .await;
        match outcome {
            RenewalOutcome::Renewed(access) => return Ok(access),
            RenewalOutcome::Failed(error) => {
                // The lease is released before returning, so a failure does not
                // block a peer for the whole lease window.
                let _ = grants.release_refresh_lease(&grant.id, &holder).await;
                return Err(error);
            }
            RenewalOutcome::PeerCommitted => {
                let _ = grants.release_refresh_lease(&grant.id, &holder).await;
                continue;
            }
        }
    }
    Err(SharedGrantError::Transient)
}

enum RenewalOutcome {
    Renewed(Zeroizing<String>),
    /// The version moved under this caller: a peer committed, so re-loop and take
    /// the fast path on their token.
    PeerCommitted,
    Failed(SharedGrantError),
}

/// One renewal attempt's inputs, grouped so the several `&str`s among them
/// cannot be transposed at the call site.
struct Renewal<'a> {
    http: &'a reqwest::Client,
    grants: &'a DaMcpGrantRepository,
    key: &'a GrantKey,
    server_id: &'a str,
    grant: &'a GrantRecord,
    /// The stored refresh-token ciphertext, re-sent unchanged when the
    /// authorization server does not rotate.
    refresh_enc: &'a str,
    identity: &'a SharedGrantClient<'a>,
    holder: &'a str,
}

async fn renew_under_lease(renewal: &Renewal<'_>) -> RenewalOutcome {
    let Renewal {
        http,
        grants,
        key,
        server_id,
        grant,
        refresh_enc,
        identity,
        holder,
    } = renewal;
    let Ok(refresh_token) = key.open(refresh_enc) else {
        // Web: an undecryptable refresh token means the KEK rotated since the row
        // was written. Reauthorization is the only recovery — no renewal can
        // read it.
        return RenewalOutcome::Failed(SharedGrantError::ReauthorizationRequired);
    };
    let Ok(token_endpoint) = reqwest::Url::parse(&grant.token_endpoint) else {
        return RenewalOutcome::Failed(SharedGrantError::Unavailable);
    };

    let scope = grant.scope.as_deref().or(identity.scope);
    let tokens = match exchange::refresh(
        http,
        &TokenRequest {
            token_endpoint: &token_endpoint,
            client: identity.client,
            resource: &grant.resource,
            scope,
        },
        &refresh_token,
    )
    .await
    {
        Ok(tokens) => tokens,
        // Web maps a definitive 4xx (or an explicit `invalid_grant`) to
        // reauthorization and lets 5xx / network stay transient.
        Err(ExchangeError::Refused) => {
            return RenewalOutcome::Failed(SharedGrantError::ReauthorizationRequired)
        }
        Err(ExchangeError::Invalid | ExchangeError::RequestFailed) => {
            return RenewalOutcome::Failed(SharedGrantError::Transient)
        }
    };

    let Ok(access_enc) = key.seal(&tokens.access_token) else {
        return RenewalOutcome::Failed(SharedGrantError::Unavailable);
    };
    // RFC 6749 §6: replace the refresh token only when the server rotated it.
    // Re-sending the existing ciphertext is what web does, and DA requires the
    // field to be present.
    let refresh_enc_next: String = match tokens.refresh_token.as_deref() {
        Some(rotated) => match key.seal(rotated) {
            Ok(sealed) => sealed,
            Err(_) => return RenewalOutcome::Failed(SharedGrantError::Unavailable),
        },
        None => (*refresh_enc).to_owned(),
    };

    let committed = grants
        .commit_refresh(
            &grant.id,
            &RefreshCommit {
                expected_version: grant.refresh_version,
                access_token_enc: &access_enc,
                refresh_token_enc: &refresh_enc_next,
                access_token_expires_at: &to_iso8601(tokens.expires_at),
                scope: tokens.scope.as_deref().or(scope),
                holder,
            },
        )
        .await;
    match committed {
        // The commit endpoint cleared the lease atomically, so the caller must not
        // release it again.
        Ok(outcome) if outcome.committed => RenewalOutcome::Renewed(tokens.access_token.clone()),
        Ok(_) => {
            // CAS lost while holding the lease. Disambiguate exactly as web
            // does, and by SERVER id: the shared row is addressed that way, and
            // a peer's commit is visible as an advanced version.
            match grants.load(server_id).await {
                Ok(reloaded) if reloaded.refresh_version == grant.refresh_version => {
                    RenewalOutcome::Failed(SharedGrantError::RenewalRefused)
                }
                Ok(_) => RenewalOutcome::PeerCommitted,
                Err(GrantRepositoryError::NotFound) => {
                    RenewalOutcome::Failed(SharedGrantError::ReauthorizationRequired)
                }
                Err(error) => RenewalOutcome::Failed(error.into()),
            }
        }
        Err(error) => RenewalOutcome::Failed(error.into()),
    }
}

/// The stored access token when it is still usable, mirroring web's fast path.
///
/// An undecryptable value is treated as absent rather than as an error: web falls
/// through to a refresh on a KEK rotation, and so does this.
fn usable_access_token(grant: &GrantRecord, key: &GrantKey) -> Option<Zeroizing<String>> {
    let access_enc = grant.access_token_enc.as_deref()?;
    let expires_at = grant.access_token_expires_at.as_deref()?;
    let expires_at = DateTime::parse_from_rfc3339(expires_at)
        .ok()?
        .with_timezone(&Utc);
    if expires_at.checked_sub_signed(chrono::Duration::from_std(REFRESH_LEEWAY).ok()?)?
        <= Utc::now()
    {
        return None;
    }
    key.open(access_enc)
        .ok()
        .filter(|token| !token.trim().is_empty())
}

/// Unix seconds → the ISO-8601 UTC string DA stores.
pub(crate) fn to_iso8601(epoch_seconds: i64) -> String {
    DateTime::from_timestamp(epoch_seconds, 0)
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_poll_delay_grows_and_stays_inside_webs_bounds() {
        for attempt in 0..6u32 {
            for _ in 0..32 {
                let delay = lease_poll_delay(attempt);
                assert!(
                    delay >= LEASE_POLL_BASE.mul_f64(0.5),
                    "attempt {attempt} waited {delay:?}"
                );
                assert!(
                    delay <= LEASE_POLL_MAX.mul_f64(1.5),
                    "attempt {attempt} waited {delay:?}"
                );
            }
        }
        // The whole retry budget stays bounded: three attempts of at most
        // 1.5 x 1500ms plus the requests themselves.
        let worst_case: Duration = (0..MAX_REFRESH_ATTEMPTS)
            .map(|_| LEASE_POLL_MAX.mul_f64(1.5))
            .sum();
        assert!(worst_case <= Duration::from_secs(7));
    }

    #[test]
    fn the_delay_is_jittered_so_peers_do_not_retry_in_lockstep() {
        let delays: std::collections::HashSet<u128> =
            (0..64).map(|_| lease_poll_delay(2).as_millis()).collect();
        assert!(
            delays.len() > 1,
            "an unjittered delay would make every waiter re-hit the row together"
        );
    }

    #[test]
    fn an_expiry_inside_the_leeway_is_not_used() {
        let key = GrantKey::decode(&base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0x33u8; 32],
        ))
        .expect("key");
        let sealed = key.seal("stored-access-token").expect("seals");

        let fresh = GrantRecord {
            id: "grant-1".to_owned(),
            issuer: "https://auth.example.test".to_owned(),
            token_endpoint: "https://auth.example.test/token".to_owned(),
            resource: "https://mcp.example.test/mcp".to_owned(),
            scope: None,
            access_token_enc: Some(sealed.clone()),
            refresh_token_enc: Some(sealed.clone()),
            access_token_expires_at: Some((Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
            refresh_version: 1,
        };
        assert_eq!(
            usable_access_token(&fresh, &key)
                .as_deref()
                .map(String::as_str),
            Some("stored-access-token")
        );

        let nearly_expired = GrantRecord {
            access_token_expires_at: Some(
                (Utc::now() + chrono::Duration::seconds(30)).to_rfc3339(),
            ),
            ..fresh.clone()
        };
        assert!(
            usable_access_token(&nearly_expired, &key).is_none(),
            "a token inside the leeway must be renewed, not sent"
        );

        let unparsable = GrantRecord {
            access_token_expires_at: Some("not a timestamp".to_owned()),
            ..fresh.clone()
        };
        assert!(usable_access_token(&unparsable, &key).is_none());

        // A KEK rotation makes the stored value unreadable; web falls through to
        // a refresh rather than failing, and so does this.
        let other_key = GrantKey::decode(&base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0x44u8; 32],
        ))
        .expect("key");
        assert!(usable_access_token(&fresh, &other_key).is_none());
    }

    #[test]
    fn an_expiry_is_written_in_the_shape_da_stores() {
        let rendered = to_iso8601(1_785_000_000);
        assert!(rendered.ends_with('Z'), "{rendered}");
        assert!(
            DateTime::parse_from_rfc3339(&rendered).is_ok(),
            "{rendered} must round-trip as RFC 3339"
        );
    }
}
