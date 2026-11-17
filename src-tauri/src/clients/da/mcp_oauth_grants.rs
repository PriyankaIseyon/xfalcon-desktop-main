//! DA adapter for `mcp_oauth_grants` — the shared, per-server OAuth grant.
//!
//! A port of xfalcon-web's `src/lib/chat-api/mcp-oauth-grants.ts`, endpoint for
//! endpoint, so the two clients operate on one grant row rather than two:
//!
//! | This module | DA endpoint |
//! |---|---|
//! | [`DaMcpGrantRepository::load`] | `GET    /api/v1/chat/mcp-oauth-grants/by-server/{server_id}` |
//! | [`DaMcpGrantRepository::upsert`] | `PUT    /api/v1/chat/mcp-oauth-grants/by-server/{server_id}` |
//! | [`DaMcpGrantRepository::evict`] | `DELETE /api/v1/chat/mcp-oauth-grants/by-server/{server_id}` |
//! | [`DaMcpGrantRepository::acquire_refresh_lease`] | `POST   /api/v1/chat/mcp-oauth-grants/{grant_id}:acquire-refresh-lease` |
//! | [`DaMcpGrantRepository::commit_refresh`] | `POST   /api/v1/chat/mcp-oauth-grants/{grant_id}:commit-refresh` |
//! | [`DaMcpGrantRepository::release_refresh_lease`] | `POST   /api/v1/chat/mcp-oauth-grants/{grant_id}:release-refresh-lease` |
//!
//! # What crosses this boundary, and what does not
//!
//! Only ciphertext. `access_token_enc` and `refresh_token_enc` are `enc:v1`
//! envelopes (`crate::mcp::oauth::envelope`) sealed *before* they reach this
//! module, exactly as web seals them before its own call — DA stores and returns
//! opaque strings and never holds a usable token. The refresh round-trip to the
//! authorization server likewise stays client-side (`crate::mcp::oauth::exchange`).
//!
//! # A grant is shared, which is why the lease exists
//!
//! `mcp_oauth_grants.server_id` is UNIQUE and the row is tenant-scoped, not
//! owner-scoped: one grant serves every client of that server in the tenant, web
//! and desktop alike. Two clients that refreshed it concurrently would both call
//! the authorization server, and one of the two resulting refresh tokens would be
//! invalidated — killing a grant that was working. The version-pinned lease plus
//! the version CAS on commit are what serialize that, and this module implements
//! the same protocol web does so the two interleave correctly.
//!
//! `granted_by_user_id` is audit-only and is not sent (DA leaves it NULL), and
//! `tenant_id` is derived from the caller's verified token rather than the body.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use super::{
    auth::{AuthenticatedDaClient, AuthenticatedPrincipal, BearerToken},
    mapping::{DaRoute, TenantBaseUrl},
};
use xfalcon_contracts::auth::IdentityProjection;

/// One grant row is a handful of short strings plus two token envelopes; this is
/// generous headroom and exists so a misbehaving response cannot be read
/// unboundedly.
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Page ceiling for [`DaMcpGrantRepository::signed_in_server_ids`].
const MAX_GRANT_PAGE: u32 = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum GrantRepositoryError {
    #[error("the signed-in session is not authorized for this tenant's grants")]
    Unauthorized,
    /// No grant exists for that server. A first-class outcome for [`load`], and
    /// distinguished here because it means "sign in", not "something broke".
    ///
    /// [`load`]: DaMcpGrantRepository::load
    #[error("no grant is stored for that server")]
    NotFound,
    #[error("DA answered outside its contract for this grant")]
    InvalidRow,
    #[error("the tenant data plane is unavailable")]
    Unavailable,
}

