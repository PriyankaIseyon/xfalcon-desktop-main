use std::fmt;

#[cfg(test)]
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use xfalcon_contracts::auth::IdentityProjection;
use xfalcon_contracts::credentials::{CredentialStatus, ProviderCredentialView, ProviderId};
use zeroize::Zeroizing;

use crate::credentials::service::{
    CredentialRecord, CredentialRepository, CredentialRepositoryError, NewCredential,
    ReplacementCredential, ResolvedCredential, MAX_PROVIDER_KEY_BYTES,
};

use super::{
    auth::{AuthenticatedDaClient, AuthenticatedPrincipal, BearerToken},
    generated,
    mapping::{map_provider, DaBoundaryError, DaRoute, TenantBaseUrl},
    secret::Secret,
};

const MAX_PROVIDER_KEY_PAGES: usize = 100;
const MAX_LIST_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROW_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_LABEL_BYTES: usize = 80;
const MAX_CIPHERTEXT_BYTES: usize = 32 * 1024;
const MAX_METADATA_BYTES: usize = 4 * 1024;
const MAX_METADATA_FIELDS: usize = 16;
const MAX_RESOLVED_RESPONSE_BYTES: usize = 16 * 1024;
const DA_SECRET_OWNER: &str = "data-analyst";
const DA_ENVELOPE_PREFIX: &str = "xfda:v1:";

pub(crate) struct DaCredentialRepository {
    client: AuthenticatedDaClient,
}

impl DaCredentialRepository {
    /// `client` is the owning service's shared client. Its connection pool is
    /// what makes repeated calls reuse one tenant connection.
    pub(crate) fn new(
        da_origin: &str,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
        client: reqwest::Client,
    ) -> Result<Self, CredentialRepositoryError> {
        let base =
            TenantBaseUrl::parse(da_origin).map_err(|_| CredentialRepositoryError::Unavailable)?;
        Self::from_parts(base, client, bearer, identity)
    }

    /// As [`Self::new`], against a loopback origin on a client the test owns.
    #[cfg(test)]
    pub(crate) fn for_test_with_client(
        da_origin: &str,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
        client: reqwest::Client,
    ) -> Result<Self, CredentialRepositoryError> {
        let base = TenantBaseUrl::parse_loopback(da_origin)
            .map_err(|_| CredentialRepositoryError::Unavailable)?;
        Self::from_parts(base, client, bearer, identity)
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        da_origin: &str,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
        timeout: Duration,
    ) -> Result<Self, CredentialRepositoryError> {
        let base = TenantBaseUrl::parse_loopback(da_origin)
            .map_err(|_| CredentialRepositoryError::Unavailable)?;
        let client = super::mapping::build_test_client(timeout)
            .map_err(|_| CredentialRepositoryError::Unavailable)?;
        Self::from_parts(base, client, bearer, identity)
    }

    fn from_parts(
        base: TenantBaseUrl,
        client: reqwest::Client,
        bearer: impl Into<String>,
        identity: &IdentityProjection,
    ) -> Result<Self, CredentialRepositoryError> {
        let bearer =
            BearerToken::new(bearer).map_err(|_| CredentialRepositoryError::Unauthorized)?;
        let principal = AuthenticatedPrincipal::from_verified_identity(identity)
            .map_err(|_| CredentialRepositoryError::Unauthorized)?;
        Ok(Self {
            client: AuthenticatedDaClient::new(base, client, bearer, principal),
        })
    }

