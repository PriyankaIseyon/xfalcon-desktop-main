# Change: Treat the bundled MCP endpoint as one server

## Why

A DA registry row can point at the same endpoint as the bundled tenant MCP
server. Desktop currently treats the two IDs as separate servers, hides the
working bundled card, and shows a false sign-in warning for the registered
alias. A registry or deployment change can make that duplicate row newly
visible without changing desktop's warning classifier, so correctness cannot
depend on the row being absent.

## What Changes

- Make the bundled endpoint the canonical desktop identity.
- Show that canonical packaged endpoint as one read-only, browser-authenticated
  workspace server card.
- Exclude registered aliases of that endpoint from Settings, workspace search,
  composer mentions, and the run-time server set.
- Reject desktop create or update requests that would create such an alias.
- Replace transport copy that claims a connection without a connection test.

## Impact

- Affected specs: `settings-mcp-servers`
- Affected code: MCP registry classification, MCP commands, workspace MCP
  summaries, and Settings copy/tests
