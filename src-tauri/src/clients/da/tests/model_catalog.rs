//! Product catalogue reads are held between workspace refreshes. Private-model
//! inference refreshes the row before binding its exact key.
//!
//! Every case here counts what the tenant actually received. A cache is only
//! worth its risk if the request count stops growing with the number of chat
//! commands, and an assertion against the cache's own fields would pass
//! whether or not that holds — so the mock, not the struct, is the witness.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::super::chat_api::{DaChatApiError, DaChatSession};
use super::super::model_catalog::ModelCatalogCache;

const ROUTE: &str = "/api/v1/chat/models";
const BEARER_CANARY: &str = "wire-bearer-model-catalog-canary";
const ACCOUNT: &str = "tenant-1:user-1";
const OTHER_ACCOUNT: &str = "tenant-1:user-2";

/// Every field spelled out rather than relying on what may be defaulted: the
/// row is the pinned `ModelEntity` contract, and a test that quietly stopped
/// exercising a field would not say so.
fn page(model_id: &str) -> Value {
    json!({
        "items": [{
            "id": "00000000-0000-0000-0000-0000000000aa",
            "model_id": model_id,
            "provider": "openrouter",
            "provider_parent": "openrouter",
            "provider_sub": null,
            "display_name": "Tenant model",
            "context_length": 1_000_000,
            "max_output_tokens": 64_000,
            "input_price_per_m": 1.0,
            "output_price_per_m": 2.0,
            "released_at": null,
            "is_chat": true,
            "is_enabled": true,
            "supports_streaming": true,
            "supports_tools": true,
            "supports_vision": false,
            "metadata": {},
            "created_at": null,
            "updated_at": null,
            "is_active": true,
            "tenant_id": "tenant-1",
            "supports_reasoning": false,
            "supports_extended_thinking": false,
            "reasoning_param_style": "none",
            "default_thinking_budget_tokens": null,
            "owner_user_id": null,
            "base_url": null,
            "upstream_model_id": null,
            "provider_key_id": null
        }]
    })
}

async fn tenant(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn session(server: &MockServer) -> DaChatSession {
    DaChatSession::for_test(&server.uri(), BEARER_CANARY).expect("valid loopback DA session")
}

async fn requests(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .expect("the mock records requests")
        .len()
}

/// The point of the change: the workspace refresh pays for the catalogue, and
/// the chat paths that used to each pay for their own now pay nothing.
#[tokio::test]
async fn the_workspace_refresh_is_the_only_request_the_send_paths_need() {
    let server = tenant(ResponseTemplate::new(200).set_body_json(page("sonnet"))).await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    catalog
        .refresh(&da, ACCOUNT)
        .await
        .expect("the refresh publishes");
    for _ in 0..3 {
        let models = catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("the snapshot answers");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "sonnet");
    }

    assert_eq!(requests(&server).await, 1);
}

/// A chat command that beats the first workspace refresh still works. It pays
/// one round trip, and it publishes what it read so the next one does not.
#[tokio::test]
async fn a_send_before_the_first_refresh_fetches_once_and_no_more() {
    let server = tenant(ResponseTemplate::new(200).set_body_json(page("sonnet"))).await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    for _ in 0..3 {
        catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("a cold read fetches");
    }

    assert_eq!(requests(&server).await, 1);
}

/// A catalogue is what one tenant enabled for one signed-in user. Another
/// account reading it would be reading someone else's entitlements, so the
/// held snapshot is a miss for anyone it was not read for. One slot: the
/// second account's read replaces the first's rather than sitting beside it.
#[tokio::test]
async fn another_account_never_reads_the_held_catalogue() {
    let server = tenant(ResponseTemplate::new(200).set_body_json(page("sonnet"))).await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    catalog
        .refresh(&da, ACCOUNT)
        .await
        .expect("the refresh publishes");
    catalog
        .read_through(&da, OTHER_ACCOUNT)
        .await
        .expect("the other account fetches its own");
    assert_eq!(requests(&server).await, 2);

    catalog
        .read_through(&da, ACCOUNT)
        .await
        .expect("the first account fetches again");
    assert_eq!(requests(&server).await, 3);
}

/// Sign-out drops the snapshot. A signed-out process has no reason to still
/// hold what a tenant enabled for the person who just left it.
#[tokio::test]
async fn sign_out_drops_the_held_catalogue() {
    let server = tenant(ResponseTemplate::new(200).set_body_json(page("sonnet"))).await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    catalog
        .refresh(&da, ACCOUNT)
        .await
        .expect("the refresh publishes");
    catalog.clear();
    catalog
        .read_through(&da, ACCOUNT)
        .await
        .expect("the next read fetches");

    assert_eq!(requests(&server).await, 2);
}