    async fn list_page(
        &self,
        cursor: Option<String>,
    ) -> Result<(Vec<CredentialRecord>, Option<String>), CredentialRepositoryError> {
        let query = generated::ListProviderKeysApiV1ChatProviderKeysGetRequestQuery {
            sort: Some("-created_at".to_owned()),
            provider: None,
            cursor,
            limit: Some(200),
        };
        let response = self
            .client
            .get(DaRoute::ProviderKeys)
            .map_err(|_| CredentialRepositoryError::Unavailable)?
            .query(&query)
            .send()
            .await
            .map_err(|_| CredentialRepositoryError::Unavailable)?;
        let response = bounded_response(response, MAX_LIST_RESPONSE_BYTES).await?;
        let response =
            generated::ListProviderKeysApiV1ChatProviderKeysGetRequest::parse_response(response)
                .await
                .map_err(|_| CredentialRepositoryError::Unavailable)?;
        let page = match response {
            generated::ListProviderKeysApiV1ChatProviderKeysGetResponse::Ok(page) => page,
            generated::ListProviderKeysApiV1ChatProviderKeysGetResponse::Unauthorized(_)
            | generated::ListProviderKeysApiV1ChatProviderKeysGetResponse::Forbidden(_) => {
                return Err(CredentialRepositoryError::Unauthorized);
            }
            _ => return Err(CredentialRepositoryError::Unavailable),
        };
        let mut records = Vec::new();
        for row in page.items.unwrap_or_default() {
            append_list_record(&self.client, &mut records, row)?;
        }
        let next_cursor = if page.has_more == Some(true) {
            Some(
                page.next_cursor
                    .filter(|value| !value.is_empty())
                    .ok_or(CredentialRepositoryError::Unavailable)?,
            )
        } else {
            None
        };
        Ok((records, next_cursor))
    }
}

impl fmt::Debug for DaCredentialRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaCredentialRepository")
            .field("client", &self.client)
            .finish()
    }
}

