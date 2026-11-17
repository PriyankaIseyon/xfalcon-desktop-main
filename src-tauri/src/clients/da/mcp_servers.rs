//! DA adapter for the shared `mcp_servers` registry.
//!
//! Desktop and xFalcon Web both register MCP servers into DA's
//! `/api/v1/chat/mcp-servers`, so a server added in one is visible in the
//! other. This module is the narrow, bounded read/write path onto those rows;
//! it makes no policy decisions. Deciding whether a row is *dialable* — https,
//! transport, auth method, whether this device holds the credential — is
//! `crate::mcp::registry`'s job, so that policy lives in exactly one place.
//!
//! # What deliberately does not cross this boundary
//!
//! DA's `auth_config` column holds credential material encrypted under a key
//! that never leaves xfalcon-web (see the generated client's own doc comment on
//! `GetMcpServerApiV1ChatMcpServersServerIdGetRequest`). Desktop cannot decrypt
//! it and must not hold that key, so [`McpServerRecord`] has no field for it
//! and [`NewMcpServer`] never writes one. A desktop-registered server's
//! credential is sealed by `crate::credentials::CredentialVault` into the OS
//! keychain instead and never reaches DA at all.
//!
//! `status` / `status_message` / `last_health_check` are likewise read-only
//! here and never written: they are one prober's view, and a desktop on a
//! captive portal must not mark a healthy server broken for the whole tenant.

use std::fmt;

use futures_util::StreamExt;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::mcp::{McpAuthType, McpPermissionMode, McpTransport};
use zeroize::Zeroizing;

use super::{
    auth::{AuthenticatedDaClient, AuthenticatedPrincipal, BearerToken},
    generated,
    mapping::{DaRoute, TenantBaseUrl},
};

/// Page ceiling for a full list walk. 200 rows per page against DA's
/// `MAX_SOURCES_PER_TENANT`-scale registry means this is a runaway-cursor
/// backstop, not a product limit.
const MAX_SERVER_PAGES: usize = 20;
const PAGE_LIMIT: i64 = 200;
const MAX_LIST_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROW_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 100;
const MAX_DISPLAY_NAME_BYTES: usize = 200;
const MAX_DESCRIPTION_BYTES: usize = 2_048;
const MAX_URL_BYTES: usize = 2_048;
const MAX_HEADER_NAME_BYTES: usize = 64;

/// `metadata.origin` stamped on rows this application creates, so an operator
/// looking at the shared registry can tell where a row came from.
const ORIGIN_VALUE: &str = "xfalcon-desktop";
const ORIGIN_KEY: &str = "origin";
/// `metadata.header_name` — the custom header a [`McpAuthType::Header`] server
/// expects. DA has no column for it, and it is not confidential (the value it
/// carries is, and that stays on the device).
const HEADER_NAME_KEY: &str = "header_name";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum McpServerRepositoryError {
    #[error("the signed-in session is not authorized for this registry")]
    Unauthorized,
    #[error("no such registered server")]
    NotFound,
    #[error("a server with that name already exists for this account")]
    Conflict,
    #[error("DA answered outside its contract for this row")]
    InvalidRow,
    #[error("the tenant registry is unavailable")]
    Unavailable,
    /// DA committed a create/update but the response could not be read as the
    /// row it produced. The caller must re-list rather than retry a
    /// non-idempotent mutation.
    #[error("the registry mutation needs reconciliation before it is retried")]
    ReconciliationRequired,
}

/// One DA `mcp_servers` row, narrowed to the fields desktop uses and bounded
/// on every string. Transport and auth method are parsed into the contract
/// vocabulary here, with anything DA adds later landing on
/// [`McpTransport::Other`] / [`McpAuthType::Other`] so an unknown value makes
/// one row unusable rather than making the whole list unreadable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpServerRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) display_name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) transport: McpTransport,
    pub(crate) url: Option<String>,
    pub(crate) auth_type: McpAuthType,
    pub(crate) permission_mode: McpPermissionMode,
    pub(crate) enabled: bool,
    /// From `metadata.header_name`. Only meaningful for
    /// [`McpAuthType::Header`].
    pub(crate) header_name: Option<String>,
    /// The row's `auth_config`, carried verbatim and still encrypted.
    ///
    /// See [`StoredAuthConfig`] for why this crosses the boundary at all and
    /// what can be done with it.
    pub(crate) auth_config: Option<StoredAuthConfig>,
}

