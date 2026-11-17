## ADDED Requirements

### Requirement: The user adds MCP servers from their tenant catalogue

The system MUST list, in Settings, the xFalcon tenants that DA reports for the
signed-in user, each with its MCP endpoint. The list SHALL be filtered by DA to the
user's tenant memberships. The desktop MUST NOT receive or filter a list that
includes tenants the user does not belong to.

Adding a catalogue tenant MUST create a registered server without the user typing an
address, client ID, or client secret. Hand entry SHALL remain available for servers
outside the catalogue.

#### Scenario: Only the user's tenants are listed

- **GIVEN** DA reports tenants `retail` and `erp` for the signed-in user
- **WHEN** the user opens the MCP servers section
- **THEN** the catalogue lists `retail` and `erp` and no other tenant

#### Scenario: One action adds a tenant

- **GIVEN** the catalogue lists `retail`
- **WHEN** the user adds it
- **THEN** a registered server for `retail` exists with the catalogue's endpoint, and no form field was typed

### Requirement: One foreign row does not fail the server list

The system MUST return the user's own registered servers when a listing page also
contains a row owned by another user or tenant. The foreign row SHALL be skipped and
logged without its contents.

#### Scenario: A foreign row on the page

- **GIVEN** a DA listing page with two of the user's rows and one foreign row
- **WHEN** the desktop lists MCP servers
- **THEN** it shows the two rows, and the listing does not fail
