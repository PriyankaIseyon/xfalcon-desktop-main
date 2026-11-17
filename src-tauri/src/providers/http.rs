//! Shared hardened streaming HTTP plumbing for provider adapters.

use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::HeaderValue;
use reqwest::{redirect::Policy, Client};
use tokio_util::sync::CancellationToken;

use super::adapter::ProviderError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest the transport waits for the NEXT byte of a streaming body.
///
/// An idle bound, not a total one. `Client::timeout` covers the whole request
/// INCLUDING the body read, so the 180 s total this replaced killed any turn
/// that legitimately streamed for more than three minutes — mid-answer, with
/// the partial text discarded — while the run it belonged to was budgeted for
/// ten (`chat::service`'s `RunBudgets::wall_clock`). A reasoning model working
/// through a hard question with tools reaches three minutes.
///
/// Generous on purpose: time-to-first-token on a large thinking request is the
/// longest legitimate silence, and every gateway this module speaks to fills it
/// with keep-alive framing (OpenRouter's processing comments, Anthropic's
/// `ping`) which counts as bytes here.
const READ_TIMEOUT: Duration = Duration::from_secs(90);
/// Ceiling on one streamed turn, set to the run's own wall clock rather than
/// under it.
///
/// It exists for the one case the idle bound cannot catch — a body that trickles
/// forever — and must never be the thing that ends a turn the run still had time
/// for. The run deadline is checked at round boundaries, so this is what bounds
/// a single request between them. Read from the run budget itself: a copy was
/// left at ten minutes when the budget grew to sixty, and cut off long answers
/// from slow models.
pub(crate) const TOTAL_TIMEOUT: Duration = crate::chat::RUN_WALL_CLOCK;
/// Longest single SSE line the transport accepts.
///
/// Public because it is the size contract for everything downstream: text the
/// transport delivered is, by construction, within this bound, so a stage that
/// rejects something smaller rejects a payload this layer already called
/// acceptable. See `crate::agent::stream::MAX_DELTA_BYTES`.
/// A 1 MiB native compaction block can occupy almost 6 MiB on an SSE line when
/// every JSON string byte needs `\u00XX` escaping. Keep the per-line bound high
/// enough for that valid wire value while the independent 16 MiB stream bound
/// still caps the full response.
pub(crate) const MAX_SSE_LINE_BYTES: usize = (6 * 1024 * 1024) + (16 * 1024);
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
/// The default bound on one streamed response. A caller whose server wraps
/// every token in a large envelope passes a bigger one to
/// [`stream_sse_lines_limited`].
pub(crate) const DEFAULT_STREAM_BYTES: usize = MAX_TOTAL_BYTES;

pub(crate) fn build_streaming_client() -> Result<Client, ProviderError> {
    Client::builder()
        .use_rustls_tls()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(|error| {
            tracing::warn!(error = %error, "the streaming HTTP client could not be built");
            #[cfg(test)]
            eprintln!("streaming client build failed: {error:?}");
            ProviderError::Unavailable
        })
}

/// The process-wide client the fixed-endpoint providers send every request on.
///
/// A `reqwest::Client` owns its connection pool, so a client built per request
/// paid a fresh TCP and TLS handshake each time. Clones share one pool. Holds
/// no credential: each request sets its own. The build runs under a lock, so
/// concurrent first callers take turns and the client is built once.
pub(crate) fn shared_streaming_client() -> Result<Client, ProviderError> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    static BUILD: Mutex<()> = Mutex::new(());
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let _building = BUILD.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = build_streaming_client()?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// Marks a credential header value non-sensitive-logging-safe (never captured by
/// reqwest/HTTP tracing layers that redact `HeaderValue::is_sensitive()` headers).
pub(crate) fn sensitive_header(value: &str) -> Result<HeaderValue, ProviderError> {
    let mut header =
        HeaderValue::from_str(value).map_err(|_| ProviderError::AuthenticationFailed)?;
    header.set_sensitive(true);
    Ok(header)
}

