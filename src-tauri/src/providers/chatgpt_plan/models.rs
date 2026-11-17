//! The models a signed-in ChatGPT account may use. The desktop offers only the
//! ones whose limits OpenAI documents, so a listed model it does not know is hidden.

use std::collections::HashSet;
use std::time::Duration;

use reqwest::header::AUTHORIZATION;
use reqwest::{Client, Response, StatusCode, Url};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::providers::http::sensitive_header;

/// The real listing is a few kilobytes. The bound keeps a misbehaving endpoint
/// from making the app buffer without limit.
const MAX_MODEL_LIST_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ListedModel {
    pub(crate) slug: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OfferedModel {
    pub(crate) slug: String,
    pub(crate) display_name: String,
    pub(crate) context_length: u64,
    pub(crate) max_output_tokens: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ModelsError {
    #[error("the sign-in was not accepted")]
    Unauthorized,
    #[error("the plan cannot list models")]
    Forbidden,
    #[error("the model list is temporarily unavailable")]
    Unavailable,
    #[error("the model list was malformed")]
    Malformed,
}

/// `(slug, display_name, context_length, max_output_tokens)` for each model whose
/// limits OpenAI documents. A slug missing from this table is hidden even when the
/// account can use it. The display name is the desktop's own: OpenAI's listed name
/// is never shown, so the picker reads the same whatever the listing says.
pub(crate) const DOCUMENTED_MODELS: [(&str, &str, u64, u64); 2] = [
    ("gpt-6.1-sol", "Sol", 1_050_000, 128_000),
    ("gpt-6-luna", "Luna", 1_050_000, 128_000),
];

/// Reads the `models` array of a listing, keeping the entries the account may
/// list. A duplicate slug keeps its first entry.
pub(crate) fn parse_model_list(body: &[u8]) -> Result<Vec<ListedModel>, ModelsError> {
    let root: Value = serde_json::from_slice(body).map_err(|_| ModelsError::Malformed)?;
    let models = root
        .get("models")
        .and_then(Value::as_array)
        .ok_or(ModelsError::Malformed)?;

    let mut seen = HashSet::new();
    let mut listed = Vec::new();
    for entry in models {
        let Some(slug) = entry
            .get("slug")
            .and_then(Value::as_str)
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        // Checked before the slug is claimed: a hidden entry must not hide a
        // visible one that the listing places after it.
        if entry.get("visibility").and_then(Value::as_str) != Some("list") {
            continue;
        }
        if !seen.insert(slug) {
            continue;
        }
        listed.push(ListedModel {
            slug: slug.to_owned(),
        });
    }
    Ok(listed)
}

/// The listed models the desktop offers, in the listed order, each with its
/// documented name and limits.
pub(crate) fn offered_models(listed: &[ListedModel]) -> Vec<OfferedModel> {
    listed
        .iter()
        .filter_map(|model| {
            let (_, display_name, context_length, max_output_tokens) = DOCUMENTED_MODELS
                .iter()
                .find(|(slug, _, _, _)| *slug == model.slug)?;
            Some(OfferedModel {
                slug: model.slug.clone(),
                display_name: (*display_name).to_owned(),
                context_length: *context_length,
                max_output_tokens: *max_output_tokens,
            })
        })
        .collect()
}

/// Lists the models the signed-in account may use, from `endpoint`, which is
/// OpenAI's public `/v1/models`. ChatGPT's `backend-api` is not a documented
/// contract and is not used.
///
/// The token is sent only as the bearer credential. Neither it nor the response
/// body is logged. `http` must not follow redirects, or the bearer would travel
/// to wherever a redirect points.
pub(crate) async fn list_models(
    http: &Client,
    endpoint: &Url,
    access_token: &str,
) -> Result<Vec<ListedModel>, ModelsError> {
    let bearer = Zeroizing::new(format!("Bearer {access_token}"));
    let authorization = sensitive_header(bearer.as_str()).map_err(|_| ModelsError::Unauthorized)?;
    let response = http
        .get(endpoint.clone())
        .header(AUTHORIZATION, authorization)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|_| ModelsError::Unavailable)?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(ModelsError::Unauthorized);
    }
    if status == StatusCode::FORBIDDEN {
        return Err(ModelsError::Forbidden);
    }
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(ModelsError::Unavailable);
    }
    if !status.is_success() {
        return Err(ModelsError::Malformed);
    }
    parse_model_list(&read_capped(response).await?)
}