#[async_trait]
impl CredentialRepository for DaCredentialRepository {
    async fn list(&self) -> Result<Vec<CredentialRecord>, CredentialRepositoryError> {
        let mut records = Vec::new();
        let mut cursor = None;
        for _ in 0..MAX_PROVIDER_KEY_PAGES {
            let (mut page, next_cursor) = self.list_page(cursor).await?;
            records.append(&mut page);
            match next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(records),
            }
        }
        Err(CredentialRepositoryError::Unavailable)
    }

    async fn reconcile_after_create(
        &self,
    ) -> Result<Vec<CredentialRecord>, CredentialRepositoryError> {
        self.list_page(None).await.map(|(records, _)| records)
    }

    async fn create(
        &self,
        credential: NewCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError> {
        let (provider, label, secret, is_default) = credential.into_parts();
        let body = ProviderKeyCreateBody {
            provider: provider_name(provider),
            secret: secret.as_str(),
            label: &label,
            base_url: None,
            is_default,
            is_enabled: true,
        };
        let response = self
            .client
            .post(DaRoute::ProviderKeys)
            .map_err(|_| CredentialRepositoryError::Unavailable)?
            .json(&body)
            .send()
            .await
            .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
        let response = bounded_response(response, MAX_ROW_RESPONSE_BYTES)
            .await
            .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
        let response =
            generated::CreateProviderKeyApiV1ChatProviderKeysPostRequest::parse_response(response)
                .await
                .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
        match response {
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::Created(row) => {
                // DA committed the non-idempotent mutation, but an invalid or
                // foreign response row prevents us from identifying the result.
                // Reconcile rather than inviting a duplicate retry.
                if row.id.is_nil()
                    || row.provider != provider_name(provider)
                    || !is_da_owned_envelope(&row.api_key, &row.metadata)
                {
                    return Err(CredentialRepositoryError::ReconciliationRequired);
                }
                domain_record(&self.client, row)
                    .map_err(|_| CredentialRepositoryError::ReconciliationRequired)
            }
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::Unauthorized(_)
            | generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::Forbidden(_) => {
                Err(CredentialRepositoryError::Unauthorized)
            }
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::Conflict(_) => {
                Err(CredentialRepositoryError::Conflict)
            }
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::UnprocessableEntity(
                _,
            ) => Err(CredentialRepositoryError::InvalidRequest),
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::TooManyRequests(_) => {
                Err(CredentialRepositoryError::RateLimited)
            }
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::ServiceUnavailable(
                _,
            ) => Err(CredentialRepositoryError::Unavailable),
            generated::CreateProviderKeyApiV1ChatProviderKeysPostResponse::Unknown => {
                Err(CredentialRepositoryError::ReconciliationRequired)
            }
            _ => Err(CredentialRepositoryError::Unavailable),
        }
    }

    async fn replace_secret(
        &self,
        credential: ReplacementCredential,
    ) -> Result<CredentialRecord, CredentialRepositoryError> {
        let ReplacementCredential {
            id,
            provider,
            secret,
        } = credential;
        let body = ProviderKeyReplaceBody {
            secret: secret.as_str(),
        };
        let id = id.to_string();
        let response = self
            .client
            .request_nested(
                reqwest::Method::PUT,
                DaRoute::ProviderKeys,
                &[&id, "secret"],
            )
            .map_err(|_| CredentialRepositoryError::Unavailable)?
            .json(&body)
            .send()
            .await
            .map_err(|_| CredentialRepositoryError::Unavailable)?;
        let response = bounded_response(response, MAX_ROW_RESPONSE_BYTES)
            .await
            .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
        match response.status() {
            reqwest::StatusCode::OK => {
                let row = response
                    .json::<generated::ProviderKey>()
                    .await
                    .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
                if row.id.to_string() != id
                    || row.provider != provider_name(provider)
                    || !is_da_owned_envelope(&row.api_key, &row.metadata)
                {
                    return Err(CredentialRepositoryError::ReconciliationRequired);
                }
                domain_record(&self.client, row)
                    .map_err(|_| CredentialRepositoryError::ReconciliationRequired)
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(CredentialRepositoryError::Unauthorized)
            }
            reqwest::StatusCode::NOT_FOUND => Err(CredentialRepositoryError::NotFound),
            reqwest::StatusCode::CONFLICT => Err(CredentialRepositoryError::Conflict),
            reqwest::StatusCode::UNPROCESSABLE_ENTITY => {
                Err(CredentialRepositoryError::InvalidRequest)
            }
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(CredentialRepositoryError::RateLimited),
            reqwest::StatusCode::SERVICE_UNAVAILABLE => Err(CredentialRepositoryError::Unavailable),
            _ => Err(CredentialRepositoryError::Unavailable),
        }
    }

    async fn delete(&self, id: uuid::Uuid) -> Result<(), CredentialRepositoryError> {
        let id = id.to_string();
        let response = self
            .client
            .request_nested(reqwest::Method::DELETE, DaRoute::ProviderKeys, &[&id])
            .map_err(|_| CredentialRepositoryError::Unavailable)?
            .send()
            .await
            // Sent but unanswered: DA may already have removed the row, so the
            // outcome is unknown, as for a lost `create` -- the caller reloads
            // the list rather than reporting a failure that may not be one.
            .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
        let response = bounded_response(response, MAX_ROW_RESPONSE_BYTES)
            .await
            .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
        match response.status() {
            reqwest::StatusCode::OK => {
                // DA answers with the row it removed. One that names another
                // row leaves what was deleted unknown, so the caller re-reads
                // the list rather than reporting this key gone.
                let row = response
                    .json::<generated::ProviderKey>()
                    .await
                    .map_err(|_| CredentialRepositoryError::ReconciliationRequired)?;
                if row.id.to_string() != id {
                    return Err(CredentialRepositoryError::ReconciliationRequired);
                }
                Ok(())
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(CredentialRepositoryError::Unauthorized)
            }
            reqwest::StatusCode::NOT_FOUND => Err(CredentialRepositoryError::NotFound),
            reqwest::StatusCode::UNPROCESSABLE_ENTITY => {
                Err(CredentialRepositoryError::InvalidRequest)
            }
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(CredentialRepositoryError::RateLimited),
            _ => Err(CredentialRepositoryError::Unavailable),
        }
    }

    async fn resolve_secret(
        &self,
        id: uuid::Uuid,
        provider: ProviderId,
    ) -> Result<ResolvedCredential, CredentialRepositoryError> {
        let action = format!("{id}:resolve");
        let response = self
            .client
            .request_nested(reqwest::Method::POST, DaRoute::ProviderKeys, &[&action])
            .map_err(|_| CredentialRepositoryError::Unavailable)?
            .send()
            .await
            .map_err(|_| CredentialRepositoryError::Unavailable)?;
        let has_no_store = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"))
            });
        let status = response.status();
        let body = bounded_body(response, MAX_RESOLVED_RESPONSE_BYTES).await?;
        match status {
            reqwest::StatusCode::OK => {
                if !has_no_store {
                    return Err(CredentialRepositoryError::Unavailable);
                }
                let resolved = serde_json::from_slice::<ProviderKeySecretResolvedBody>(&body)
                    .map_err(|_| CredentialRepositoryError::Unavailable)?;
                let secret = resolved.secret;
                if resolved.id != id
                    || resolved.provider != provider_name(provider)
                    || secret.is_empty()
                    || secret.len() > MAX_PROVIDER_KEY_BYTES
                {
                    return Err(CredentialRepositoryError::Unavailable);
                }
                Ok(ResolvedCredential {
                    credential_id: resolved.id.to_string(),
                    secret,
                    base_url: resolved.base_url,
                })
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(CredentialRepositoryError::Unauthorized)
            }
            reqwest::StatusCode::NOT_FOUND => Err(CredentialRepositoryError::NotFound),
            reqwest::StatusCode::CONFLICT => Err(CredentialRepositoryError::Conflict),
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(CredentialRepositoryError::RateLimited),
            reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::INTERNAL_SERVER_ERROR => {
                Err(CredentialRepositoryError::Unavailable)
            }
            _ => Err(CredentialRepositoryError::Unavailable),
        }
    }
}