/// Buffers a chunked SSE body and calls `on_data` once per `data: ...` line, stripping
/// the SSE-standard `data: ` prefix. Blank lines, `event:`/`id:`/`retry:` framing lines,
/// and comment lines are ignored — every vendor this module supports encodes the full
/// event discriminant inside the JSON payload, not the SSE `event:` field.
pub(crate) async fn stream_sse_lines(
    response: reqwest::Response,
    cancel: &CancellationToken,
    on_data: impl FnMut(&str) + Send,
) -> Result<(), ProviderError> {
    stream_sse_lines_limited(response, cancel, DEFAULT_STREAM_BYTES, on_data).await
}

/// [`stream_sse_lines`] with its own bound on the whole response.
pub(crate) async fn stream_sse_lines_limited(
    response: reqwest::Response,
    cancel: &CancellationToken,
    byte_limit: usize,
    mut on_data: impl FnMut(&str) + Send,
) -> Result<(), ProviderError> {
    let mut reader = SseLineReader::new(byte_limit);
    let mut body = response.bytes_stream();

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            chunk = body.next() => {
                let Some(chunk) = chunk else { break };
                let chunk = chunk.map_err(|error| classify_body_error(&error))?;
                reader.push(&chunk, &mut on_data)?;
            }
        }
    }

    reader.finish(&mut on_data)
}

/// Reads a whole response body, up to the same cap as a stream.
pub(crate) async fn read_body_capped(
    response: reqwest::Response,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut body = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            chunk = body.next() => {
                let Some(chunk) = chunk else { return Ok(bytes) };
                let chunk = chunk.map_err(|error| classify_body_error(&error))?;
                if bytes.len().saturating_add(chunk.len()) > MAX_TOTAL_BYTES {
                    return Err(ProviderError::InvalidResponse);
                }
                bytes.extend_from_slice(&chunk);
            }
        }
    }
}

/// Accumulates a chunked SSE body and yields one `data:` payload per line.
///
/// A struct rather than two locals inside the read loop, because the property
/// that matters is about the boundary BETWEEN chunks and a test can only choose
/// that boundary if it can hand the decoder its own slices. The network chooses
/// it otherwise, which is exactly why the bug it now pins went unnoticed.
struct SseLineReader {
    /// BYTES, decoded per line rather than per chunk. A chunk boundary falls
    /// wherever the network put it, so decoding each chunk on its own replaced
    /// any multi-byte character straddling one with U+FFFD — a streamed answer
    /// containing CJK or an emoji arrived visibly corrupted, at a rate set by
    /// how the packets happened to split. A newline can never fall inside a
    /// UTF-8 sequence, so a line IS a safe decode boundary.
    buffer: Vec<u8>,
    total: usize,
    limit: usize,
}

impl SseLineReader {
    fn new(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            total: 0,
            limit,
        }
    }

    fn push(&mut self, chunk: &[u8], on_data: &mut impl FnMut(&str)) -> Result<(), ProviderError> {
        self.total = self.total.saturating_add(chunk.len());
        if self.total > self.limit {
            return Err(ProviderError::InvalidResponse);
        }
        self.buffer.extend_from_slice(chunk);

        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            Self::deliver(&line, on_data)?;
        }
        Ok(())
    }

    /// Whatever arrived without a closing newline.
    fn finish(mut self, on_data: &mut impl FnMut(&str)) -> Result<(), ProviderError> {
        let line = std::mem::take(&mut self.buffer);
        Self::deliver(line.as_slice(), on_data)
    }

    fn deliver(line: &[u8], on_data: &mut impl FnMut(&str)) -> Result<(), ProviderError> {
        if line.len() > MAX_SSE_LINE_BYTES {
            return Err(ProviderError::InvalidResponse);
        }
        let line = String::from_utf8_lossy(line);
        // Blank lines, `event:`/`id:`/`retry:` framing lines and comments are
        // ignored: every vendor this module supports encodes the full event
        // discriminant inside the JSON payload, not in the SSE `event:` field.
        if let Some(data) = line.trim_end().strip_prefix("data:") {
            on_data(data.trim_start());
        }
        Ok(())
    }
}

