## ADDED Requirements

### Requirement: Browser sign-in is the only method this app registers

Add/Edit MCP server SHALL register servers that authenticate by browser sign-in
and SHALL NOT offer a choice of authentication method. The sheet SHALL NOT
collect a static credential or a header name, and a server card SHALL NOT offer
to add a credential on this device.

A row the shared registry reports with another method SHALL still be listed,
described by its method, and removable. It is not editable into a working state
here, and the surface SHALL NOT imply that it is.

#### Scenario: Adding a server

- **WHEN** the user opens Add MCP server
- **THEN** no authentication chooser, header-name field or credential field is
  present
- **AND** the sheet states that sign-in is completed in the browser
- **AND** saving registers the server with browser sign-in

#### Scenario: A row registered elsewhere with a static credential

- **WHEN** the registry reports a row whose method needs a device credential and
  this device holds none
- **THEN** the card explains that the credential is missing and is per device
- **AND** no action offers to add one here

### Requirement: A saved client secret is visible as saved

The Client secret field SHALL NOT be seeded with the sealed value. Where this
device holds one for the client ID in the field, the field SHALL show a
placeholder standing for it, and SHALL state that leaving it blank keeps it.

Where the field's client ID is not the one the sealed secret was issued for, no
placeholder SHALL be shown and the field SHALL state that the saved secret will
be replaced.

#### Scenario: Opening Edit for a server with a sealed secret

- **WHEN** the user opens Edit for a server this device holds a client secret for
- **THEN** the secret field is empty but shows a placeholder for the saved value
- **AND** the caption states that leaving it blank keeps the saved secret

#### Scenario: Replacing the client ID

- **WHEN** the user changes the client ID away from the stored one
- **THEN** the secret field shows no placeholder
- **AND** the caption states that the saved secret will be replaced

### Requirement: Every value the sheet needs is marked and enforced

Name, Server address, Client ID and Client secret SHALL be marked required, on
the label so the marking reaches assistive technology, and the sheet SHALL NOT
submit until each is supplied. Display name and Description remain optional.

A sealed client secret SHALL satisfy the requirement while the client ID it was
issued for is unchanged, so an edit is never asked to re-enter a value this
device already holds.

Tool approvals SHALL default to Allow for a new server. An edit SHALL show the
mode the row actually holds.

#### Scenario: Saving a new server

- **WHEN** the user has entered a name and address but no client details
- **THEN** Add server stays disabled until a client ID and secret are entered

#### Scenario: Editing a server whose secret is sealed here

- **WHEN** the user opens Edit and changes only the description
- **THEN** Save changes is enabled with the secret field left blank

#### Scenario: Approvals on a new server

- **WHEN** the user opens Add MCP server
- **THEN** Allow is the selected tool-approval mode

## REMOVED Requirements

### Requirement: Connection testing from the server sheet

**Reason**: Unreachable from the flow it existed for. The control required a
saved, unedited row: a new server had nothing to test, entering any client detail
made the form dirty, and saving an OAuth server closed the sheet to open the
browser. A test of a server that has not signed in yet could only report that a
sign-in is needed, which sign-in itself reports on the row.

**Migration**: Sign-in reports whether the server answered, on the row. The
`mcp_server_test` command is unchanged and still serves the re-authorization
prompt (`mcp-reauth-prompt`).
