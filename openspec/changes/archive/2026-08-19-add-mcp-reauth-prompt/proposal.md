# Prompt for a fresh authorization when an MCP grant expires

## Why

When an MCP server's OAuth grant expires mid-session, the run does not say
so. It loses that server's tools and carries on as if they were never
there. The user sees the model explain that it has no way to execute a
query — while the server sits one sign-in away from working.

The information needed to say the right thing already exists and is
already correctly classified. `McpClientError::AuthorizationRequired` is
raised for a 401 carrying a `WWW-Authenticate` challenge, and
`mcp_server_test` turns it into exactly the right sentence: _"This server
rejected the saved sign-in. Sign in again to reconnect it."_ But that only
happens on the Settings panel's explicit connect path. On a run, both
routes throw the reason away:

- `runtime.connected()` failing becomes
  `ToolExecutionError::Failed(format!("{error}"))` — the typed variant
  flattened into a string.
- a `tools/call` answered 401 becomes `McpExecutorError::Call(String)` and
  then `ToolExecutionError::Disconnected` — indistinguishable from a dead
  pipe, and treated like one.

Neither carries which server was refused. So the frontend cannot tell an
expired grant from any other tool failure, cannot name the server, and has
nothing to offer. The user is left to guess that Settings → MCP → Sign in
is the fix.

xfalcon-web solved this and the shape is worth porting rather than
reinventing: a definitive credential rejection is classified as
`oauth_reauth_required`, the poisoned grant is evicted so the next attempt
authorizes instead of replaying it, the failure is _not_ idempotently
retried, and the tool-call event carries the code and the server id so the
stream handler can raise a persistent, front-and-center reconnect prompt —
deduped per server, because one run can fail several calls on the same one.

## What Changes

- **The reason survives the tool-execution path.** A credential rejection
  keeps its identity from the MCP client through the executor and dispatch,
  instead of being flattened to a string at either boundary. It reaches the
  agent layer as its own `ToolExecutionError` variant carrying the server's
  id and label, so nothing downstream has to parse a message to recover
  what the type already knew.
- **A rejected grant is not retried.** The tool loop treats this variant as
  terminal for that call rather than a transport blip worth another
  attempt: the same token would be refused the same way, and a retry only
  spends a turn to arrive at the same place.
- **The poisoned grant is evicted.** The stored grant is deleted on a
  structural rejection, so the next attempt on that server runs the
  authorization flow rather than replaying a credential the server has
  already refused.
- **The user is prompted immediately, and persistently.** A new
  `mcp:reauth-required` event raises a modal naming the server and offering
  one action: reconnect. It persists until the user reconnects or
  dismisses, and its scrim does not dismiss it — a rejected grant blocks
  every tool call on that server, so a transient toast is too easy to miss.
  It is deduped per server.
- **Reconnect is one click from the prompt.** It runs the existing
  `mcp_server_sign_in` flow and then `mcp_server_test`, so a successful
  reconnect leaves the server connected and the panel agreeing, without the
  user navigating to Settings at all.

A dedicated event rather than a widened `ToolInvocationEvent::Failed`: the
prompt is app-global, and it must also fire when the _connection attempt_
is what was refused, which is not attached to any invocation.

## Capabilities

### Added Capabilities

- `mcp-reauth-prompt`: detect an expired or refused MCP grant during a run,
  evict it, and prompt for a fresh authorization.

## Impact

- Changed: `src-tauri/src/mcp/{client,executor,dispatch,runtime}.rs`,
  `src-tauri/src/agent/tool_loop.rs`, `crates/contracts/src/mcp.rs`,
  `src/lib/generated/contracts.ts` (regenerated), `src/lib/native.ts`,
  `src/app/AppShell.tsx`.
- New: `src/features/mcp/McpReauthPrompt.tsx`.
- No new Tauri command: reconnect reuses `mcp_server_sign_in` and
  `mcp_server_test`, both already registered and permitted.

## Not in this change

**Silent background refresh.** This change prompts; it does not attempt to
refresh a grant on the user's behalf before asking. A refresh-token path
that renews without interrupting the run is a separate change, and the
prompt is the correct fallback for it either way — a refresh that fails
still has to ask.

**Insufficient scope as a distinct prompt.** A 403 `insufficient_scope`
is a real and different condition (the grant is valid but too narrow) and
already has its own typed variant. It is routed to the same prompt here,
because signing in again is the action in both cases. Asking for a
specific scope, and saying which, is left to its own change.
