## ADDED Requirements

### Requirement: MCP client packaging is platform-specific

The release workflow MUST omit `mcp_client_id` and `mcp_client_secret` from
macOS tenant config resources. It MUST retain the supplied MCP client values in
Windows tenant config resources. Both platform artifact checks MUST fail when
the resulting resource violates this split. The four public DA/Casdoor tenant
login coordinates remain required on both platforms.

#### Scenario: macOS build

- **WHEN** the macOS matrix job writes and verifies tenant config
- **THEN** the four tenant login fields are present
- **AND** no MCP client ID or secret is present

#### Scenario: Windows build

- **WHEN** the Windows matrix job receives an MCP client ID and optional secret
- **THEN** those supplied values are written and verified as before

#### Scenario: Windows confidential client is incomplete

- **WHEN** a Windows job receives an MCP client secret without an MCP client ID
- **THEN** the job fails before packaging
