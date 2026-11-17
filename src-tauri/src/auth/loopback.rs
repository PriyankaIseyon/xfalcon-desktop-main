use std::time::Duration;

use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    time,
};
use tokio_util::sync::CancellationToken;

use super::pkce::constant_time_eq;

pub(crate) const CALLBACK_PATH: &str = "/callback";
const MAX_REQUEST_BYTES: usize = 8 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    #[error("loopback listener failed")]
    BindFailed,
    #[error("sign-in was cancelled")]
    Cancelled,
    #[error("sign-in timed out")]
    TimedOut,
    #[error("authorization callback was invalid")]
    InvalidCallback,
    #[error("loopback callback failed")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopbackRedirectUri {
    pub value: String,
}

pub struct LoopbackWaiter {
    cancel: CancellationToken,
    result_rx: oneshot::Receiver<Result<AuthorizationCallback, LoopbackError>>,
}

/// Releases the port however the sign-in ends.
///
/// The listener lives in a spawned task, not in this struct, so dropping the
/// waiter used to leave the port bound for the life of the PROCESS. Only one
/// caller cancelled explicitly — the browser-would-not-open path — and every
/// other early return between binding and waiting leaked it.
///
/// That is not a rare corner. An MCP sign-in binds the port BEFORE it settles a
/// client identity, because a dynamic registration has to declare the exact
/// redirect URI this listener serves; an authorization server with dynamic
/// registration disabled then refuses, and the flow returns to ask the operator
/// for a client id. So the first sign-in against any such tenant leaked the
/// port, and every attempt after it failed to bind — reported as "this server
/// refused the sign-in", which named the wrong machine entirely. Cancelling here
/// covers every exit, including the ones no one has written yet.
impl Drop for LoopbackWaiter {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationCallback {
    pub code: String,
    pub state: String,
}

impl LoopbackWaiter {
    pub async fn bind_on_port(port: u16) -> Result<(Self, LoopbackRedirectUri), LoopbackError> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|_| LoopbackError::BindFailed)?;
        let port = listener
            .local_addr()
            .map_err(|_| LoopbackError::BindFailed)?
            .port();
        Self::listen(listener, port).await
    }

    async fn listen(
        listener: TcpListener,
        port: u16,
    ) -> Result<(Self, LoopbackRedirectUri), LoopbackError> {
        let redirect_uri = LoopbackRedirectUri {
            value: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
        };
        let cancel = CancellationToken::new();
        let (result_tx, result_rx) = oneshot::channel();
        let server_cancel = cancel.clone();
        tokio::spawn(async move {
            let outcome = accept_one_callback(listener, server_cancel).await;
            let _ = result_tx.send(outcome);
        });

        Ok((Self { cancel, result_rx }, redirect_uri))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn bind() -> Result<(Self, LoopbackRedirectUri), LoopbackError> {
        Self::bind_on_port(0).await
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub async fn wait_for_callback(
        mut self,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<AuthorizationCallback, LoopbackError> {
        let cancel = self.cancel.clone();
        // Borrowed rather than moved out: `LoopbackWaiter` owns a `Drop` that
        // releases the port, and a type with a destructor cannot be taken apart
        // field by field. `oneshot::Receiver` is `Unpin`, so `&mut` is a future
        // in its own right.
        let receiver = &mut self.result_rx;
        let wait = async move {
            match receiver.await {
                Ok(Ok(callback)) => validate_state(expected_state, callback),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(LoopbackError::InvalidCallback),
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => Err(LoopbackError::Cancelled),
            outcome = time::timeout(timeout, wait) => {
                match outcome {
                    Ok(result) => result,
                    Err(_) => {
                        cancel.cancel();
                        Err(LoopbackError::TimedOut)
                    }
                }
            }
        }
    }
}

async fn accept_one_callback(
    listener: TcpListener,
    cancel: CancellationToken,
) -> Result<AuthorizationCallback, LoopbackError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Err(LoopbackError::Cancelled),
            accepted = listener.accept() => {
                let (mut stream, peer) = accepted?;
                if !peer.ip().is_loopback() {
                    continue;
                }
                let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
                let read = stream.read(&mut buffer).await?;
                if read == 0 {
                    continue;
                }
                let request = String::from_utf8_lossy(&buffer[..read]);
                match parse_callback_request(&request) {
                    Ok(callback) => {
                        let _ = stream.write_all(&http_response("200 OK", SIGNED_IN_PAGE)).await;
                        return Ok(callback);
                    }
                    Err(_) => {
                        let _ = stream
                            .write_all(&http_response("400 Bad Request", SIGN_IN_FAILED_PAGE))
                            .await;
                    }
                }
            }
        }
    }
}