/// A refusal is not an answer, and must not be held as one. Without this the
/// first send after a tenant blip would cache nothing useful and every later
/// one would read that nothing back.
#[tokio::test]
async fn a_refused_request_is_not_held() {
    let server = tenant(ResponseTemplate::new(503)).await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    // `.err()` rather than `expect_err`: the success type is the pinned
    // `ModelEntity`, which carries no `Debug` and is not ours to give one.
    for _ in 0..2 {
        assert_eq!(
            catalog.read_through(&da, ACCOUNT).await.err(),
            Some(DaChatApiError::RequestFailed)
        );
    }

    assert_eq!(requests(&server).await, 2);
}

/// An empty catalogue is a 200 with nothing a send can run on. Holding it
/// would keep every later read from retrying until the next foreground
/// refresh, which is a worse state than the round trip it saves.
#[tokio::test]
async fn an_empty_catalogue_is_not_held() {
    let server = tenant(ResponseTemplate::new(200).set_body_json(json!({ "items": [] }))).await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    for _ in 0..2 {
        assert!(catalog
            .read_through(&da, ACCOUNT)
            .await
            .expect("an empty page is a valid response")
            .is_empty());
    }

    assert_eq!(requests(&server).await, 2);
}

#[tokio::test]
async fn an_empty_refresh_clears_a_previously_held_catalogue() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with({
            let calls = Arc::clone(&calls);
            move |_request: &wiremock::Request| match calls.fetch_add(1, Ordering::SeqCst) {
                0 => ResponseTemplate::new(200).set_body_json(page("before-revocation")),
                1 | 2 => ResponseTemplate::new(200).set_body_json(json!({ "items": [] })),
                _ => panic!("catalogue read beyond the expected retry"),
            }
        })
        .expect(3)
        .mount(&server)
        .await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    assert_eq!(
        catalog
            .refresh(&da, ACCOUNT)
            .await
            .expect("initial refresh")[0]
            .model_id,
        "before-revocation"
    );
    assert!(catalog
        .refresh(&da, ACCOUNT)
        .await
        .expect("revocation refresh")
        .is_empty());
    assert!(catalog
        .read_through(&da, ACCOUNT)
        .await
        .expect("empty catalogue is retried")
        .is_empty());
    assert_eq!(requests(&server).await, 3);
}

/// Custom-model create, edit, and remove call this same refresh after DA
/// commits the mutation. A refresh must replace the held account snapshot so
/// the next picker read cannot return the row set from before the mutation.
#[tokio::test]
async fn a_post_mutation_refresh_replaces_the_held_catalogue() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path(ROUTE))
        .respond_with({
            let calls = Arc::clone(&calls);
            move |_request: &wiremock::Request| {
                let model_id = match calls.fetch_add(1, Ordering::SeqCst) {
                    0 => "before-mutation",
                    1 => "after-mutation",
                    _ => panic!("catalog fetched again after the refreshed snapshot was held"),
                };
                ResponseTemplate::new(200).set_body_json(page(model_id))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let da = session(&server);
    let catalog = ModelCatalogCache::default();

    catalog
        .refresh(&da, ACCOUNT)
        .await
        .expect("initial workspace refresh");
    let refreshed = catalog
        .refresh(&da, ACCOUNT)
        .await
        .expect("post-mutation refresh");
    assert_eq!(refreshed[0].model_id, "after-mutation");

    let held = catalog
        .read_through(&da, ACCOUNT)
        .await
        .expect("picker reads refreshed snapshot");
    assert_eq!(held[0].model_id, "after-mutation");
    assert_eq!(requests(&server).await, 2);
}

#[tokio::test]
async fn a_foreign_private_row_is_not_published_to_this_accounts_picker() {
    let mut response = page("sonnet");
    let mut foreign = response["items"][0].clone();
    foreign["id"] = json!("00000000-0000-0000-0000-0000000000bb");
    foreign["model_id"] = json!("custom/00000000-0000-0000-0000-0000000000bb");
    foreign["provider"] = json!("custom_openai");
    foreign["provider_parent"] = json!("custom_openai");
    foreign["owner_user_id"] = json!("user-2");
    foreign["base_url"] = json!("https://private.example.com/v1");
    foreign["upstream_model_id"] = json!("private-model");
    foreign["provider_key_id"] = json!("00000000-0000-0000-0000-0000000000cc");
    response["items"]
        .as_array_mut()
        .expect("items array")
        .push(foreign);
    let server = tenant(ResponseTemplate::new(200).set_body_json(response)).await;
    let catalog = ModelCatalogCache::default();

    let models = catalog
        .refresh(&session(&server), ACCOUNT)
        .await
        .expect("catalog response");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "sonnet");
    assert_eq!(requests(&server).await, 1);
}
