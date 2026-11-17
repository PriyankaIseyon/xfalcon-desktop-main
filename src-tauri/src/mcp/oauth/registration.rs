//! Dynamic client registration (RFC 7591).
//!
//! A native app cannot ship a client id per authorization server it has never
//! met, which is exactly the problem RFC 7591 exists to solve: the client asks
//! the authorization server to issue it one, and gets back credentials scoped to
//! the redirect URI it just declared.
//!
//! # This is implemented *and* expected to be refused
//!
//! Both hops of the Iseyon stack disable it today: DA's `POST /register` answers
//! `405 dynamic_client_registration_disabled` and points at the admin
//! connector-config endpoint, and its Casdoor answers `400
//! invalid_client_metadata` — "dynamic client registration is disabled for this
//! organization". Third-party MCP servers overwhelmingly do support it.
//!
//! So registration is attempted, and a refusal is a *first-class outcome*
//! ([`RegistrationError::Unavailable`]) rather than a failure: it is what tells
//! the surface to ask for the client id an operator issued out of band. That
//! ordering — try, then ask — is why signing in to a public MCP server needs no
//! configuration at all, while signing in to a DA tenant needs one field.

use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

use super::{endpoint_is_usable, read_json, CLIENT_NAME};
use reqwest::{Client, StatusCode, Url};

/// The client identity a flow authenticates with, however it was obtained.
#[derive(Clone)]
pub(crate) struct OauthClient {
    pub(crate) client_id: String,
    /// Present only when the authorization server issued one. A native app is a
    /// public client (RFC 8252 §8.4) and PKCE, not a secret, is what protects
    /// the code exchange — but an authorization server that issues a secret
    /// anyway will then *require* it, so it is kept and sent.
    pub(crate) client_secret: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for OauthClient {
    /// The client id is not confidential; the secret is, and never appears.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OauthClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum RegistrationError {
    /// The authorization server publishes no registration endpoint, or refused
    /// to register this client. Recoverable by supplying an operator-issued
    /// client id — see the module comment.
    #[error("this authorization server does not register clients dynamically")]
    Unavailable,
    #[error("the registration response was not usable")]
    Invalid,
    #[error("the registration request could not be completed")]
    RequestFailed,
}

#[derive(Debug, Deserialize)]
struct RegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

/// RFC 7591 §2 metadata for a native, public, authorization-code client.
#[derive(Debug, Serialize)]
struct RegistrationRequest<'a> {
    client_name: &'a str,
    redirect_uris: [&'a str; 1],
    grant_types: [&'a str; 2],
    response_types: [&'a str; 1],
    /// `none` declares this a public client: it holds no secret, and PKCE is
    /// what binds the code to this client instead (OAuth 2.1 §7.5.2). An
    /// authorization server free to ignore the declaration and issue a secret
    /// anyway is handled by [`OauthClient::client_secret`].
    token_endpoint_auth_method: &'a str,
    application_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<&'a str>,
}

/// Registers this app with `endpoint` for exactly `redirect_uri`.
///
/// The redirect URI is declared at registration and checked by the
/// authorization server on every later authorization request, which is why the
/// loopback port this app listens on has to be stable rather than ephemeral —
/// see `super::flow`.
pub(crate) async fn register(
    client: &Client,
    endpoint: &Url,
    redirect_uri: &str,
    scope: Option<&str>,
) -> Result<OauthClient, RegistrationError> {
    if !endpoint_is_usable(endpoint) {
        return Err(RegistrationError::Unavailable);
    }
    let body = RegistrationRequest {
        client_name: CLIENT_NAME,
        redirect_uris: [redirect_uri],
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
        token_endpoint_auth_method: "none",
        application_type: "native",
        scope,
    };

    let response = client
        .post(endpoint.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&json!(body))
        .send()
        .await
        .map_err(|_| RegistrationError::RequestFailed)?;

    let status = response.status();
    if !status.is_success() {
        // Every "we do not do this" shape lands on one outcome, because the
        // caller's next move is identical for all of them: a disabled policy
        // (Casdoor answers 400 `invalid_client_metadata`), a route that refuses
        // the method (DA answers 405), an endpoint that needs a token this app
        // has no way to hold (401/403), or one that is simply absent (404).
        return Err(match status {
            StatusCode::BAD_REQUEST
            | StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::NOT_FOUND
            | StatusCode::METHOD_NOT_ALLOWED => RegistrationError::Unavailable,
            _ => RegistrationError::RequestFailed,
        });
    }

    let registered: RegistrationResponse = read_json(response)
        .await
        .map_err(|_| RegistrationError::Invalid)?;
    if registered.client_id.trim().is_empty() {
        return Err(RegistrationError::Invalid);
    }
    Ok(OauthClient {
        client_id: registered.client_id.trim().to_owned(),
        client_secret: registered
            .client_secret
            .map(|secret| Zeroizing::new(secret.trim().to_owned()))
            .filter(|secret| !secret.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn registration_endpoint(server: &MockServer) -> Url {
        Url::parse(&format!("{}/register", server.uri())).expect("url")
    }

    #[tokio::test]
    async fn a_registered_public_client_declares_its_redirect_uri_and_native_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "client_id": "issued-client" })),
            )
            .mount(&server)
            .await;

        let client = super::super::build_client().expect("client");
        let registered = register(
            &client,
            &registration_endpoint(&server).await,
            "http://127.0.0.1:19848/callback",
            Some("openid offline_access"),
        )
        .await
        .expect("registration succeeds");

        assert_eq!(registered.client_id, "issued-client");
        assert!(registered.client_secret.is_none());

        let request = &server.received_requests().await.expect("requests")[0];
        let sent: serde_json::Value = serde_json::from_slice(&request.body).expect("json body");
        assert_eq!(sent["client_name"], json!(CLIENT_NAME));
        assert_eq!(
            sent["redirect_uris"],
            json!(["http://127.0.0.1:19848/callback"])
        );
        assert_eq!(sent["token_endpoint_auth_method"], json!("none"));
        assert_eq!(sent["application_type"], json!("native"));
        assert_eq!(
            sent["grant_types"],
            json!(["authorization_code", "refresh_token"])
        );
        assert_eq!(sent["scope"], json!("openid offline_access"));
    }

    #[tokio::test]
    async fn an_issued_secret_is_kept_because_the_server_will_then_require_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(ResponseTemplate::new(201).set_body_json(
                json!({ "client_id": "issued-client", "client_secret": "issued-secret" }),
            ))
            .mount(&server)
            .await;

        let client = super::super::build_client().expect("client");
        let registered = register(
            &client,
            &registration_endpoint(&server).await,
            "http://127.0.0.1:19848/callback",
            None,
        )
        .await
        .expect("registration succeeds");

        assert_eq!(
            registered.client_secret.as_deref().map(String::as_str),
            Some("issued-secret")
        );
        assert!(
            !format!("{registered:?}").contains("issued-secret"),
            "a client secret must never reach a debug rendering"
        );
    }

