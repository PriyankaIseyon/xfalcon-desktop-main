//! Local diagnostics logging and bundle generation (spec: local-diagnostics).
//!
//! Provides zero-network-egress diagnostics export and renderer error capture.

use regex::{Captures, Regex};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use tauri::Manager;
use xfalcon_contracts::diagnostics::{DiagnosticsBundleResponse, RendererErrorReport};
use xfalcon_contracts::CONTRACT_VERSION;

use crate::audit::{
    emit, render_diagnostics_section, AuditAction, AuditHub, AuditOutcome, AuditResourceType,
};

mod redacting_writer;

pub(crate) use redacting_writer::{redact_existing_logs, RedactingMakeWriter};

/// Largest piece of a log line read at once. A longer line is processed in pieces of
/// this size, so memory stays bounded whatever the line length.
const LINE_CHUNK_BYTES: u64 = 1024 * 1024;

// Static pre-compiled LazyLock regexes to ensure zero runtime regex compile failures (CWE-532).
// Covers base64 and base64url tokens (`+ / =` included) and shorter tokens than the
// 20-character floor it replaces.
// Matches `Bearer <token>`. Whether the token is redacted is decided by redact_bearer_tokens.
static BEARER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=\-]{8,}").expect("valid bearer regex")
});

static AUTH_HEADER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bauthorization:\s*[^\r\n,;]+").expect("valid auth header regex")
});

static SK_KEY_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bsk-[a-zA-Z0-9_\-]{20,}").expect("valid sk key regex"));

static IDA_SK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bida_sk_[a-zA-Z0-9_.\-]+").expect("valid ida_sk key regex"));

// The value runs to the end of the line, so a header line is redacted whole. In
// JSON-ish text (`"Cookie: a=b","next":...` or `{"Cookie: a=b"}`) it instead stops
// at the closing `"` that is followed by `,`, `}` or `]`, which is kept, so the
// later JSON fields survive. Group 2 is that terminator and is written back.
static HEADER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?im)\b(set-cookie|cookie|proxy-authorization|x-api-key|api-key):[^\r\n]*?("[ \t]*[,}\]]|\r?$)"#,
    )
    .expect("valid credential header regex")
});

// The value stops at a query separator, whitespace, a quote, a closing bracket or
// a comma or semicolon, so `?code=abc)` or `"url":"...?code=abc","next"` keep the
// text after the secret.
static QUERY_PARAM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)([?&])(access_token|api_key|token|key|secret|password|code|state|id_token|refresh_token|client_secret|sig|signature|session|x-amz-[a-z\-]+)=([^&\s"')\\,;]+)"#,
    )
    .expect("valid query param regex")
});

static JWT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]*")
        .expect("valid jwt regex")
});

// The local part accepts Unicode letters and digits; the domain stays ASCII.
static EMAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\p{L}\p{N}._%+\-]+@[A-Za-z0-9\-]+(\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}")
        .expect("valid email regex")
});

static BODY_FIELD_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)"(password|secret|token|api[_-]?key|client[_-]?secret|refresh[_-]?token|access[_-]?token|id[_-]?token|authorization|cookie|set-cookie|proxy-authorization|x-api-key)"\s*:\s*"(?:\\.|[^"\\])*""#,
    )
    .expect("valid json secret regex")
});

// BODY_FIELD_REGEX for JSON carried inside a string, where every quote is backslash-escaped
// (`body="{\"refresh_token\":\"rt-123\"}"`).
static ESCAPED_BODY_FIELD_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\\"(password|secret|token|api[_-]?key|client[_-]?secret|refresh[_-]?token|access[_-]?token|id[_-]?token|authorization|cookie|set-cookie|proxy-authorization|x-api-key)\\"\s*:\s*\\"[^"\\]*\\""#,
    )
    .expect("valid escaped json secret regex")
});

// Plain `name=value` and Debug-style `name: value` text. The name must start a word:
// `_` separates words, so `DB_PASSWORD=` matches, but `mysecret=` does not. The
// separator must follow the name directly, so `tokens_used=` and `token_count=` do
// not match. JSON-quoted names such as `"token": "x"` are left to BODY_FIELD_REGEX.
// The value is, in order: a double-quoted string with `\"` escapes; a constructor
// wrapping one, such as `Some("x")`; a braced Debug struct, such as
// `Secret { inner: "x" }`; or a bare run that cannot start with `[`, which keeps an
// existing `[REDACTED...]` placeholder from matching again. The whole value becomes
// `[REDACTED]`.
static KEY_VALUE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(^|[^A-Za-z0-9])(password|passwd|secret|token|api[_-]?key|client[_-]?secret|refresh[_-]?token|access[_-]?token|id[_-]?token|authorization|cookie|(?:secret[_-]?)?access[_-]?key)([ \t]*[=:][ \t]*)("(?:\\.|[^"\\\r\n])*"|\w+\(\s*"(?:\\.|[^"\\\r\n])*"\s*\)|\w+\s*\{[^{}\r\n]*\}|(?:(?:Basic|Digest|Bearer)\s+)?[^\s,;&)}\]\[][^\s,;&)}\]"]*)"#,
    )
    .expect("valid key value regex")
});

// `scheme://user:password@host`. The user and password stop at `"`, `?` and `#`, so a
// quoted or query-embedded `@` is not taken as userinfo. A password containing a raw
// `@` is not matched past it (such a URL is invalid unless percent-encoded).
static USER_INFO_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(\b[a-z][a-z0-9+.\-]*://)[^\s/?#@:"]+(?::[^\s/?#@"]*)?@"#)
        .expect("valid url userinfo regex")
});

// Conservative phone shapes: an international number with a leading `+`, or a North
// American 3-3-4 grouping. Digit runs, ISO times and dates, UUIDs, versions and IPv4
// addresses have neither shape.
static PHONE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\+\d{1,3}[\s.\-]?(?:\(\d{1,4}\)[\s.\-]?)?\d{2,4}(?:[\s.\-]\d{2,4}){1,4}|(?:\(\d{3}\)\s?|\b\d{3}[\s.\-])\d{3}[\s.\-]\d{4}\b",
    )
    .expect("valid phone regex")
});

static USER_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:[a-zA-Z]:\\Users\\|/home/|/Users/)([^/\\:\s\x22'\n]+)")
        .expect("valid user path regex")
});

// rmcp logs the remote initialization document at INFO. Its Debug payload can
// include server identity and instructions, so retain the event but exclude
// that metadata from exports, including logs written by earlier versions.
static MCP_PEER_INFO_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bpeer_info=[^\r\n]*").expect("valid MCP peer metadata regex"));

/// Redacts `Bearer <token>` when the token looks like a credential: it has a digit or
/// one of `._~+/=-`, or it is at least 20 characters long. Ordinary words after
/// "Bearer" are left alone.
fn redact_bearer_tokens(text: &str) -> String {
    BEARER_REGEX
        .replace_all(text, |caps: &Captures<'_>| {
            let matched = &caps[0];
            let token = matched.split_whitespace().next_back().unwrap_or_default();
            let looks_like_credential = token.len() >= 20
                || token
                    .bytes()
                    .any(|b| b.is_ascii_digit() || b"._~+/=-".contains(&b));
            if looks_like_credential {
                "Bearer [REDACTED_BEARER_TOKEN]".to_owned()
            } else {
                matched.to_owned()
            }
        })
        .to_string()
}

