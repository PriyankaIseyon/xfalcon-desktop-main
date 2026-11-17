## ADDED Requirements

### Requirement: OAuth server creation uses the configured public client

When OAuth is selected for a new MCP server, Settings MUST create the server
without asking for OAuth client credentials and start browser sign-in
immediately. Sign-in SHALL use the configured public client ID, a previously
stored client, or dynamic client registration. If none is available, Settings
MUST ask only for a client ID and retry sign-in without a client secret.

#### Scenario: Add an OAuth server

- **WHEN** the reader selects OAuth and enters valid server details
- **THEN** the shared server row is created without client credentials
- **AND** browser sign-in starts without asking for client credentials

#### Scenario: A client ID is required

- **WHEN** automatic sign-in cannot obtain a client identity
- **THEN** Settings asks for the operator-issued client ID
- **AND** retries sign-in without a client secret

#### Scenario: Non-OAuth server

- **WHEN** another authentication method is selected
- **THEN** OAuth client details are not shown or submitted