/// The subset of the row this client reads, as DA returns it (snake_case,
/// ISO-8601 timestamps, token columns opaque). Mirrors DA's `McpOauthGrant` and
/// web's `GrantWire`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct GrantRecord {
    pub(crate) id: String,
    pub(crate) issuer: String,
    pub(crate) token_endpoint: String,
    pub(crate) resource: String,
    #[serde(default)]
    pub(crate) scope: Option<String>,
    #[serde(default)]
    pub(crate) access_token_enc: Option<String>,
    #[serde(default)]
    pub(crate) refresh_token_enc: Option<String>,
    #[serde(default)]
    pub(crate) access_token_expires_at: Option<String>,
    #[serde(default)]
    pub(crate) refresh_version: i64,
}

/// DA's `McpOauthGrantUpsert`. Written after an authorization-code exchange.
///
/// `refresh_version` is owned and bumped server-side, and `granted_by_user_id`
/// stays NULL, so neither is sent — the same fields web omits.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct GrantUpsert {
    pub(crate) issuer: String,
    pub(crate) token_endpoint: String,
    pub(crate) resource: String,
    pub(crate) scope: Option<String>,
    pub(crate) access_token_enc: String,
    pub(crate) refresh_token_enc: Option<String>,
    /// ISO-8601, UTC.
    pub(crate) access_token_expires_at: String,
}

/// One page of `GET /api/v1/chat/mcp-oauth-grants`. Only the server id is read:
/// the question this answers is which rows have a grant, and the token columns
/// are neither needed nor wanted in a list.
#[derive(Clone, Debug, Deserialize)]
struct GrantPage {
    #[serde(default)]
    items: Vec<GrantPageItem>,
}

#[derive(Clone, Debug, Deserialize)]
struct GrantPageItem {
    #[serde(default)]
    server_id: Option<String>,
}

/// DA's `RefreshLeaseResult`.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct LeaseOutcome {
    pub(crate) acquired: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) refresh_version: Option<i64>,
    /// Whether another holder's lease is still live. Diagnostic only: the caller
    /// backs off on any unacquired lease.
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) lease_active: Option<bool>,
}

/// DA's `RefreshCommitResult`.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct CommitOutcome {
    pub(crate) committed: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) refresh_version: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
struct LeaseAcquireBody<'a> {
    expected_version: i64,
    lease_ms: u64,
    holder: &'a str,
}

/// What a commit carries. Grouped rather than passed as six positional
/// arguments so the version, the two ciphertexts, and the lease holder cannot be
/// transposed at a call site — every one of them is a `&str` or an integer.
pub(crate) struct RefreshCommit<'a> {
    /// The version read before the lease was claimed. DA rejects the commit if
    /// the row has moved since.
    pub(crate) expected_version: i64,
    pub(crate) access_token_enc: &'a str,
    /// The rotated ciphertext, or the existing one re-sent unchanged when the
    /// authorization server did not rotate (RFC 6749 §6).
    pub(crate) refresh_token_enc: &'a str,
    /// ISO-8601, UTC.
    pub(crate) access_token_expires_at: &'a str,
    pub(crate) scope: Option<&'a str>,
    /// The lease this caller holds. DA clears exactly that lease on success.
    pub(crate) holder: &'a str,
}

#[derive(Clone, Debug, Serialize)]
struct CommitBody<'a> {
    expected_version: i64,
    access_token_enc: &'a str,
    /// DA requires the field, so an unrotated refresh token is re-sent as the
    /// same ciphertext rather than omitted — which is also what web does.
    refresh_token_enc: &'a str,
    access_token_expires_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<&'a str>,
    holder: &'a str,
}

#[derive(Clone, Debug, Serialize)]
struct ReleaseBody<'a> {
    holder: &'a str,
}

pub(crate) struct DaMcpGrantRepository {
    client: AuthenticatedDaClient,
}

impl DaMcpGrantRepository {
    /// `client` is the owning service's shared client. Its connection pool is
    /// what makes repeated calls reuse one tenant connection.
    pub(crate) fn new(
        tenant_origin: &str,
        bearer: &str,
        identity: &IdentityProjection,
        client: reqwest::Client,
    ) -> Result<Self, GrantRepositoryError> {
        let base_url = TenantBaseUrl::parse(tenant_origin).map_err(|_| {
            // A misconfigured origin is not something a user can act on and is
            // not this tenant's data plane being down; both read as unavailable.
            GrantRepositoryError::Unavailable
        })?;
        Self::assemble(base_url, client, bearer, identity)
    }

