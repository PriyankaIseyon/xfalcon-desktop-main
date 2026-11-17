//! Loopback MCP server that declares Desktop's tools to the `claude` program.
//!
//! `claude` runs with `--tools ""`, so the only tools it can call are the ones
//! listed here. It never runs one: a `tools/call` is held open until shutdown
//! and then answered as handled, and the tool loop that runs the tool stays in
//! Desktop.

use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::{auth::pkce::constant_time_eq, providers::ProviderToolSchema};

pub(crate) const SERVER_KEY: &str = "xf";
pub(crate) const MAX_FULL_TOOL_NAME: usize = 64;

const MAX_TOOLS: usize = 128;
const MAX_CONNECTIONS: usize = 16;
const MAX_HEAD_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const ACCEPT_RETRY: Duration = Duration::from_millis(50);
const ALIAS_HASH_CHARS: usize = 10;
/// Newest first. A client that asks for a version this server does not know
/// is answered with the newest one.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
pub(crate) const HANDLED_TEXT: &str = "handled by xFalcon";
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DeclareError {
    #[error("could not open the local tool server")]
    BindFailed,
    #[error("two tools share one name")]
    DuplicateToolName,
    #[error("too many tools")]
    TooManyTools,
    #[error("the local tool server failed")]
    Io,
}

/// The name `claude` uses for a tool from this server.
fn full_name(declared: &str) -> String {
    format!("mcp__{SERVER_KEY}__{declared}")
}

fn declared_from_full(full: &str) -> Option<&str> {
    full.strip_prefix("mcp__")?
        .strip_prefix(SERVER_KEY)?
        .strip_prefix("__")
}

/// A name too long for `claude` is listed as a prefix of itself plus a hash of
/// the whole name, so two long names that share a prefix stay distinct.
fn declared_for(wire: &str) -> String {
    let full = full_name(wire);
    if full.len() <= MAX_FULL_TOOL_NAME {
        return wire.to_owned();
    }
    let digest = format!("{:x}", Sha256::digest(full.as_bytes()));
    // The alias is `prefix_hash` after the server's name, so the prefix gets the bytes
    // that remain. Bytes, not characters: the limit is in bytes.
    let budget = MAX_FULL_TOOL_NAME - full_name("").len() - "_".len() - ALIAS_HASH_CHARS;
    let mut prefix = String::new();
    for character in wire.chars() {
        if prefix.len() + character.len_utf8() > budget {
            break;
        }
        prefix.push(character);
    }
    format!("{prefix}_{}", &digest[..ALIAS_HASH_CHARS])
}

/// Maps the names Desktop uses for its tools to the names listed on the wire.
pub(crate) struct ToolNameMap {
    declared_by_wire: HashMap<String, String>,
    wire_by_declared: HashMap<String, String>,
}

impl ToolNameMap {
    pub(crate) fn new(tools: &[ProviderToolSchema]) -> Result<Self, DeclareError> {
        if tools.len() > MAX_TOOLS {
            return Err(DeclareError::TooManyTools);
        }
        let mut declared_by_wire = HashMap::with_capacity(tools.len());
        let mut wire_by_declared = HashMap::with_capacity(tools.len());
        for tool in tools {
            let declared = declared_for(&tool.name);
            if declared_by_wire
                .insert(tool.name.clone(), declared.clone())
                .is_some()
                || wire_by_declared
                    .insert(declared, tool.name.clone())
                    .is_some()
            {
                return Err(DeclareError::DuplicateToolName);
            }
        }
        Ok(Self {
            declared_by_wire,
            wire_by_declared,
        })
    }

    pub(crate) fn declared_name(&self, wire_name: &str) -> Option<&str> {
        self.declared_by_wire.get(wire_name).map(String::as_str)
    }

    /// The wire name of the declared tool that a model called. The model can name
    /// a tool in four ways: `mcp__xf__<declared>`, `<declared>`, `mcp__xf__<wire>`
    /// and `<wire>`. Each is matched exactly, and any other name is `None`.
    pub(crate) fn resolve(&self, called: &str) -> Option<String> {
        let by_declared = |name: &str| self.wire_by_declared.get(name).cloned();
        let by_wire = |name: &str| {
            self.declared_by_wire
                .contains_key(name)
                .then(|| name.to_owned())
        };
        // The prefixed forms come first, because `claude` calls a tool by
        // `mcp__xf__<declared>`.
        let prefixed = declared_from_full(called);
        prefixed
            .and_then(by_declared)
            .or_else(|| by_declared(called))
            .or_else(|| prefixed.and_then(by_wire))
            .or_else(|| by_wire(called))
    }

