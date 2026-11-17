## MODIFIED Requirements

### Requirement: The bundled tenant server has one read-only Settings card

The MCP servers section SHALL render one card for the bundled tenant server.
The card SHALL show the packaged MCP endpoint, actual transport, and that
authentication comes from the xFalcon browser session. It SHALL identify the
connection as a workspace server and signed in. It SHALL NOT offer edit,
delete, sign-in, or sign-out actions, and SHALL NOT claim a tool-approval policy.

A registered row whose normalized remote endpoint equals the bundled tenant
endpoint SHALL be treated as an alias of the bundled server. Desktop SHALL NOT
render, search, mention, or dial that alias as an additional server. Desktop
SHALL reject create and update requests that would create such an alias without
changing the shared registry.

Other registered servers MUST keep their cards and run-time behavior unchanged.

#### Scenario: Settings page shows the packaged tenant connection

- **WHEN** the reader opens the MCP servers section
- **THEN** one read-only workspace-server card shows the packaged endpoint,
  transport, and browser-session authentication
- **AND** the card shows no unavailable or per-device sign-in warning
- **AND** registered servers at other endpoints render as before

#### Scenario: Registered row aliases the tenant endpoint

- **WHEN** a shared registry row resolves to the bundled tenant MCP endpoint
- **THEN** desktop exposes only the bundled tenant card and connection
- **AND** it does not show a device sign-in warning for the alias
- **AND** it does not create a second run-time connection or tool namespace

#### Scenario: Reader attempts to register the tenant endpoint

- **WHEN** a desktop create or update request targets the bundled tenant MCP
  endpoint
- **THEN** the request fails before the shared registry is changed