/// One row's `auth_config` column, exactly as DA returned it.
///
/// # Why this is carried, given the module comment above
///
/// The secret-bearing fields are ciphertext under the key-encryption key
/// xfalcon-web reads from `PROVIDER_KEYS_ENCRYPTION_KEY`, so this value is
/// *useless* to a build that does not hold that key — which is the default. It is
/// carried because a deployment that does configure the same key
/// (`crate::mcp::oauth::envelope`) can then reuse the OAuth client an operator
/// already registered on the web app, instead of asking every desktop user to
/// paste a client id that already exists.
///
/// Held as the serialized JSON rather than a map so the record stays `Eq` (the
/// runtime compares classified rows to decide whether to rebuild sessions), and
/// rendered as a placeholder by [`std::fmt::Debug`] so it cannot reach a log even
/// as ciphertext.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct StoredAuthConfig(String);

impl fmt::Debug for StoredAuthConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredAuthConfig([REDACTED])")
    }
}

/// The `oauth` arm of `auth_config`, decrypted. A port of the same arm of web's
/// `decryptAuthConfigForUse` (`src/lib/server/mcp-auth-config.ts`).
///
/// Every field except the two credentials is optional, and each one means the
/// same thing it does in web: an explicit issuer or resource skips discovery, a
/// scope overrides what discovery would negotiate, and a metadata URL points
/// discovery at a document other than the well-known one.
pub(crate) struct DecryptedOauthConfig {
    pub(crate) client_id: String,
    pub(crate) client_secret: Zeroizing<String>,
    pub(crate) issuer_url: Option<String>,
    pub(crate) resource: Option<String>,
    pub(crate) scope: Option<String>,
    pub(crate) resource_metadata_url: Option<String>,
}

impl fmt::Debug for DecryptedOauthConfig {
    /// The client id is not confidential and is what an operator needs to
    /// recognise; the secret never appears.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecryptedOauthConfig")
            .field("client_id", &self.client_id)
            .field("issuer_url", &self.issuer_url)
            .field("resource", &self.resource)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl StoredAuthConfig {
    /// A stable digest of the stored ciphertext.
    ///
    /// The runtime caches one session per server and rebuilds when the classified
    /// row changes; without this, an operator rotating a server's OAuth client on
    /// the web app would leave every desktop holding a session built from the
    /// previous secret until the app restarted. A digest rather than the value so
    /// nothing comparable to a credential is held in the cache key.
    #[must_use]
    pub(crate) fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};

        let digest = Sha256::digest(self.0.as_bytes());
        // Half the digest is far beyond what distinguishing two ciphertexts needs.
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn from_wire(raw: &IndexMap<String, Value>) -> Option<Self> {
        if raw.is_empty() {
            return None;
        }
        serde_json::to_string(raw).ok().map(Self)
    }

    /// Decrypts the `oauth` arm with `open`, or `None` when this column holds no
    /// usable OAuth client.
    ///
    /// `open` is a closure rather than a key type so this module keeps no
    /// dependency on the OAuth layer above it; `crate::mcp::oauth::envelope`
    /// supplies it. Only `client_secret` is a secret field, matching web's
    /// `SECRET_FIELDS_BY_TYPE`; `client_id` and the four optional hints are stored
    /// in the clear by web too, so the dialog can edit them without re-entering
    /// the secret.
    pub(crate) fn decrypt_oauth(
        &self,
        open: &dyn Fn(&str) -> Option<Zeroizing<String>>,
    ) -> Option<DecryptedOauthConfig> {
        let parsed: IndexMap<String, Value> = serde_json::from_str(&self.0).ok()?;
        let text = |field: &str| -> Option<String> {
            parsed
                .get(field)?
                .as_str()
                .map(str::to_owned)
                .filter(|value| !value.trim().is_empty())
        };

        let client_id = text("client_id")?;
        let client_secret = open(&text("client_secret")?)?;
        if client_secret.trim().is_empty() {
            return None;
        }
        Some(DecryptedOauthConfig {
            client_id,
            client_secret,
            issuer_url: text("issuer_url"),
            resource: text("resource"),
            scope: text("scope"),
            resource_metadata_url: text("resource_metadata_url"),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NewMcpServer {
    pub(crate) name: String,
    pub(crate) display_name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) url: String,
    pub(crate) auth_type: McpAuthType,
    pub(crate) header_name: Option<String>,
    pub(crate) permission_mode: McpPermissionMode,
    pub(crate) enabled: bool,
}

/// A partial update. `None` leaves DA's value alone; the absence of any
/// `status`/`auth_config` field is the point — this type cannot express those
/// writes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct McpServerPatch {
    pub(crate) display_name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) auth_type: Option<McpAuthType>,
    pub(crate) header_name: Option<String>,
    pub(crate) permission_mode: Option<McpPermissionMode>,
    pub(crate) enabled: Option<bool>,
}

pub(crate) struct DaMcpServerRepository {
    client: AuthenticatedDaClient,
}

impl DaMcpServerRepository {
    /// `client` is the owning service's shared client. Its connection pool is
    /// what makes repeated calls reuse one tenant connection.
    pub(crate) fn new(
        da_origin: &str,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
        client: reqwest::Client,
    ) -> Result<Self, McpServerRepositoryError> {
        let base =
            TenantBaseUrl::parse(da_origin).map_err(|_| McpServerRepositoryError::Unavailable)?;
        Self::from_parts(base, client, bearer, identity)
    }