    /// The name the model is shown for a wire name: `mcp__xf__<declared>` for a
    /// declared tool, and the input unchanged otherwise.
    pub(crate) fn callable_name(&self, wire: &str) -> String {
        match self.declared_name(wire) {
            Some(declared) => full_name(declared),
            None => wire.to_owned(),
        }
    }

    pub(crate) fn allowed_tools_arg(&self) -> String {
        full_name("*")
    }
}

/// A running declare server. Dropping it stops the server; `shutdown` also waits
/// for the connections to finish.
pub(crate) struct DeclareServer {
    routes: Arc<Routes>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl DeclareServer {
    pub(crate) async fn start(
        tools: &[ProviderToolSchema],
        names: &ToolNameMap,
    ) -> Result<Self, DeclareError> {
        let listing = tools
            .iter()
            .map(|tool| -> Result<Value, DeclareError> {
                let declared = names.declared_name(&tool.name).ok_or(DeclareError::Io)?;
                Ok(json!({
                    "name": declared,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| DeclareError::BindFailed)?;
        let port = listener
            .local_addr()
            .map_err(|_| DeclareError::BindFailed)?
            .port();
        let routes = Arc::new(Routes {
            token: random_token(),
            host: format!("127.0.0.1:{port}"),
            tools: json!({ "tools": listing }),
        });
        let cancel = CancellationToken::new();
        let task = tokio::spawn(accept_loop(listener, Arc::clone(&routes), cancel.clone()));
        Ok(Self {
            routes,
            cancel,
            task: Some(task),
        })
    }

    pub(crate) fn mcp_config_json(&self) -> String {
        json!({
            "mcpServers": {
                SERVER_KEY: {
                    "type": "http",
                    "url": format!("http://{}/mcp", self.routes.host),
                    "headers": { "Authorization": format!("Bearer {}", self.routes.token) },
                    // Without this, `claude` may defer these tools behind a tool-search step
                    // before the model can call them.
                    "alwaysLoad": true,
                }
            }
        })
        .to_string()
    }

    pub(crate) async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for DeclareServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// What every connection needs to answer a request.
struct Routes {
    token: String,
    /// The only `Host` accepted. A page reached through DNS rebinding names
    /// some other host, so it is refused here.
    host: String,
    /// The complete `tools/list` result, built once.
    tools: Value,
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn accept_loop(listener: TcpListener, routes: Arc<Routes>, cancel: CancellationToken) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    time::sleep(ACCEPT_RETRY).await;
                    continue;
                };
                // Finished connections are reaped here so the count below only
                // includes connections that are still open.
                while connections.try_join_next().is_some() {}
                if connections.len() >= MAX_CONNECTIONS {
                    drop(stream);
                    continue;
                }
                connections.spawn(serve_connection(stream, Arc::clone(&routes), cancel.clone()));
            }
        }
    }
    while connections.join_next().await.is_some() {}
}

/// Why a request is answered without a JSON-RPC reply.
#[derive(Clone, Copy)]
enum Refusal {
    BadRequest,
    Forbidden,
    Unauthorized,
    NotFound,
    MethodNotAllowed,
    LengthRequired,
    PayloadTooLarge,
    HeaderTooLarge,
}

impl Refusal {
    fn response(self) -> Vec<u8> {
        match self {
            Self::BadRequest => response("400 Bad Request", &[], &[], true),
            Self::Forbidden => response("403 Forbidden", &[], &[], true),
            Self::Unauthorized => response(
                "401 Unauthorized",
                &[("WWW-Authenticate", "Bearer")],
                &[],
                true,
            ),
            Self::NotFound => response("404 Not Found", &[], &[], true),
            Self::MethodNotAllowed => response(
                "405 Method Not Allowed",
                &[("Allow", "POST, DELETE")],
                &[],
                true,
            ),
            Self::LengthRequired => response("411 Length Required", &[], &[], true),
            Self::PayloadTooLarge => response("413 Payload Too Large", &[], &[], true),
            Self::HeaderTooLarge => response("431 Request Header Fields Too Large", &[], &[], true),
        }
    }
}

/// How a connection ends before a JSON-RPC message is handled.
enum Stop {
    /// The peer left or stopped sending, so there is nobody to answer.
    Silent,
    /// `unread` bytes of the refused body are still arriving. They are drained
    /// after the refusal is sent, so the client reads the status rather than a
    /// connection reset.
    Refuse { refusal: Refusal, unread: usize },
}

fn refuse(refusal: Refusal) -> Stop {
    Stop::Refuse { refusal, unread: 0 }
}

/// A request that passed the gate in `receive`.
enum Incoming {
    Delete,
    Post(Vec<u8>),
}

enum Reply {
    Now(Vec<u8>),
    /// A `tools/call`, answered only at shutdown.
    HoldForShutdown(Value),
}

async fn serve_connection(mut stream: TcpStream, routes: Arc<Routes>, cancel: CancellationToken) {
    let received = tokio::select! {
        _ = cancel.cancelled() => return,
        received = time::timeout(READ_TIMEOUT, receive(&mut stream, &routes)) => received,
    };
    let Ok(received) = received else {
        return;
    };
    match received {
        Ok(Incoming::Delete) => reply(&mut stream, &response("200 OK", &[], &[], true)).await,
        Ok(Incoming::Post(body)) => match dispatch(&body, &routes) {
            Reply::Now(bytes) => reply(&mut stream, &bytes).await,
            Reply::HoldForShutdown(id) => {
                if hold_until_shutdown(&mut stream, &cancel).await {
                    let answer = json_http(&rpc_result(id, tool_call_result()), false);
                    reply(&mut stream, &answer).await;
                }
            }
        },
        Err(Stop::Silent) => {}
        Err(Stop::Refuse { refusal, unread }) => {
            reply(&mut stream, &refusal.response()).await;
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = time::timeout(READ_TIMEOUT, drain(&mut stream, unread)) => {}
            }
        }
    }
}

/// Reads one request and applies the gate: the Host and Origin checks, the
/// bearer token, the path, and the method. Only then is a POST body read.
async fn receive(stream: &mut TcpStream, routes: &Routes) -> Result<Incoming, Stop> {
    let mut buffered = Vec::new();
    let head_end = read_head(stream, &mut buffered).await?;
    let head = parse_head(&buffered[..head_end - 4]).ok_or(refuse(Refusal::BadRequest))?;

    if header_values(&head.headers, "host") != [routes.host.as_str()] {
        return Err(refuse(Refusal::Forbidden));
    }
    if !header_values(&head.headers, "origin").is_empty() {
        return Err(refuse(Refusal::Forbidden));
    }
    let authorized =
        bearer_token(&head.headers).is_some_and(|token| constant_time_eq(token, &routes.token));
    if !authorized {
        return Err(refuse(Refusal::Unauthorized));
    }
    if head.target != "/mcp" {
        return Err(refuse(Refusal::NotFound));
    }
    match head.method.as_str() {
        "DELETE" => return Ok(Incoming::Delete),
        "POST" => {}
        _ => return Err(refuse(Refusal::MethodNotAllowed)),
    }

    let length = content_length(&head.headers).map_err(refuse)?;
    if length > MAX_BODY_BYTES {
        let buffered_body = buffered.len() - head_end;
        return Err(Stop::Refuse {
            refusal: Refusal::PayloadTooLarge,
            unread: length.saturating_sub(buffered_body),
        });
    }
    let mut body = buffered.split_off(head_end);
    body.truncate(length);
    let have = body.len();
    body.resize(length, 0);
    stream
        .read_exact(&mut body[have..])
        .await
        .map_err(|_| Stop::Silent)?;
    Ok(Incoming::Post(body))
}

/// Reads through the blank line that ends the head and returns the index just
/// past it. Bytes read beyond that point stay in `buffered`.
async fn read_head(stream: &mut TcpStream, buffered: &mut Vec<u8>) -> Result<usize, Stop> {
    loop {
        if let Some(end) = buffered
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|at| at + 4)
        {
            return if end <= MAX_HEAD_BYTES {
                Ok(end)
            } else {
                Err(refuse(Refusal::HeaderTooLarge))
            };
        }
        if buffered.len() >= MAX_HEAD_BYTES {
            return Err(refuse(Refusal::HeaderTooLarge));
        }
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.map_err(|_| Stop::Silent)?;
        if read == 0 {
            return Err(Stop::Silent);
        }
        buffered.extend_from_slice(&chunk[..read]);
    }
}

