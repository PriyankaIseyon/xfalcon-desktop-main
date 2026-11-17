## Implementation and verification

- [x] Remove the authentication chooser, the header-name field and the static
      credential field from the server sheet, and always submit `oauth`.
- [x] Remove the "Add credential on this device" action and the sheet's
      credential-request mode from the servers section.
- [x] Remove the Test connection control, its connection report, and the dirty-
      form bookkeeping that existed to gate it.
- [x] Show a placeholder for a sealed client secret in the field that cannot
      seed it, and show none once the client ID it belongs to is changed.
- [x] Drop the now-uncallable `credential` parameter from `mcpServerCreate` and
      `mcpServerUpdate`, sending `null` explicitly rather than omitting the key.
- [x] Remove `MCP_EDITABLE_AUTH_TYPES`, `McpEditableAuthType`,
      `authNeedsCredential` and `authNeedsSignIn`, which had no remaining caller.
- [x] Mark Name, Server address, Client ID and Client secret required, enforce
      them on submit, and let a sealed secret satisfy its field on an edit.
- [x] Default tool approvals to Allow for a new server, while an edit still
      shows the mode the row holds.
- [x] Update the renderer suites and the Settings visual flow to the OAuth-only
      sheet.
- [ ] Regenerate the Settings visual baselines (`mcp-add-oauth-*.png`): the
      sheet is shorter by one radio group and one action row, so the stored
      screenshots no longer match. Needs a Playwright run on a host with the
      browsers installed.

Verification on 2026-09-21: `npm run typecheck`, `npm run lint`,
`npm run check:styles`, `npm run check:tooltips` and `vitest run` passed;
`cargo test --lib mcp::` (275 tests) and `cargo clippy --lib --all-targets`
passed unchanged, the backend being untouched.
