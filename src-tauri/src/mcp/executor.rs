//! call/cancel/progress/result normalization for bundled DA MCP tool calls.
//!
//! Normalizes `rmcp`'s `CallToolResult` content union (text | image | audio |
//! resource | resource_link) into a bounded [`McpToolResult`]: text is
//! retained exactly up to the shared 8 MiB tool-result ceiling, inline base64
//! image/audio bytes are summarized (mime type + declared length) rather
//! than copied into memory once they exceed a small inline threshold, and
//! resource/resource-link content — which already carries a DA-hosted URI —
//! is kept as a reference only, never fetched or inlined. This is the
//! "persist full large results in DA references rather than model/UI
//! memory" requirement from
//! `docs/decisions.md` (Task 2, Step
//! 4): Task 2 has no DA reference *store* of its own to write into (that is
//! out of scope here), so the bound this module can actually enforce is
//! "never let a resource's bytes reach model/UI memory through this path" —
//! the URI is retained so a later task's reference store can resolve it.
//!
//! An unrecognized `ContentBlock` variant is rejected for free: `rmcp`'s
//! typed model deserializes the wire union strictly, so a `type` tag this
//! `rmcp` release does not know surfaces as a deserialize error from the
//! `tools/call` response itself (mapped to [`McpExecutorError::Call`] below)
//! rather than reaching this normalizer at all.

use rmcp::model::{
    CallToolRequest, CallToolResult, CancelledNotification, CancelledNotificationParam,
    ClientRequest, ContentBlock, ResourceContents, ServerResult,
};
use rmcp::service::PeerRequestOptions;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::tools::MAX_EXACT_TOOL_RESULT_BYTES;

use super::catalog::{ArgumentsError, McpToolCatalog};
use super::client::{McpNotifications, McpRateLimit, McpRefusal, McpSession, ProgressSnapshot};

const MAX_REFERENCE_ITEMS: usize = 64;
/// Inline base64 image/audio content has no URI to keep as a reference —
/// beyond this threshold the bytes are dropped (never retained), not
/// truncated-and-kept, since a partial base64 payload is not useful.
const MAX_INLINE_BINARY_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpResourceRef {
    uri: String,
    mime_type: Option<String>,
    name: Option<String>,
}

impl McpResourceRef {
    #[must_use]
    pub(crate) fn uri(&self) -> &str {
        &self.uri
    }

    #[must_use]
    pub(crate) fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }

    #[must_use]
    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum McpContentItem {
    Text(String),
    /// Inline binary content summarized without retaining its bytes.
    Binary {
        mime_type: String,
        byte_length: usize,
    },
    Reference(McpResourceRef),
}

/// A bounded, normalized tool result. Mirrors the shape (and bounding
/// discipline) of `crate::tools::invocation::ToolResult` without importing
/// it: Task 2 is a standalone client module and does not wire into
/// `crate::tools` (that integration is a later task).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct McpToolResult {
    content: Vec<McpContentItem>,
    structured: Option<Value>,
    is_error: bool,
    truncated: bool,
}

impl McpToolResult {
    #[must_use]
    pub(crate) fn content(&self) -> &[McpContentItem] {
        &self.content
    }

    #[must_use]
    pub(crate) fn structured(&self) -> Option<&Value> {
        self.structured.as_ref()
    }

    #[must_use]
    pub(crate) fn is_error(&self) -> bool {
        self.is_error
    }