/// Sanitize any potential sensitive tokens, user paths, or secrets from a text string.
pub fn sanitize_diagnostics_text(text: &str) -> String {
    let mut result = text.to_string();

    result = MCP_PEER_INFO_REGEX
        .replace_all(&result, "peer_info=[REDACTED_MCP_METADATA]")
        .to_string();

    result = redact_bearer_tokens(&result);
    result = AUTH_HEADER_REGEX
        .replace_all(&result, "Authorization: [REDACTED_AUTH_HEADER]")
        .to_string();
    result = HEADER_REGEX
        .replace_all(&result, "${1}: [REDACTED_HEADER]${2}")
        .to_string();
    result = SK_KEY_REGEX
        .replace_all(&result, "[REDACTED_API_KEY]")
        .to_string();
    result = IDA_SK_REGEX
        .replace_all(&result, "[REDACTED_API_KEY]")
        .to_string();
    result = QUERY_PARAM_REGEX
        .replace_all(&result, "${1}${2}=[REDACTED]")
        .to_string();
    result = BODY_FIELD_REGEX
        .replace_all(&result, r#""$1": "[REDACTED]""#)
        .to_string();
    result = ESCAPED_BODY_FIELD_REGEX
        .replace_all(&result, r#"\"${1}\": \"[REDACTED]\""#)
        .to_string();
    // After BODY_FIELD_REGEX, so JSON-quoted names are already handled there.
    result = KEY_VALUE_REGEX
        .replace_all(&result, "${1}${2}${3}[REDACTED]")
        .to_string();
    // Before the email pass, so `user:pass@host` is not read as an email address.
    result = USER_INFO_REGEX
        .replace_all(&result, "${1}[REDACTED_USERINFO]@")
        .to_string();
    result = USER_PATH_REGEX
        .replace_all(&result, "[USER_PATH]/[USER]")
        .to_string();

    // Run last: the placeholders above are not token-shaped, and the userinfo
    // placeholder's `@` follows `]`, which no email local part can end with, so
    // these passes cannot match inside them.
    result = JWT_REGEX.replace_all(&result, "[REDACTED_JWT]").to_string();
    result = PHONE_REGEX
        .replace_all(&result, "[REDACTED_PHONE]")
        .to_string();
    result = EMAIL_REGEX
        .replace_all(&result, "[REDACTED_EMAIL]")
        .to_string();

    result
}

const DIAGNOSTICS_TRUNCATED_MARKER: &str = "…[truncated]";
const MAX_DIAGNOSTICS_RAW_BYTES: usize = 1024 * 1024;

/// Redacts a bounded raw value before limiting its exported byte length.
fn sanitize_diagnostics_prefix(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(MAX_DIAGNOSTICS_RAW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < text.len();
    let prefix = &text[..end];
    let mut sanitized = if truncated {
        let complete_end = prefix.rfind('\n').map_or(0, |index| index + 1);
        sanitize_diagnostics_text(&prefix[..complete_end])
    } else {
        sanitize_diagnostics_text(prefix)
    };
    if truncated {
        append_truncation_marker(&mut sanitized, max_bytes);
    } else {
        truncate_sanitized_output(&mut sanitized, max_bytes);
    }
    sanitized
}

/// Handles a raw log piece known to be the beginning of an incomplete line.
fn sanitize_truncated_diagnostics_line(text: &str, max_bytes: usize) -> String {
    let complete_end = text.rfind('\n').map_or(0, |index| index + 1);
    let mut sanitized = sanitize_diagnostics_text(&text[..complete_end]);
    append_truncation_marker(&mut sanitized, max_bytes);
    sanitized
}

fn append_truncation_marker(text: &mut String, max_bytes: usize) {
    let marker = if max_bytes >= DIAGNOSTICS_TRUNCATED_MARKER.len() {
        DIAGNOSTICS_TRUNCATED_MARKER
    } else {
        // A fixed ASCII fallback avoids slicing the three-byte ellipsis at a
        // non-UTF-8 boundary when a small internal limit is used.
        &"..."[..max_bytes.min(3)]
    };
    let content_limit = max_bytes.saturating_sub(marker.len());
    if text.len() > content_limit {
        let mut boundary = content_limit.min(text.len());
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
    }
    text.push_str(marker);
}

fn truncate_sanitized_output(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let marker = if max_bytes >= DIAGNOSTICS_TRUNCATED_MARKER.len() {
        DIAGNOSTICS_TRUNCATED_MARKER
    } else {
        &"..."[..max_bytes.min(3)]
    };
    let content_limit = max_bytes.saturating_sub(marker.len());
    let mut boundary = content_limit.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    text.push_str(marker);
}

/// Sanitizes only complete lines from a prefix whose tail has already been dropped.
pub(super) fn sanitize_complete_diagnostics_prefix(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &text[..end];
    let complete_end = prefix.rfind('\n').map_or(0, |index| index + 1);
    let mut sanitized = sanitize_diagnostics_text(&prefix[..complete_end]);
    if sanitized.len() > max_bytes {
        let mut boundary = max_bytes.min(sanitized.len());
        while !sanitized.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let complete_end = sanitized[..boundary]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        sanitized.truncate(complete_end);
    }
    sanitized
}

/// Check if a log filename matches the current active daily naming convention: `xfalcon-desktop.YYYY-MM-DD.log`.
/// Legacy format files (`xfalcon-desktop.log.YYYY-MM-DD` or unredacted history) return false.
pub fn is_active_log_file(filename: &str) -> bool {
    const PREFIX: &str = "xfalcon-desktop.";
    const SUFFIX: &str = ".log";
    const EXPECTED_LEN: usize = PREFIX.len() + 10 + SUFFIX.len(); // 30

    if filename.len() != EXPECTED_LEN
        || !filename.starts_with(PREFIX)
        || !filename.ends_with(SUFFIX)
    {
        return false;
    }
    let middle = &filename[PREFIX.len()..filename.len() - SUFFIX.len()];
    let chars: Vec<char> = middle.chars().collect();
    chars.len() == 10
        && chars[0..4].iter().all(|c| c.is_ascii_digit())
        && chars[4] == '-'
        && chars[5..7].iter().all(|c| c.is_ascii_digit())
        && chars[7] == '-'
        && chars[8..10].iter().all(|c| c.is_ascii_digit())
}

/// Prune legacy un-redacted log files (`xfalcon-desktop.log.*`) that are not managed by the active rolling file appender.
pub fn prune_legacy_log_files(logs_dir: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(logs_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("xfalcon-desktop.log") && !is_active_log_file(&name) {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// What this device is configured to talk to, gathered before composition.
///
/// Plain data, collected by the command and rendered here, so composition stays
/// a pure function over a directory and this struct.
///
/// Deliberately excludes everything the registry holds: no MCP server name,
/// transport, host, or credential, and composing a bundle makes no registry
/// request (spec: local-diagnostics). These are this install's own coordinates,
/// which are OAuth client-registration information rather than secrets — the
/// same values the release workflow writes into `tenant-config.json` in plain
/// text. Secrets appear only as whether one is configured.
#[derive(Debug, Default, Clone)]
pub struct ConfigurationFacts {
    /// Bundled tenant config, the process environment (debug builds only), or
    /// none, and why.
    pub source: &'static str,
    /// `None` when auth is not configured at all — itself the answer.
    pub organization: Option<String>,
    pub da_origin: Option<String>,
    pub casdoor_issuer: Option<String>,
    pub login_loopback_port: Option<u16>,
    /// One above the login port. The redirect URI an MCP sign-in needs
    /// registered, and a frequent cause of a refused sign-in.
    pub mcp_loopback_port: Option<u16>,
    /// Whether this build packages the MCP fallback client. Without it the
    /// user supplies the client details in Settings.
    pub packaged_mcp_client_id: bool,
    /// Presence only, never the value.
    pub packaged_mcp_client_secret: bool,
    /// Presence only, never the value. Decides whether MCP sign-ins are shared
    /// with the account or held per device.
    pub shared_grant_key_configured: bool,
    /// How many registered servers this device holds an operator-issued OAuth
    /// client for. A count, never an identifier: which servers an account
    /// registered is registry data this bundle deliberately excludes.
    pub device_mcp_clients: usize,
    pub signed_in: bool,
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// Compose the diagnostics bundle text from system info, app version,
/// configuration state, and recent log tails.
pub fn compose_diagnostics_bundle(app_local_data: &Path, facts: &ConfigurationFacts) -> String {
    let app_version = env!("CARGO_PKG_VERSION");
    let os_family = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let num_cpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    let now = chrono::Utc::now().to_rfc3339();

    let mut report = String::with_capacity(16 * 1024);
    report.push_str("# xFalcon Desktop Diagnostics Report\n");
    report.push_str(&format!("Generated: {now}\n\n"));
    report.push_str(
        "This report contains local application diagnostics, system environment details,\n",
    );
    report.push_str(
        "this install's tenant and sign-in configuration, recent log records, and the local\n",
    );
    report.push_str(
        "security audit trail. All credentials, API keys, tokens, and conversation message\n",
    );
    report.push_str(
        "content have been excluded or redacted, and no registered MCP server is named.\n",
    );
    report.push_str("Composing it contacts nothing.\n\n");

    report.push_str("## 1. System & Application Environment\n");
    report.push_str(&format!("- App Version: {app_version}\n"));
    report.push_str(&format!("- OS: {os_family} ({arch})\n"));
    report.push_str(&format!("- Logical CPU Cores: {num_cpus}\n"));
    report.push_str(&format!(
        "- Build Profile: {}\n\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    ));

    report.push_str("## 2. Tenant and Sign-in Configuration\n");
    report.push_str(&format!("- Configuration source: {}\n", facts.source));
    report.push_str(&format!(
        "- Organization: {}\n",
        facts.organization.as_deref().unwrap_or("(not configured)")
    ));
    report.push_str(&format!(
        "- Workspace origin: {}\n",
        facts.da_origin.as_deref().unwrap_or("(not configured)")
    ));
    report.push_str(&format!(
        "- Sign-in issuer: {}\n",
        facts
            .casdoor_issuer
            .as_deref()
            .unwrap_or("(not configured)")
    ));
    report.push_str(&format!(
        "- Signed in on this device: {}\n",
        yes_no(facts.signed_in)
    ));
    match (facts.login_loopback_port, facts.mcp_loopback_port) {
        (Some(login), Some(mcp)) => report.push_str(&format!(
            "- Loopback ports: {login} (sign-in), {mcp} (MCP sign-in)\n"
        )),
        _ => report.push_str("- Loopback ports: (not configured)\n"),
    }
    report.push_str(&format!(
        "- Packaged MCP client id: {}\n",
        yes_no(facts.packaged_mcp_client_id)
    ));
    report.push_str(&format!(
        "- Packaged MCP client secret: {}\n",
        yes_no(facts.packaged_mcp_client_secret)
    ));
    report.push_str(&format!(
        "- Shared MCP grant key configured: {}\n",
        yes_no(facts.shared_grant_key_configured)
    ));
    report.push_str(&format!(
        "- Servers with an MCP client saved on this device: {}\n",
        facts.device_mcp_clients
    ));
    // The two answers this section exists to give, spelled out: a reader
    // comparing two installs should not have to know what the lines above
    // imply.
    if !facts.packaged_mcp_client_id && facts.device_mcp_clients == 0 {
        report.push_str(
            "\n_This build packages no MCP sign-in client and none is saved on this device. Add a\n             server's OAuth client details in Settings._\n",
        );
    }
    if facts.organization.is_none() {
        report.push_str(
            "\n_Auth is not configured, so Settings shows no MCP servers regardless of what is\n             registered for the account._\n",
        );
    }
    report.push('\n');

    report.push_str("## 3. Recent Application Logs\n");
    let logs_dir = app_local_data.join("logs");
    if let Ok(entries) = fs::read_dir(&logs_dir) {
        let mut log_files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                is_active_log_file(&name)
            })
            .collect();

        // Most recent first, by the date in the file name rather than by
        // mtime. `is_active_log_file` has already guaranteed the exact
        // `xfalcon-desktop.YYYY-MM-DD.log` shape, so a descending lexical
        // sort IS a descending date sort, and it needs no filesystem
        // metadata at all.
        //
        // mtime was the previous key and was wrong twice over: files written
        // in the same instant tie, and `sort_by_key` is stable, so the
        // surviving order was `read_dir` order -- alphabetical on some
        // filesystems and hash order on others, which made the `take(7)`
        // below pick an arbitrary seven and failed
        // `test_log_rotation_bounded_to_cap` on Linux while passing on
        // Windows. mtime is also the weaker signal even when it does not
        // tie: a restored or copied log carries the wrong one, and the name
        // is what the reader is being shown. One log file per date means the
        // names are unique, so nothing can tie and sort stability is moot.
        log_files.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));

        if log_files.is_empty() {
            report.push_str("_No local log files found in log directory._\n");
        } else {
            // Include up to 7 log files, taking up to 500 lines per file
            for log_file in log_files.iter().take(7) {
                let file_name = log_file.file_name().to_string_lossy().to_string();
                report.push_str(&format!("### Log File: {file_name}\n```\n"));
                if let Ok(file) = fs::File::open(log_file.path()) {
                    let mut reader = BufReader::new(file);
                    // Use a bounded ring/deque for the last 500 lines to avoid loading unbounded files into memory
                    let mut ring = std::collections::VecDeque::with_capacity(500);
                    let mut piece = Vec::new();
                    // A line longer than LINE_CHUNK_BYTES arrives in pieces. Discard its
                    // incomplete first piece and drain the rest through the next newline.
                    let mut continuation = false;
                    loop {
                        piece.clear();
                        let read = (&mut reader)
                            .take(LINE_CHUNK_BYTES)
                            .read_until(b'\n', &mut piece);
                        match read {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                        let incomplete_line = piece.last() != Some(&b'\n')
                            && reader.fill_buf().map_or(true, |buffer| !buffer.is_empty());
                        if !continuation {
                            // A line that is not UTF-8 is read lossily and does not stop the
                            // lines after it. Complete lines are redacted before the smaller
                            // export cap; a line cut by the raw read budget is discarded.
                            let text = String::from_utf8_lossy(&piece);
                            let line = if incomplete_line {
                                sanitize_truncated_diagnostics_line(&text, 1024)
                            } else {
                                let complete_line = text.trim_end_matches(&['\r', '\n'][..]);
                                sanitize_diagnostics_prefix(complete_line, 1024)
                            };
                            if ring.len() >= 500 {
                                ring.pop_front();
                            }
                            ring.push_back(line);
                        }
                        continuation = incomplete_line;
                    }
                    let tail_content = ring.into_iter().collect::<Vec<_>>().join("\n");
                    let sanitized = sanitize_diagnostics_text(&tail_content);
                    report.push_str(&sanitized);
                } else {
                    report.push_str("_Unable to read log file contents._");
                }
                report.push_str("\n```\n\n");
            }
        }
    } else {
        report.push_str("_Log directory not accessible._\n");
    }

    report
}

/// Command: Log a renderer failure caught by an error boundary.
///
/// Strictly bounded and sanitized at ingestion to avoid memory exhaustion
/// or sensitive state leaking into log files (CWE-532).
#[tauri::command]
pub async fn diagnostics_log_renderer_error(mut report: RendererErrorReport) -> Result<(), String> {
    report.message = sanitize_diagnostics_prefix(&report.message, 2048);

    if let Some(ref mut stack) = report.component_stack {
        *stack = sanitize_diagnostics_prefix(stack, 4096);
    }

    tracing::error!(
        version = ?report.version,
        message = %report.message,
        component_stack = ?report.component_stack,
        "renderer failure caught by error boundary"
    );
    Ok(())
}

/// Reads this install's own coordinates. No registry request, no network, and
/// no secret value — see [`ConfigurationFacts`].
fn collect_configuration_facts(app: &tauri::AppHandle) -> ConfigurationFacts {
    let mut facts = ConfigurationFacts {
        source: crate::auth::tenant_config_source(),
        shared_grant_key_configured: crate::mcp::oauth::envelope::GrantKey::from_env().is_ok(),
        device_mcp_clients: app
            .try_state::<crate::mcp::commands::McpServerRuntime>()
            .map_or(0, |runtime| runtime.device_oauth_client_count()),
        signed_in: app
            .try_state::<std::sync::Arc<crate::auth::AuthSessionStore>>()
            .is_some_and(|store| store.access_token().is_some()),
        ..ConfigurationFacts::default()
    };
    // A config this build cannot resolve leaves every field below `None`, which
    // is the most useful thing the bundle can say about it.
    if let Ok(config) = crate::auth::AuthConfig::from_env() {
        facts.organization = Some(config.organization.clone());
        facts.da_origin = Some(config.da_base_url.to_string());
        facts.casdoor_issuer = Some(config.casdoor_issuer.to_string());
        facts.login_loopback_port = Some(config.loopback_port);
        facts.mcp_loopback_port = Some(crate::mcp::oauth::flow::sign_in_loopback_port(
            config.loopback_port,
        ));
        facts.packaged_mcp_client_id = config.mcp_client_id.is_some();
        facts.packaged_mcp_client_secret = config.mcp_client_secret.is_some();
    }
    facts
}

/// Section 4: the local security audit trail, redacted like the rest of the
/// report. Without a hub (the log could not be opened) it says so instead.
pub(crate) fn audit_section(hub: Option<&Arc<AuditHub>>) -> String {
    match hub {
        Some(hub) => format!(
            "\n{}",
            sanitize_diagnostics_text(&render_diagnostics_section(hub.store(), Some(hub), 200))
        ),
        None => "\n## 4. Security Audit Trail\n\n_The audit trail is not available on this device: the OS key store or the log folder could not be opened. See the application log._\n".to_owned(),
    }
}

/// Command: Generate a sanitized diagnostics bundle text report.
#[tauri::command]
pub async fn diagnostics_bundle_generate(
    app: tauri::AppHandle,
) -> Result<DiagnosticsBundleResponse, String> {
    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("failed to determine app local data directory: {e}"))?;

    let mut content =
        compose_diagnostics_bundle(&app_local_data, &collect_configuration_facts(&app));
    // Recorded before the trail is read, so the export lists itself.
    emit::record(
        &app,
        AuditAction::DiagnosticsExport,
        AuditResourceType::Diagnostics,
        "report",
        AuditOutcome::Success,
    );
    // Verifying the trail reads and MACs the whole retained log, so it runs off
    // the async worker. A panicked or cancelled render reads like a failed one.
    let audit_hub = app.try_state::<Arc<AuditHub>>().as_deref().cloned();
    let audit = tokio::task::spawn_blocking(move || audit_section(audit_hub.as_ref()))
        .await
        .unwrap_or_else(|_| {
            format!(
                "\n## 4. Security Audit Trail\n\n_The audit trail could not be read._ Error kind: {}\n",
                crate::domain::error::AppError::AuditStorageFailed.code()
            )
        });
    content.push_str(&audit);
    let suggested_filename = format!(
        "xfalcon-diagnostics-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );

    Ok(DiagnosticsBundleResponse {
        version: CONTRACT_VERSION,
        content,
        suggested_filename,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_diagnostics_sanitization_redacts_tokens_and_jwt() {
        let raw = "User auth failed: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9 and sk-1234567890abcdef1234567890 with ida_sk_9876543210fedcba and https://api.com?access_token=secret_abc and C:\\Users\\jeeva\\secret.txt and Authorization: Basic dXNlcjpwYXNz";
        let sanitized = sanitize_diagnostics_text(raw);
        assert!(!sanitized.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(!sanitized.contains("sk-1234567890abcdef1234567890"));
        assert!(!sanitized.contains("ida_sk_9876543210fedcba"));
        assert!(!sanitized.contains("secret_abc"));
        assert!(!sanitized.contains("jeeva"));
        assert!(!sanitized.contains("dXNlcjpwYXNz"));
        assert!(sanitized.contains("[REDACTED_BEARER_TOKEN]"));
        assert!(sanitized.contains("[REDACTED_API_KEY]"));
        assert!(sanitized.contains("[REDACTED_AUTH_HEADER]"));
        assert!(sanitized.contains("[USER_PATH]/[USER]"));
    }

    #[test]
    fn bounded_renderer_ingestion_redacts_a_secret_cut_at_its_limit() {
        let canary = "renderer-secret-canary-that-crosses-the-output-limit-0123456789";
        let raw = format!("{}\npassword=\"{canary}\"", "x".repeat(2000));
        assert!(raw.find(canary).expect("canary offset") < 2048);
        let sanitized = sanitize_diagnostics_prefix(&raw, 2048);
        assert!(!sanitized.contains(&canary[..37]));
        assert!(sanitized.contains("password=[REDACTED]"));
        assert!(sanitized.len() <= 2048);
    }

    #[test]
    fn truncation_marker_respects_tiny_utf8_byte_limits() {
        for limit in 0..=2 {
            let sanitized = sanitize_diagnostics_prefix("long", limit);
            assert!(sanitized.len() <= limit);
            assert!(sanitized.is_char_boundary(sanitized.len()));
        }
    }

    #[test]
    fn test_compose_diagnostics_bundle_includes_system_info_and_logs_but_not_remote_mcp_data() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs dir");

        // Seed an MCP configuration
        fs::write(
            temp.path().join("mcp_servers.json"),
            r#"{"servers":[{"name":"sqlite-explorer","transport":"stdio"}]}"#,
        )
        .expect("write mcp");

        let log_content = "2026-08-19T10:00:00Z INFO logging initialized\n2026-08-19T10:01:00Z ERROR crash simulated";
        fs::write(logs_dir.join("xfalcon-desktop.2026-08-19.log"), log_content).expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(bundle.contains("# xFalcon Desktop Diagnostics Report"));
        assert!(bundle.contains("App Version:"));
        assert!(!bundle.contains("Registered MCP Servers"));
        assert!(!bundle.contains("sqlite-explorer"));
        assert!(bundle.contains("Log File: xfalcon-desktop.2026-08-19.log"));
        assert!(bundle.contains("logging initialized"));
        assert!(bundle.contains("crash simulated"));
    }

    #[test]
    fn test_diagnostics_bundle_excludes_stored_credentials_and_tokens_even_if_planted_in_logs() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs dir");

        let secret_token = "sk-ant-api03-VERY_SECRET_KEY_NEVER_DISCLOSE";
        let bearer_jwt = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.e30.fake";
        let ida_secret = "ida_sk_SUPER_SECRET_TOKEN_VALUE_ABC";

        // Place a mock credential in credentials.json
        fs::write(
            temp.path().join("credentials.json"),
            format!("{{\"token\": \"{secret_token}\"}}"),
        )
        .expect("write creds");

        // Plant secrets directly in the log file to test the active sanitization control
        let log_content = format!(
            "2026-08-19T10:00:00Z INFO auth initialized with key {secret_token} and {bearer_jwt} and {ida_secret}\n"
        );
        fs::write(logs_dir.join("xfalcon-desktop.2026-08-19.log"), log_content).expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        // Assert none of the secret tokens survive into the bundle
        assert!(!bundle.contains(secret_token));
        assert!(!bundle.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(!bundle.contains(ida_secret));
        assert!(bundle.contains("[REDACTED_API_KEY]"));
        assert!(bundle.contains("[REDACTED_BEARER_TOKEN]"));
    }

    #[test]
    fn test_diagnostics_bundle_excludes_conversations_and_messages() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs dir");

        let private_user_message = "Confidential financial analysis message text 12345";

        // Place a conversation SQLite db or file in app data
        fs::write(temp.path().join("conversations.db"), private_user_message).expect("write conv");
        fs::write(temp.path().join("chat_history.json"), private_user_message).expect("write chat");

        // Write a log file with operational events
        fs::write(
            logs_dir.join("xfalcon-desktop.2026-08-19.log"),
            "2026-08-19T10:00:00Z INFO chat session active\n",
        )
        .expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(!bundle.contains(private_user_message));
        assert!(bundle.contains("chat session active"));
    }

    #[test]
    fn test_log_rotation_bounded_to_cap() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs");

        // Seed 10 daily log files
        for i in 1..=10 {
            fs::write(
                logs_dir.join(format!("xfalcon-desktop.2026-08-{:02}.log", i)),
                format!("Log entry for day {i}"),
            )
            .expect("write log");
        }

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        // Exactly the seven most recent days, 04 through 10, and nothing
        // older. Both boundaries are asserted for every day rather than
        // sampling one on each side: the ten files are written in the same
        // instant, so any selection keyed on mtime ties and degrades to
        // `read_dir` order, which is alphabetical on some filesystems and
        // hash order on others. A one-sample assertion passed on Windows and
        // failed on Linux for exactly that reason.
        for day in 4..=10 {
            let name = format!("Log File: xfalcon-desktop.2026-08-{day:02}.log");
            assert!(bundle.contains(&name), "expected {name} in the bundle");
        }
        for day in 1..=3 {
            let name = format!("Log File: xfalcon-desktop.2026-08-{day:02}.log");
            assert!(!bundle.contains(&name), "expected {name} to be dropped");
        }
    }

    #[test]
    fn test_is_active_log_file_validator() {
        assert!(is_active_log_file("xfalcon-desktop.2026-08-19.log"));
        assert!(is_active_log_file("xfalcon-desktop.2025-12-31.log"));
        // Legacy or non-standard format files
        assert!(!is_active_log_file("xfalcon-desktop.log.2026-08-05"));
        assert!(!is_active_log_file("xfalcon-desktop.log"));
        assert!(!is_active_log_file("other.log"));
        assert!(!is_active_log_file("xfalcon-desktop.invalid.log"));
    }

    #[test]
    fn test_legacy_log_files_pruned_and_excluded_from_bundle() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs");

        // Write legacy unredacted log files
        let legacy_content = "2026-08-05T10:00:00Z UNREDACTED legacy log with raw secrets";
        fs::write(
            logs_dir.join("xfalcon-desktop.log.2026-08-05"),
            legacy_content,
        )
        .expect("write legacy 1");
        fs::write(
            logs_dir.join("xfalcon-desktop.log.2026-08-06"),
            legacy_content,
        )
        .expect("write legacy 2");

        // Write a new active log file
        fs::write(
            logs_dir.join("xfalcon-desktop.2026-08-19.log"),
            "2026-08-19T10:00:00Z Active modern log entry",
        )
        .expect("write active");

        // 1. Assert bundle never includes the legacy files even before pruning
        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(!bundle.contains("xfalcon-desktop.log.2026-08-05"));
        assert!(!bundle.contains("UNREDACTED legacy log"));
        assert!(bundle.contains("xfalcon-desktop.2026-08-19.log"));
        assert!(bundle.contains("Active modern log entry"));

        // 2. Run startup pruning and assert legacy files are physically cleaned from disk
        assert_eq!(
            prune_legacy_log_files(&logs_dir).expect("prune legacy logs"),
            2
        );
        assert!(!logs_dir.join("xfalcon-desktop.log.2026-08-05").exists());
        assert!(!logs_dir.join("xfalcon-desktop.log.2026-08-06").exists());
        assert!(logs_dir.join("xfalcon-desktop.2026-08-19.log").exists());
    }

    #[test]
    fn test_log_tail_truncation_preserves_utf8_boundaries() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs");
        let line = format!(
            "{}🎉TRUNCATION_SENTINEL\nnext context survives\n",
            "a".repeat(1023)
        );
        fs::write(logs_dir.join("xfalcon-desktop.2026-08-19.log"), line).expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(bundle.contains(DIAGNOSTICS_TRUNCATED_MARKER));
        assert!(!bundle.contains("TRUNCATION_SENTINEL"));
        assert!(bundle.contains("next context survives"));
    }

    #[test]
    fn export_redacts_a_jwt_that_straddles_the_1024_byte_cut() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs");
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
        // The JWT starts at byte 1000. Cutting the raw line at 1024 bytes first would
        // leave an unredactable fragment. The semicolon keeps the harmless suffix
        // outside the JWT's final base64url segment, so sanitized output still exceeds
        // the 1024-byte export cap and must carry a truncation marker.
        let line = format!(
            "{}{jwt};{}\nfollowing context survives\n",
            "a".repeat(1000),
            "b".repeat(100)
        );
        fs::write(logs_dir.join("xfalcon-desktop.2026-08-19.log"), line).expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(bundle.contains(DIAGNOSTICS_TRUNCATED_MARKER));
        assert!(!bundle.contains("eyJhbGci"));
        assert!(!bundle.contains("dozjgNryP4J3jVmNHl0w5N"));
        assert!(bundle.contains(&"a".repeat(64)));
        assert!(bundle.contains("following context survives"));
    }

    #[test]
    fn export_discards_a_raw_secret_cut_at_the_log_read_budget() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs");
        let partial = "raw-boundary-secret-canary";
        let mut line = vec![b'x'; LINE_CHUNK_BYTES as usize - 21];
        line.extend_from_slice(b" password=\"");
        line.extend_from_slice(&partial.as_bytes()[..10]);
        line.extend_from_slice(&partial.as_bytes()[10..]);
        line.extend_from_slice(b"\"\nfollowing context survives\n");
        fs::write(logs_dir.join("xfalcon-desktop.2026-08-19.log"), line).expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(!bundle.contains(&partial[..10]));
        assert!(!bundle.contains(partial));
        assert!(bundle.contains(DIAGNOSTICS_TRUNCATED_MARKER));
        assert!(bundle.contains("following context survives"));
    }

    #[test]
    fn export_keeps_the_lines_after_an_invalid_utf8_line() {
        let temp = tempdir().expect("tempdir");
        let logs_dir = temp.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs");
        fs::write(
            logs_dir.join("xfalcon-desktop.2026-08-19.log"),
            b"first line ok\n\xff\xfe broken bytes\nlast line ok\n",
        )
        .expect("write log");

        let bundle = compose_diagnostics_bundle(temp.path(), &ConfigurationFacts::default());
        assert!(bundle.contains("first line ok"));
        assert!(bundle.contains("broken bytes"));
        assert!(bundle.contains("last line ok"));
    }

    #[tokio::test]
    async fn test_renderer_message_truncation_preserves_utf8_boundaries() {
        let report = RendererErrorReport {
            version: CONTRACT_VERSION,
            message: format!("{}🎉tail", "a".repeat(2047)),
            component_stack: None,
        };

        diagnostics_log_renderer_error(report)
            .await
            .expect("renderer report remains loggable");
    }

    #[tokio::test]
    async fn test_renderer_stack_truncation_preserves_utf8_boundaries() {
        let report = RendererErrorReport {
            version: CONTRACT_VERSION,
            message: "render failed".to_string(),
            component_stack: Some(format!("{}🎉tail", "a".repeat(4095))),
        };

        diagnostics_log_renderer_error(report)
            .await
            .expect("renderer stack remains loggable");
    }

    #[test]
    fn test_redaction_preserves_query_separators_and_covers_wire_field_names() {
        let raw = concat!(
            "https://api.example.test/run?a=1&token=query-secret ",
            r#"{"client_secret":"client-secret","api_key":"api-secret","refresh_token":"ab\"cd"}"#,
        );

        let sanitized = sanitize_diagnostics_text(raw);
        assert!(sanitized.contains("?a=1&token=[REDACTED]"));
        for secret in ["query-secret", "client-secret", "api-secret", "ab", "cd"] {
            assert!(
                !sanitized.contains(secret),
                "secret fragment survived: {secret}"
            );
        }
        assert!(sanitized.contains(r#""client_secret": "[REDACTED]""#));
        assert!(sanitized.contains(r#""api_key": "[REDACTED]""#));
        assert!(sanitized.contains(r#""refresh_token": "[REDACTED]""#));
    }

    #[test]
    fn mcp_metadata_redaction_stays_on_its_log_line() {
        let raw = concat!(
            "INFO rmcp::service: Service initialized as client ",
            r#"peer_info=Some(InitializeResult { server_info: "private-server", instructions: "private\nquoted\"instruction" })"#,
            "\r\nWARN xfalcon_desktop_lib::mcp::runtime: background MCP discovery failed failure=connection_failed\n",
        );
        let sanitized = sanitize_diagnostics_text(raw);
        assert!(sanitized
            .contains("Service initialized as client peer_info=[REDACTED_MCP_METADATA]\r\nWARN"));
        assert!(sanitized.contains("failure=connection_failed"));
        assert!(!sanitized.contains("private"));
        assert!(!sanitized.contains("instruction"));
    }
}

#[cfg(test)]
mod configuration_section_tests {
    use super::*;
    use tempfile::tempdir;

    fn configured() -> ConfigurationFacts {
        ConfigurationFacts {
            source: "bundled tenant config",
            organization: Some("iseyon".to_owned()),
            da_origin: Some("https://iseyon.mcp-staging.iseyon.com/".to_owned()),
            casdoor_issuer: Some("https://auth.mcp-staging.iseyon.com/".to_owned()),
            login_loopback_port: Some(19847),
            mcp_loopback_port: Some(19848),
            packaged_mcp_client_id: true,
            packaged_mcp_client_secret: true,
            shared_grant_key_configured: true,
            device_mcp_clients: 2,
            signed_in: true,
        }
    }

    /// The questions a bundle is opened to answer: which tenant this install
    /// talks to, whether it is signed in, and which sign-in path it will take.
    #[test]
    fn the_bundle_states_which_tenant_this_install_is_configured_for() {
        let temp = tempdir().expect("tempdir");
        let bundle = compose_diagnostics_bundle(temp.path(), &configured());
        assert!(bundle.contains("## 2. Tenant and Sign-in Configuration"));
        assert!(bundle.contains("bundled tenant config"));
        assert!(bundle.contains("iseyon"));
        assert!(bundle.contains("19847 (sign-in), 19848 (MCP sign-in)"));
        assert!(bundle.contains("Signed in on this device: yes"));
    }

    /// The bundle carries whether a secret is configured, never the secret. A
    /// bundle is a file a person emails to support.
    #[test]
    fn a_configured_secret_is_reported_as_presence_and_never_as_a_value() {
        let temp = tempdir().expect("tempdir");
        let bundle = compose_diagnostics_bundle(temp.path(), &configured());
        assert!(bundle.contains("Packaged MCP client secret: yes"));
        assert!(bundle.contains("Shared MCP grant key configured: yes"));
        // A count, never an identifier: which servers an account registered is
        // registry data this bundle excludes.
        assert!(bundle.contains("Servers with an MCP client saved on this device: 2"));
        // `ConfigurationFacts` has nowhere to put either value, which is the
        // guarantee; this asserts the rendering cannot reintroduce one.
        for secret in ["s3cret", "PROVIDER_KEYS_ENCRYPTION_KEY", "client_secret="] {
            assert!(!bundle.contains(secret), "{secret} leaked into the bundle");
        }
    }

    /// A build with no packaged client and nothing saved on the device directs
    /// the user to Settings. The bundle says so in words rather than leaving it
    /// to be inferred.
    #[test]
    fn a_build_with_no_packaged_mcp_client_directs_the_user_to_settings() {
        let temp = tempdir().expect("tempdir");
        let facts = ConfigurationFacts {
            packaged_mcp_client_id: false,
            packaged_mcp_client_secret: false,
            device_mcp_clients: 0,
            ..configured()
        };
        let bundle = compose_diagnostics_bundle(temp.path(), &facts);
        assert!(bundle.contains("Packaged MCP client id: no"));
        assert!(bundle.contains("Servers with an MCP client saved on this device: 0"));
        assert!(bundle.contains("Add a\n             server's OAuth client details in Settings"));
    }

    /// ...and stops directing them there once one is saved. Repeating the
    /// instruction after it has been followed is how a bundle sends a reader
    /// back to a screen they have already filled in, and hides the real cause.
    #[test]
    fn a_device_that_already_holds_a_client_is_not_told_to_add_one() {
        let temp = tempdir().expect("tempdir");
        let facts = ConfigurationFacts {
            packaged_mcp_client_id: false,
            packaged_mcp_client_secret: false,
            device_mcp_clients: 1,
            ..configured()
        };
        let bundle = compose_diagnostics_bundle(temp.path(), &facts);
        assert!(bundle.contains("Servers with an MCP client saved on this device: 1"));
        assert!(!bundle.contains("OAuth client details in Settings"));
    }

    /// The macOS-versus-Windows report this section exists for: an install that
    /// resolved no tenant shows an empty MCP list whatever the account holds,
    /// and the bundle must say that rather than leaving four blank fields.
    #[test]
    fn an_unconfigured_install_says_why_settings_is_empty() {
        let temp = tempdir().expect("tempdir");
        let facts = ConfigurationFacts {
            source: "none (no bundled config, or it was rejected)",
            ..ConfigurationFacts::default()
        };
        let bundle = compose_diagnostics_bundle(temp.path(), &facts);
        assert!(bundle.contains("Organization: (not configured)"));
        assert!(bundle.contains("Loopback ports: (not configured)"));
        assert!(bundle.contains("Settings shows no MCP servers"));
        assert!(bundle.contains("it was rejected"));
    }

    /// spec: local-diagnostics — composing a bundle names no registered MCP
    /// server and makes no registry request.
    ///
    /// `ConfigurationFacts` structurally cannot carry a registry row: it holds
    /// this install's own coordinates and three booleans. The tenant's own
    /// workspace origin is not a registered server's host, and it is the value
    /// that answers "are these two installs even pointed at the same place".
    #[test]
    fn the_configuration_section_names_no_registered_mcp_server() {
        let temp = tempdir().expect("tempdir");
        let bundle = compose_diagnostics_bundle(temp.path(), &configured());
        // A registered row's display name and endpoint reach the bundle from
        // nowhere: composition never reads the registry.
        for registry_value in [
            "acme-crm",
            "https://crm.example.com/mcp",
            "mcp.acme-crm.list_contacts",
        ] {
            assert!(
                !bundle.contains(registry_value),
                "{registry_value} reached the bundle"
            );
        }
        // What it does carry is this install's own tenant, on purpose.
        assert!(bundle.contains("https://iseyon.mcp-staging.iseyon.com/"));
    }
}

#[cfg(test)]
mod write_time_redaction_tests {
    use super::*;

    const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
    const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";

    #[test]
    fn email_addresses_are_redacted() {
        let sanitized = sanitize_diagnostics_text("who=alice.smith+tag@mail.example.co.uk ok");
        assert_eq!(sanitized, "who=[REDACTED_EMAIL] ok");
    }

    #[test]
    fn jwt_shaped_tokens_are_redacted() {
        let sanitized = sanitize_diagnostics_text(&format!("id={JWT} done"));
        assert_eq!(sanitized, "id=[REDACTED_JWT] done");
    }

    #[test]
    fn credential_query_parameters_are_redacted_with_separators_kept() {
        let raw = "https://x.test/cb?code=abc123&state=xyz&id_token=idt&refresh_token=rt\
                   &client_secret=cs&sig=s1&signature=s2&session=ss&X-Amz-Signature=s3\
                   &X-Amz-Credential=AKIA%2F20261013&keep=yes";
        let sanitized = sanitize_diagnostics_text(raw);
        assert_eq!(
            sanitized,
            "https://x.test/cb?code=[REDACTED]&state=[REDACTED]&id_token=[REDACTED]\
             &refresh_token=[REDACTED]&client_secret=[REDACTED]&sig=[REDACTED]\
             &signature=[REDACTED]&session=[REDACTED]&X-Amz-Signature=[REDACTED]\
             &X-Amz-Credential=[REDACTED]&keep=yes"
        );
    }

    #[test]
    fn credential_header_lines_are_redacted_to_end_of_line() {
        let raw = "Cookie: sid=abc; other=def\r\nset-cookie: sid=ghi; Path=/\n\
                   Proxy-Authorization: Basic xyz=\nX-Api-Key: k-123, more\n\
                   api-key: k-456\nnext line stays";
        let sanitized = sanitize_diagnostics_text(raw);
        assert_eq!(
            sanitized,
            "Cookie: [REDACTED_HEADER]\r\nset-cookie: [REDACTED_HEADER]\n\
             Proxy-Authorization: [REDACTED_HEADER]\nX-Api-Key: [REDACTED_HEADER]\n\
             api-key: [REDACTED_HEADER]\nnext line stays"
        );
    }

    #[test]
    fn near_misses_are_not_changed() {
        let raw = format!(
            "run {UUID} version 0.5.37 at /opt/app/logs/x.log, user@ and alice@localhost, \
             cookies are ok, bearer-less text"
        );
        assert_eq!(sanitize_diagnostics_text(&raw), raw);
    }

    #[test]
    fn redaction_placeholders_are_not_matched_by_later_passes() {
        let raw = format!("Cookie: a=b\n{JWT} user@example.com /home/alice/x");
        let sanitized = sanitize_diagnostics_text(&raw);
        assert_eq!(
            sanitized,
            "Cookie: [REDACTED_HEADER]\n[REDACTED_JWT] [REDACTED_EMAIL] [USER_PATH]/[USER]/x"
        );
    }

    #[test]
    fn sanitization_is_idempotent() {
        let raw = format!(
            "Bearer abcdefghijklmnopqrstuvwxyz0123456789 Authorization: Basic zz \
             sk-1234567890abcdef1234567890 ida_sk_9876543210fedcba \
             https://a.test/?token=t&code=c&X-Amz-Signature=s \
             Cookie: sid=1\nSet-Cookie: sid=2\nX-Api-Key: k\n{JWT} who@example.com \
             {{\"password\": \"p\"}} C:\\Users\\jeeva\\a.txt /home/bob/b {UUID} 0.5.37"
        );
        let once = sanitize_diagnostics_text(&raw);
        assert_eq!(sanitize_diagnostics_text(&once), once);
    }

    /// Every input the patterns below act on, run through the sanitizer twice.
    #[test]
    fn new_pattern_outputs_are_stable_under_a_second_pass() {
        let inputs = [
            "call Bearer AAAAAAAA+BBBB/CCCC== ok",
            "password=hunter2 next",
            r#"Creds { token: "abc" }"#,
            r#"refresh_token: "ab\"cd" end"#,
            "password=[REDACTED] token: [REDACTED_HEADER]",
            r#"{"headers":"Cookie: sid=abc","next":"ok"}"#,
            "Cookie: sid=\"a b\"; x=y\nnext",
            r#"{"url":"https://x/?code=abc","next":"ok"} (see https://x.test/cb?state=xyz)"#,
            "https://u:p@host.example.com/x",
            "https://example.com?email=bob@x.com",
            "josé@example.com",
            "call +1 (555) 123-4567 ok",
            "call (555) 123-4567 ok",
            "2026-10-13T07:17:34Z 07:17:34.123 0.5.37 1.2.3.4 10.0.0.5 1234567890",
            "DB_PASSWORD=hunter2 ok",
            "GITHUB_TOKEN=ghp_xxx",
            "AWS_SECRET_ACCESS_KEY=abc",
            "x_api_key=k1",
            r#"session token: Some("abc123xyz")"#,
            r#"password: Secret { inner: "hunter2" }"#,
            r#"body="{\"refresh_token\":\"rt-123\"}""#,
            "auth Bearer authentication failed",
            "Bearer AAAAAAAA+BBBB/CCCC==",
        ];
        for raw in inputs {
            let once = sanitize_diagnostics_text(raw);
            assert_eq!(sanitize_diagnostics_text(&once), once, "input: {raw}");
        }
    }

    #[test]
    fn bearer_tokens_are_redacted_only_when_they_look_like_credentials() {
        assert_eq!(
            sanitize_diagnostics_text("call Bearer AAAAAAAA+BBBB/CCCC== ok"),
            "call Bearer [REDACTED_BEARER_TOKEN] ok"
        );
        // A digit, or a token of 20 characters or more, marks a credential.
        assert_eq!(
            sanitize_diagnostics_text("x Bearer abcdefgh1 end"),
            "x Bearer [REDACTED_BEARER_TOKEN] end"
        );
        assert_eq!(
            sanitize_diagnostics_text("x Bearer abcdefghijklmnopqrst end"),
            "x Bearer [REDACTED_BEARER_TOKEN] end"
        );
        // Eight plain letters are an ordinary word, not a token.
        assert_eq!(
            sanitize_diagnostics_text("x Bearer abcdefgh end"),
            "x Bearer abcdefgh end"
        );
        // Seven characters is below the floor, so it is left alone.
        assert_eq!(
            sanitize_diagnostics_text("Bearer abcdefg end"),
            "Bearer abcdefg end"
        );
        assert_eq!(
            sanitize_diagnostics_text("auth Bearer authentication failed"),
            "auth Bearer authentication failed"
        );
    }

    #[test]
    fn credentials_with_an_underscore_prefix_or_suffix_are_redacted() {
        assert_eq!(
            sanitize_diagnostics_text("DB_PASSWORD=hunter2 ok"),
            "DB_PASSWORD=[REDACTED] ok"
        );
        assert_eq!(
            sanitize_diagnostics_text("GITHUB_TOKEN=ghp_xxx"),
            "GITHUB_TOKEN=[REDACTED]"
        );
        assert_eq!(
            sanitize_diagnostics_text("AWS_SECRET_ACCESS_KEY=abc"),
            "AWS_SECRET_ACCESS_KEY=[REDACTED]"
        );
        assert_eq!(
            sanitize_diagnostics_text("x_api_key=k1"),
            "x_api_key=[REDACTED]"
        );
    }

    #[test]
    fn debug_formatted_credential_values_are_redacted_whole() {
        assert_eq!(
            sanitize_diagnostics_text(r#"session token: Some("abc123xyz")"#),
            "session token: [REDACTED]"
        );
        assert_eq!(
            sanitize_diagnostics_text(r#"password: Secret { inner: "hunter2" }"#),
            "password: [REDACTED]"
        );
    }

    #[test]
    fn json_inside_a_string_has_its_credential_fields_redacted() {
        assert_eq!(
            sanitize_diagnostics_text(r#"body="{\"refresh_token\":\"rt-123\"}""#),
            r#"body="{\"refresh_token\": \"[REDACTED]\"}""#
        );
    }

    #[test]
    fn json_authorization_cookie_and_api_key_fields_are_redacted() {
        let line = r#"{"authorization": "Basic dXNlcjpwYXNz", "cookie": "sid=abc123", "x-api-key": "k9Lm3vQ8xR2tY7pZ", "ok": "kept"}"#;
        let redacted = sanitize_diagnostics_text(line);
        for secret in ["dXNlcjpwYXNz", "sid=abc123", "k9Lm3vQ8xR2tY7pZ"] {
            assert!(!redacted.contains(secret), "{secret} leaked: {redacted}");
        }
        assert!(redacted.contains(r#""ok": "kept""#), "{redacted}");
        assert_eq!(sanitize_diagnostics_text(&redacted), redacted, "idempotent");
        // The same fields inside a string-encoded body.
        let escaped = sanitize_diagnostics_text(r#"body="{\"authorization\":\"Bearer abc.def\"}""#);
        assert!(!escaped.contains("abc.def"), "{escaped}");
    }

    #[test]
    fn an_authorization_scheme_is_redacted_with_its_credential() {
        assert_eq!(
            sanitize_diagnostics_text("authorization=Basic dXNlcjpwYXNzd29yZA== next"),
            "authorization=[REDACTED] next"
        );
        assert_eq!(
            sanitize_diagnostics_text("token: Bearer abc123"),
            "token: [REDACTED]"
        );
    }

    #[test]
    fn plain_key_value_credentials_are_redacted_keeping_the_name_and_separator() {
        assert_eq!(
            sanitize_diagnostics_text("password=hunter2 next"),
            "password=[REDACTED] next"
        );
        assert_eq!(
            sanitize_diagnostics_text(r#"Creds { token: "abc" }"#),
            "Creds { token: [REDACTED] }"
        );
        assert_eq!(
            sanitize_diagnostics_text("api_key = abc123, ok"),
            "api_key = [REDACTED], ok"
        );
        assert_eq!(
            sanitize_diagnostics_text(r#"refresh_token: "ab\"cd" end"#),
            "refresh_token: [REDACTED] end"
        );
        assert_eq!(
            sanitize_diagnostics_text("login cookie=sid123;theme=dark"),
            "login cookie=[REDACTED];theme=dark"
        );
    }

    #[test]
    fn key_value_near_misses_and_placeholders_are_not_changed() {
        let raw = "tokens_used=123 token_count=5 keyboard=us monkey=1 mysecret=x \
                   secret_sauce=y password=[REDACTED] token: [REDACTED_HEADER]";
        assert_eq!(sanitize_diagnostics_text(raw), raw);
    }

    #[test]
    fn json_quoted_credentials_are_redacted_by_the_body_field_pass() {
        assert_eq!(
            sanitize_diagnostics_text(r#"{"password": "p1", "token": "t1"}"#),
            r#"{"password": "[REDACTED]", "token": "[REDACTED]"}"#
        );
    }

    #[test]
    fn header_values_in_json_stop_at_the_closing_quote() {
        assert_eq!(
            sanitize_diagnostics_text(r#"{"headers":"Cookie: sid=abc","next":"ok"}"#),
            r#"{"headers":"Cookie: [REDACTED_HEADER]","next":"ok"}"#
        );
        assert_eq!(
            sanitize_diagnostics_text(r#"{"x":"api-key: k1"}"#),
            r#"{"x":"api-key: [REDACTED_HEADER]"}"#
        );
    }

    #[test]
    fn a_quoted_cookie_value_on_a_plain_line_is_redacted_to_end_of_line() {
        assert_eq!(
            sanitize_diagnostics_text("Cookie: sid=\"a b\"; x=y\nnext"),
            "Cookie: [REDACTED_HEADER]\nnext"
        );
    }

    #[test]
    fn query_values_stop_at_quotes_and_closing_brackets() {
        assert_eq!(
            sanitize_diagnostics_text(
                r#"{"url":"https://x/?code=abc","next":"ok"} (see https://x.test/cb?state=xyz)"#
            ),
            r#"{"url":"https://x/?code=[REDACTED]","next":"ok"} (see https://x.test/cb?state=[REDACTED])"#
        );
    }

    #[test]
    fn url_userinfo_is_redacted_with_and_without_a_password() {
        assert_eq!(
            sanitize_diagnostics_text("https://u:p@host.example.com/x"),
            "https://[REDACTED_USERINFO]@host.example.com/x"
        );
        assert_eq!(
            sanitize_diagnostics_text("open https://alice@host.example.com/x ok"),
            "open https://[REDACTED_USERINFO]@host.example.com/x ok"
        );
    }

    #[test]
    fn urls_without_userinfo_are_not_changed() {
        let raw = "https://host.example.com/a?b=c";
        assert_eq!(sanitize_diagnostics_text(raw), raw);
        // An `@` after `?` or inside a JSON string is not userinfo; the email pass
        // still redacts the address itself.
        assert_eq!(
            sanitize_diagnostics_text("https://example.com?email=bob@x.com"),
            "https://example.com?email=[REDACTED_EMAIL]"
        );
        assert_eq!(
            sanitize_diagnostics_text(r#"{"url":"https://x","user":"a@b.com"}"#),
            r#"{"url":"https://x","user":"[REDACTED_EMAIL]"}"#
        );
    }

    #[test]
    fn email_addresses_with_unicode_local_parts_are_redacted() {
        assert_eq!(
            sanitize_diagnostics_text("josé@example.com"),
            "[REDACTED_EMAIL]"
        );
        assert_eq!(
            sanitize_diagnostics_text("who=josé.López@mail.example.org ok"),
            "who=[REDACTED_EMAIL] ok"
        );
    }

    #[test]
    fn phone_numbers_are_redacted() {
        for phone in [
            "+1 (555) 123-4567",
            "+44 20 7946 0958",
            "(555) 123-4567",
            "555-123-4567",
            "555.123.4567",
        ] {
            assert_eq!(
                sanitize_diagnostics_text(&format!("call {phone} ok")),
                "call [REDACTED_PHONE] ok",
                "phone: {phone}"
            );
        }
    }

    #[test]
    fn timestamps_dates_ids_versions_addresses_and_numbers_are_not_phone_numbers() {
        let raw = format!(
            "2026-10-13T07:17:34Z 07:17:34.123 2026-10-13 {UUID} 0.5.37 1.2.3.4 \
             10.0.0.5 203.0.113.10 1234567890 status=404 port :9000 req-42 \
             9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
        assert_eq!(sanitize_diagnostics_text(&raw), raw);
    }

    /// The trail is the last section, and says why it is empty when the log
    /// could not be opened, rather than leaving a silent gap in the report.
    #[test]
    fn the_audit_section_says_so_when_the_log_could_not_be_opened() {
        let section = audit_section(None);
        assert!(section.contains("## 4. Security Audit Trail"));
        assert!(section.contains("not available on this device"));
    }

    /// The export lists itself (the caller records it first), and an identifier
    /// shaped like a token is redacted rather than shown.
    #[test]
    fn the_audit_section_lists_events_and_redacts_token_shaped_identifiers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = crate::audit::LocalAuditStore::open(
            temp.path().join("audit"),
            std::sync::Arc::new(crate::persistence::InMemorySecretStore::default()),
        )
        .expect("open audit log");
        let hub = std::sync::Arc::new(crate::audit::AuditHub::new(std::sync::Arc::new(store)));
        hub.record(
            crate::audit::AuditAction::DiagnosticsExport,
            crate::audit::AuditResourceType::Diagnostics,
            "report",
            crate::audit::AuditOutcome::Success,
            "desktop.app.v1",
            "app",
        );
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
        hub.record(
            crate::audit::AuditAction::ToolDeny,
            crate::audit::AuditResourceType::ToolCall,
            token,
            crate::audit::AuditOutcome::Rejected,
            "tools.policy.v1",
            "run-1",
        );

        let section = audit_section(Some(&hub));

        assert!(section.contains("## 4. Security Audit Trail"));
        assert!(section.contains("diagnostics.export diagnostics:report outcome=success"));
        assert!(section.contains("_REDACTED_JWT_"), "{section}");
        assert!(!section.contains(token));
        assert!(!section.contains("eyJhbGci"));
    }
}
