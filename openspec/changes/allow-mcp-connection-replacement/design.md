## Decisions

Workspace authentication and MCP authorization have different lifetimes. Removing an MCP connection must not erase the packaged workspace registration or sign out the xFalcon account. Persist the bundled connection opt-out per account and device, omit its tools from subsequent runs, and expose a restore action.

Reuse native OAuth discovery, system browser, PKCE, resource binding, loopback callback and credential vault. Optional client details go only to the sign-in command; they do not enter DA server metadata. Failed/cancelled authorization leaves the registered server available to retry. Editing its URL or client details requires a new browser sign-in.

Keep release bundles configured for login on both platforms. Verify all four required values against the dispatch inputs without printing credentials. Optional Windows MCP fallback remains scoped to its issuer.

## Research

- https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization — server discovery, pre-registered client information and resource-bound authorization.
- https://www.rfc-editor.org/info/rfc8252/ — external browser, PKCE, native application client credentials and loopback redirects.
- xfalcon-web MCPPanel.tsx and OAuth start route — save then browser redirect; existing workspace sign-in is required before MCP management.