    /// As [`Self::new`], against a loopback origin on a client the test owns.
    #[cfg(test)]
    pub(crate) fn for_test_with_client(
        tenant_origin: &str,
        bearer: &str,
        identity: &IdentityProjection,
        client: reqwest::Client,
    ) -> Result<Self, GrantRepositoryError> {
        let base_url = TenantBaseUrl::parse_loopback(tenant_origin)
            .map_err(|_| GrantRepositoryError::Unavailable)?;
        Self::assemble(base_url, client, bearer, identity)
    }

    /// As [`Self::new`], against a loopback origin and a short timeout.
    #[cfg(test)]
    pub(crate) fn for_test(
        tenant_origin: &str,
        bearer: &str,
        identity: &IdentityProjection,
        timeout: Duration,
    ) -> Result<Self, GrantRepositoryError> {
        let base_url = TenantBaseUrl::parse_loopback(tenant_origin)
            .map_err(|_| GrantRepositoryError::Unavailable)?;
        let http = super::mapping::build_test_client(timeout)
            .map_err(|_| GrantRepositoryError::Unavailable)?;
        Self::assemble(base_url, http, bearer, identity)
    }

    fn assemble(
        base_url: TenantBaseUrl,
        http: reqwest::Client,
        bearer: &str,
        identity: &IdentityProjection,
    ) -> Result<Self, GrantRepositoryError> {
        let bearer = BearerToken::new(bearer).map_err(|_| GrantRepositoryError::Unauthorized)?;
        let principal = AuthenticatedPrincipal::from_verified_identity(identity)
            .map_err(|_| GrantRepositoryError::Unauthorized)?;
        Ok(Self {
            client: AuthenticatedDaClient::new(base_url, http, bearer, principal),
        })
    }

    /// The shared grant for `server_id`, or [`GrantRepositoryError::NotFound`].
    ///
    /// Read through `by-server` rather than the owner-scoped list, for the reason
    /// web documents: the row's `granted_by_user_id` is typically NULL, so an
    /// owner-scoped read would hide a grant that exists.
    pub(crate) async fn load(&self, server_id: &str) -> Result<GrantRecord, GrantRepositoryError> {
        let request = self
            .client
            .request_nested(
                Method::GET,
                DaRoute::McpOauthGrants,
                &["by-server", server_id],
            )
            .map_err(|_| GrantRepositoryError::Unauthorized)?;
        self.send(request).await
    }

    /// Every server id this tenant holds a grant for, in one request.
    ///
    /// The registry surface asks "is this signed in?" of every row at once, and a
    /// per-row `by-server` read would be one request per server. Bounded to a
    /// single page: past that many OAuth servers the list is not the constraint,
    /// and a runaway cursor must not turn a list into an unbounded walk.
    pub(crate) async fn signed_in_server_ids(&self) -> Result<Vec<String>, GrantRepositoryError> {
        let request = self
            .client
            .get(DaRoute::McpOauthGrants)
            .map_err(|_| GrantRepositoryError::Unauthorized)?
            .query(&[("limit", MAX_GRANT_PAGE.to_string())]);
        let page: GrantPage = self.send(request).await?;
        Ok(page
            .items
            .into_iter()
            .filter_map(|item| item.server_id)
            .collect())
    }

    /// Writes the grant an authorization-code exchange produced.
    ///
    /// A 404 is *not* swallowed: it means the server is not in this tenant, and
    /// reporting success there would leave the user looking at a connected server
    /// with no grant behind it.
    pub(crate) async fn upsert(
        &self,
        server_id: &str,
        grant: &GrantUpsert,
    ) -> Result<GrantRecord, GrantRepositoryError> {
        let request = self
            .client
            .request_nested(
                Method::PUT,
                DaRoute::McpOauthGrants,
                &["by-server", server_id],
            )
            .map_err(|_| GrantRepositoryError::Unauthorized)?
            .json(grant);
        self.send(request).await
    }

