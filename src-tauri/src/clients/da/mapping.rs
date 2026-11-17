use std::{error::Error, fmt, time::Duration};

use reqwest::{redirect::Policy, Client, Url};
use xfalcon_contracts::credentials::ProviderId;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DaRoute {
    ProviderKeys,
    Models,
    AuditEvents,
    McpServers,
    McpOauthGrants,
    ReportKits,
}

impl DaRoute {
    const fn path(self) -> &'static str {
        match self {
            Self::ProviderKeys => "/api/v1/chat/provider-keys",
            Self::Models => "/api/v1/chat/models",
            Self::AuditEvents => "/api/v1/chat/audit-events",
            Self::McpServers => "/api/v1/chat/mcp-servers",
            Self::McpOauthGrants => "/api/v1/chat/mcp-oauth-grants",
            Self::ReportKits => "/api/v1/user/report-kits",
        }
    }

    /// The `{id}`-scoped path under this collection. Built by appending a
    /// percent-encoded segment through `Url::path_segments_mut` rather than by
    /// formatting a string, so an id carrying `/`, `?`, `#`, or `..` cannot
    /// reshape the request into a different DA route.
    fn scoped_url(self, base: &Url, id: &str) -> Result<Url, DaBoundaryError> {
        self.nested_url(base, &[id])
    }

    /// The path built by appending each of `segments` under this collection, each
    /// percent-encoded as one segment.
    ///
    /// Same guarantee as [`Self::scoped_url`], extended to the two shapes DA's
    /// grant endpoints use: a nested collection (`…/by-server/{id}`) and an
    /// action suffixed onto an id with a colon
    /// (`…/{id}:acquire-refresh-lease`). The colon is legal inside a path
    /// segment and is not escaped, while a `/` or `..` smuggled into an id still
    /// is — which is the property that matters.
    fn nested_url(self, base: &Url, segments: &[&str]) -> Result<Url, DaBoundaryError> {
        let mut url = base.clone();
        url.set_path(self.path());
        url.path_segments_mut()
            .map_err(|_| DaBoundaryError::InvalidTenantOrigin)?
            .extend(segments);
        Ok(url)
    }
}

#[derive(Clone)]
pub(crate) struct TenantBaseUrl(Url);

impl TenantBaseUrl {
    pub(crate) fn parse(configured_origin: &str) -> Result<Self, DaBoundaryError> {
        let url =
            Url::parse(configured_origin).map_err(|_| DaBoundaryError::InvalidTenantOrigin)?;
        let is_root_path = url.path().is_empty() || url.path() == "/";
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !is_root_path
        {
            return Err(DaBoundaryError::InvalidTenantOrigin);
        }

        let mut origin = url;
        origin.set_path("/");
        Ok(Self(origin))
    }

    #[cfg(test)]
    pub(crate) fn parse_loopback(configured_origin: &str) -> Result<Self, DaBoundaryError> {
        let url =
            Url::parse(configured_origin).map_err(|_| DaBoundaryError::InvalidTenantOrigin)?;
        let is_root_path = url.path().is_empty() || url.path() == "/";
        let is_loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|address| address.is_loopback());
        if url.scheme() != "http"
            || !is_loopback
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !is_root_path
        {
            return Err(DaBoundaryError::InvalidTenantOrigin);
        }

        let mut origin = url;
        origin.set_path("/");
        Ok(Self(origin))
    }

    pub(crate) fn url_for(&self, route: DaRoute) -> Url {
        let mut url = self.0.clone();
        url.set_path(route.path());
        url
    }

    pub(crate) fn url_with_path(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        url.set_path(path);
        url
    }

    pub(crate) fn nested_url_for(
        &self,
        route: DaRoute,
        segments: &[&str],
    ) -> Result<Url, DaBoundaryError> {
        route.nested_url(&self.0, segments)
    }

    pub(crate) fn scoped_url_for(&self, route: DaRoute, id: &str) -> Result<Url, DaBoundaryError> {
        route.scoped_url(&self.0, id)
    }
}

impl fmt::Debug for TenantBaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TenantBaseUrl")
            .field(&self.0.as_str())
            .finish()
    }
}

pub(crate) fn build_hardened_client() -> Result<Client, DaBoundaryError> {
    Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(|_| DaBoundaryError::ClientBuild)
}

/// A client for a request the server legitimately works on for a long time,
/// such as a report refresh that re-runs SQL. The ordinary client's read timeout
/// also covers the wait for the first response byte, so it would cut such a
/// request off at 30 s whatever per-request timeout is set; this one bounds the
/// whole request by `total` instead.
pub(crate) fn build_long_request_client(total: Duration) -> Result<Client, DaBoundaryError> {
    Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(total)
        .build()
        .map_err(|_| DaBoundaryError::ClientBuild)
}

#[cfg(test)]
pub(crate) fn build_test_client(timeout: Duration) -> Result<Client, DaBoundaryError> {
    Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(timeout)
        .read_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|_| DaBoundaryError::ClientBuild)
}

pub(crate) fn map_provider(provider: &str) -> Result<ProviderId, DaBoundaryError> {
    match provider {
        "openai" => Ok(ProviderId::OpenAi),
        "anthropic" => Ok(ProviderId::Anthropic),
        "google" => Ok(ProviderId::Google),
        "openrouter" => Ok(ProviderId::OpenRouter),
        "custom_openai" => Ok(ProviderId::CustomOpenAi),
        _ => Err(DaBoundaryError::UnsupportedProvider),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DaBoundaryError {
    InvalidTenantOrigin,
    InvalidBearer,
    InvalidIdentity,
    ClientBuild,
    UnsupportedProvider,
    UnavailableModel,
}

impl fmt::Display for DaBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidTenantOrigin => "the configured DA tenant origin is invalid",
            Self::InvalidBearer => "the DA bearer token is invalid",
            Self::InvalidIdentity => "the verified DA identity is invalid",
            Self::ClientBuild => "the hardened DA HTTP client could not be built",
            Self::UnsupportedProvider => "the DA row names an unsupported provider",
            Self::UnavailableModel => "the DA model is unavailable to this desktop build",
        };
        formatter.write_str(message)
    }
}

impl Error for DaBoundaryError {}