#[derive(Serialize)]
struct ProviderKeyReplaceBody<'a> {
    secret: &'a str,
}

#[derive(Serialize)]
struct ProviderKeyCreateBody<'a> {
    provider: &'static str,
    secret: &'a str,
    label: &'a str,
    base_url: Option<&'a str>,
    is_default: bool,
    is_enabled: bool,
}

#[derive(Deserialize)]
struct ProviderKeySecretResolvedBody {
    id: uuid::Uuid,
    provider: String,
    base_url: Option<String>,
    #[serde(deserialize_with = "deserialize_zeroizing_string")]
    secret: Zeroizing<String>,
}

fn deserialize_zeroizing_string<'de, D>(deserializer: D) -> Result<Zeroizing<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Zeroizing::new)
}

pub(super) fn domain_record(
    client: &AuthenticatedDaClient,
    raw: generated::ProviderKey,
) -> Result<CredentialRecord, CredentialRepositoryError> {
    if !client.matches_principal(&raw.user_id, &raw.tenant_id) {
        return Err(CredentialRepositoryError::Unauthorized);
    }
    let provider = map_provider(&raw.provider)
        .map_err(|_| CredentialRepositoryError::UnsupportedProviderRow)?;
    validate_raw_row(&raw)?;
    Ok(CredentialRecord::new(
        raw.id,
        provider,
        raw.label,
        raw.api_key,
        raw.metadata,
        raw.is_default,
        raw.is_enabled,
    ))
}

