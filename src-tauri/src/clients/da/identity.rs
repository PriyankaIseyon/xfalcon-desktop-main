use reqwest::Url;
use thiserror::Error;
use xfalcon_contracts::auth::IdentityProjection;

use super::generated::IdentityResponse;

#[derive(Debug, Error)]
pub(crate) enum DaIdentityError {
    #[error("identity request failed")]
    RequestFailed,
    #[error("identity response returned status {0}")]
    UnexpectedStatus(u16),
    #[error("identity response was not readable as an identity")]
    InvalidResponse,
    #[error("identity names no user or no tenant")]
    IncompleteIdentity,
    /// The tenant DA reports is not the one the access token was minted for.
    /// Both are org slugs rather than secrets, and which pair disagrees is the
    /// whole diagnosis when a sign-in stops working after a tenant change.
    #[error("DA reports tenant {actual:?} for a token minted for {expected:?}")]
    TenantMismatch { expected: String, actual: String },
    #[error("session is not authorized")]
    Unauthorized,
}

pub(crate) async fn fetch_chat_identity(
    client: &reqwest::Client,
    da_base: &Url,
    access_token: &str,
    expected_tenant_slug: &str,
) -> Result<IdentityProjection, DaIdentityError> {
    let mut url = da_base.clone();
    url.set_path("/api/v1/chat/identity");
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| DaIdentityError::RequestFailed)?;

    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err(DaIdentityError::Unauthorized),
        status => return Err(DaIdentityError::UnexpectedStatus(status)),
    }

    let body: IdentityResponse = response
        .json()
        .await
        .map_err(|_| DaIdentityError::InvalidResponse)?;
    if body.user_id.trim().is_empty() || body.tenant_id.trim().is_empty() {
        return Err(DaIdentityError::IncompleteIdentity);
    }
    if body.tenant_slug != expected_tenant_slug {
        return Err(DaIdentityError::TenantMismatch {
            expected: expected_tenant_slug.to_owned(),
            actual: body.tenant_slug,
        });
    }
    // The display name is the person, never the tenant: the account menu and
    // the chat greeting both address the signed-in user, and web greets with
    // the user's own name. DA's identity endpoint carries no user display
    // name, so the Casdoor username (`user_id` = preferred_username) is it.
    let display_name = body.user_id.clone();
    Ok(IdentityProjection::new(
        body.user_id,
        body.tenant_id,
        display_name,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn identity_server(tenant_slug: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/chat/identity"))
            .and(header("authorization", "Bearer access-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tenant_id": "tenant-id",
                "tenant_name": "Tenant Name",
                "tenant_slug": tenant_slug,
                "user_id": "user-123",
                "role": "member",
                "status": "active"
            })))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn accepts_identity_with_verified_token_owner() {
        let server = identity_server("tenant").await;
        let identity = fetch_chat_identity(
            &reqwest::Client::new(),
            &Url::parse(&server.uri()).expect("DA base URL"),
            "access-token",
            "tenant",
        )
        .await
        .expect("matching DA tenant");

        assert_eq!(identity.tenant_id, "tenant-id");
        assert_eq!(identity.user_id, "user-123");
        assert_eq!(
            identity.display_name, "user-123",
            "the display name addresses the person, never the tenant"
        );
    }

    #[tokio::test]
    async fn rejects_identity_with_different_tenant_slug() {
        let server = identity_server("different-tenant").await;
        let result = fetch_chat_identity(
            &reqwest::Client::new(),
            &Url::parse(&server.uri()).expect("DA base URL"),
            "access-token",
            "tenant",
        )
        .await;

        // Named rather than lumped in with a malformed body: a tenant that
        // disagrees with the token's is a configuration fact an operator can
        // act on, and the log is where they read it.
        assert!(matches!(
            result,
            Err(DaIdentityError::TenantMismatch { expected, actual })
                if expected == "tenant" && actual == "different-tenant"
        ));
    }
}
