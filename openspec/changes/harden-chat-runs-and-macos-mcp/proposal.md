# Change: Harden chat runs and macOS MCP onboarding

## Why

A durable terminal snapshot can leave the renderer's previous active run in
place, so Stop appears to do nothing. The existing 30-minute run budget is
checked only between rounds, not while a round is blocked. Separately, macOS
must not receive the packaged MCP OAuth client used by Windows, and its add
server flow must obtain the public client ID needed to sign in.

## What Changes

- Treat an authoritative snapshot with no active run as terminal in the chat
  store, including after Stop.
- Enforce the existing 30-minute run budget as a real wall-clock deadline and
  fail timed-out runs with retry guidance.
- Fail a run immediately when its MCP authorization is invalidated, while
  preserving the reconnect prompt and retry path.
- Start OAuth sign-in immediately after creating an MCP server. Ask only for a
  public client ID when configured, stored, and dynamically registered clients
  are unavailable.
- Omit MCP OAuth client values from macOS bundles while retaining them in
  Windows bundles, with platform-specific artifact checks.

## Impact

- Affected specs: `chat-run-recovery`, `settings-mcp-servers`,
  `desktop-tenant-config`
- Affected code: chat store and native run driver, MCP Settings form, release
  workflow and verification
- Dependency: extends the active `fix-bundled-mcp-alias` and
  `add-tenant-config-provisioning` changes without changing their endpoint or
  tenant login contracts
