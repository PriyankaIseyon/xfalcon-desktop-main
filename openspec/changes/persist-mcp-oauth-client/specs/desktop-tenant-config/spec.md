## REMOVED Requirements

### Requirement: MCP client packaging is platform-specific

**Reason**: The split existed so a macOS user supplied their own MCP OAuth
client rather than shipping a tenant-wide secret in the .app. It cost every
macOS user a manual client entry for the tenant's own MCP endpoints, which is
the common case, and the product decision is now to package the client on both
platforms.

**Migration**: Package `mcp_client_id` and its secret on both platforms, and
verify both bundles against the dispatched value identically.

## ADDED Requirements

### Requirement: The MCP fallback client is packaged on both platforms

The release workflow SHALL write the dispatched `mcp_client_id`, and the
`MCP_CLIENT_SECRET` environment secret when one is configured, into the tenant
config resource of **both** the macOS and Windows bundles. A secret supplied
without an id MUST fail the build rather than ship. An absent `mcp_client_id`
MUST remain a valid build.

Artifact verification MUST compare each platform's bundled `mcp_client_id` with
the dispatched value and fail on a mismatch or a lost value, and MUST fail when
a configured secret did not reach the bundle. No check may print a secret.

The MCP grant key MUST still never be bundled on either platform.

#### Scenario: A dispatched client reaches both bundles

- **WHEN** a release is dispatched with `mcp_client_id` and the
  `MCP_CLIENT_SECRET` environment secret configured
- **THEN** both the macOS and the Windows tenant config carry both values
- **AND** verification of either bundle fails if the id does not match the
  dispatched value or the secret is absent

#### Scenario: A build with no packaged client

- **WHEN** a release is dispatched with `mcp_client_id` left blank
- **THEN** neither bundle carries an MCP client and both builds succeed
- **AND** users supply a client per server on their own device

#### Scenario: A secret with no client

- **WHEN** `MCP_CLIENT_SECRET` is configured and `mcp_client_id` is blank
- **THEN** the build fails rather than shipping a secret that belongs to no
  client