    /// The two refusals the Iseyon stack actually returns, plus the shapes other
    /// authorization servers use, must all become the one recoverable outcome.
    #[tokio::test]
    async fn every_refusal_shape_reports_that_the_user_must_supply_a_client_id() {
        for status in [400, 401, 403, 404, 405] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/register"))
                .respond_with(ResponseTemplate::new(status).set_body_json(
                    json!({ "error": "invalid_client_metadata", "error_description": "disabled" }),
                ))
                .mount(&server)
                .await;

            let client = super::super::build_client().expect("client");
            let outcome = register(
                &client,
                &registration_endpoint(&server).await,
                "http://127.0.0.1:19848/callback",
                None,
            )
            .await;

            assert_eq!(
                outcome.err(),
                Some(RegistrationError::Unavailable),
                "HTTP {status} must be reported as recoverable"
            );
        }
    }

    #[tokio::test]
    async fn a_response_without_a_client_id_is_not_a_registration() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "client_id": "   " })))
            .mount(&server)
            .await;

        let client = super::super::build_client().expect("client");
        let outcome = register(
            &client,
            &registration_endpoint(&server).await,
            "http://127.0.0.1:19848/callback",
            None,
        )
        .await;

        assert_eq!(outcome.err(), Some(RegistrationError::Invalid));
    }
}