struct Head {
    method: String,
    target: String,
    /// Names are lowercased, so lookups ignore case.
    headers: Vec<(String, String)>,
}

/// `block` is the head without its terminating blank line.
fn parse_head(block: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(block).ok()?;
    let mut lines = text.split("\r\n");
    let request_line: Vec<&str> = lines.next()?.split(' ').collect();
    let [method, target, version] = request_line.as_slice() else {
        return None;
    };
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        // Rejects folded continuation lines, which start with whitespace.
        if name.is_empty() || name.contains(char::is_whitespace) {
            return None;
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    Some(Head {
        method: method.to_string(),
        target: target.to_string(),
        headers,
    })
}

fn header_values<'a>(headers: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .collect()
}

fn bearer_token(headers: &[(String, String)]) -> Option<&str> {
    match header_values(headers, "authorization").as_slice() {
        [value] => {
            let (scheme, token) = value.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then_some(token)
        }
        _ => None,
    }
}

fn content_length(headers: &[(String, String)]) -> Result<usize, Refusal> {
    // Only Content-Length framing is accepted. A chunked body could not be
    // measured against the body cap before it is read.
    if !header_values(headers, "transfer-encoding").is_empty() {
        return Err(Refusal::LengthRequired);
    }
    match header_values(headers, "content-length").as_slice() {
        [] => Err(Refusal::LengthRequired),
        [value] if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            // A value too large for `usize` is over the cap either way.
            Ok(value.parse().unwrap_or(usize::MAX))
        }
        _ => Err(Refusal::BadRequest),
    }
}