    /// As [`Self::new`], against a loopback origin on a client the test owns.
    #[cfg(test)]
    pub(crate) fn for_test_with_client(
        da_origin: &str,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
        client: reqwest::Client,
    ) -> Result<Self, McpServerRepositoryError> {
        let base = TenantBaseUrl::parse_loopback(da_origin)
            .map_err(|_| McpServerRepositoryError::Unavailable)?;
        Self::from_parts(base, client, bearer, identity)
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        da_origin: &str,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
        timeout: std::time::Duration,
    ) -> Result<Self, McpServerRepositoryError> {
        let base = TenantBaseUrl::parse_loopback(da_origin)
            .map_err(|_| McpServerRepositoryError::Unavailable)?;
        let client = super::mapping::build_test_client(timeout)
            .map_err(|_| McpServerRepositoryError::Unavailable)?;
        Self::from_parts(base, client, bearer, identity)
    }

    fn from_parts(
        base: TenantBaseUrl,
        client: reqwest::Client,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
    ) -> Result<Self, McpServerRepositoryError> {
        let bearer =
            BearerToken::new(bearer).map_err(|_| McpServerRepositoryError::Unauthorized)?;
        let principal = AuthenticatedPrincipal::from_verified_identity(identity)
            .map_err(|_| McpServerRepositoryError::Unauthorized)?;
        Ok(Self {
            client: AuthenticatedDaClient::new(base, client, bearer, principal),
        })
    }

    pub(crate) async fn list(&self) -> Result<Vec<McpServerRecord>, McpServerRepositoryError> {
        let mut records = Vec::new();
        let mut cursor = None;
        for _ in 0..MAX_SERVER_PAGES {
            let (mut page, next_cursor) = self.list_page(cursor).await?;
            records.append(&mut page);
            match next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(records),
            }
        }
        Err(McpServerRepositoryError::Unavailable)
    }

    async fn list_page(
        &self,
        cursor: Option<String>,
    ) -> Result<(Vec<McpServerRecord>, Option<String>), McpServerRepositoryError> {
        let query = generated::ListMcpServersApiV1ChatMcpServersGetRequestQuery {
            sort: Some("name".to_owned()),
            is_enabled: None,
            transport: None,
            since: None,
            until: None,
            limit: Some(PAGE_LIMIT),
            cursor,
        };
        let response = self
            .client
            .get(DaRoute::McpServers)
            .map_err(|_| McpServerRepositoryError::Unavailable)?
            .query(&query)
            .send()
            .await
            .map_err(|_| McpServerRepositoryError::Unavailable)?;
        let response = bounded_response(response, MAX_LIST_RESPONSE_BYTES).await?;
        let response =
            generated::ListMcpServersApiV1ChatMcpServersGetRequest::parse_response(response)
                .await
                .map_err(|_| McpServerRepositoryError::Unavailable)?;
        let page = match response {
            generated::ListMcpServersApiV1ChatMcpServersGetResponse::Ok(page) => page,
            generated::ListMcpServersApiV1ChatMcpServersGetResponse::Unauthorized(_)
            | generated::ListMcpServersApiV1ChatMcpServersGetResponse::Forbidden(_) => {
                return Err(McpServerRepositoryError::Unauthorized);
            }
            _ => return Err(McpServerRepositoryError::Unavailable),
        };

        let mut records = Vec::new();
        for row in page.items.unwrap_or_default() {
            // One malformed row fails the listing rather than being dropped:
            // silently omitting a server the account registered would leave a
            // user staring at a list that disagrees with the web app.
            records.push(self.record(row)?);
        }
        let next_cursor = if page.has_more == Some(true) {
            Some(
                page.next_cursor
                    .filter(|value| !value.is_empty())
                    .ok_or(McpServerRepositoryError::Unavailable)?,
            )
        } else {
            None
        };
        Ok((records, next_cursor))
    }

