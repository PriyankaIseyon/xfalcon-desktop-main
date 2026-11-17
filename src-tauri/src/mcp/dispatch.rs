//! Executes registry tool calls against whichever MCP server published them.
//!
//! The counterpart to `crate::tools::dispatch::ProjectToolDispatcher`: same
//! `crate::agent::tool_loop::ToolExecutorPort` seam, different tool source.
//! `crate::agent::tool_session` composes the two behind one executor and
//! routes by tool id; this module routes the MCP half again, by the server slug
//! inside that id.
//!
//! # A tool result is untrusted content
//!
//! Whatever comes back here is model-visible the instant it is returned
//! (`docs/threat-model.md` TM-006). It is normalized into a
//! `crate::tools::invocation::ToolResult` — bounded by that type's own
//! ceilings — and inline binary content is summarized rather than carried, so
//! a server cannot push arbitrary bytes into the model's context through a
//! tool result. Nothing here reads a result back into a policy decision.

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::mcp::{McpReauthReason, McpReauthRequired};
use xfalcon_contracts::tools::{ToolId, ToolSource};

use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort};
use crate::tools::invocation::{ContentRef, ToolResult};

use super::client::{McpClientError, McpRateLimit, McpRefusal};
use super::executor::{McpContentItem, McpExecutor, McpExecutorError, McpToolResult};
use super::runtime::{
    mcp_tool_route, McpRuntime, McpRuntimeError, McpRuntimeSet, SessionInvalidation,
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The wait before the one retry when a 429 names no `Retry-After`.
const RATE_LIMIT_DEFAULT_WAIT: Duration = Duration::from_secs(5);
/// The longest `Retry-After` waited out inside a call. Longer waits go back to
/// the model as a rate-limit failure.
const RATE_LIMIT_MAX_WAIT: Duration = Duration::from_secs(30);

/// Where a "this server needs a new sign-in" prompt goes.
///
/// A port rather than an `AppHandle` so the classification above can be tested
/// without a Tauri runtime, and so nothing in this module has to know that the
/// destination is a window.
pub(crate) trait ReauthNotifier: Send + Sync {
    fn reauth_required(&self, prompt: McpReauthRequired);
}

/// The event a window listens for. Named like the other frontend-facing
/// channels so a reader greps one convention.
pub(crate) const REAUTH_EVENT: &str = "mcp:reauth-required";

/// Emits the prompt to the frontend.
///
/// Best-effort: a window that has gone away cannot be asked anything, and
/// failing the tool call over it would replace a recoverable "sign in again"
/// with an unrecoverable error. The refusal itself is already reported through
/// the returned `ToolExecutionError` either way.
pub(crate) struct WindowReauthNotifier {
    app: tauri::AppHandle,
}

impl WindowReauthNotifier {
    #[must_use]
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl ReauthNotifier for WindowReauthNotifier {
    fn reauth_required(&self, prompt: McpReauthRequired) {
        use tauri::Emitter as _;
        if let Err(error) = self.app.emit(REAUTH_EVENT, &prompt) {
            tracing::warn!(%error, "could not raise the MCP reconnect prompt");
        }
    }
}

pub(crate) struct McpToolDispatcher {
    servers: Arc<McpRuntimeSet>,
    reauth: Option<Arc<dyn ReauthNotifier>>,
    /// Servers already prompted for during this run.
    ///
    /// A run can fail several calls against one server — a round with three
    /// tool calls fails all three — and each would otherwise raise its own
    /// prompt. Deduping here rather than in the UI keeps the guarantee where the
    /// count is known.
    prompted: Mutex<BTreeSet<String>>,
}

impl McpToolDispatcher {
    #[must_use]
    pub(crate) fn new(servers: Arc<McpRuntimeSet>) -> Self {
        Self {
            servers,
            reauth: None,
            prompted: Mutex::new(BTreeSet::new()),
        }
    }

    #[must_use]
    pub(crate) fn with_reauth_notifier(mut self, notifier: Arc<dyn ReauthNotifier>) -> Self {
        self.reauth = Some(notifier);
        self
    }

    fn refusal_error(&self, runtime: &McpRuntime) -> ToolExecutionError {
        ToolExecutionError::AuthorizationRefused {
            label: runtime.identity().map_or_else(
                || runtime.slug().to_owned(),
                |identity| identity.label.clone(),
            ),
        }
    }

    /// Turns a current session's refusal into the one thing the user can act on.
    ///
    /// Three steps, in this order: forget the refused credential, prompt for a
    /// replacement, and report the failure as terminal. The eviction comes first
    /// because it is what makes the prompt's reconnect meaningful — a retained
    /// grant would be reattached and refused again.
    ///
    /// A server with no DA row (the bundled one) is evicted but not prompted
    /// for: its credential is the app's own session, and offering to reconnect a
    /// server would point the user at something that cannot fix it.
    fn on_refusal(&self, runtime: &McpRuntime, refusal: &McpRefusal) -> ToolExecutionError {
        runtime.forget_credential();

        let Some(identity) = runtime.identity() else {
            return self.refusal_error(runtime);
        };

        let first_time = self
            .prompted
            .lock()
            .map(|mut seen| seen.insert(identity.id.clone()))
            // A poisoned lock must not cost the user the prompt; at worst they
            // see it twice, which beats not being told at all.
            .unwrap_or(true);
        if first_time {
            if let Some(notifier) = &self.reauth {
                notifier.reauth_required(McpReauthRequired::new(
                    identity.id.clone(),
                    identity.label.clone(),
                    match refusal {
                        McpRefusal::AuthorizationRequired => McpReauthReason::CredentialRefused,
                        McpRefusal::InsufficientScope { .. } => McpReauthReason::InsufficientScope,
                    },
                ));
            }
        }

        self.refusal_error(runtime)
    }
}

#[async_trait]
impl ToolExecutorPort for McpToolDispatcher {
    async fn execute(
        &self,
        tool_id: &ToolId,
        arguments: &Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolExecutionError> {
        let Some((slug, name)) = mcp_tool_route(tool_id.as_str()) else {
            return Err(ToolExecutionError::Failed(format!(
                "{} is not a namespaced MCP tool id",
                tool_id.as_str()
            )));
        };
        // A run only ever declares tools this set registered, so an unknown slug
        // means the id did not come from this run's registry. Refusing beats
        // guessing which server was meant.
        let Some(runtime) = self.servers.find(slug) else {
            return Err(ToolExecutionError::Failed(format!(
                "{} names no MCP server this run connected",
                tool_id.as_str()
            )));
        };

        // Establishing the session is the first place a stale grant shows up:
        // the credential is attached here, so a server that has stopped
        // accepting it refuses before any tool is called. Classified rather than
        // stringified, or an expired grant reads as "something went wrong".
        let (session, catalog) = match runtime
            .connected_with_failure_action(&cancel, |error| {
                if let Some(refusal) = refusal_of(error) {
                    self.on_refusal(runtime.as_ref(), &refusal);
                }
            })
            .await
        {
            Ok(connected) => connected,
            Err(error) => {
                // A connect the server rate limited is the model's rate limit,
                // waited out by the tool loop, not a second wait here.
                return Err(match (refusal_of(&error), rate_limit_of(&error)) {
                    (Some(_), _) => self.refusal_error(runtime.as_ref()),
                    (None, Some(limit)) => ToolExecutionError::RateLimited {
                        retry_after: limit.retry_after,
                    },
                    (None, None) => ToolExecutionError::Failed(format!("{error}")),
                });
            }
        };

        // A rate limit with a short `Retry-After` is waited out once, inside
        // the call's own time budget: the model has no way to wait, so handing
        // it the failure at once only buys a second refusal.
        let mut waited = false;
        let outcome = loop {
            let outcome = McpExecutor::new(session.as_ref(), catalog.as_ref())
                .call_tool(name, arguments.clone(), &cancel)
                .await;
            if let Err(McpExecutorError::RateLimited(limit)) = &outcome {
                let wait = limit.retry_after.unwrap_or(RATE_LIMIT_DEFAULT_WAIT);
                if !waited && wait <= RATE_LIMIT_MAX_WAIT {
                    waited = true;
                    tokio::select! {
                        biased;
                        () = cancel.cancelled() => return Err(ToolExecutionError::Cancelled),
                        () = tokio::time::sleep(wait) => {}
                    }
                    continue;
                }
            }
            break outcome;
        };

        match outcome {
            Ok(result) => normalize(&result),
            Err(McpExecutorError::Cancelled) => Err(ToolExecutionError::Cancelled),
            Err(McpExecutorError::ResultTooLarge {
                actual_bytes,
                max_bytes,
            }) => Err(ToolExecutionError::ResultTooLarge {
                actual_bytes,
                max_bytes,
            }),
            // A refusal from the latest generation needs reauthorization:
            // reconnecting with its refused credential cannot fix it. The
            // generation check keeps that terminal outcome tied to the session.
            Err(McpExecutorError::AuthorizationRefused(refusal)) => {
                match runtime
                    .invalidate_with(&session, || {
                        self.on_refusal(runtime.as_ref(), &refusal);
                    })
                    .await
                {
                    SessionInvalidation::LatestGeneration => {
                        Err(self.refusal_error(runtime.as_ref()))
                    }
                    // A newer session has connected. Let the normal retry
                    // path use it without making this old refusal terminal.
                    SessionInvalidation::StaleGeneration => Err(ToolExecutionError::Disconnected),
                }
            }
            // The session is healthy, so it is kept: dropping it would add a
            // reconnect to the load that caused the limit.
            Err(McpExecutorError::RateLimited(limit)) => Err(ToolExecutionError::RateLimited {
                retry_after: limit.retry_after,
            }),
            Err(McpExecutorError::Call(_)) => {
                // A call-layer failure implicates the transport, not the
                // arguments: drop the session so the next call reconnects
                // rather than replaying every request down a dead pipe. The
                // message is deliberately not forwarded — it is server-shaped
                // text about a connection, of no use to the model.
                runtime.invalidate(&session).await;
                Err(ToolExecutionError::Disconnected)
            }
            Err(error) => Err(ToolExecutionError::Failed(format!("{error}"))),
        }
    }
}

/// Whether a failed connect was the server refusing the credential.
///
/// `ReconnectExhausted` is unwrapped rather than treated as its own failure: it
/// describes an attempt sequence, and every attempt in it was refused for the
/// same reason. Reporting the count instead of the reason is what would leave
/// the user reading "reconnect exhausted" with no idea a sign-in fixes it.
pub(crate) fn refusal_of(error: &McpRuntimeError) -> Option<McpRefusal> {
    match error {
        McpRuntimeError::Connect(connect) => connect_refusal(connect),
        // A missing grant includes a refresh token that was definitively
        // rejected. Storage faults and malformed catalogs need other remedies.
        McpRuntimeError::Auth(super::auth::McpAuthError::MissingBearer) => {
            Some(McpRefusal::AuthorizationRequired)
        }
        McpRuntimeError::Auth(_) | McpRuntimeError::Catalog => None,
    }
}

fn connect_refusal(error: &McpClientError) -> Option<McpRefusal> {
    match error {
        McpClientError::AuthorizationRequired
        | McpClientError::Auth(super::auth::McpAuthError::MissingBearer) => {
            Some(McpRefusal::AuthorizationRequired)
        }
        McpClientError::InsufficientScope { scope } => Some(McpRefusal::InsufficientScope {
            scope: scope.clone(),
        }),
        McpClientError::ReconnectExhausted { last_error, .. } => connect_refusal(last_error),
        _ => None,
    }
}

/// Whether a failed connect was the server rate limiting this account.
///
/// Unwrapped the same way as [`refusal_of`]: the rate limit is the answer, not
/// the attempt sequence that carried it.
fn rate_limit_of(error: &McpRuntimeError) -> Option<McpRateLimit> {
    match error {
        McpRuntimeError::Connect(connect) => connect_rate_limit(connect),
        McpRuntimeError::Auth(_) | McpRuntimeError::Catalog => None,
    }
}

fn connect_rate_limit(error: &McpClientError) -> Option<McpRateLimit> {
    match error {
        McpClientError::RateLimited(limit) => Some(*limit),
        McpClientError::ReconnectExhausted { last_error, .. } => connect_rate_limit(last_error),
        _ => None,
    }
}

/// Classifies and handles refusals. Callers run this under the runtime's
/// session-generation lock. Only explicit connection checks pass a notifier;
/// background discovery must not interrupt unrelated work.
pub(crate) fn notify_connection_refusal(
    runtime: &McpRuntime,
    error: &McpRuntimeError,
    notifier: Option<&dyn ReauthNotifier>,
) {
    let Some(refusal) = refusal_of(error) else {
        return;
    };
    runtime.forget_credential();
    if let (Some(identity), Some(notifier)) = (runtime.identity(), notifier) {
        notifier.reauth_required(McpReauthRequired::new(
            identity.id.clone(),
            identity.label.clone(),
            match refusal {
                McpRefusal::AuthorizationRequired => McpReauthReason::CredentialRefused,
                McpRefusal::InsufficientScope { .. } => McpReauthReason::InsufficientScope,
            },
        ));
    }
}

/// Maps a server's result onto the registry's own result type.
///
/// `is_error` becomes part of the structured body rather than a dispatch
/// error: a tool that ran and reported a problem *has* a result the model
/// should see and can react to, unlike a call that never executed.
fn normalize(result: &McpToolResult) -> Result<ToolResult, ToolExecutionError> {
    let mut content_refs = Vec::new();
    for item in result.content() {
        match item {
            McpContentItem::Text(_) => {}
            McpContentItem::Binary {
                mime_type,
                byte_length,
            } => content_refs.push(ContentRef::new(format!("binary:{mime_type}"), *byte_length)),
            McpContentItem::Reference(reference) => {
                content_refs.push(ContentRef::new(reference.uri().to_owned(), 0));
            }
        }
    }

    let structured = result.exact_structured_value();

    ToolResult::new(content_refs, Some(structured), ToolSource::Mcp).map_err(Into::into)
}