fn dispatch(body: &[u8], routes: &Routes) -> Reply {
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return Reply::Now(json_http(
            &rpc_error(Value::Null, PARSE_ERROR, "Parse error"),
            true,
        ));
    };
    let Some(object) = message.as_object() else {
        return Reply::Now(json_http(
            &rpc_error(Value::Null, INVALID_REQUEST, "Invalid Request"),
            true,
        ));
    };
    // A message without an id is a notification, which gets no JSON-RPC reply.
    let Some(id) = object.get("id").cloned() else {
        return Reply::Now(response("202 Accepted", &[], &[], true));
    };
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Reply::Now(json_http(
            &rpc_error(id, INVALID_REQUEST, "Invalid Request"),
            true,
        ));
    };
    let reply = match method {
        "initialize" => rpc_result(id, initialize_result(object.get("params"))),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, routes.tools.clone()),
        // The call is only held open. It is never run, so nothing here can
        // execute a tool.
        "tools/call" => return Reply::HoldForShutdown(id),
        _ => rpc_error(id, METHOD_NOT_FOUND, "Method not found"),
    };
    Reply::Now(json_http(&reply, true))
}

fn initialize_result(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = requested
        .filter(|version| PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "xfalcon-declare", "version": "1" },
    })
}

fn tool_call_result() -> Value {
    json!({
        "content": [{ "type": "text", "text": HANDLED_TEXT }],
        "isError": true,
    })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn json_http(message: &Value, close: bool) -> Vec<u8> {
    response(
        "200 OK",
        &[("Content-Type", "application/json")],
        message.to_string().as_bytes(),
        close,
    )
}

/// A complete HTTP/1.1 response. `close` is false only for the held
/// `tools/call`, which is answered without `Connection: close`.
fn response(status: &str, headers: &[(&str, &str)], body: &[u8], close: bool) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    if close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Best effort. The client may already be gone, and a client that stops reading
/// must not hold up shutdown past the read timeout.
async fn reply(stream: &mut TcpStream, bytes: &[u8]) {
    let _ = time::timeout(READ_TIMEOUT, stream.write_all(bytes)).await;
    let _ = stream.shutdown().await;
}

/// Keeps a `tools/call` open. Returns false when the client leaves first.
async fn hold_until_shutdown(stream: &mut TcpStream, cancel: &CancellationToken) -> bool {
    let mut probe = [0u8; 64];
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return true,
            read = stream.read(&mut probe) => match read {
                Ok(0) | Err(_) => return false,
                Ok(_) => {}
            },
        }
    }
}

async fn drain(stream: &mut TcpStream, mut remaining: usize) {
    let mut sink = [0u8; 8192];
    while remaining > 0 {
        let window = remaining.min(sink.len());
        match stream.read(&mut sink[..window]).await {
            Ok(0) | Err(_) => return,
            Ok(read) => remaining -= read,
        }
    }
}

