# Change: Keep an MCP server's OAuth client details once they are entered

## Why

A user can paste an OAuth client ID and secret into Add/Edit MCP server, save,
and find both fields empty the next time the sheet opens. Nothing was stored:
the pair was only ever a sign-in argument, so it survived only inside a grant
that a completed browser flow wrote. A declined, timed-out or refused flow
discarded it, a sign-out deleted it, and no field on the server row could report
it back. On macOS, where the release deliberately packages no fallback client,
this made every sign-in attempt start with the same paste.

## What Changes

- Store an `oauth` server's operator-issued client per device, for as long as
  the server is registered and independently of any grant. The identifier is
  stored in the clear (it names a registration); its confidential half is sealed
  under its own vault scope.
- Seal the pair when the server is saved, and when it is supplied to a sign-in,
  in both cases before the browser opens — so a failed flow is retryable without
  re-entry.
- Report the stored identifier on `McpServerView`, and whether a confidential
  half exists, so the edit sheet shows what is saved instead of an empty field
  over it. A blanked identifier removes the stored client; a blank secret beside
  an unchanged identifier keeps the sealed one.
- Resolve a sign-in's client as: device-held, then packaged, then dynamic
  registration. A sign-out keeps the client; unregistering the server or moving
  it off browser sign-in removes it.
- Resolve Protected Resource Metadata by operator override and by the server's
  own `WWW-Authenticate` pointer (RFC 9728 §5.1), not only the well-known URLs.
  Both were already parsed and validated and neither had a caller.
- Report the count of device-held clients in the diagnostics bundle.
- Package the MCP fallback client on macOS as well as Windows, and verify both
  bundles against the dispatched value identically. This reverses the
  platform-specific split in `harden-chat-runs-and-macos-mcp`, on the user's
  explicit instruction, and accepts its stated trade: a bundled secret is
  extractable from any copy of the build, so a public client is preferred where
  Casdoor will issue one.

## Impact

- Affected capabilities: settings-mcp-servers, desktop-tenant-config.
- `McpServerView` gains two fields; `mcp_server_create` / `mcp_server_update`
  gain two arguments. No DA contract changes: the pair never reaches the shared
  registry, which cannot hold anything this build could decrypt.
- New device-local table in the existing per-account MCP store, so "clear local
  data" already removes it.