/// How much of an error body is read to tell a context overflow from any other
/// rejection. Past it the rest is dropped: the first lines carry the message.
const MAX_ERROR_BODY_READ_BYTES: usize = 32 * 1024;

/// Phrases providers use to say a request does not fit the model's context
/// window, lower-cased.
///
/// Each provider words it its own way, and most answer with a plain 400 that the
/// status table alone reads as "bad request". Matching the text is the only way
/// to tell "your prompt is too long" (which compaction fixes) from "your
/// request is malformed" (which nothing fixes). The list is made of phrases, not
/// a bare "tokens", which would also match an output-limit complaint. A few are
/// short on purpose ("context length", "context window"); a rejected
/// `max_tokens` can match them too, which costs at most the bounded compaction
/// retries before the real error is shown.
const CONTEXT_OVERFLOW_MARKERS: [&str; 12] = [
    // OpenAI, OpenRouter, and most compatible servers. The two shortest are what
    // the adapters matched before this list existed; they stay so that nothing
    // that used to be recognised stops being.
    "context length",
    "maximum context",
    "context_length_exceeded",
    "exceed context limit",
    "reduce the length of the messages",
    // Anthropic.
    "prompt is too long",
    "input is too long",
    // xAI and other hosted models.
    "maximum prompt length",
    // Gemini.
    "exceeds the maximum number of tokens",
    // Generic.
    "too many tokens",
    "context window",
    "input too long",
];

/// Whether an error that carries `status` can be a prompt that did not fit.
///
/// Only the statuses a provider uses for a rejected request can. A throttle,
/// an outage or a gateway timeout whose message happens to mention "too many
/// tokens" or a "context window" is not an overflow, and treating it as one
/// would shrink the run's input room and skip the retry it deserves. A failure
/// with no numeric status (`"code": "context_length_exceeded"`) is judged by
/// its text alone.
#[must_use]
pub(crate) const fn can_be_context_overflow(status: Option<u16>) -> bool {
    match status {
        Some(status) => matches!(status, 400 | 413 | 422),
        None => true,
    }
}

/// Whether `lowercase_text` (an error body or message, already lower-cased)
/// says the prompt did not fit the context window.
#[must_use]
pub(crate) fn is_context_overflow_text(lowercase_text: &str) -> bool {
    CONTEXT_OVERFLOW_MARKERS
        .iter()
        .any(|marker| lowercase_text.contains(marker))
}

/// Classifies a non-success response, carrying `Retry-After` off it before the
/// body is dropped, and reading the body for the one thing the status cannot
/// say: that the prompt was too long.
pub(crate) async fn classify_response(response: reqwest::Response) -> ProviderError {
    let status = response.status().as_u16();
    tracing::warn!(
        status,
        host = response.url().host_str().unwrap_or("unknown"),
        "provider request rejected"
    );
    let retry_after = retry_after(&response);
    // Only these statuses can carry an overflow, and reading a body costs a
    // round trip's worth of waiting on every other rejection.
    if !matches!(status, 400 | 413 | 422) {
        return classify_status_code(status, retry_after);
    }
    let mut bytes = Vec::new();
    let mut body = response.bytes_stream();
    while let Some(Ok(chunk)) = body.next().await {
        let room = MAX_ERROR_BODY_READ_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if bytes.len() >= MAX_ERROR_BODY_READ_BYTES {
            break;
        }
    }
    if is_context_overflow_text(&String::from_utf8_lossy(&bytes).to_ascii_lowercase()) {
        return ProviderError::ContextOverflow;
    }
    classify_status_code(status, retry_after)
}

