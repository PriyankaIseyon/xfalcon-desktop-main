use std::fmt;

use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, RequestBuilder,
};
use xfalcon_contracts::auth::IdentityProjection;
use zeroize::Zeroizing;

use crate::auth::canonical_account_id;

use super::mapping::{DaBoundaryError, DaRoute, TenantBaseUrl};

pub(crate) struct BearerToken(Zeroizing<String>);

impl BearerToken {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, DaBoundaryError> {
        let value = value.into();
        if value.is_empty() || value.trim() != value {
            return Err(DaBoundaryError::InvalidBearer);
        }
        let token = Self(Zeroizing::new(value));
        token.authorization_header()?;
        Ok(token)
    }

    pub(crate) fn authorization_header(&self) -> Result<HeaderValue, DaBoundaryError> {
        let mut header = HeaderValue::from_str(&format!("Bearer {}", self.0.as_str()))
            .map_err(|_| DaBoundaryError::InvalidBearer)?;
        header.set_sensitive(true);
        Ok(header)
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BearerToken([REDACTED])")
    }
}

pub(crate) struct AuthenticatedDaClient {
    base_url: TenantBaseUrl,
    client: Client,
    bearer: BearerToken,
    principal: AuthenticatedPrincipal,
}

impl AuthenticatedDaClient {
    pub(crate) fn new(
        base_url: TenantBaseUrl,
        client: Client,
        bearer: BearerToken,
        principal: AuthenticatedPrincipal,
    ) -> Self {
        Self {
            base_url,
            client,
            bearer,
            principal,
        }
    }

    pub(crate) fn get(&self, route: DaRoute) -> Result<RequestBuilder, DaBoundaryError> {
        Ok(self
            .client
            .get(self.base_url.url_for(route))
            .header(AUTHORIZATION, self.bearer.authorization_header()?))
    }

    pub(crate) fn post(&self, route: DaRoute) -> Result<RequestBuilder, DaBoundaryError> {
        Ok(self
            .client
            .post(self.base_url.url_for(route))
            .header(AUTHORIZATION, self.bearer.authorization_header()?))
    }

    /// `PATCH <route>/{id}`. `id` is percent-encoded as a single path segment
    /// by [`TenantBaseUrl::scoped_url_for`], so it cannot escape the route.
    pub(crate) fn patch_scoped(
        &self,
        route: DaRoute,
        id: &str,
    ) -> Result<RequestBuilder, DaBoundaryError> {
        Ok(self
            .client
            .patch(self.base_url.scoped_url_for(route, id)?)
            .header(AUTHORIZATION, self.bearer.authorization_header()?))
    }

    /// `<method> <route>/<segments…>`, each segment percent-encoded on its own.
    ///
    /// The general form behind the shapes above, for DA routes that are not a
    /// flat `collection/{id}`: `mcp-oauth-grants/by-server/{server_id}` and
    /// `mcp-oauth-grants/{grant_id}:acquire-refresh-lease`.
    pub(crate) fn request_nested(
        &self,
        method: reqwest::Method,
        route: DaRoute,
        segments: &[&str],
    ) -> Result<RequestBuilder, DaBoundaryError> {
        Ok(self
            .client
            .request(method, self.base_url.nested_url_for(route, segments)?)
            .header(AUTHORIZATION, self.bearer.authorization_header()?))
    }

    /// `DELETE <route>/{id}`. Same encoding guarantee as
    /// [`Self::patch_scoped`].
    pub(crate) fn delete_scoped(
        &self,
        route: DaRoute,
        id: &str,
    ) -> Result<RequestBuilder, DaBoundaryError> {
        Ok(self
            .client
            .delete(self.base_url.scoped_url_for(route, id)?)
            .header(AUTHORIZATION, self.bearer.authorization_header()?))
    }

    pub(crate) fn matches_principal(&self, actor_id: &str, tenant_id: &str) -> bool {
        self.principal.actor_id == actor_id && self.principal.tenant_id == tenant_id
    }
}

impl fmt::Debug for AuthenticatedDaClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedDaClient")
            .field("base_url", &self.base_url)
            .field("bearer", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

pub(crate) struct AuthenticatedPrincipal {
    actor_id: String,
    tenant_id: String,
}

impl AuthenticatedPrincipal {
    pub(crate) fn from_verified_identity(
        identity: &IdentityProjection,
    ) -> Result<Self, DaBoundaryError> {
        canonical_account_id(identity).map_err(|_| DaBoundaryError::InvalidIdentity)?;
        Ok(Self {
            actor_id: identity.user_id.clone(),
            tenant_id: identity.tenant_id.clone(),
        })
    }
}

impl fmt::Debug for AuthenticatedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedPrincipal([REDACTED])")
    }
}
