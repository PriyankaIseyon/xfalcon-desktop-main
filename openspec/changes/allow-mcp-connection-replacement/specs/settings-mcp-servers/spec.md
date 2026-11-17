## REMOVED Requirements

### Requirement: The bundled tenant server has no Settings card

**Reason**: Users now explicitly need to remove and restore the bundled connection.
**Migration**: Render a removable connection until this account removes it on this device, then provide a restore action.

## ADDED Requirements

### Requirement: Removable bundled MCP connection

The MCP servers section SHALL show the configured bundled MCP connection with a Remove action until the user removes it from this device. Removal SHALL persist for the signed-in account, omit the connection from subsequent tool registration, and preserve workspace login, DA account access and other registered MCP servers. Settings SHALL offer restoration after removal.

#### Scenario: Remove the bundled connection

- **WHEN** the user confirms removal of the bundled MCP connection
- **THEN** it is absent from Settings and subsequent chat tool registration on this device
- **AND** restarting the app preserves that choice
- **AND** the user stays signed in and can add another MCP server

#### Scenario: Restore the bundled connection

- **WHEN** the user restores the workspace MCP connection
- **THEN** the connection appears again and is available to subsequent chat runs
- **AND** the choice does not alter another account's preferences

### Requirement: Optional operator-issued OAuth client details

Add/Edit MCP server and OAuth retry SHALL accept a client ID and optional client secret. A supplied secret MUST have an accompanying ID. These values MUST be submitted only to native sign-in, never the shared server registry. Blank values SHALL retain automatic discovery and registration. Browser sign-in SHALL begin after OAuth creation or connection/client changes. Cancelled or failed authorization SHALL leave a recoverable registered server. Fields MUST clear when the dialog closes or the target server or authentication method changes.

#### Scenario: Supply a client before creation

- **WHEN** the user adds an OAuth server with its URL and issued client details
- **THEN** the server is registered without those details in the registry payload
- **AND** native browser sign-in receives the supplied client details for that server

#### Scenario: Change an existing OAuth connection

- **WHEN** an OAuth server's address or client details change
- **THEN** saving starts browser sign-in for the updated connection

#### Scenario: Automatic setup

- **WHEN** OAuth client fields are blank
- **THEN** native sign-in resolves a scoped configured client, stored client or dynamic registration
- **AND** missing registration can be completed with the retry dialog

### Requirement: Packaged workspace login on both platforms

Windows and macOS release bundles MUST contain the four required workspace login registration values. Release verification MUST compare each value with the dispatched build inputs and reject missing or mismatched configuration before publication.

#### Scenario: macOS workspace sign-in configuration

- **WHEN** a macOS app is packaged
- **THEN** its actual bundled DA URL, issuer, client ID and organization match the release inputs
- **AND** a missing or mismatched value fails the release verification

### Requirement: OAuth grants remain bound to their authorized resource

Native MCP authorization MUST reject a stored device or account grant before token use or renewal when its resource belongs to a different origin from the destination. Editing an OAuth server URL MUST clear its old device grant before the changed connection is reported as signed in. Failed or cancelled replacement sign-in MUST NOT send the previous token to the replacement endpoint.

#### Scenario: Changed OAuth destination

- **WHEN** an OAuth server URL changes and replacement sign-in does not complete
- **THEN** the prior device grant cannot authenticate the changed connection
- **AND** a mismatched device or shared grant causes no request to the replacement resource or authorization endpoint
