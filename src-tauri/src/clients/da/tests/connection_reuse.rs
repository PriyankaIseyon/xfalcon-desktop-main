//! Sessions built on one shared client reuse one tenant connection, where a
//! client per session opens one each.
//!
//! The witness is the loopback server's own accept count, not a field of the
//! client: a pool that quietly stopped being shared would show up here as extra
//! accepted connections. The server speaks HTTP/1.1 keep-alive, so a connection
//! that was returned to the pool is the one the next request uses.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::super::chat_api::DaChatSession;
use super::super::mapping::build_hardened_client;

const SESSIONS: usize = 6;
const EMPTY_PAGE: &str = r#"{"items":[],"next_cursor":null}"#;

/// Starts a loopback server that accepts TCP connections and counts them. It
/// answers every request on a connection with an empty model page until the
/// client closes that connection.
pub(super) async fn spawn_keep_alive_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let origin = format!("http://{}", listener.local_addr().expect("local address"));
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(serve_keep_alive(stream));
        }
    });
    (origin, accepted)
}

async fn serve_keep_alive(mut stream: TcpStream) {
    let mut pending = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        // Requests here are bodiless GETs, so each one ends at its blank line.
        while !pending.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(read) => pending.extend_from_slice(&chunk[..read]),
            }
        }
        let end = pending
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("request head terminator")
            + 4;
        pending.drain(..end);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{EMPTY_PAGE}",
            EMPTY_PAGE.len()
        );
        if stream.write_all(response.as_bytes()).await.is_err() {
            return;
        }
    }
}

#[tokio::test]
async fn sessions_on_one_shared_client_reuse_one_tenant_connection() {
    let (origin, accepted) = spawn_keep_alive_server().await;

    let shared = build_hardened_client().expect("hardened client");
    for _ in 0..SESSIONS {
        let session = DaChatSession::for_test_with_client(&origin, "test-token", shared.clone())
            .expect("loopback session");
        session.list_models().await.expect("shared-client read");
    }
    let shared_connections = accepted.swap(0, Ordering::SeqCst);

    for _ in 0..SESSIONS {
        let client = build_hardened_client().expect("hardened client");
        let session = DaChatSession::for_test_with_client(&origin, "test-token", client)
            .expect("loopback session");
        session.list_models().await.expect("fresh-client read");
    }
    let fresh_connections = accepted.load(Ordering::SeqCst);

    eprintln!(
        "DA connection reuse over {SESSIONS} sessions: shared client = {shared_connections} accepted connection(s), fresh client per session = {fresh_connections} accepted connection(s)"
    );
    // One connection is the usual result. A second one is tolerated: when
    // the next request starts before the previous connection is back in
    // the pool, hyper opens another. A client per call opens one per call,
    // so with N >= 4 calls `<= 2` still fails for it.
    assert!(
        shared_connections <= 2,
        "sessions on one shared client must reuse a connection, saw {shared_connections}"
    );
    assert_eq!(
        fresh_connections, SESSIONS,
        "each fresh client opens its own connection"
    );
}