#[cfg(test)]
mod tests {
    use reqwest::{
        header::{AUTHORIZATION, HOST, ORIGIN},
        StatusCode, Url,
    };
    use tokio::net::TcpStream;

    use super::*;

    /// Sixty characters: `mcp__xf__` plus this is 69, past the limit.
    fn long_name() -> String {
        "abcdefghij".repeat(6)
    }

    /// The first 44 characters of `long_name()`, then `_`, then the first ten
    /// hex characters of SHA-256 over `mcp__xf__abcdefghij...`. The hash was
    /// computed independently with `sha256sum`.
    const LONG_ALIAS: &str = "abcdefghijabcdefghijabcdefghijabcdefghijabcd_863fc48df1";

    fn tool(name: &str) -> ProviderToolSchema {
        ProviderToolSchema {
            name: name.to_owned(),
            description: format!("{name} description"),
            input_schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "minLength": 1 } },
                "required": ["query"],
                "additionalProperties": false,
            }),
        }
    }

    async fn start(tools: &[ProviderToolSchema]) -> DeclareServer {
        let names = ToolNameMap::new(tools).expect("names");
        DeclareServer::start(tools, &names).await.expect("start")
    }

    /// The URL and bearer header exactly as `claude` reads them from the config.
    fn endpoint(server: &DeclareServer) -> (String, String) {
        let config: Value =
            serde_json::from_str(&server.mcp_config_json()).expect("config is JSON");
        let entry = &config["mcpServers"][SERVER_KEY];
        let url = entry["url"].as_str().expect("url").to_owned();
        let auth = entry["headers"]["Authorization"]
            .as_str()
            .expect("authorization")
            .to_owned();
        (url, auth)
    }

    fn port_of(url: &str) -> u16 {
        Url::parse(url).expect("a URL").port().expect("a port")
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client")
    }

    async fn post(url: &str, auth: &str, body: &Value) -> reqwest::Response {
        client()
            .post(url)
            .header(AUTHORIZATION, auth)
            .json(body)
            .send()
            .await
            .expect("the server answers")
    }

    async fn rpc(url: &str, auth: &str, message: Value) -> Value {
        post(url, auth, &message)
            .await
            .json()
            .await
            .expect("a JSON-RPC body")
    }

    async fn raw_exchange(port: u16, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream.write_all(request).await.expect("write");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.expect("read");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn a_name_within_the_limit_is_listed_as_it_is() {
        let names = ToolNameMap::new(&[tool("project_search")]).expect("names");
        assert_eq!(
            names.declared_name("project_search"),
            Some("project_search")
        );
        assert_eq!(
            names.resolve("mcp__xf__project_search"),
            Some("project_search".to_owned())
        );
        assert_eq!(names.resolve("mcp__other__project_search"), None);
        assert_eq!(names.allowed_tools_arg(), "mcp__xf__*");
    }

    #[test]
    fn a_sixty_character_name_is_listed_under_a_55_character_alias() {
        let long = long_name();
        assert_eq!(full_name(&long).len(), 69);
        let names = ToolNameMap::new(&[tool(&long)]).expect("names");
        let alias = names.declared_name(&long).expect("declared");
        assert_eq!(alias, LONG_ALIAS);
        assert_eq!(alias.len(), 55);
        assert!(full_name(alias).len() <= MAX_FULL_TOOL_NAME);
        assert_eq!(names.resolve(&full_name(alias)), Some(long.clone()));
    }

    /// The four tool names of the data server that the app declares in practice.
    /// They are short, so none is aliased.
    const DATA_TOOLS: [&str; 4] = [
        "mcp_tenant_ida_query",
        "mcp_tenant_ida_save_dashboard",
        "mcp_tenant_ida_get_query_patterns",
        "mcp_tenant_ida_annotation_feedback",
    ];

    #[test]
    fn resolve_accepts_each_form_a_model_can_call_a_short_tool_by() {
        let names = ToolNameMap::new(&[tool("project_search")]).expect("names");
        for called in ["mcp__xf__project_search", "project_search"] {
            assert_eq!(
                names.resolve(called),
                Some("project_search".to_owned()),
                "{called}"
            );
        }
    }

    #[test]
    fn resolve_accepts_each_form_a_model_can_call_an_aliased_tool_by() {
        let long = long_name();
        let names = ToolNameMap::new(&[tool(&long)]).expect("names");
        let calls = [
            format!("mcp__xf__{LONG_ALIAS}"),
            LONG_ALIAS.to_owned(),
            format!("mcp__xf__{long}"),
            long.clone(),
        ];
        for called in calls {
            assert_eq!(names.resolve(&called), Some(long.clone()), "{called}");
        }
    }

    #[test]
    fn resolve_accepts_the_data_server_names_in_each_form() {
        let tools: Vec<ProviderToolSchema> = DATA_TOOLS.iter().map(|name| tool(name)).collect();
        let names = ToolNameMap::new(&tools).expect("names");
        for wire in DATA_TOOLS {
            assert_eq!(names.resolve(wire), Some(wire.to_owned()), "{wire}");
            assert_eq!(
                names.resolve(&format!("mcp__xf__{wire}")),
                Some(wire.to_owned()),
                "mcp__xf__{wire}"
            );
        }
        assert_eq!(
            names.resolve("mcp__xf__mcp_tenant_ida_save_dashboard"),
            Some("mcp_tenant_ida_save_dashboard".to_owned())
        );
        assert_eq!(
            names.resolve("mcp_tenant_ida_save_dashboard"),
            Some("mcp_tenant_ida_save_dashboard".to_owned())
        );
    }

    #[test]
    fn resolve_refuses_undeclared_names_case_variants_and_prefixes() {
        let long = long_name();
        let names = ToolNameMap::new(&[tool("project_search"), tool(&long)]).expect("names");
        let refused = [
            "mcp__xf__not_declared".to_owned(),
            "not_declared".to_owned(),
            "MCP__XF__project_search".to_owned(),
            "Project_Search".to_owned(),
            "mcp__xf__Project_Search".to_owned(),
            "project_search ".to_owned(),
            "project".to_owned(),
            "mcp__xf__project".to_owned(),
            "mcp__xf__".to_owned(),
            "mcp__xf__mcp__xf__project_search".to_owned(),
            "mcp__other__project_search".to_owned(),
            long[..50].to_owned(),
            format!("mcp__xf__{}", &long[..44]),
            LONG_ALIAS.to_uppercase(),
            "mcp_tenant_ida_query".to_owned(),
            "".to_owned(),
        ];
        for called in refused {
            assert_eq!(names.resolve(&called), None, "{called:?}");
        }
    }

    #[test]
    fn callable_name_is_the_full_name_of_a_declared_tool_and_the_input_otherwise() {
        let long = long_name();
        let names = ToolNameMap::new(&[tool("project_search"), tool(&long)]).expect("names");
        assert_eq!(
            names.callable_name("project_search"),
            "mcp__xf__project_search"
        );
        assert_eq!(names.callable_name(&long), format!("mcp__xf__{LONG_ALIAS}"));
        assert_eq!(names.callable_name("not_declared"), "not_declared");
        for wire in ["project_search", long.as_str()] {
            assert_eq!(
                names.resolve(&names.callable_name(wire)),
                Some(wire.to_owned())
            );
        }
    }

    #[test]
    fn callable_name_of_each_data_server_tool_resolves_back_to_it() {
        let tools: Vec<ProviderToolSchema> = DATA_TOOLS.iter().map(|name| tool(name)).collect();
        let names = ToolNameMap::new(&tools).expect("names");
        assert_eq!(
            names.callable_name("mcp_tenant_ida_save_dashboard"),
            "mcp__xf__mcp_tenant_ida_save_dashboard"
        );
        for wire in DATA_TOOLS {
            assert_eq!(
                names.resolve(&names.callable_name(wire)),
                Some(wire.to_owned()),
                "{wire}"
            );
        }
    }

    #[test]
    fn a_long_non_ascii_name_is_aliased_within_the_byte_limit() {
        // Sixty two-byte characters: 120 bytes, so the prefix stops at 44 bytes (22
        // characters), not at 44 characters, which would be 88 bytes.
        let wire = "é".repeat(60);
        let declared = declared_for(&wire);
        assert!(declared.starts_with(&"é".repeat(22)));
        assert_eq!(declared.len(), 44 + 1 + ALIAS_HASH_CHARS);
        assert!(full_name(&declared).len() <= MAX_FULL_TOOL_NAME);
        let names = ToolNameMap::new(&[tool(&wire)]).expect("names");
        assert_eq!(
            names.callable_name(&wire).len(),
            9 + 44 + 1 + ALIAS_HASH_CHARS
        );
    }

    #[test]
    fn a_name_of_exactly_the_limit_is_not_aliased() {
        let name = "a".repeat(MAX_FULL_TOOL_NAME - "mcp__xf__".len());
        let names = ToolNameMap::new(&[tool(&name)]).expect("names");
        assert_eq!(names.declared_name(&name), Some(name.as_str()));
    }

    #[test]
    fn duplicate_wire_names_and_colliding_aliases_are_rejected() {
        assert_eq!(
            ToolNameMap::new(&[tool("a"), tool("a")]).err(),
            Some(DeclareError::DuplicateToolName)
        );
        let long = long_name();
        let alias = ToolNameMap::new(&[tool(&long)])
            .expect("names")
            .declared_name(&long)
            .expect("alias")
            .to_owned();
        assert_eq!(
            ToolNameMap::new(&[tool(&long), tool(&alias)]).err(),
            Some(DeclareError::DuplicateToolName)
        );
    }

    #[test]
    fn more_than_128_tools_is_rejected() {
        let many: Vec<ProviderToolSchema> = (0..=MAX_TOOLS)
            .map(|index| tool(&format!("tool_{index}")))
            .collect();
        assert_eq!(
            ToolNameMap::new(&many).err(),
            Some(DeclareError::TooManyTools)
        );
        assert!(ToolNameMap::new(&many[..MAX_TOOLS]).is_ok());
    }

    #[tokio::test]
    async fn mcp_config_points_claude_at_the_bearer_protected_endpoint() {
        let server = start(&[tool("a")]).await;
        let config: Value =
            serde_json::from_str(&server.mcp_config_json()).expect("config is JSON");
        let entry = &config["mcpServers"][SERVER_KEY];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["alwaysLoad"], true);
        let url = Url::parse(entry["url"].as_str().expect("url")).expect("a URL");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.path(), "/mcp");
        assert!(url.port().is_some_and(|port| port != 0));
        let auth = entry["headers"]["Authorization"].as_str().expect("auth");
        let token = auth.strip_prefix("Bearer ").expect("a bearer token");
        assert_eq!(token.len(), 43);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn initialize_echoes_a_known_protocol_version() {
        let server = start(&[]).await;
        let (url, auth) = endpoint(&server);
        let reply = rpc(
            &url,
            &auth,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "claude", "version": "1" },
                },
            }),
        )
        .await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(
            reply["result"]["capabilities"],
            json!({ "tools": { "listChanged": false } })
        );
        assert_eq!(
            reply["result"]["serverInfo"],
            json!({ "name": "xfalcon-declare", "version": "1" })
        );
        server.shutdown().await;
    }

    #[tokio::test]
    async fn initialize_answers_the_newest_version_for_an_unknown_one() {
        let server = start(&[]).await;
        let (url, auth) = endpoint(&server);
        let reply = rpc(
            &url,
            &auth,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "1999-01-01" },
            }),
        )
        .await;
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        server.shutdown().await;
    }

    #[tokio::test]
    async fn tools_list_declares_each_tool_with_its_schema_verbatim() {
        let long = long_name();
        let tools = [tool("project_search"), tool(&long)];
        let server = start(&tools).await;
        let (url, auth) = endpoint(&server);
        let reply = rpc(
            &url,
            &auth,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .await;
        let listed = reply["result"]["tools"].as_array().expect("tools");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["name"], "project_search");
        assert_eq!(listed[1]["name"], LONG_ALIAS);
        for (declared, original) in listed.iter().zip(&tools) {
            assert_eq!(declared["description"], original.description.as_str());
            assert_eq!(declared["inputSchema"], original.input_schema);
        }
        server.shutdown().await;
    }

    #[tokio::test]
    async fn a_wrong_or_missing_token_is_401_with_no_body() {
        let server = start(&[tool("a")]).await;
        let (url, _) = endpoint(&server);
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

        let wrong = post(&url, "Bearer not-the-token", &ping).await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        let missing = client()
            .post(&url)
            .json(&ping)
            .send()
            .await
            .expect("answer");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        assert!(missing.text().await.expect("body").is_empty());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn get_is_405_and_another_path_is_404() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);

        let get = client()
            .get(&url)
            .header(AUTHORIZATION, auth.as_str())
            .send()
            .await
            .expect("answer");
        assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);

        let other = url.replace("/mcp", "/other");
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
        let missing = post(&other, &auth, &ping).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn a_foreign_host_is_403() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
        let reply = client()
            .post(&url)
            .header(AUTHORIZATION, auth.as_str())
            .header(HOST, "evil.example:80")
            .json(&ping)
            .send()
            .await
            .expect("answer");
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn an_origin_header_is_403() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
        let reply = client()
            .post(&url)
            .header(AUTHORIZATION, auth.as_str())
            .header(ORIGIN, "http://localhost:1420")
            .json(&ping)
            .send()
            .await
            .expect("answer");
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn a_notification_is_202_with_an_empty_body() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let reply = post(
            &url,
            &auth,
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
        assert_eq!(reply.status(), StatusCode::ACCEPTED);
        assert!(reply.text().await.expect("body").is_empty());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn server_discover_is_method_not_found_and_ping_is_empty() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);

        let discover = rpc(
            &url,
            &auth,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "server/discover", "params": {} }),
        )
        .await;
        assert_eq!(discover["id"], 4);
        assert_eq!(discover["error"]["code"], -32601);

        let ping = rpc(
            &url,
            &auth,
            json!({ "jsonrpc": "2.0", "id": 5, "method": "ping" }),
        )
        .await;
        assert_eq!(ping["result"], json!({}));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn invalid_json_is_a_parse_error_with_a_null_id() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let reply = client()
            .post(&url)
            .header(AUTHORIZATION, auth.as_str())
            .body("{not json")
            .send()
            .await
            .expect("answer");
        assert_eq!(reply.status(), StatusCode::OK);
        let body: Value = reply.json().await.expect("a JSON-RPC body");
        assert_eq!(body["error"]["code"], -32700);
        assert!(body["id"].is_null());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn tools_call_is_held_until_shutdown_and_then_answered_unexecuted() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let call = tokio::spawn(async move {
            post(
                &url,
                &auth,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": {
                        "name": "mcp__xf__a",
                        "arguments": { "query": "x" },
                        "_meta": { "claudecode/toolUseId": "toolu_1", "progressToken": 1 },
                    },
                }),
            )
            .await
        });

        time::sleep(Duration::from_millis(300)).await;
        assert!(!call.is_finished(), "tools/call must stay open");

        server.shutdown().await;
        let answer = call.await.expect("the request task");
        assert_eq!(answer.status(), StatusCode::OK);
        let body: Value = answer.json().await.expect("a JSON-RPC body");
        assert_eq!(body["id"], 3);
        assert_eq!(body["result"]["isError"], true);
        assert_eq!(body["result"]["content"][0]["text"], "handled by xFalcon");
    }

    #[tokio::test]
    async fn a_body_over_the_cap_is_413() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let reply = client()
            .post(&url)
            .header(AUTHORIZATION, auth.as_str())
            .body(vec![b'x'; 2 * 1024 * 1024])
            .send()
            .await
            .expect("answer");
        assert_eq!(reply.status(), StatusCode::PAYLOAD_TOO_LARGE);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn delete_is_200_with_an_empty_body() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let reply = client()
            .delete(&url)
            .header(AUTHORIZATION, auth.as_str())
            .send()
            .await
            .expect("answer");
        assert_eq!(reply.status(), StatusCode::OK);
        assert!(reply.text().await.expect("body").is_empty());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn header_names_match_case_insensitively_over_crlf() {
        let server = start(&[tool("a")]).await;
        let (url, auth) = endpoint(&server);
        let body = r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHOST: 127.0.0.1:{}\r\nauthorization: {auth}\r\ncontent-LENGTH: {}\r\n\r\n{body}",
            port_of(&url),
            body.len()
        );
        let answer = raw_exchange(port_of(&url), request.as_bytes()).await;
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
        let (_, json_body) = answer.split_once("\r\n\r\n").expect("a head and a body");
        let body: Value = serde_json::from_str(json_body).expect("a JSON-RPC body");
        assert_eq!(body["id"], 7);
        assert_eq!(body["result"], json!({}));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn connections_beyond_the_limit_are_closed_at_once() {
        let server = start(&[tool("a")]).await;
        let (url, _) = endpoint(&server);
        let port = port_of(&url);
        let mut stalled = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            stalled.push(
                TcpStream::connect(("127.0.0.1", port))
                    .await
                    .expect("connect"),
            );
        }
        let mut extra = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let mut byte = [0u8; 1];
        let read = time::timeout(Duration::from_secs(5), extra.read(&mut byte))
            .await
            .expect("closed promptly")
            .expect("read");
        assert_eq!(read, 0, "the extra connection is closed without an answer");
        drop(stalled);
        server.shutdown().await;
    }
}
