//! Repositories built on one shared client reuse one tenant connection, where a
//! client per call opens one each.
//!
//! Same witness as `connection_reuse.rs`: the loopback server's accept count.
//! The calls alternate between the provider-key and MCP-server repositories, so
//! the shared client is proven to pool across both.

use std::sync::atomic::Ordering;

use xfalcon_contracts::auth::IdentityProjection;

use super::super::mapping::build_hardened_client;
use super::super::mcp_oauth_grants::DaMcpGrantRepository;
use super::super::mcp_servers::DaMcpServerRepository;
use super::super::provider_keys::DaCredentialRepository;
use super::connection_reuse::spawn_keep_alive_server;
use crate::credentials::service::CredentialRepository;

const CALLS: usize = 4;

async fn one_call(
    index: usize,
    origin: &str,
    identity: &IdentityProjection,
    client: reqwest::Client,
) {
    if index.is_multiple_of(2) {
        let repository =
            DaCredentialRepository::for_test_with_client(origin, "test-token", identity, client)
                .expect("loopback credential repository");
        repository.list().await.expect("credential list");
    } else {
        let repository =
            DaMcpServerRepository::for_test_with_client(origin, "test-token", identity, client)
                .expect("loopback MCP server repository");
        repository.list().await.expect("MCP server list");
    }
}

#[tokio::test]
async fn repositories_on_one_shared_client_reuse_one_tenant_connection() {
    let (origin, accepted) = spawn_keep_alive_server().await;
    let identity = IdentityProjection::new("user-1", "tenant-1", "Test User", None);

    let shared = build_hardened_client().expect("hardened client");
    for index in 0..CALLS {
        one_call(index, &origin, &identity, shared.clone()).await;
    }
    let shared_connections = accepted.swap(0, Ordering::SeqCst);

    for index in 0..CALLS {
        let client = build_hardened_client().expect("hardened client");
        one_call(index, &origin, &identity, client).await;
    }
    let fresh_connections = accepted.load(Ordering::SeqCst);

    eprintln!(
        "DA repository connection reuse over {CALLS} calls: shared client = {shared_connections} accepted connection(s), fresh client per call = {fresh_connections} accepted connection(s)"
    );
    // One connection is the usual result. A second one is tolerated: when
    // the next request starts before the previous connection is back in
    // the pool, hyper opens another. A client per call opens one per call,
    // so with N >= 4 calls `<= 2` still fails for it.
    assert!(
        shared_connections <= 2,
        "repositories on one shared client must reuse a connection, saw {shared_connections}"
    );
    assert_eq!(
        fresh_connections, CALLS,
        "each fresh client per call opens its own connection"
    );
}

#[tokio::test]
async fn grant_repository_on_one_shared_client_reuses_one_tenant_connection() {
    let (origin, accepted) = spawn_keep_alive_server().await;
    let identity = IdentityProjection::new("user-1", "tenant-1", "Test User", None);

    let shared = build_hardened_client().expect("hardened client");
    for _ in 0..CALLS {
        let repository = DaMcpGrantRepository::for_test_with_client(
            &origin,
            "test-token",
            &identity,
            shared.clone(),
        )
        .expect("loopback grant repository");
        repository.signed_in_server_ids().await.expect("grant list");
    }
    let shared_connections = accepted.swap(0, Ordering::SeqCst);

    for _ in 0..CALLS {
        let client = build_hardened_client().expect("hardened client");
        let repository =
            DaMcpGrantRepository::for_test_with_client(&origin, "test-token", &identity, client)
                .expect("loopback grant repository");
        repository.signed_in_server_ids().await.expect("grant list");
    }
    let fresh_connections = accepted.load(Ordering::SeqCst);

    eprintln!(
        "DA grant repository connection reuse over {CALLS} calls: shared client = {shared_connections} accepted connection(s), fresh client per call = {fresh_connections} accepted connection(s)"
    );
    // One connection is the usual result. A second one is tolerated: when
    // the next request starts before the previous connection is back in
    // the pool, hyper opens another. A client per call opens one per call,
    // so with N >= 4 calls `<= 2` still fails for it.
    assert!(
        shared_connections <= 2,
        "grant repositories on one shared client must reuse a connection, saw {shared_connections}"
    );
    assert_eq!(
        fresh_connections, CALLS,
        "each fresh client per call opens its own connection"
    );
}