    pub(crate) async fn create(
        &self,
        server: NewMcpServer,
    ) -> Result<McpServerRecord, McpServerRepositoryError> {
        let body = McpServerCreateBody {
            name: &server.name,
            transport: TRANSPORT_HTTP,
            display_name: server.display_name.as_deref(),
            description: server.description.as_deref(),
            url: &server.url,
            auth_type: auth_type_wire(server.auth_type)
                .ok_or(McpServerRepositoryError::InvalidRow)?,
            is_enabled: server.enabled,
            permission_mode: permission_mode_wire(server.permission_mode),
            metadata: metadata_for(server.header_name.as_deref()),
            visibility: VISIBILITY_PRIVATE,
        };
        let response = self
            .client
            .post(DaRoute::McpServers)
            .map_err(|_| McpServerRepositoryError::Unavailable)?
            .json(&body)
            .send()
            .await
            .map_err(|_| McpServerRepositoryError::ReconciliationRequired)?;
        let response = bounded_response(response, MAX_ROW_RESPONSE_BYTES)
            .await
            .map_err(|_| McpServerRepositoryError::ReconciliationRequired)?;
        let response =
            generated::CreateMcpServerApiV1ChatMcpServersPostRequest::parse_response(response)
                .await
                .map_err(|_| McpServerRepositoryError::ReconciliationRequired)?;
        match response {
            generated::CreateMcpServerApiV1ChatMcpServersPostResponse::Created(row) => {
                if row.name != server.name {
                    // DA committed a row we cannot identify as ours. Re-list
                    // rather than retry a non-idempotent create.
                    return Err(McpServerRepositoryError::ReconciliationRequired);
                }
                self.record(row)
                    .map_err(|_| McpServerRepositoryError::ReconciliationRequired)
            }
            generated::CreateMcpServerApiV1ChatMcpServersPostResponse::Unauthorized(_)
            | generated::CreateMcpServerApiV1ChatMcpServersPostResponse::Forbidden(_) => {
                Err(McpServerRepositoryError::Unauthorized)
            }
            generated::CreateMcpServerApiV1ChatMcpServersPostResponse::Conflict(_) => {
                Err(McpServerRepositoryError::Conflict)
            }
            generated::CreateMcpServerApiV1ChatMcpServersPostResponse::ServiceUnavailable(_)
            | generated::CreateMcpServerApiV1ChatMcpServersPostResponse::Unknown => {
                Err(McpServerRepositoryError::ReconciliationRequired)
            }
            _ => Err(McpServerRepositoryError::Unavailable),
        }
    }

    pub(crate) async fn update(
        &self,
        id: &str,
        patch: McpServerPatch,
    ) -> Result<McpServerRecord, McpServerRepositoryError> {
        let body = McpServerUpdateBody {
            display_name: patch.display_name.as_deref(),
            description: patch.description.as_deref(),
            url: patch.url.as_deref(),
            auth_type: patch
                .auth_type
                .map(|auth| auth_type_wire(auth).ok_or(McpServerRepositoryError::InvalidRow))
                .transpose()?,
            is_enabled: patch.enabled,
            permission_mode: patch.permission_mode.map(permission_mode_wire),
            metadata: patch
                .header_name
                .as_deref()
                .map(|header| metadata_for(Some(header))),
        };
        let response = self
            .client
            .patch_scoped(DaRoute::McpServers, id)
            .map_err(|_| McpServerRepositoryError::Unavailable)?
            .json(&body)
            .send()
            .await
            .map_err(|_| McpServerRepositoryError::ReconciliationRequired)?;
        let response = bounded_response(response, MAX_ROW_RESPONSE_BYTES)
            .await
            .map_err(|_| McpServerRepositoryError::ReconciliationRequired)?;
        let response =
            generated::UpdateMcpServerApiV1ChatMcpServersServerIdPatchRequest::parse_response(
                response,
            )
            .await
            .map_err(|_| McpServerRepositoryError::ReconciliationRequired)?;
        match response {
            generated::UpdateMcpServerApiV1ChatMcpServersServerIdPatchResponse::Ok(row) => {
                self.record(row)
            }
            generated::UpdateMcpServerApiV1ChatMcpServersServerIdPatchResponse::Unauthorized(_)
            | generated::UpdateMcpServerApiV1ChatMcpServersServerIdPatchResponse::Forbidden(_) => {
                Err(McpServerRepositoryError::Unauthorized)
            }
            generated::UpdateMcpServerApiV1ChatMcpServersServerIdPatchResponse::NotFound(_) => {
                Err(McpServerRepositoryError::NotFound)
            }
            generated::UpdateMcpServerApiV1ChatMcpServersServerIdPatchResponse::Conflict(_) => {
                Err(McpServerRepositoryError::Conflict)
            }
            _ => Err(McpServerRepositoryError::Unavailable),
        }
    }

