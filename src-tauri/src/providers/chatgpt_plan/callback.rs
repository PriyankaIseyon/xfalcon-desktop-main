//! Receives the browser redirect of OpenAI's sign-in on a one-shot loopback listener.

use std::{fmt, io, time::Duration};

use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::auth::pkce::constant_time_eq;

const CALLBACK_PATH: &str = "/auth/callback";
const MAX_HEAD_BYTES: usize = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// A run of bad callbacks ends the attempt. One stray request cannot. Only a
/// callback that names this listener's path and host but carries the wrong state
/// counts; a request that fails the Host or path check does not.
const MAX_BAD_CALLBACKS: usize = 20;
/// A run of failed `accept` calls ends the attempt. One failed accept does not.
const MAX_ACCEPT_ERRORS: usize = 20;
/// The pause after an `accept` failure that is not a client going away.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);
const BAD_REQUEST: &str = "400 Bad Request";

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct CallbackParams {
    pub(crate) code: Option<String>,
    pub(crate) state: String,
    pub(crate) error: Option<String>,
    pub(crate) client_id: Option<String>,
}

impl fmt::Debug for CallbackParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackParams")
            .field("code", &self.code.as_ref().map(|_| "<redacted>"))
            .field("state", &"<redacted>")
            .field("error", &self.error)
            .field("client_id", &self.client_id)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CallbackError {
    #[error("could not open the local sign-in listener")]
    BindFailed,
    #[error("sign-in timed out")]
    TimedOut,
    #[error("sign-in was cancelled")]
    Cancelled,
    #[error("the sign-in response did not match this attempt")]
    StateMismatch,
    #[error("the sign-in listener failed")]
    Io,
}

/// Holds the port for one sign-in. Finishing `wait` drops the socket, so a
/// redirect that arrives after the sign-in ended finds nothing listening.
///
/// Deliberately not `Debug`: the expected state must never reach a log line.
pub(crate) struct CallbackListener {
    listener: TcpListener,
    port: u16,
    expected_state: String,
}

/// A request head the listener could read: its method, its target resolved
/// against the loopback origin, and every `Host` value it carried.
struct Request {
    method: String,
    url: Url,
    hosts: Vec<String>,
}

impl CallbackListener {
    pub(crate) async fn bind(expected_state: &str) -> Result<Self, CallbackError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| CallbackError::BindFailed)?;
        let port = listener
            .local_addr()
            .map_err(|_| CallbackError::BindFailed)?
            .port();
        Ok(Self {
            listener,
            port,
            expected_state: expected_state.to_owned(),
        })
    }

    pub(crate) fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}{CALLBACK_PATH}", self.port)
    }

    /// Cancellation is checked first, so a sign-in that was cancelled never
    /// reports a callback that happened to be ready at the same moment.
    pub(crate) async fn wait(
        self,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<CallbackParams, CallbackError> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(CallbackError::Cancelled),
            outcome = time::timeout(timeout, self.serve()) => match outcome {
                Ok(result) => result,
                Err(_) => Err(CallbackError::TimedOut),
            },
        }
    }

    /// Consuming `self` drops the socket on every exit, errors included.
    async fn serve(self) -> Result<CallbackParams, CallbackError> {
        let host = format!("127.0.0.1:{}", self.port);
        let mut rejected = 0;
        let mut accept_errors = 0;
        loop {
            let (mut stream, _) = match self.listener.accept().await {
                Ok(accepted) => {
                    accept_errors = 0;
                    accepted
                }
                Err(error) => {
                    accept_errors += 1;
                    if accept_errors >= MAX_ACCEPT_ERRORS {
                        return Err(CallbackError::Io);
                    }
                    if !accept_error_is_transient(error.kind()) {
                        time::sleep(ACCEPT_BACKOFF).await;
                    }
                    continue;
                }
            };
            // A request that cannot be read is dropped without counting. An idle
            // browser connection looks the same, and it is not a callback.
            let Some(request) = time::timeout(REQUEST_TIMEOUT, read_request(&mut stream))
                .await
                .ok()
                .flatten()
            else {
                respond(&mut stream, BAD_REQUEST, SIGN_IN_FAILED_PAGE).await;
                continue;
            };
            if !matches!(request.hosts.as_slice(), [value] if *value == host) {
                respond(&mut stream, BAD_REQUEST, SIGN_IN_FAILED_PAGE).await;
                continue;
            }
            if request.method != "GET" || request.url.path() != CALLBACK_PATH {
                respond(&mut stream, "404 Not Found", "").await;
                continue;
            }
            let Some(state) = query_value(&request.url, "state")
                .filter(|state| constant_time_eq(state, &self.expected_state))
            else {
                reject(&mut stream, &mut rejected).await?;
                continue;
            };
            let code = query_value(&request.url, "code");
            let error = query_value(&request.url, "error");
            if code.is_none() && error.is_none() {
                respond(&mut stream, BAD_REQUEST, SIGN_IN_FAILED_PAGE).await;
                continue;
            }
            respond(&mut stream, "200 OK", SIGNED_IN_PAGE).await;
            return Ok(CallbackParams {
                code,
                state,
                error,
                client_id: query_value(&request.url, "client_id"),
            });
        }
    }
}