    #[must_use]
    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    /// Exact JSON value handed to `ToolResult` by the MCP dispatcher.
    /// Keeping this shape here lets the executor enforce the same byte count
    /// before dispatch without maintaining a second estimate.
    #[must_use]
    pub(crate) fn exact_structured_value(&self) -> Value {
        let text = self
            .content
            .iter()
            .filter_map(|item| match item {
                McpContentItem::Text(value) => Some(value.clone()),
                McpContentItem::Binary { .. } | McpContentItem::Reference(_) => None,
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "is_error": self.is_error,
            "text": text,
            "structured": self.structured.clone().unwrap_or(Value::Null),
            "truncated": self.truncated,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpExecutorError {
    #[error("unknown mcp tool {0:?}")]
    UnknownTool(String),
    #[error("tool arguments must be a JSON object")]
    ArgumentsNotObject,
    #[error("tool arguments do not match the advertised schema")]
    SchemaViolation,
    #[error("cancelled")]
    Cancelled,
    #[error("structured MCP tool result is {actual_bytes} bytes; maximum is {max_bytes} bytes")]
    ResultTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    /// The server refused the credential while answering this call — a grant
    /// that expired or was revoked after the session was established.
    ///
    /// Separate from [`Self::Call`] because the recovery is different in kind:
    /// a transport fault is worth reconnecting, while a refused credential is
    /// worth asking the user to sign in, and reconnecting on one attaches the
    /// same refused token again.
    #[error("the server requires a new authorization")]
    AuthorizationRefused(McpRefusal),
    /// The server answered HTTP 429. The connection is fine; the account sent
    /// too many requests, so this is neither a transport fault to reconnect
    /// from nor a call that will succeed if repeated at once.
    #[error("the server is rate limiting this account")]
    RateLimited(McpRateLimit),
    #[error("mcp tool call failed: {0}")]
    Call(String),
}

impl From<ArgumentsError> for McpExecutorError {
    fn from(value: ArgumentsError) -> Self {
        match value {
            ArgumentsError::UnknownTool(name) => Self::UnknownTool(name),
            ArgumentsError::SchemaViolation => Self::SchemaViolation,
        }
    }
}

/// Executes bounded, cancellable tool calls against one already-connected,
/// already-cataloged session.
pub(crate) struct McpExecutor<'a> {
    session: &'a McpSession,
    catalog: &'a McpToolCatalog,
}

/// Removes one call's progress slot on every exit, including when an outer
/// timeout drops the executor future during bounded cancellation cleanup.
struct ProgressRegistration<'a> {
    notifications: &'a McpNotifications,
    token: rmcp::model::ProgressToken,
}

impl Drop for ProgressRegistration<'_> {
    fn drop(&mut self) {
        self.notifications.clear_progress(&self.token);
    }
}

impl<'a> McpExecutor<'a> {
    #[must_use]
    pub(crate) fn new(session: &'a McpSession, catalog: &'a McpToolCatalog) -> Self {
        Self { session, catalog }
    }

    /// The most recent progress snapshot reported for a specific in-flight
    /// call, if any arrived since the last read.
    #[must_use]
    pub(crate) fn progress(&self, token: &rmcp::model::ProgressToken) -> Option<ProgressSnapshot> {
        self.session.notifications().progress_for(token)
    }

    /// Calls `name` with `arguments`, validating first against the catalog's
    /// compiled schema (so a model/skill can never send arguments the
    /// advertised contract rejects), then racing the in-flight request
    /// against `cancel` — cancellation always wins ties (`biased`), and a
    /// cancellation sends the server a real `notifications/cancelled` rather
    /// than merely abandoning the local future.
    /// Classifies one failed `tools/call`.
    ///
    /// The error `rmcp` hands back says only that the request did not complete;
    /// what the server *answered* is in the challenge the transport recorded.
    /// Asking the session first is what keeps a refused credential from being
    /// reported as a broken pipe — and the check is a take, so it only ever
    /// attributes a refusal to the failure that actually caused it.
    fn call_failure(&self, error: &dyn std::fmt::Display) -> McpExecutorError {
        if let Some(refusal) = self.session.take_refusal() {
            return McpExecutorError::AuthorizationRefused(refusal);
        }
        match self.session.take_rate_limit() {
            Some(limit) => McpExecutorError::RateLimited(limit),
            None => McpExecutorError::Call(error.to_string()),
        }
    }

    pub(crate) async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<McpToolResult, McpExecutorError> {
        if cancel.is_cancelled() {
            return Err(McpExecutorError::Cancelled);
        }
        self.catalog.validate_arguments(name, &arguments)?;

        let arguments_map = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => return Err(McpExecutorError::ArgumentsNotObject),
        };

        let mut params = rmcp::model::CallToolRequestParams::new(name.to_owned());
        if let Some(map) = arguments_map {
            params = params.with_arguments(map);
        }
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));

        let handle = self
            .session
            .peer()
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|error| self.call_failure(&error))?;

        let progress_token = handle.progress_token;
        let request_id = handle.id;
        let peer = handle.peer.clone();
        let mut response_rx = handle.rx;

        // Registering before racing the response is what makes this call's
        // progress notifications acceptable at all: `on_progress` only ever
        // stores an update for a token `register_progress` already
        // registered, so this must happen before the server could possibly
        // have anything to say about `progress_token`.
        self.session
            .notifications()
            .register_progress(progress_token.clone());
        let _progress_registration = ProgressRegistration {
            notifications: self.session.notifications(),
            token: progress_token,
        };

        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                let notification = CancelledNotification::new(CancelledNotificationParam::new(
                    Some(request_id),
                    Some("cancelled by caller".to_owned()),
                ));
                let _ = peer.send_notification(notification.into()).await;
                return Err(McpExecutorError::Cancelled);
            }
            response = &mut response_rx => response,
        };

        let response = outcome
            .map_err(|_| self.call_failure(&"mcp transport closed"))?
            .map_err(|error| self.call_failure(&error))?;
        let ServerResult::CallToolResult(result) = response else {
            return Err(McpExecutorError::Call(
                "bundled server returned an unexpected response to tools/call".to_owned(),
            ));
        };
        normalize(result)
    }
}