    pub(crate) async fn delete(&self, id: &str) -> Result<(), McpServerRepositoryError> {
        let response = self
            .client
            .delete_scoped(DaRoute::McpServers, id)
            .map_err(|_| McpServerRepositoryError::Unavailable)?
            .send()
            .await
            .map_err(|_| McpServerRepositoryError::Unavailable)?;
        let response = bounded_response(response, MAX_ROW_RESPONSE_BYTES).await?;
        let response =
            generated::DeleteMcpServerApiV1ChatMcpServersServerIdDeleteRequest::parse_response(
                response,
            )
            .await
            .map_err(|_| McpServerRepositoryError::Unavailable)?;
        match response {
            generated::DeleteMcpServerApiV1ChatMcpServersServerIdDeleteResponse::Ok(_) => Ok(()),
            generated::DeleteMcpServerApiV1ChatMcpServersServerIdDeleteResponse::Unauthorized(
                _,
            )
            | generated::DeleteMcpServerApiV1ChatMcpServersServerIdDeleteResponse::Forbidden(_) => {
                Err(McpServerRepositoryError::Unauthorized)
            }
            generated::DeleteMcpServerApiV1ChatMcpServersServerIdDeleteResponse::NotFound(_) => {
                Err(McpServerRepositoryError::NotFound)
            }
            _ => Err(McpServerRepositoryError::Unavailable),
        }
    }

    /// Narrows one DA row, rejecting anything that does not belong to the
    /// signed-in principal or exceeds a field bound.
    fn record(
        &self,
        raw: generated::McpServer,
    ) -> Result<McpServerRecord, McpServerRepositoryError> {
        if !self.client.matches_principal(&raw.user_id, &raw.tenant_id) {
            return Err(McpServerRepositoryError::Unauthorized);
        }
        let name = bounded(raw.name, MAX_NAME_BYTES)?;
        if name.is_empty() {
            return Err(McpServerRepositoryError::InvalidRow);
        }
        Ok(McpServerRecord {
            id: raw.id.to_string(),
            name,
            display_name: bounded_option(raw.display_name, MAX_DISPLAY_NAME_BYTES)?,
            description: bounded_option(raw.description, MAX_DESCRIPTION_BYTES)?,
            transport: parse_transport(&raw.transport),
            url: bounded_option(raw.url, MAX_URL_BYTES)?,
            auth_type: parse_auth_type(&raw.auth_type),
            permission_mode: parse_permission_mode(raw.permission_mode.as_deref()),
            enabled: raw.is_enabled.unwrap_or(false),
            header_name: header_name_from_metadata(raw.metadata.as_ref())?,
            auth_config: raw
                .auth_config
                .as_ref()
                .and_then(StoredAuthConfig::from_wire),
        })
    }
}

impl fmt::Debug for DaMcpServerRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaMcpServerRepository")
            .field("client", &self.client)
            .finish()
    }
}

const TRANSPORT_HTTP: &str = "http";
const VISIBILITY_PRIVATE: &str = "private";

/// DA's create body, restricted to the columns desktop is allowed to write.
/// `auth_config`, `status`, `status_message`, `command`, and `args` are absent
/// by construction — see the module comment.
#[derive(Serialize)]
struct McpServerCreateBody<'a> {
    name: &'a str,
    transport: &'static str,
    display_name: Option<&'a str>,
    description: Option<&'a str>,
    url: &'a str,
    auth_type: &'static str,
    is_enabled: bool,
    permission_mode: &'static str,
    metadata: IndexMap<String, Value>,
    visibility: &'static str,
}