pub(super) fn append_list_record(
    client: &AuthenticatedDaClient,
    records: &mut Vec<CredentialRecord>,
    raw: generated::ProviderKey,
) -> Result<(), CredentialRepositoryError> {
    match domain_record(client, raw) {
        Ok(record) => records.push(record),
        Err(CredentialRepositoryError::UnsupportedProviderRow) => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn validate_raw_row(raw: &generated::ProviderKey) -> Result<(), CredentialRepositoryError> {
    let metadata_bytes = serde_json::to_vec(&raw.metadata)
        .map_err(|_| CredentialRepositoryError::Unavailable)?
        .len();
    if raw.label.len() > MAX_LABEL_BYTES
        || raw.api_key.is_empty()
        || raw.api_key.len() > MAX_CIPHERTEXT_BYTES
        || raw.metadata.len() > MAX_METADATA_FIELDS
        || metadata_bytes > MAX_METADATA_BYTES
    {
        return Err(CredentialRepositoryError::Unavailable);
    }
    Ok(())
}

fn metadata_is_da_owned(metadata: &IndexMap<String, Value>) -> bool {
    metadata.get("secret_owner").and_then(Value::as_str) == Some(DA_SECRET_OWNER)
}

fn is_da_owned_envelope(ciphertext: &str, metadata: &IndexMap<String, Value>) -> bool {
    ciphertext.starts_with(DA_ENVELOPE_PREFIX) && metadata_is_da_owned(metadata)
}

pub(super) async fn bounded_response(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<reqwest::Response, CredentialRepositoryError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(CredentialRepositoryError::Unavailable);
    }
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| CredentialRepositoryError::Unavailable)?;
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or(CredentialRepositoryError::Unavailable)?;
        if next_len > maximum_bytes {
            return Err(CredentialRepositoryError::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    let mut rebuilt = http::Response::builder().status(status);
    *rebuilt
        .headers_mut()
        .ok_or(CredentialRepositoryError::Unavailable)? = headers;
    let rebuilt = rebuilt
        .body(reqwest::Body::from(body))
        .map_err(|_| CredentialRepositoryError::Unavailable)?;
    Ok(reqwest::Response::from(rebuilt))
}

async fn bounded_body(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, CredentialRepositoryError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(CredentialRepositoryError::Unavailable);
    }
    let mut stream = response.bytes_stream();
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| CredentialRepositoryError::Unavailable)?;
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or(CredentialRepositoryError::Unavailable)?;
        if next_len > maximum_bytes {
            return Err(CredentialRepositoryError::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

const fn provider_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::OpenAi => "openai",
        ProviderId::Anthropic => "anthropic",
        ProviderId::Google => "google",
        ProviderId::OpenRouter => "openrouter",
        ProviderId::CustomOpenAi => "custom_openai",
        ProviderId::ClaudeCode => "claude_code",
        ProviderId::ChatgptPlan => "chatgpt_plan",
    }
}

pub(crate) struct ProviderKeyRecord {
    id: uuid::Uuid,
    provider: String,
    label: String,
    ciphertext: Secret<String>,
    metadata: Secret<IndexMap<String, Value>>,
    is_default: bool,
    is_enabled: bool,
}

impl ProviderKeyRecord {
    pub(crate) fn from_generated(raw: generated::ProviderKey) -> Self {
        Self {
            id: raw.id,
            provider: raw.provider,
            label: raw.label,
            ciphertext: Secret::new(raw.api_key),
            metadata: Secret::new(raw.metadata),
            is_default: raw.is_default,
            is_enabled: raw.is_enabled,
        }
    }

    fn has_compatible_desktop_envelope(&self) -> bool {
        let metadata = self.metadata.expose();
        let has_device_key = metadata
            .get("device_key_id")
            .and_then(Value::as_str)
            .is_some_and(|key_id| !key_id.trim().is_empty());

        self.ciphertext.expose().starts_with("xfdc:v1:") && has_device_key
    }

    fn is_da_owned(&self) -> bool {
        is_da_owned_envelope(self.ciphertext.expose(), self.metadata.expose())
    }
}

impl fmt::Debug for ProviderKeyRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderKeyRecord")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("label", &self.label)
            .field("is_default", &self.is_default)
            .field("is_enabled", &self.is_enabled)
            .finish()
    }
}

pub(crate) fn project_provider_key(
    raw: &ProviderKeyRecord,
) -> Result<ProviderCredentialView, DaBoundaryError> {
    let provider = map_provider(&raw.provider)?;
    let status = if !raw.is_enabled {
        CredentialStatus::Disabled
    } else if raw.is_da_owned() || raw.has_compatible_desktop_envelope() {
        CredentialStatus::Ready
    } else {
        CredentialStatus::NeedsReentry
    };
    let display_name = if raw.label.trim().is_empty() {
        provider_display_name(provider)
    } else {
        raw.label.trim().to_owned()
    };

    Ok(ProviderCredentialView::new(
        raw.id.to_string(),
        provider,
        display_name,
        status,
        raw.is_default,
    ))
}

pub(crate) fn project_provider_keys(
    rows: impl IntoIterator<Item = ProviderKeyRecord>,
) -> Result<Vec<ProviderCredentialView>, DaBoundaryError> {
    rows.into_iter()
        .map(|row| project_provider_key(&row))
        .collect()
}

fn provider_display_name(provider: ProviderId) -> String {
    match provider {
        ProviderId::OpenAi => "OpenAI",
        ProviderId::Anthropic => "Anthropic",
        ProviderId::Google => "Google",
        ProviderId::OpenRouter => "OpenRouter",
        ProviderId::CustomOpenAi => "Custom model",
        ProviderId::ClaudeCode => "Claude account",
        ProviderId::ChatgptPlan => "ChatGPT plan",
    }
    .to_owned()
}