fn normalize(result: CallToolResult) -> Result<McpToolResult, McpExecutorError> {
    let mut truncated = false;
    let mut content = Vec::new();

    for block in result.content {
        match block {
            ContentBlock::Text(text) => {
                content.push(McpContentItem::Text(text.text));
            }
            ContentBlock::Image(image) => {
                let byte_length = image.data.len();
                if byte_length > MAX_INLINE_BINARY_BYTES {
                    truncated = true;
                }
                content.push(McpContentItem::Binary {
                    mime_type: image.mime_type,
                    byte_length,
                });
            }
            ContentBlock::Audio(audio) => {
                let byte_length = audio.data.len();
                if byte_length > MAX_INLINE_BINARY_BYTES {
                    truncated = true;
                }
                content.push(McpContentItem::Binary {
                    mime_type: audio.mime_type,
                    byte_length,
                });
            }
            ContentBlock::Resource(embedded) => {
                if reference_count(&content) >= MAX_REFERENCE_ITEMS {
                    truncated = true;
                    continue;
                }
                content.push(McpContentItem::Reference(resource_contents_ref(
                    embedded.resource,
                )));
            }
            ContentBlock::ResourceLink(resource) => {
                if reference_count(&content) >= MAX_REFERENCE_ITEMS {
                    truncated = true;
                    continue;
                }
                content.push(McpContentItem::Reference(McpResourceRef {
                    uri: resource.uri,
                    mime_type: resource.mime_type,
                    name: Some(resource.name),
                }));
            }
            // `ContentBlock` is `#[non_exhaustive]`: a future MCP revision
            // may add a content kind this pinned `rmcp` release does not
            // know about. In practice this arm is unreachable today — an
            // unrecognized `type` tag fails to deserialize out of the
            // `tools/call` response before it ever reaches this normalizer
            // (see the module doc comment) — but the fallback is the same
            // fail-closed choice as every bound above: drop it and mark the
            // result truncated rather than guess at its shape.
            _ => {
                truncated = true;
            }
        }
    }

    let normalized = McpToolResult {
        content,
        structured: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
        truncated,
    };
    let actual_bytes = serde_json::to_vec(&normalized.exact_structured_value())
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if actual_bytes > MAX_EXACT_TOOL_RESULT_BYTES {
        return Err(McpExecutorError::ResultTooLarge {
            actual_bytes,
            max_bytes: MAX_EXACT_TOOL_RESULT_BYTES,
        });
    }
    Ok(normalized)
}

fn reference_count(content: &[McpContentItem]) -> usize {
    content
        .iter()
        .filter(|item| matches!(item, McpContentItem::Reference(_)))
        .count()
}