/// Builds the response with a `Content-Length` measured from the body.
///
/// It is computed rather than written by hand because a hand-written one was
/// wrong — it declared 24 bytes for a 23-byte body, so the browser sat waiting
/// for a byte that never arrived and rendered an empty tab.
fn http_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {length}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        length = body.len()
    )
    .into_bytes()
}

/// The page the browser lands on after a completed sign-in.
///
/// Entirely static: no part of the callback — not the code, the state, nor any
/// error text — is interpolated into it, so a crafted callback URL cannot place
/// content on a page rendered in the user's browser.
const SIGNED_IN_PAGE: &str = concat!(
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
    "<title>Signed in to xFalcon</title><style>",
    "body{margin:0;min-height:100vh;display:grid;place-items:center;",
    "background:#0b0d10;color:#e8eaed;",
    "font:16px/1.5 system-ui,-apple-system,\"Segoe UI\",sans-serif}",
    "main{text-align:center;padding:2rem}",
    "h1{font-size:1.25rem;font-weight:600;margin:0 0 .5rem}",
    "p{margin:0;color:#9aa3ad}",
    ".badge{width:56px;height:56px;margin:0 auto 1.25rem;border-radius:999px;",
    "display:grid;place-items:center;background:#123524;",
    "border:1px solid #1f6f43;animation:pop .35s cubic-bezier(.25,.75,.6,1.2)}",
    ".badge svg{stroke:#4ade80;stroke-width:2.5;fill:none;",
    "stroke-linecap:round;stroke-linejoin:round}",
    ".badge path{stroke-dasharray:24;stroke-dashoffset:24;",
    "animation:draw .45s .2s ease-out forwards}",
    "@keyframes pop{from{transform:scale(.6);opacity:0}to{transform:scale(1);opacity:1}}",
    "@keyframes draw{to{stroke-dashoffset:0}}",
    "@media (prefers-reduced-motion:reduce){.badge,.badge path{animation:none;stroke-dashoffset:0}}",
    "</style></head><body><main>",
    "<div class=\"badge\" role=\"img\" aria-label=\"Success\">",
    "<svg width=\"28\" height=\"28\" viewBox=\"0 0 24 24\"><path d=\"M4 12.5l5 5L20 6.5\"/></svg>",
    "</div>",
    "<h1>Signed in to xFalcon</h1>",
    "<p>You can safely close this tab and return to the desktop app.</p>",
    "</main></body></html>"
);

/// Shown when the request that reached the loopback listener was not a callback
/// this app can act on. Deliberately says nothing about *why*: the listener is
/// reachable by any local process, so its replies describe no internal state.
const SIGN_IN_FAILED_PAGE: &str = concat!(
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
    "<title>Sign-in did not complete</title><style>",
    "body{margin:0;min-height:100vh;display:grid;place-items:center;",
    "background:#0b0d10;color:#e8eaed;",
    "font:16px/1.5 system-ui,-apple-system,\"Segoe UI\",sans-serif}",
    "main{text-align:center;padding:2rem}",
    "h1{font-size:1.25rem;font-weight:600;margin:0 0 .5rem}",
    "p{margin:0;color:#9aa3ad}",
    "</style></head><body><main>",
    "<h1>Sign-in did not complete</h1>",
    "<p>Close this tab and start sign-in again from the desktop app.</p>",
    "</main></body></html>"
);

fn parse_callback_request(request: &str) -> Result<AuthorizationCallback, LoopbackError> {
    let request_line = request
        .lines()
        .next()
        .ok_or(LoopbackError::InvalidCallback)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(LoopbackError::InvalidCallback)?;
    let target = parts.next().ok_or(LoopbackError::InvalidCallback)?;
    if method != "GET" {
        return Err(LoopbackError::InvalidCallback);
    }
    let path_and_query = target.strip_prefix('/').unwrap_or(target);
    let url = Url::parse(&format!("http://127.0.0.1/{path_and_query}"))
        .map_err(|_| LoopbackError::InvalidCallback)?;
    if url.path() != CALLBACK_PATH {
        return Err(LoopbackError::InvalidCallback);
    }
    if url.query_pairs().any(|(key, _)| key == "error") {
        return Err(LoopbackError::InvalidCallback);
    }
    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())
        .ok_or(LoopbackError::InvalidCallback)?;
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or(LoopbackError::InvalidCallback)?;
    Ok(AuthorizationCallback { code, state })
}