#[derive(Serialize)]
struct McpServerUpdateBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<IndexMap<String, Value>>,
}

fn metadata_for(header_name: Option<&str>) -> IndexMap<String, Value> {
    let mut metadata = IndexMap::new();
    metadata.insert(
        ORIGIN_KEY.to_owned(),
        Value::String(ORIGIN_VALUE.to_owned()),
    );
    if let Some(header) = header_name {
        metadata.insert(HEADER_NAME_KEY.to_owned(), Value::String(header.to_owned()));
    }
    metadata
}

fn header_name_from_metadata(
    metadata: Option<&IndexMap<String, Value>>,
) -> Result<Option<String>, McpServerRepositoryError> {
    let Some(raw) = metadata.and_then(|map| map.get(HEADER_NAME_KEY)) else {
        return Ok(None);
    };
    let Some(header) = raw.as_str() else {
        return Err(McpServerRepositoryError::InvalidRow);
    };
    // A header name reaches `reqwest::header::HeaderName` later; reject
    // anything outside the RFC 7230 token set here so a hostile row cannot
    // attempt header injection further in.
    if header.is_empty()
        || header.len() > MAX_HEADER_NAME_BYTES
        || !header
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        return Err(McpServerRepositoryError::InvalidRow);
    }
    Ok(Some(header.to_owned()))
}

fn parse_transport(raw: &str) -> McpTransport {
    match raw {
        "http" => McpTransport::Http,
        "sse" => McpTransport::Sse,
        "stdio" => McpTransport::Stdio,
        _ => McpTransport::Other,
    }
}

fn parse_auth_type(raw: &str) -> McpAuthType {
    match raw {
        "none" => McpAuthType::None,
        "api_key" => McpAuthType::Header,
        "bearer" => McpAuthType::Bearer,
        "oauth" => McpAuthType::Oauth,
        _ => McpAuthType::Other,
    }
}

/// `None`/unrecognized becomes [`McpPermissionMode::Ask`]: a server that says
/// nothing about review must not get the lenient reading by omission.
fn parse_permission_mode(raw: Option<&str>) -> McpPermissionMode {
    match raw {
        Some("allow") => McpPermissionMode::Allow,
        Some("deny") => McpPermissionMode::Deny,
        _ => McpPermissionMode::Ask,
    }
}

/// `None` for a method desktop cannot write to DA. OAuth rows are created
/// without a grant; the user completes sign-in afterward on this device.
fn auth_type_wire(auth: McpAuthType) -> Option<&'static str> {
    match auth {
        McpAuthType::None => Some("none"),
        McpAuthType::Header => Some("api_key"),
        McpAuthType::Bearer => Some("bearer"),
        McpAuthType::Oauth => Some("oauth"),
        McpAuthType::Other => None,
    }
}

fn permission_mode_wire(mode: McpPermissionMode) -> &'static str {
    match mode {
        McpPermissionMode::Allow => "allow",
        McpPermissionMode::Ask => "ask",
        McpPermissionMode::Deny => "deny",
    }
}

fn bounded(value: String, maximum: usize) -> Result<String, McpServerRepositoryError> {
    if value.len() > maximum {
        return Err(McpServerRepositoryError::InvalidRow);
    }
    Ok(value)
}

fn bounded_option(
    value: Option<String>,
    maximum: usize,
) -> Result<Option<String>, McpServerRepositoryError> {
    value.map(|value| bounded(value, maximum)).transpose()
}

/// Reads at most `maximum_bytes` of a response body, then rebuilds it so the
/// generated parser sees an ordinary `reqwest::Response`. Mirrors
/// `super::provider_keys::bounded_response`'s two-part discipline: a
/// `Content-Length` pre-check where one is advertised, plus a running total
/// across streamed chunks for when it is not.
async fn bounded_response(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<reqwest::Response, McpServerRepositoryError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(McpServerRepositoryError::Unavailable);
    }
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| McpServerRepositoryError::Unavailable)?;
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or(McpServerRepositoryError::Unavailable)?;
        if next_len > maximum_bytes {
            return Err(McpServerRepositoryError::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    let mut rebuilt = http::Response::builder().status(status);
    *rebuilt
        .headers_mut()
        .ok_or(McpServerRepositoryError::Unavailable)? = headers;
    let rebuilt = rebuilt
        .body(reqwest::Body::from(body))
        .map_err(|_| McpServerRepositoryError::Unavailable)?;
    Ok(reqwest::Response::from(rebuilt))
}