fn resource_contents_ref(resource: ResourceContents) -> McpResourceRef {
    match resource {
        ResourceContents::TextResourceContents { uri, mime_type, .. }
        | ResourceContents::BlobResourceContents { uri, mime_type, .. } => McpResourceRef {
            uri,
            mime_type,
            name: None,
        },
        // Same fail-closed rationale as `ContentBlock`'s wildcard arm above:
        // `ResourceContents` is `#[non_exhaustive]` too.
        _ => McpResourceRef {
            uri: String::new(),
            mime_type: None,
            name: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{AudioContent, EmbeddedResource, ImageContent, Resource, TextContent};

    #[test]
    fn keeps_text_above_the_old_64_kib_limit_exact() {
        let large = "x".repeat(128 * 1024);
        let result = normalize(CallToolResult::success(vec![ContentBlock::Text(
            TextContent::new(large.clone()),
        )]))
        .expect("128 KiB is below the exact result ceiling");
        assert_eq!(result.content().len(), 1);
        match &result.content()[0] {
            McpContentItem::Text(text) => assert_eq!(text, &large),
            other => panic!("expected text, got {other:?}"),
        }
        assert!(!result.truncated());
    }

    #[test]
    fn keeps_small_text_untouched() {
        let result = normalize(CallToolResult::success(vec![ContentBlock::Text(
            TextContent::new("hello"),
        )]))
        .expect("small result");
        assert_eq!(
            result.content(),
            &[McpContentItem::Text("hello".to_owned())]
        );
        assert!(!result.truncated());
    }

    #[test]
    fn summarizes_inline_binary_without_retaining_bytes_over_cap() {
        let oversized_data = "a".repeat(MAX_INLINE_BINARY_BYTES + 100);
        let result = normalize(CallToolResult::success(vec![ContentBlock::Image(
            ImageContent::new(oversized_data.clone(), "image/png"),
        )]))
        .expect("binary content is represented by metadata");
        match &result.content()[0] {
            McpContentItem::Binary {
                mime_type,
                byte_length,
            } => {
                assert_eq!(mime_type, "image/png");
                assert_eq!(*byte_length, oversized_data.len());
            }
            other => panic!("expected binary summary, got {other:?}"),
        }
        assert!(result.truncated());
    }

    #[test]
    fn small_inline_binary_is_not_marked_truncated() {
        let result = normalize(CallToolResult::success(vec![ContentBlock::Audio(
            AudioContent::new("small", "audio/wav"),
        )]))
        .expect("small binary summary");
        assert!(!result.truncated());
    }

    #[test]
    fn keeps_embedded_resource_as_reference_never_inlining_bytes() {
        let huge_blob = "b".repeat(1024 * 1024);
        let result = normalize(CallToolResult::success(vec![ContentBlock::Resource(
            EmbeddedResource::new(ResourceContents::BlobResourceContents {
                uri: "da://reports/huge-report.pdf".to_owned(),
                mime_type: Some("application/pdf".to_owned()),
                blob: huge_blob,
                meta: None,
            }),
        )]))
        .expect("embedded bytes remain behind their reference");
        match &result.content()[0] {
            McpContentItem::Reference(reference) => {
                assert_eq!(reference.uri(), "da://reports/huge-report.pdf");
                assert_eq!(reference.mime_type(), Some("application/pdf"));
            }
            other => panic!("expected a reference, got {other:?}"),
        }
        // A reference never inlines bytes, so it never trips truncation.
        assert!(!result.truncated());
    }

    #[test]
    fn keeps_resource_link_as_reference() {
        let result = normalize(CallToolResult::success(vec![ContentBlock::ResourceLink(
            Resource::new("da://reports/summary.csv", "summary.csv")
                .with_mime_type("text/csv")
                .with_size(9_999_999),
        )]))
        .expect("resource link remains a reference");
        match &result.content()[0] {
            McpContentItem::Reference(reference) => {
                assert_eq!(reference.uri(), "da://reports/summary.csv");
                assert_eq!(reference.name(), Some("summary.csv"));
            }
            other => panic!("expected a reference, got {other:?}"),
        }
    }

    #[test]
    fn structured_content_above_eight_mib_fails_before_dispatch() {
        let huge = Value::String("z".repeat(MAX_EXACT_TOOL_RESULT_BYTES));
        let error = normalize(CallToolResult::structured(huge))
            .expect_err("oversized exact content must not become null");
        assert!(matches!(
            error,
            McpExecutorError::ResultTooLarge {
                actual_bytes,
                max_bytes: MAX_EXACT_TOOL_RESULT_BYTES,
            } if actual_bytes > MAX_EXACT_TOOL_RESULT_BYTES
        ));
    }

    #[test]
    fn keeps_multi_megabyte_structured_content_exact() {
        // `CallToolResult::structured` includes both a text rendering and the
        // structured field, so this produces an exact envelope above 6 MiB.
        let value = Value::String("z".repeat(3 * 1024 * 1024));
        let result = normalize(CallToolResult::structured(value.clone()))
            .expect("multi-megabyte result remains below exact ceiling");
        assert_eq!(result.structured(), Some(&value));
        assert!(!result.truncated());
    }

    #[test]
    fn keeps_small_structured_content() {
        let value = serde_json::json!({"ok": true});
        let result = normalize(CallToolResult::structured(value.clone())).expect("small result");
        assert_eq!(result.structured(), Some(&value));
        assert!(!result.truncated());
    }

    #[test]
    fn preserves_is_error_flag() {
        let result = normalize(CallToolResult::error(vec![ContentBlock::text("failed")]))
            .expect("small error result");
        assert!(result.is_error());
    }

    #[test]
    fn retains_all_text_items_below_the_exact_byte_ceiling() {
        let blocks: Vec<ContentBlock> = (0..100)
            .map(|index| ContentBlock::text(format!("item {index}")))
            .collect();
        let result = normalize(CallToolResult::success(blocks)).expect("small aggregate result");
        assert_eq!(result.content().len(), 100);
        assert!(!result.truncated());
    }
}