/// The status-code half of [`classify_response`], for the one caller that has a
/// status without a response to read it from: a gateway reporting a mid-stream
/// failure inside a 200 body (see [`super::openai_compat`]). Sharing the table
/// is the point — two copies would eventually disagree about what a 429 means.
pub(crate) const fn classify_status_code(
    status: u16,
    retry_after: Option<Duration>,
) -> ProviderError {
    match status {
        401 | 403 => ProviderError::AuthenticationFailed,
        // Deterministic, so they must not fall through to `Unavailable` — which
        // is retried three times before reporting a cause that was never the
        // real one.
        402 => ProviderError::PaymentRequired,
        413 => ProviderError::PayloadTooLarge,
        429 => ProviderError::RateLimited { retry_after },
        400 | 404 | 422 => ProviderError::InvalidResponse,
        _ => ProviderError::Unavailable,
    }
}

/// Markers rustls renders into a certificate verification failure.
///
/// Narrow on purpose: anything unmatched falls back to [`ProviderError::Unavailable`],
/// which is what every transport error produced before, so a miss costs the
/// better diagnostic and nothing else.
const CERTIFICATE_FAILURE_MARKERS: [&str; 4] = [
    "invalid peer certificate",
    "InvalidCertificate",
    "UnknownIssuer",
    "certificate verify failed",
];

/// Classifies a transport failure that never reached a status line.
///
/// Reads the verdict out of the error chain's rendered text because there is
/// nothing to downcast to: reqwest does not re-export its TLS backend's error
/// type, so rustls' own message is the only signal that crosses the boundary.
pub(crate) fn classify_transport_error(error: &reqwest::Error) -> ProviderError {
    // A connect timeout never reached the provider, so it is an outage, not a
    // slow model.
    if error.is_timeout() && !error.is_connect() {
        return ProviderError::TimedOut;
    }
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(link) = current {
        if is_certificate_failure(&link.to_string()) {
            return ProviderError::TlsVerificationFailed;
        }
        current = link.source();
    }
    #[cfg(test)]
    eprintln!("transport error: {error:?}");
    ProviderError::Unavailable
}

/// Classifies a failure while reading a response body that already started.
///
/// A body has no certificate step left to fail, so only the timeout is told
/// apart: an idle model and a dropped connection need different remedies.
pub(crate) fn classify_body_error(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::TimedOut
    } else {
        #[cfg(test)]
        eprintln!("body error: {error:?}");
        ProviderError::Unavailable
    }
}

/// Whether one rendered link of an error chain names a certificate failure.
///
/// Split out because it is the only fragile part of the classification and the
/// only part that can be tested without a server holding a certificate this
/// machine distrusts.
fn is_certificate_failure(rendered: &str) -> bool {
    CERTIFICATE_FAILURE_MARKERS
        .iter()
        .any(|marker| rendered.contains(marker))
}