/// Answers a callback whose state does not match this attempt, and keeps listening.
/// Only a run of them ends the attempt.
async fn reject(stream: &mut TcpStream, rejected: &mut usize) -> Result<(), CallbackError> {
    respond(stream, BAD_REQUEST, SIGN_IN_FAILED_PAGE).await;
    *rejected += 1;
    if *rejected < MAX_BAD_CALLBACKS {
        return Ok(());
    }
    Err(CallbackError::StateMismatch)
}

/// Whether an `accept` failure is only one client going away. macOS and the BSDs
/// report a client that reset before it was accepted as `ECONNABORTED`. That says
/// nothing about the listener, so the next accept is tried at once.
fn accept_error_is_transient(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionRefused
    )
}

/// Reads the request head through its blank line. `None` when the head is
/// incomplete, larger than `MAX_HEAD_BYTES`, or has no usable request line.
async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = vec![0u8; MAX_HEAD_BYTES];
    let mut filled = 0;
    loop {
        if filled == buffer.len() {
            return None;
        }
        let read = stream.read(&mut buffer[filled..]).await.ok()?;
        if read == 0 {
            return None;
        }
        filled += read;
        if head_ends(&buffer[..filled]) {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer[..filled]);
    let mut lines = head.lines();
    let mut parts = lines.next()?.split_ascii_whitespace();
    let method = parts.next()?.to_owned();
    let url = Url::parse("http://127.0.0.1/")
        .ok()?
        .join(parts.next()?)
        .ok()?;
    let hosts: Vec<String> = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host")
                .then(|| value.trim().to_owned())
        })
        .collect();
    Some(Request { method, url, hosts })
}

/// Clients may end lines with LF or CRLF, and a head may mix them, so the blank
/// line that ends it is either `\n\n` or `\n\r\n`.
fn head_ends(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|pair| pair == b"\n\n") || bytes.windows(3).any(|run| run == b"\n\r\n")
}

/// The first non-empty value of `key`, percent-decoded by the URL parser.
fn query_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// Best effort. A failed write means the browser has already left, and that
/// does not change whether a callback that was received counts.
async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {length}\r\n\
         Cache-Control: no-store\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        length = body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

/// Static on purpose: no part of the request reaches a page the browser renders.
const SIGNED_IN_PAGE: &str = concat!(
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
    "<title>Sign-in code received</title><style>",
    "body{margin:0;min-height:100vh;display:grid;place-items:center;",
    "font:16px/1.5 system-ui,-apple-system,\"Segoe UI\",sans-serif}",
    "main{text-align:center;padding:2rem}",
    "h1{font-size:1.25rem;font-weight:600;margin:0 0 .5rem}",
    "</style></head><body><main>",
    "<h1>Sign-in code received</h1>",
    "<p>Return to xFalcon to finish signing in.</p>",
    "</main></body></html>"
);

