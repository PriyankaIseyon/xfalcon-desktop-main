# settings-mcp-servers — Spec Delta

## ADDED Requirements

### Requirement: The bundled tenant server has no Settings card

The MCP servers section SHALL NOT render a card for the bundled tenant
server. The tenant connection has nothing to configure, and its former card
carried approval copy that auto-approve made false. User-registered servers
keep their cards unchanged.

#### Scenario: Settings page without the tenant card

- **WHEN** the user opens Settings → MCP servers
- **THEN** no "Provided by your workspace" card is present
- **AND** user-registered server cards render as before