/// The `Retry-After` delay, when the provider sent one as delay-seconds.
///
/// RFC 9110 also permits an HTTP-date, which is deliberately not parsed: every
/// provider this crate speaks to sends delay-seconds, and an unrecognised value
/// simply falls back to [`super::retry`]'s own backoff — which is the same
/// behaviour as no header at all, and cheaper than carrying a date parser for a
/// spelling nobody uses.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A client like [`build_streaming_client`] with bounds a test can outwait.
    fn short_client(read: Duration) -> Client {
        Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_millis(500))
            .read_timeout(read)
            .build()
            .expect("test client")
    }

    /// A model that is still reading its prompt sends nothing. That must reach
    /// the reader as a timeout, not as an outage or an unreachable host.
    #[tokio::test]
    async fn a_silent_provider_before_the_status_line_is_a_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&server)
            .await;
        let error = short_client(Duration::from_millis(50))
            .post(server.uri())
            .send()
            .await
            .expect_err("the read bound fires first");
        assert_eq!(classify_transport_error(&error), ProviderError::TimedOut);
    }

    #[tokio::test]
    async fn a_refused_connection_is_unavailable_not_a_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("free port");
        let address = listener.local_addr().expect("address");
        drop(listener);
        // The read bound also runs while connecting, and Windows takes longer
        // than a short one to report a refusal, which would read as a timeout.
        let error = short_client(Duration::from_secs(10))
            .post(format!("http://{address}/"))
            .send()
            .await
            .expect_err("nothing listens");
        assert_eq!(classify_transport_error(&error), ProviderError::Unavailable);
    }

    /// The headers arrive, then the model goes quiet mid-answer. The stream
    /// reader is the consumer that sees it.
    #[tokio::test]
    async fn a_body_that_goes_silent_is_a_timeout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("free port");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                      transfer-encoding: chunked\r\n\r\n\
                      10\r\ndata: {\"a\":1}\n\n\n\r\n",
                )
                .await
                .expect("write head");
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let response = short_client(Duration::from_millis(200))
            .post(format!("http://{address}/"))
            .send()
            .await
            .expect("headers arrive");
        let mut seen = Vec::new();
        let outcome = stream_sse_lines(response, &CancellationToken::new(), |data| {
            seen.push(data.to_owned());
        })
        .await;
        server.abort();
        assert_eq!(seen, vec![r#"{"a":1}"#.to_owned()]);
        assert_eq!(outcome, Err(ProviderError::TimedOut));
    }

    /// Feeds the decoder chunks whose boundaries the TEST chooses, and returns
    /// every payload it delivered.
    fn read_chunks(chunks: &[&[u8]]) -> Result<Vec<String>, ProviderError> {
        let mut reader = SseLineReader::new(DEFAULT_STREAM_BYTES);
        let mut seen: Vec<String> = Vec::new();
        let mut collect = |data: &str| seen.push(data.to_owned());
        for chunk in chunks {
            reader.push(chunk, &mut collect)?;
        }
        reader.finish(&mut collect)?;
        Ok(seen)
    }

    /// The corruption this decoder exists to prevent: a chunk boundary inside a
    /// multi-byte character. Decoding each chunk on its own turned the halves
    /// into U+FFFD, so an answer containing CJK or an emoji arrived visibly
    /// broken at a rate the network chose.
    #[test]
    fn a_character_split_across_two_chunks_survives_whole() {
        let line = "data: {\"text\":\"净销售额 📊\"}\n".as_bytes();
        for split in 1..line.len() {
            let seen = read_chunks(&[&line[..split], &line[split..]]).expect("decodes");
            assert_eq!(
                seen,
                vec!["{\"text\":\"净销售额 📊\"}".to_owned()],
                "boundary at byte {split} corrupted the payload"
            );
        }
    }

    #[test]
    fn a_line_split_across_many_chunks_is_delivered_once() {
        let seen = read_chunks(&[b"da", b"ta: {\"a\"", b":1}", b"\n\ndata: {\"a\":2}\n"])
            .expect("decodes");
        assert_eq!(seen, vec!["{\"a\":1}".to_owned(), "{\"a\":2}".to_owned()]);
    }

    #[test]
    fn a_payload_that_never_closes_its_line_is_still_delivered() {
        // A body that ends without a trailing newline: the last event is real
        // and dropping it would strand whatever it carried.
        let seen = read_chunks(&[b"data: [DONE]"]).expect("decodes");
        assert_eq!(seen, vec!["[DONE]".to_owned()]);
    }

    #[test]
    fn a_line_past_the_transport_bound_is_refused() {
        let mut line = b"data: ".to_vec();
        line.extend(std::iter::repeat_n(b'x', MAX_SSE_LINE_BYTES + 1));
        line.push(b'\n');
        assert_eq!(read_chunks(&[&line]), Err(ProviderError::InvalidResponse));
    }

    #[test]
    fn framing_lines_and_comments_deliver_nothing() {
        let seen = read_chunks(&[b": OPENROUTER PROCESSING\n\nevent: ping\nid: 7\n\ndata: {}\n"])
            .expect("decodes");
        assert_eq!(seen, vec!["{}".to_owned()]);
    }

    #[tokio::test]
    async fn stream_sse_lines_splits_multiple_data_lines_across_chunks() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"a\":1}\n\ndata: {\"a\":2}\n\n",
                "text/event-stream",
            ))
            .mount(&server)
            .await;

        let client = build_streaming_client().expect("client builds");
        let response = client
            .get(server.uri())
            .send()
            .await
            .expect("request sends");

        let seen = Arc::new(Mutex::new(Vec::new()));
        let collected = seen.clone();
        stream_sse_lines(response, &CancellationToken::new(), move |data| {
            collected.lock().expect("lock").push(data.to_owned());
        })
        .await
        .expect("stream completes");

        assert_eq!(*seen.lock().expect("lock"), vec!["{\"a\":1}", "{\"a\":2}"]);
    }

    #[tokio::test]
    async fn stream_sse_lines_stops_immediately_on_cancellation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("data: {}\n\n", "text/event-stream"),
            )
            .mount(&server)
            .await;

        let client = build_streaming_client().expect("client builds");
        let response = client
            .get(server.uri())
            .send()
            .await
            .expect("request sends");

        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = stream_sse_lines(response, &cancel, |_| {}).await;
        assert_eq!(result, Err(ProviderError::Cancelled));
    }

    /// Drives the classifier through a real response so the `Retry-After` read
    /// is exercised on the same path production takes, rather than against a
    /// hand-built `HeaderMap` that could drift from what reqwest produces.
    async fn classify(status: u16, retry_after: Option<&str>) -> ProviderError {
        let server = MockServer::start().await;
        let mut template = ResponseTemplate::new(status);
        if let Some(value) = retry_after {
            template = template.insert_header("retry-after", value);
        }
        Mock::given(method("GET"))
            .respond_with(template)
            .mount(&server)
            .await;
        let response = build_streaming_client()
            .expect("client builds")
            .get(server.uri())
            .send()
            .await
            .expect("request sends");
        classify_response(response).await
    }

    #[tokio::test]
    async fn classify_response_maps_known_codes() {
        assert_eq!(
            classify(401, None).await,
            ProviderError::AuthenticationFailed
        );
        assert_eq!(
            classify(403, None).await,
            ProviderError::AuthenticationFailed
        );
        assert_eq!(
            classify(429, None).await,
            ProviderError::RateLimited { retry_after: None }
        );
        assert_eq!(classify(400, None).await, ProviderError::InvalidResponse);
        assert_eq!(classify(500, None).await, ProviderError::Unavailable);
        assert_eq!(classify(503, None).await, ProviderError::Unavailable);
    }

    #[tokio::test]
    async fn a_delay_seconds_retry_after_is_carried_off_the_response() {
        assert_eq!(
            classify(429, Some("7")).await,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            }
        );
    }

    #[tokio::test]
    async fn an_http_date_retry_after_falls_back_to_backoff_rather_than_failing() {
        // Unparsed by design; `None` means "use our own backoff", which is the
        // same thing the absent-header case means.
        assert_eq!(
            classify(429, Some("Wed, 21 Oct 2026 07:28:00 GMT")).await,
            ProviderError::RateLimited { retry_after: None }
        );
    }

    #[tokio::test]
    async fn retry_after_is_only_read_off_the_status_that_defines_it() {
        assert_eq!(classify(500, Some("7")).await, ProviderError::Unavailable);
    }

    #[tokio::test]
    async fn deterministic_statuses_do_not_fall_through_to_unavailable() {
        // Both used to land in the `_` arm, which `retry` treats as worth
        // waiting for — so a request that could never succeed spent two
        // backoffs and then reported an outage that was not happening.
        assert_eq!(classify(402, None).await, ProviderError::PaymentRequired);
        assert_eq!(classify(413, None).await, ProviderError::PayloadTooLarge);
    }

    #[test]
    fn a_certificate_failure_is_recognised_from_the_error_chain() {
        // Real rustls renderings. reqwest does not re-export its TLS backend's
        // error type, so the message is the only signal that crosses the
        // boundary, and these strings are the contract.
        assert!(is_certificate_failure(
            "invalid peer certificate: UnknownIssuer"
        ));
        assert!(is_certificate_failure("invalid peer certificate: Expired"));
        assert!(is_certificate_failure(
            "error:0A000086:SSL routines:tls_post_process_server_certificate:certificate verify failed"
        ));
    }

    #[test]
    fn an_ordinary_transport_failure_is_not_read_as_a_certificate_one() {
        // The fallback has to stay wide: misreading a dropped connection as a
        // certificate problem would stop it being retried, which is the one
        // thing that actually fixes it.
        assert!(!is_certificate_failure(
            "connection closed before message completed"
        ));
        assert!(!is_certificate_failure(
            "dns error: failed to lookup address information"
        ));
        assert!(!is_certificate_failure("operation timed out"));
    }

    #[test]
    fn every_providers_context_overflow_wording_is_recognised() {
        for body in [
            "This model's maximum context length is 128000 tokens. However, your messages resulted in 130000 tokens.",
            r#"{"error":{"code":"context_length_exceeded","message":"x"}}"#,
            "prompt is too long: 215000 tokens > 200000 maximum",
            "The input token count (1100000) exceeds the maximum number of tokens allowed (1048576).",
            "This model's maximum prompt length is 131072 but the request contains 140000 tokens.",
            "Input is too long for requested model.",
            "Please reduce the length of the messages or completion.",
        ] {
            assert!(is_context_overflow_text(&body.to_ascii_lowercase()), "{body}");
        }
    }

    #[test]
    fn an_unrelated_bad_request_is_not_a_context_overflow() {
        for body in [
            "Invalid value for 'temperature': must be at most 2",
            "messages.0.content: Input should be a valid string",
            "max_tokens must be greater than 0",
            "max_tokens is over this model's output token limit of 128000",
        ] {
            assert!(
                !is_context_overflow_text(&body.to_ascii_lowercase()),
                "{body}"
            );
        }
    }

    async fn classify_body(status: u16, body: &'static str) -> ProviderError {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
        let response = reqwest::Client::new()
            .post(server.uri())
            .send()
            .await
            .expect("response");
        classify_response(response).await
    }

    #[tokio::test]
    async fn a_400_that_says_the_prompt_is_too_long_is_a_context_overflow() {
        assert_eq!(
            classify_body(400, "prompt is too long: 215000 tokens > 200000 maximum").await,
            ProviderError::ContextOverflow
        );
    }

    #[tokio::test]
    async fn any_other_400_stays_an_invalid_response() {
        assert_eq!(
            classify_body(400, "max_tokens must be greater than 0").await,
            ProviderError::InvalidResponse
        );
    }

    #[tokio::test]
    async fn a_status_that_cannot_carry_an_overflow_is_classified_without_reading_the_body() {
        assert_eq!(
            classify_body(401, "context window").await,
            ProviderError::AuthenticationFailed
        );
    }

    /// Request heads a [`keep_alive_server`] received, one entry per request.
    type RecordedHeads = Arc<Mutex<Vec<String>>>;

    /// Answers every request on a keep-alive HTTP/1.1 connection and counts the
    /// TCP connections it accepts. Hand-written so the count is the server's own
    /// observation, not a reqwest internal.
    async fn keep_alive_server() -> (String, Arc<AtomicUsize>, RecordedHeads) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binds");
        let url = format!("http://{}/", listener.local_addr().expect("addr"));
        let accepted = Arc::new(AtomicUsize::new(0));
        let heads: RecordedHeads = Arc::new(Mutex::new(Vec::new()));
        let (accepted_for_task, heads_for_task) = (accepted.clone(), heads.clone());
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                accepted_for_task.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(serve_keep_alive(socket, heads_for_task.clone()));
            }
        });
        (url, accepted, heads)
    }

    async fn serve_keep_alive(socket: tokio::net::TcpStream, heads: RecordedHeads) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

        let mut reader = BufReader::new(socket);
        loop {
            let mut head = String::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().expect("numeric content-length");
                }
                head.push_str(&line);
            }
            let mut body = vec![0; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
            heads.lock().expect("lock").push(head);
            let response =
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok";
            if reader
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
    }

    /// Sends one request, a POST when `post` is set, and reads the body to the
    /// end, which is what returns the connection to the pool.
    async fn send_one(client: &Client, url: &str, post: bool) {
        let request = if post {
            client.post(url).body("{}")
        } else {
            client.get(url)
        };
        let response = request.send().await.expect("request sends");
        assert_eq!(response.bytes().await.expect("body").as_ref(), b"ok");
    }

    #[tokio::test]
    async fn the_shared_client_reuses_one_connection_across_requests() {
        let (url, shared_accepted, _) = keep_alive_server().await;
        for post in [false, false, true, false] {
            let client = shared_streaming_client().expect("client builds");
            send_one(&client, &url, post).await;
        }
        let shared = shared_accepted.load(Ordering::SeqCst);

        let (url, fresh_accepted, _) = keep_alive_server().await;
        for post in [false, false, true, false] {
            let client = build_streaming_client().expect("client builds");
            send_one(&client, &url, post).await;
        }
        let fresh = fresh_accepted.load(Ordering::SeqCst);

        eprintln!("accepted connections for 4 requests: shared client = {shared}, fresh client per request = {fresh}");
        // One connection is the usual result. A second one is tolerated: when
        // the next request starts before the previous connection is back in
        // the pool, hyper opens another. A client per request opens one per
        // request, so with 4 requests `<= 2` still fails for it.
        assert!(
            shared <= 2,
            "shared client must reuse its pooled connection, saw {shared}"
        );
        assert_eq!(
            fresh, 4,
            "a client per request opens a connection per request"
        );
    }

    #[tokio::test]
    async fn the_shared_client_carries_each_requests_own_credential_only() {
        let (url, _, heads) = keep_alive_server().await;
        let client = shared_streaming_client().expect("client builds");
        for secret in ["first-secret-value", "second-secret-value"] {
            let response = client
                .get(&url)
                .header("x-api-key", sensitive_header(secret).expect("header"))
                .send()
                .await
                .expect("request sends");
            assert_eq!(response.bytes().await.expect("body").as_ref(), b"ok");
        }

        let heads = heads.lock().expect("lock").clone();
        assert_eq!(heads.len(), 2, "both requests reached the server");
        for (head, secret) in heads
            .iter()
            .zip(["first-secret-value", "second-secret-value"])
        {
            let lower = head.to_ascii_lowercase();
            assert!(
                !lower.contains("authorization:"),
                "no default auth header: {head}"
            );
            let keys: Vec<&str> = head
                .lines()
                .filter(|line| line.to_ascii_lowercase().starts_with("x-api-key:"))
                .collect();
            assert_eq!(
                keys,
                vec![format!("x-api-key: {secret}").as_str()],
                "{head}"
            );
        }
    }

    /// Eight first calls released together. The client is process-wide, so
    /// this reaches the construction path only if no earlier test has built
    /// it; the assertion holds either way. It cannot reproduce the macOS
    /// failure on Linux, where the build does not fail under concurrency, so a
    /// pass here does not prove that race is gone. It pins that concurrent
    /// first calls all succeed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_first_calls_to_the_shared_client_all_succeed() {
        let barrier = Arc::new(tokio::sync::Barrier::new(8));
        let calls: Vec<_> = (0..8)
            .map(|_| {
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    shared_streaming_client()
                })
            })
            .collect();
        for call in calls {
            call.await.expect("task joins").expect("client builds");
        }
    }
}