async fn read_capped(mut response: Response) -> Result<Vec<u8>, ModelsError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ModelsError::Unavailable)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_MODEL_LIST_BYTES {
            return Err(ModelsError::Malformed);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn model(slug: &str) -> ListedModel {
        ListedModel {
            slug: slug.to_owned(),
        }
    }

    fn parse(body: &str) -> Result<Vec<ListedModel>, ModelsError> {
        parse_model_list(body.as_bytes())
    }

    #[test]
    fn only_entries_visible_in_the_list_are_kept_in_server_order() {
        let body = r#"{"models":[
            {"slug":"gpt-6-luna","display_name":"GPT-6 Luna","visibility":"list"},
            {"slug":"internal-only","display_name":"Internal","visibility":"hide"},
            {"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list"},
            {"slug":"no-visibility","display_name":"Missing"}
        ]}"#;
        assert_eq!(
            parse(body),
            Ok(vec![model("gpt-6-luna"), model("gpt-6.1-sol")])
        );
    }

    #[test]
    fn a_duplicate_slug_is_kept_once_at_its_first_listed_place() {
        let body = r#"{"models":[
            {"slug":"gpt-6.1-sol","display_name":"First","visibility":"list"},
            {"slug":"gpt-6-luna","display_name":"Luna","visibility":"list"},
            {"slug":"gpt-6.1-sol","display_name":"Second","visibility":"list"}
        ]}"#;
        assert_eq!(
            parse(body),
            Ok(vec![model("gpt-6.1-sol"), model("gpt-6-luna")])
        );
    }

    #[test]
    fn a_hidden_entry_does_not_claim_its_slug_from_a_visible_one() {
        let body = r#"{"models":[
            {"slug":"gpt-6.1-sol","display_name":"Hidden","visibility":"hide"},
            {"slug":"gpt-6.1-sol","display_name":"Visible","visibility":"list"}
        ]}"#;
        assert_eq!(parse(body), Ok(vec![model("gpt-6.1-sol")]));
    }

    #[test]
    fn entries_without_a_usable_slug_are_skipped() {
        let body = r#"{"models":[
            {"display_name":"No slug","visibility":"list"},
            {"slug":"","display_name":"Empty slug","visibility":"list"},
            {"slug":42,"display_name":"Numeric slug","visibility":"list"},
            "not an object",
            {"slug":"gpt-6-luna","display_name":"GPT-6 Luna","visibility":"list"}
        ]}"#;
        assert_eq!(parse(body), Ok(vec![model("gpt-6-luna")]));
    }

    #[test]
    fn a_missing_or_empty_display_name_does_not_drop_the_entry() {
        let body = r#"{"models":[
            {"slug":"gpt-6.1-sol","visibility":"list"},
            {"slug":"gpt-6-luna","display_name":"","visibility":"list"}
        ]}"#;
        assert_eq!(
            parse(body),
            Ok(vec![model("gpt-6.1-sol"), model("gpt-6-luna")])
        );
    }

    #[test]
    fn an_empty_model_list_is_not_an_error() {
        assert_eq!(parse(r#"{"models":[]}"#), Ok(vec![]));
    }

    #[test]
    fn a_body_without_a_models_array_is_malformed() {
        for body in [
            "not json",
            "[]",
            "{}",
            r#"{"models":null}"#,
            r#"{"models":{"slug":"gpt-6-luna"}}"#,
        ] {
            assert_eq!(parse(body), Err(ModelsError::Malformed), "{body}");
        }
    }

    #[test]
    fn an_undocumented_slug_is_not_offered_and_documented_ones_carry_their_limits() {
        let listed = vec![
            model("gpt-6-nova"),
            model("gpt-5-private"),
            model("gpt-6-luna"),
            model("gpt-6.1-sol"),
        ];
        assert_eq!(
            offered_models(&listed),
            vec![
                OfferedModel {
                    slug: "gpt-6-luna".to_owned(),
                    display_name: "Luna".to_owned(),
                    context_length: 1_050_000,
                    max_output_tokens: 128_000,
                },
                OfferedModel {
                    slug: "gpt-6.1-sol".to_owned(),
                    display_name: "Sol".to_owned(),
                    context_length: 1_050_000,
                    max_output_tokens: 128_000,
                },
            ]
        );
    }

    #[test]
    fn the_offered_names_are_fixed_whatever_the_listing_calls_them() {
        let listed = parse(
            r#"{"models":[{"slug":"gpt-6-luna","display_name":"GPT-6 Luna (preview)","visibility":"list"}]}"#,
        )
        .expect("listing parses");

        let offered = offered_models(&listed);

        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].display_name, "Luna");
    }

    fn client() -> Client {
        Client::builder()
            .retry(reqwest::retry::never())
            .build()
            .expect("client builds")
    }

    fn endpoint(server: &MockServer) -> Url {
        Url::parse(&format!("{}/v1/models", server.uri())).expect("url parses")
    }

    /// Serves one response to every request and lists from it.
    async fn list_from(response: ResponseTemplate) -> Result<Vec<ListedModel>, ModelsError> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(response)
            .mount(&server)
            .await;
        list_models(&client(), &endpoint(&server), "token").await
    }

    /// A valid listing whose body is exactly `total` bytes long.
    fn padded_listing(total: usize) -> String {
        const HEAD: &str = r#"{"models":[],"padding":""#;
        const TAIL: &str = r#""}"#;
        format!(
            "{HEAD}{}{TAIL}",
            "x".repeat(total - HEAD.len() - TAIL.len())
        )
    }

    #[tokio::test]
    async fn the_bearer_token_is_sent_and_a_200_listing_is_parsed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer access-token-value"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"models":[{"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list"}]}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let listed = list_models(&client(), &endpoint(&server), "access-token-value").await;
        assert_eq!(listed, Ok(vec![model("gpt-6.1-sol")]));
    }

    #[tokio::test]
    async fn a_rejected_status_maps_to_its_error() {
        assert_eq!(
            list_from(ResponseTemplate::new(401)).await,
            Err(ModelsError::Unauthorized)
        );
        assert_eq!(
            list_from(ResponseTemplate::new(403)).await,
            Err(ModelsError::Forbidden)
        );
        assert_eq!(
            list_from(ResponseTemplate::new(429)).await,
            Err(ModelsError::Unavailable)
        );
        assert_eq!(
            list_from(ResponseTemplate::new(503)).await,
            Err(ModelsError::Unavailable)
        );
        assert_eq!(
            list_from(ResponseTemplate::new(500)).await,
            Err(ModelsError::Unavailable)
        );
        assert_eq!(
            list_from(ResponseTemplate::new(400)).await,
            Err(ModelsError::Malformed)
        );
        assert_eq!(
            list_from(ResponseTemplate::new(404)).await,
            Err(ModelsError::Malformed)
        );
    }

    #[tokio::test]
    async fn a_dropped_connection_is_unavailable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binds");
        let endpoint = Url::parse(&format!(
            "http://{}/v1/models",
            listener.local_addr().expect("address")
        ))
        .expect("url parses");
        tokio::spawn(async move {
            // Accepts the connection and closes it without answering.
            let _ = listener.accept().await;
        });

        assert_eq!(
            list_models(&client(), &endpoint, "token").await,
            Err(ModelsError::Unavailable)
        );
    }

    #[tokio::test]
    async fn a_listing_exactly_at_the_byte_cap_is_read() {
        let body = padded_listing(MAX_MODEL_LIST_BYTES);
        assert_eq!(
            list_from(ResponseTemplate::new(200).set_body_string(body)).await,
            Ok(vec![])
        );
    }

    #[tokio::test]
    async fn a_listing_over_the_byte_cap_is_malformed() {
        // Valid JSON, so only the size bound can refuse it.
        let body = padded_listing(MAX_MODEL_LIST_BYTES + 1);
        assert_eq!(
            list_from(ResponseTemplate::new(200).set_body_string(body)).await,
            Err(ModelsError::Malformed)
        );
    }
}
