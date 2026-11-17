# Change: Replace MCP connections without losing workspace sign-in

## Why

Users need a configured macOS app that can sign in immediately, then remove the bundled MCP connection and add their own servers with operator-issued OAuth client details.

## What Changes

- Bundle and verify workspace login registration values on both desktop platforms.
- Persist removal of the bundled MCP connection per account on this device, with a restore action. Keep workspace login and DA account data available.
- Accept optional client ID and secret in Add/Edit MCP server and browser reauthorization, while preserving automatic discovery and registration when fields are blank.
- Start browser sign-in after creating an OAuth server or changing its connection or client details. Credentials remain outside the shared registry payload.

## Impact

- Affected capability: settings-mcp-servers; native MCP connection selection and persistence; release bundle verification.
- Supersedes the read-only bundled card and the no-client-input restriction in the earlier unarchived MCP changes for this explicitly requested behavior.
- Implementation and release are authorized by the user's current request. No change to workspace identity or DA contracts.