    /// Deletes the shared grant. Idempotent: DA answers `deleted=false` when
    /// there was none, which is success.
    pub(crate) async fn evict(&self, server_id: &str) -> Result<(), GrantRepositoryError> {
        let request = self
            .client
            .request_nested(
                Method::DELETE,
                DaRoute::McpOauthGrants,
                &["by-server", server_id],
            )
            .map_err(|_| GrantRepositoryError::Unauthorized)?;
        let response = request
            .send()
            .await
            .map_err(|_| GrantRepositoryError::Unavailable)?;
        match response.status() {
            status if status.is_success() => Ok(()),
            StatusCode::NOT_FOUND => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(GrantRepositoryError::Unauthorized)
            }
            _ => Err(GrantRepositoryError::Unavailable),
        }
    }

    /// Claims the refresh lease, pinned to the version this caller read.
    ///
    /// `acquired == false` means a peer holds a live lease or this caller's
    /// snapshot is stale. Both mean the same thing to the caller: back off and
    /// re-read, because the peer is about to persist a token this caller can use.
    pub(crate) async fn acquire_refresh_lease(
        &self,
        grant_id: &str,
        expected_version: i64,
        lease: Duration,
        holder: &str,
    ) -> Result<LeaseOutcome, GrantRepositoryError> {
        let body = LeaseAcquireBody {
            expected_version,
            lease_ms: lease.as_millis().try_into().unwrap_or(u64::MAX),
            holder,
        };
        let request = self.action(grant_id, "acquire-refresh-lease")?.json(&body);
        self.send(request).await
    }

    /// Commits re-encrypted tokens under the same version CAS, clearing the lease
    /// atomically on success.
    pub(crate) async fn commit_refresh(
        &self,
        grant_id: &str,
        commit: &RefreshCommit<'_>,
    ) -> Result<CommitOutcome, GrantRepositoryError> {
        let body = CommitBody {
            expected_version: commit.expected_version,
            access_token_enc: commit.access_token_enc,
            refresh_token_enc: commit.refresh_token_enc,
            access_token_expires_at: commit.access_token_expires_at,
            scope: commit.scope,
            holder: commit.holder,
        };
        let request = self.action(grant_id, "commit-refresh")?.json(&body);
        self.send(request).await
    }

    /// Releases a lease this caller holds. Best effort by contract: the lease
    /// expires on its own, so the caller treats a failure as non-fatal.
    pub(crate) async fn release_refresh_lease(
        &self,
        grant_id: &str,
        holder: &str,
    ) -> Result<(), GrantRepositoryError> {
        let request = self
            .action(grant_id, "release-refresh-lease")?
            .json(&ReleaseBody { holder });
        let response = request
            .send()
            .await
            .map_err(|_| GrantRepositoryError::Unavailable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(GrantRepositoryError::Unavailable)
        }
    }

    /// `POST …/{grant_id}:<action>`.
    ///
    /// The colon belongs to the id's own path segment, so the whole thing is
    /// pushed as one segment — which also means a `/` inside `grant_id` is
    /// escaped rather than being able to redirect the call to another route.
    fn action(
        &self,
        grant_id: &str,
        action: &str,
    ) -> Result<reqwest::RequestBuilder, GrantRepositoryError> {
        self.client
            .request_nested(
                Method::POST,
                DaRoute::McpOauthGrants,
                &[&format!("{grant_id}:{action}")],
            )
            .map_err(|_| GrantRepositoryError::Unauthorized)
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, GrantRepositoryError> {
        let response = request
            .send()
            .await
            .map_err(|_| GrantRepositoryError::Unavailable)?;
        match response.status() {
            status if status.is_success() => {}
            StatusCode::NOT_FOUND => return Err(GrantRepositoryError::NotFound),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(GrantRepositoryError::Unauthorized)
            }
            StatusCode::UNPROCESSABLE_ENTITY | StatusCode::BAD_REQUEST => {
                return Err(GrantRepositoryError::InvalidRow)
            }
            _ => return Err(GrantRepositoryError::Unavailable),
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| GrantRepositoryError::Unavailable)?;
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(GrantRepositoryError::InvalidRow);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| GrantRepositoryError::InvalidRow)
    }
}

