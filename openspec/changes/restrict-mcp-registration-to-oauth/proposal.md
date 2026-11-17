# Change: Register MCP servers by browser sign-in only

## Why

Three reports from one setup session, all from the same two controls.

The Client secret field opens blank over a secret this device holds, because a
sealed value is never sent back to the surface. The caption says so, but an
empty box reads as "it was wiped", and the user re-typed a secret that was
already saved.

Re-typing it then disabled Test connection, which is only offered for a saved,
unedited row. That is every route to it: a new server has nothing to test, and
saving an OAuth server closes the sheet and opens the browser. So the one
control that promised "check this before you leave" could not be reached from
the flow that registers a server, and a test of a server that is not signed in
yet reports only that a sign-in is needed.

Behind both: a method chooser offering four ways to authenticate, where this
product connects to one kind of server — its own, behind the tenant's identity
provider. Three of the four choices were routes to a row this deployment never
wants, and the credential field, the header-name field and the dirty-form
bookkeeping existed only to serve them.

## What Changes

- Register `oauth` only. The Authentication chooser, the header-name field and
  the static credential field are removed from Add/Edit MCP server, along with
  the "Add credential on this device" action on a row's card. `McpServerInput`
  is still submitted with a method, and this app always sets `oauth`.
- Remove the Test connection control and its connection report from the sheet.
  Sign-in already reports whether the server answered, on the row itself.
- Stand in for a sealed client secret with a placeholder in the field it cannot
  seed, so a saved secret is visible as saved rather than as missing.
- Native command surface is unchanged: `mcp_server_create` / `mcp_server_update`
  still accept a credential for a `header` or `bearer` row, and `mcp_server_test`
  still serves the re-authorization prompt. Rows DA reports with another method
  are still listed, described and removable — they are no longer something this
  app can create or repair.

## Impact

- Affected capabilities: settings-mcp-servers.
- No contract, backend or DA changes. `mcpServerCreate` / `mcpServerUpdate` drop
  their `credential` parameter in `src/lib/native.ts` and send `null`, because no
  caller can supply one.
- A row registered elsewhere with `header` or `bearer` authentication can no
  longer be given its credential on this device. The user confirmed no such row
  exists on this deployment.
- Visual baselines for the Add/Edit sheet change: the sheet is shorter by one
  radio group and one action row.