fn validate_state(
    expected_state: &str,
    callback: AuthorizationCallback,
) -> Result<AuthorizationCallback, LoopbackError> {
    if !constant_time_eq(&callback.state, expected_state) {
        return Err(LoopbackError::InvalidCallback);
    }
    Ok(callback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_callback_extracts_code_and_state() {
        let request = "GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1:8765\r\n\r\n";
        let parsed = parse_callback_request(request).expect("callback");
        assert_eq!(parsed.code, "abc");
        assert_eq!(parsed.state, "xyz");
    }

    /// Regression: the listener lives in a spawned task, so dropping the waiter
    /// left the port bound for the life of the process. An MCP sign-in binds
    /// BEFORE it settles a client identity, and a tenant with dynamic client
    /// registration disabled makes the flow return right after that -- so the
    /// first sign-in leaked the port and every attempt after it failed to bind,
    /// which the surface reported as the SERVER refusing the sign-in.
    #[tokio::test]
    async fn dropping_the_waiter_releases_the_port_for_the_next_attempt() {
        let (waiter, first) = LoopbackWaiter::bind_on_port(0).await.expect("first bind");
        let port = Url::parse(&first.value).unwrap().port().unwrap();
        assert_ne!(port, 0);
        assert!(first.value.contains(&port.to_string()));
        // Exactly what an early return does: no explicit cancel, just a drop.
        drop(waiter);

        // The spawned accept loop wakes on the cancellation and releases the
        // listener; yielding lets it run before the port is claimed again.
        for _ in 0..64 {
            tokio::task::yield_now().await;
            if LoopbackWaiter::bind_on_port(port).await.is_ok() {
                return;
            }
        }
        panic!("the port was still bound after the waiter was dropped");
    }

    #[tokio::test]
    async fn a_requested_fixed_port_never_falls_back_when_occupied() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = occupied.local_addr().unwrap().port();
        assert!(matches!(
            LoopbackWaiter::bind_on_port(port).await,
            Err(LoopbackError::BindFailed)
        ));
    }

    #[tokio::test]
    async fn loopback_accepts_localhost_redirect() {
        let (waiter, redirect_uri) = LoopbackWaiter::bind().await.expect("bind");
        let state = "test-state-value";
        let client = reqwest::Client::new();
        let url = format!("{}?code=the-code&state={state}", redirect_uri.value);
        let fire = client.get(url);
        let wait = waiter.wait_for_callback(state, Duration::from_secs(5));
        let (_, callback) = tokio::join!(fire.send(), wait);
        let callback = callback.expect("callback");
        assert_eq!(callback.code, "the-code");
    }

    /// Regression: the hand-written response declared one more byte than the
    /// body held, so the browser waited for it and rendered an empty tab.
    #[test]
    fn every_response_declares_the_length_it_actually_sends() {
        for (status, body) in [
            ("200 OK", SIGNED_IN_PAGE),
            ("400 Bad Request", SIGN_IN_FAILED_PAGE),
        ] {
            let response = String::from_utf8(http_response(status, body)).expect("utf-8 response");
            let (head, sent) = response
                .split_once("\r\n\r\n")
                .expect("headers end with a blank line");
            let declared: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .expect("a declared length")
                .parse()
                .expect("a numeric length");

            assert_eq!(declared, sent.len(), "{status} would hang the browser");
            assert!(head.starts_with(&format!("HTTP/1.1 {status}\r\n")));
            assert!(head.contains("Content-Type: text/html; charset=utf-8"));
        }
    }

    /// The listener answers any local process, so its pages must never echo
    /// callback input back into rendered HTML.
    #[tokio::test]
    async fn the_landing_page_says_sign_in_worked_without_echoing_the_callback() {
        let (waiter, redirect_uri) = LoopbackWaiter::bind().await.expect("bind");
        let state = "test-state-value";
        let client = reqwest::Client::new();
        let url = format!(
            "{}?code=secret-authorization-code&state={state}",
            redirect_uri.value
        );
        let fire = client.get(url);
        let wait = waiter.wait_for_callback(state, Duration::from_secs(5));
        let (response, callback) = tokio::join!(fire.send(), wait);
        callback.expect("callback");

        let response = response.expect("the browser gets a response");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let page = response.text().await.expect("a page body");

        assert!(page.contains("Signed in to xFalcon"));
        assert!(page.contains("You can safely close this tab"));
        assert!(
            !page.contains("secret-authorization-code") && !page.contains(state),
            "the callback's own values must not reach the rendered page"
        );
    }

    #[tokio::test]
    async fn loopback_rejects_wrong_state() {
        let (waiter, redirect_uri) = LoopbackWaiter::bind().await.expect("bind");
        let client = reqwest::Client::new();
        let url = format!("{}?code=the-code&state=wrong", redirect_uri.value);
        let _ = client.get(url).send().await;
        let result = waiter
            .wait_for_callback("expected", Duration::from_secs(2))
            .await;
        assert!(matches!(result, Err(LoopbackError::InvalidCallback)));
    }
}