impl std::fmt::Debug for DaMcpGrantRepository {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaMcpGrantRepository")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_TIMEOUT: Duration = Duration::from_millis(750);

    fn identity() -> IdentityProjection {
        IdentityProjection::new("user-1", "tenant-1", "Ada Lovelace", None)
    }

    fn repository(server: &MockServer) -> DaMcpGrantRepository {
        DaMcpGrantRepository::for_test(&server.uri(), "session-bearer", &identity(), TEST_TIMEOUT)
            .expect("a repository")
    }

    fn grant_body() -> serde_json::Value {
        json!({
            "id": "grant-1",
            "tenant_id": "tenant-1",
            "server_id": "srv-1",
            "issuer": "https://auth.example.test",
            "token_endpoint": "https://auth.example.test/token",
            "resource": "https://mcp.example.test/mcp",
            "scope": "openid offline_access",
            "access_token_enc": "enc:v1:aa:bb:cc",
            "refresh_token_enc": "enc:v1:dd:ee:ff",
            "access_token_expires_at": "2026-08-04T09:00:00Z",
            "refresh_version": 7
        })
    }

    #[tokio::test]
    async fn a_grant_is_read_from_the_shared_by_server_route() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/mcp-oauth-grants/by-server/srv-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(grant_body()))
            .mount(&server)
            .await;

        let loaded = repository(&server)
            .load("srv-1")
            .await
            .expect("the grant loads");

        assert_eq!(loaded.id, "grant-1");
        assert_eq!(loaded.refresh_version, 7);
        assert_eq!(loaded.access_token_enc.as_deref(), Some("enc:v1:aa:bb:cc"));
        assert_eq!(loaded.resource, "https://mcp.example.test/mcp");

        let request = &server.received_requests().await.expect("requests")[0];
        assert_eq!(
            request
                .headers
                .get("authorization")
                .expect("a bearer")
                .to_str()
                .expect("ascii"),
            "Bearer session-bearer"
        );
    }

    #[tokio::test]
    async fn a_missing_grant_is_reported_as_such_rather_than_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/mcp-oauth-grants/by-server/srv-1"))
            .respond_with(
                ResponseTemplate::new(404).set_body_json(json!({ "detail": "not found" })),
            )
            .mount(&server)
            .await;

        assert_eq!(
            repository(&server).load("srv-1").await.err(),
            Some(GrantRepositoryError::NotFound)
        );
    }

    /// `refresh_version` and `granted_by_user_id` are owned by DA, so neither may
    /// appear in the body — sending a version would let a client roll the CAS
    /// counter backwards.
    #[tokio::test]
    async fn an_upsert_sends_only_the_fields_da_owns_from_the_client() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/chat/mcp-oauth-grants/by-server/srv-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(grant_body()))
            .mount(&server)
            .await;

        repository(&server)
            .upsert(
                "srv-1",
                &GrantUpsert {
                    issuer: "https://auth.example.test".to_owned(),
                    token_endpoint: "https://auth.example.test/token".to_owned(),
                    resource: "https://mcp.example.test/mcp".to_owned(),
                    scope: Some("openid".to_owned()),
                    access_token_enc: "enc:v1:aa:bb:cc".to_owned(),
                    refresh_token_enc: Some("enc:v1:dd:ee:ff".to_owned()),
                    access_token_expires_at: "2026-08-04T09:00:00Z".to_owned(),
                },
            )
            .await
            .expect("the upsert succeeds");

        let sent: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.expect("requests")[0].body)
                .expect("a json body");
        assert_eq!(sent["access_token_enc"], json!("enc:v1:aa:bb:cc"));
        assert!(sent.get("refresh_version").is_none());
        assert!(sent.get("granted_by_user_id").is_none());
        assert!(sent.get("tenant_id").is_none());
    }

    #[tokio::test]
    async fn the_lease_is_claimed_at_the_colon_suffixed_action_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/chat/mcp-oauth-grants/grant-1:acquire-refresh-lease",
            ))
            .and(body_json(json!({
                "expected_version": 7,
                "lease_ms": 10_000,
                "holder": "holder-1"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({ "acquired": true, "refresh_version": 7, "lease_active": false }),
            ))
            .mount(&server)
            .await;

        let outcome = repository(&server)
            .acquire_refresh_lease("grant-1", 7, Duration::from_secs(10), "holder-1")
            .await
            .expect("the lease request succeeds");
        assert!(outcome.acquired);
    }

    #[tokio::test]
    async fn a_commit_carries_the_expected_version_and_the_lease_holder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/chat/mcp-oauth-grants/grant-1:commit-refresh"))
            .and(body_json(json!({
                "expected_version": 7,
                "access_token_enc": "enc:v1:new:access:token",
                "refresh_token_enc": "enc:v1:new:refresh:token",
                "access_token_expires_at": "2026-08-04T10:00:00Z",
                "scope": "openid",
                "holder": "holder-1"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "committed": true, "refresh_version": 8 })),
            )
            .mount(&server)
            .await;

        let outcome = repository(&server)
            .commit_refresh(
                "grant-1",
                &RefreshCommit {
                    expected_version: 7,
                    access_token_enc: "enc:v1:new:access:token",
                    refresh_token_enc: "enc:v1:new:refresh:token",
                    access_token_expires_at: "2026-08-04T10:00:00Z",
                    scope: Some("openid"),
                    holder: "holder-1",
                },
            )
            .await
            .expect("the commit request succeeds");
        assert!(outcome.committed);
    }

    #[tokio::test]
    async fn eviction_is_idempotent_across_both_answers_da_gives() {
        for (status, body) in [
            (200, json!({ "deleted": true, "id": "grant-1" })),
            (200, json!({ "deleted": false, "id": null })),
            (404, json!({ "detail": "not found" })),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("DELETE"))
                .and(path("/api/v1/chat/mcp-oauth-grants/by-server/srv-1"))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&server)
                .await;

            assert!(
                repository(&server).evict("srv-1").await.is_ok(),
                "HTTP {status} must be a successful eviction"
            );
        }
    }

    /// An id carrying path syntax must not be able to address another route.
    #[tokio::test]
    async fn a_server_id_containing_path_syntax_cannot_escape_the_route() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/api/v1/chat/mcp-oauth-grants/by-server/..%2F..%2Fprovider-keys",
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        assert_eq!(
            repository(&server).load("../../provider-keys").await.err(),
            Some(GrantRepositoryError::NotFound),
            "the id must have been encoded into one segment"
        );
    }

    #[tokio::test]
    async fn a_refused_session_is_distinguished_from_an_unavailable_data_plane() {
        for (status, expected) in [
            (401, GrantRepositoryError::Unauthorized),
            (403, GrantRepositoryError::Unauthorized),
            (422, GrantRepositoryError::InvalidRow),
            (500, GrantRepositoryError::Unavailable),
            (503, GrantRepositoryError::Unavailable),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/chat/mcp-oauth-grants/by-server/srv-1"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            assert_eq!(
                repository(&server).load("srv-1").await.err(),
                Some(expected),
                "HTTP {status}"
            );
        }
    }

    #[tokio::test]
    async fn a_row_outside_das_contract_is_refused_rather_than_half_read() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/mcp-oauth-grants/by-server/srv-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "grant-1" })))
            .mount(&server)
            .await;

        assert_eq!(
            repository(&server).load("srv-1").await.err(),
            Some(GrantRepositoryError::InvalidRow)
        );
    }
}
