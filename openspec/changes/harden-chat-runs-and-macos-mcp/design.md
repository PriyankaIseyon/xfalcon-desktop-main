## Context

Desktop executes runs locally and mirrors terminal state to DA. A snapshot is
authoritative for whether a conversation still has an active run. A public MCP
OAuth client ID used by a native authorization-code flow is device-local input;
it must not be added to the shared MCP server row.

## Goals / Non-Goals

- Goals: make Stop visibly settle, enforce a strict 30-minute run ceiling,
  obtain a public OAuth client when sign-in needs one, and keep packaged MCP
  client values out of macOS bundles.
- Non-goals: change DA schemas, generated IPC contracts, tenant login config,
  MCP grant persistence, or the Windows packaged MCP connection.

## Decisions

- A snapshot with `active_run_id: null` retires the previous running or
  cancelled run. A failed/interrupted run id can remain only as the existing
  retry target. A snapshot naming the same active run keeps its live-only
  presentation state.
- Enforce the deadline around each active drive using the `Run`'s existing
  deadline. On expiry, cancel outstanding work, transition the run to failed,
  finalize DA as failed, and emit the existing retryable time-budget failure.
- Treat an MCP authorization refusal as a run-invalidating event. Finalize the
  tool attempt, prompt for reconnection, and fail the run before another model
  turn can use the invalidated session.
- Keep OAuth client details outside `McpServerInput`. Start sign-in after create
  with configured, stored, or dynamically registered client details.
- When the authorization server cannot supply a client and none is configured,
  ask only for the operator-issued public client ID and retry sign-in without a
  client secret.
- The workflow writes MCP client values only in the Windows matrix job. macOS
  still bundles the four public tenant login coordinates required for app
  browser sign-in.

## Risks / Trade-offs

- A hard deadline can interrupt a valid long answer. Thirty minutes is the
  requested ceiling and already the declared run budget.
- A terminal snapshot can arrive while local presentation is settling. The
  durable snapshot is authoritative and replaces that presentation.
- OAuth server creation first uses configured, stored, or dynamically
  registered public client details. When none is available, Settings asks only
  for the public client ID and retries without a client secret.

## Migration Plan

No persisted schema changes. Existing MCP grants and server rows remain valid.
New macOS artifacts omit `mcp_client_id` and `mcp_client_secret`; Windows
artifacts retain current behavior.