const SIGN_IN_FAILED_PAGE: &str = concat!(
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
    "<title>Sign-in did not complete</title><style>",
    "body{margin:0;min-height:100vh;display:grid;place-items:center;",
    "font:16px/1.5 system-ui,-apple-system,\"Segoe UI\",sans-serif}",
    "main{text-align:center;padding:2rem}",
    "h1{font-size:1.25rem;font-weight:600;margin:0 0 .5rem}",
    "</style></head><body><main>",
    "<h1>Sign-in did not complete</h1>",
    "<p>Close this tab and start sign-in again from xFalcon.</p>",
    "</main></body></html>"
);

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::*;

    const STATE: &str = "expected-state";
    const WAIT: Duration = Duration::from_secs(5);

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client")
    }

    fn header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    /// Sends raw request bytes and returns the whole reply. The listener closes
    /// after every reply, so reading to the end is complete.
    async fn exchange(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("read");
        response
    }

    /// Once `wait` has returned the socket is gone, so nothing accepts here.
    async fn assert_refused(redirect_uri: &str) {
        let error = client()
            .get(redirect_uri)
            .send()
            .await
            .expect_err("the listener is closed");
        assert!(error.is_connect(), "{error}");
    }

    #[tokio::test]
    async fn redirect_uri_is_the_loopback_callback_on_the_bound_port() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        assert_ne!(listener.port, 0);
        assert_eq!(
            listener.redirect_uri(),
            format!("http://127.0.0.1:{}/auth/callback", listener.port)
        );
    }

    #[tokio::test]
    async fn a_matching_callback_returns_its_parameters_and_a_static_page() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let url = format!(
            "{}?code=the-code&state={STATE}&client_id=app-1",
            listener.redirect_uri()
        );
        let cancel = CancellationToken::new();
        let fire = client().get(url).send();
        let (response, result) = tokio::join!(fire, listener.wait(&cancel, WAIT));

        let response = response.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header(&response, "content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(header(&response, "cache-control"), Some("no-store"));
        assert_eq!(
            header(&response, "content-security-policy"),
            Some("default-src 'none'; style-src 'unsafe-inline'")
        );
        let page = response.text().await.expect("page");
        assert!(page.contains("Sign-in code received"));
        assert!(page.contains("Return to xFalcon to finish signing in."));
        assert!(!page.contains("Sign-in finished"));
        assert!(!page.contains("the-code") && !page.contains(STATE));

        assert_eq!(
            result,
            Ok(CallbackParams {
                code: Some("the-code".to_owned()),
                state: STATE.to_owned(),
                error: None,
                client_id: Some("app-1".to_owned()),
            })
        );
    }

    #[tokio::test]
    async fn query_values_are_percent_decoded() {
        let state = "ex/pected+1";
        let listener = CallbackListener::bind(state).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let fire = client()
            .get(format!(
                "{uri}?code=a%2Bb%20c&state=ex%2Fpected%2B1&client_id=app%26x"
            ))
            .send();
        let (response, result) = tokio::join!(fire, listener.wait(&cancel, WAIT));

        assert_eq!(response.expect("response").status(), StatusCode::OK);
        let params = result.expect("callback");
        assert_eq!(params.code.as_deref(), Some("a+b c"));
        assert_eq!(params.state, state);
        assert_eq!(params.client_id.as_deref(), Some("app&x"));
    }

    #[tokio::test]
    async fn a_wrong_state_answers_400_and_keeps_listening() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            let wrong = client
                .get(format!("{uri}?code=the-code&state=wrong-state"))
                .send()
                .await
                .expect("response");
            assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    #[tokio::test]
    async fn a_missing_state_answers_400_and_keeps_listening() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            let missing = client
                .get(format!("{uri}?code=the-code"))
                .send()
                .await
                .expect("response");
            assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    #[tokio::test]
    async fn a_callback_without_code_or_error_answers_400_and_keeps_listening() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            let empty = client
                .get(format!("{uri}?state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    #[tokio::test]
    async fn a_host_other_than_the_bound_address_answers_400_and_keeps_listening() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let port = listener.port;
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            let foreign = format!(
                "GET /auth/callback?code=the-code&state={STATE} HTTP/1.1\r\nHost: evil.example\r\n\r\n"
            );
            let absent = format!("GET /auth/callback?code=the-code&state={STATE} HTTP/1.1\r\n\r\n");
            for request in [foreign, absent] {
                let response = exchange(port, &request).await;
                assert!(
                    response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
                    "{response}"
                );
            }

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    #[tokio::test]
    async fn other_paths_get_404_and_the_real_callback_still_succeeds() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let port = listener.port;
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            let other = client
                .get(format!("http://127.0.0.1:{port}/other"))
                .send()
                .await
                .expect("response");
            assert_eq!(other.status(), StatusCode::NOT_FOUND);

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    #[tokio::test]
    async fn a_post_gets_404_and_the_real_callback_still_succeeds() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            let post = client
                .post(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(post.status(), StatusCode::NOT_FOUND);

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    /// Each bad callback gets its own 400. Nineteen of them leave the attempt open.
    #[tokio::test]
    async fn nineteen_bad_callbacks_keep_the_sign_in_open() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            for _ in 0..MAX_BAD_CALLBACKS - 1 {
                let bad = client
                    .get(format!("{uri}?code=the-code&state=wrong-state"))
                    .send()
                    .await
                    .expect("response");
                assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
            }

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    /// The twentieth bad callback still gets its 400, then ends the attempt.
    #[tokio::test]
    async fn twenty_bad_callbacks_end_the_sign_in_with_state_mismatch() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            for _ in 0..MAX_BAD_CALLBACKS {
                let bad = client
                    .get(format!("{uri}?code=the-code&state=wrong-state"))
                    .send()
                    .await
                    .expect("response");
                assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
            }
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result, Err(CallbackError::StateMismatch));
        assert_refused(&uri).await;
    }

    #[tokio::test]
    async fn access_denied_is_returned_as_data_once_the_state_matches() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let fire = client()
            .get(format!("{uri}?error=access_denied&state={STATE}"))
            .send();
        let (response, result) = tokio::join!(fire, listener.wait(&cancel, WAIT));

        assert_eq!(response.expect("response").status(), StatusCode::OK);
        let params = result.expect("callback");
        assert_eq!(params.error.as_deref(), Some("access_denied"));
        assert_eq!(params.code, None);
    }

    #[tokio::test]
    async fn lf_only_request_lines_are_accepted() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let port = listener.port;
        let cancel = CancellationToken::new();
        let client_side = async {
            let request = format!(
                "GET /auth/callback?code=lf-code&state={STATE} HTTP/1.1\nHost: 127.0.0.1:{port}\n\n"
            );
            let response = exchange(port, &request).await;
            assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("lf-code"));
    }

    /// The stray request names the callback, the right state and the right Host,
    /// so if the size cap were not enforced it would finish the sign-in with
    /// `stray-code`. Its reply is not asserted: the listener closes with unread
    /// bytes, and Windows can reset the connection before the reply is read.
    #[tokio::test]
    async fn an_oversized_request_line_is_rejected_without_panic() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let port = listener.port;
        let cancel = CancellationToken::new();
        let stray = format!(
            "GET /auth/callback?code=stray-code&state={STATE}&pad={} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
            "a".repeat(MAX_HEAD_BYTES)
        );
        let client = client();
        let client_side = async {
            let mut stream = TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect");
            let _ = stream.write_all(stray.as_bytes()).await;
            let mut response = Vec::new();
            let _ = stream.read_to_end(&mut response).await;

            let real = client
                .get(format!("{uri}?code=real-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("real-code"));
    }

    /// Requests that fail the Host or path checks are answered and do not count, so
    /// any number of them leaves the sign-in open.
    #[tokio::test]
    async fn foreign_hosts_and_other_paths_never_end_the_sign_in() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let port = listener.port;
        let cancel = CancellationToken::new();
        let client = client();
        let client_side = async {
            for _ in 0..MAX_BAD_CALLBACKS + 5 {
                let foreign = format!(
                    "GET /auth/callback?code=the-code&state={STATE} HTTP/1.1\r\nHost: evil.example\r\n\r\n"
                );
                let response = exchange(port, &foreign).await;
                assert!(
                    response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
                    "{response}"
                );

                let other = format!(
                    "GET /other?code=the-code&state={STATE} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
                );
                let response = exchange(port, &other).await;
                assert!(
                    response.starts_with("HTTP/1.1 404 Not Found\r\n"),
                    "{response}"
                );
            }

            let real = client
                .get(format!("{uri}?code=the-code&state={STATE}"))
                .send()
                .await
                .expect("response");
            assert_eq!(real.status(), StatusCode::OK);
        };
        let (_, result) = tokio::join!(client_side, listener.wait(&cancel, WAIT));

        assert_eq!(result.expect("callback").code.as_deref(), Some("the-code"));
    }

    #[test]
    fn a_client_that_went_away_before_accept_is_not_a_listener_failure() {
        for kind in [
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionRefused,
        ] {
            assert!(accept_error_is_transient(kind), "{kind:?}");
        }
        for kind in [io::ErrorKind::Other, io::ErrorKind::PermissionDenied] {
            assert!(!accept_error_is_transient(kind), "{kind:?}");
        }
    }

    #[test]
    fn callback_params_debug_shows_no_code_or_state() {
        let params = CallbackParams {
            code: Some("the-code".to_owned()),
            state: STATE.to_owned(),
            error: None,
            client_id: Some("app-1".to_owned()),
        };
        let debug = format!("{params:?}");

        assert!(!debug.contains("the-code"), "{debug}");
        assert!(!debug.contains(STATE), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[tokio::test]
    async fn cancelling_returns_cancelled_and_releases_the_port() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        let cancel_soon = async move {
            time::sleep(Duration::from_millis(50)).await;
            canceller.cancel();
        };
        let (_, result) = tokio::join!(cancel_soon, listener.wait(&cancel, WAIT));

        assert_eq!(result, Err(CallbackError::Cancelled));
        assert_refused(&uri).await;
    }

    #[tokio::test]
    async fn a_timeout_returns_timed_out_and_releases_the_port() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let result = listener
            .wait(&CancellationToken::new(), Duration::from_millis(100))
            .await;

        assert_eq!(result, Err(CallbackError::TimedOut));
        assert_refused(&uri).await;
    }

    #[tokio::test]
    async fn a_second_connection_after_success_is_refused() {
        let listener = CallbackListener::bind(STATE).await.expect("bind");
        let uri = listener.redirect_uri();
        let cancel = CancellationToken::new();
        let fire = client()
            .get(format!("{uri}?code=the-code&state={STATE}"))
            .send();
        let (response, result) = tokio::join!(fire, listener.wait(&cancel, WAIT));

        assert_eq!(response.expect("response").status(), StatusCode::OK);
        result.expect("callback");
        assert_refused(&uri).await;
    }
}
