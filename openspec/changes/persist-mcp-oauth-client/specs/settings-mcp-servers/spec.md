## MODIFIED Requirements

### Requirement: Optional operator-issued OAuth client details

Add/Edit MCP server and OAuth retry SHALL accept a client ID and optional client
secret. A supplied secret MUST have an accompanying ID. These values MUST be
submitted only to native storage and native sign-in, never the shared server
registry. Blank values SHALL retain automatic discovery and registration.

Supplied values SHALL be sealed on this device before browser authorization
begins, and SHALL persist for as long as the server is registered — across a
cancelled, timed-out or refused authorization, across a sign-out, and across
restart. Unregistering the server, or changing it to a method that does not use
browser sign-in, SHALL remove them.

The server row SHALL report the stored client ID and whether a confidential
value accompanies it. Add/Edit SHALL open seeded with that ID. Clearing the ID
SHALL remove the stored client; leaving the secret blank SHALL retain the sealed
value while the ID is unchanged, and SHALL NOT carry it to a changed ID.

Browser sign-in SHALL begin after OAuth creation, or after a connection or
client change. Native sign-in SHALL resolve its client as this device's stored
client, then a packaged client scoped to its issuer, then dynamic registration,
resolving each ID and secret from a single source.

#### Scenario: Cancelled authorization keeps the client

- **WHEN** the user supplies client details and the browser authorization is
  cancelled, times out, or is refused
- **THEN** the server remains registered with those details saved on this device
- **AND** retrying sign-in uses them without asking for them again

#### Scenario: Editing shows what is saved

- **WHEN** the user opens Edit for an OAuth server this device holds a client for
- **THEN** the client ID field shows the saved ID and the form is not dirty
- **AND** the secret field states that a saved secret is retained if left blank

#### Scenario: Removing a stored client

- **WHEN** the user clears the client ID and saves
- **THEN** this device holds no client for that server
- **AND** sign-in returns to packaged or automatic client resolution

#### Scenario: A reissued client ID drops the previous secret

- **WHEN** the user changes the client ID and leaves the secret blank
- **THEN** the value sealed for the previous ID is not sent with the new one

#### Scenario: Signing out keeps the client

- **WHEN** the user signs out of an OAuth server
- **THEN** the grant is gone and the stored client remains
- **AND** signing back in requires no re-entry

## ADDED Requirements

### Requirement: Protected Resource Metadata is resolved by every conforming route

Native MCP sign-in SHALL resolve a server's Protected Resource Metadata from an
operator-configured metadata URL when the shared server row carries one, then
from the RFC 9728 §3.1 well-known URLs, then from the `resource_metadata`
parameter of the server's own `WWW-Authenticate` challenge.

A challenge pointer that is not same-origin with the server being dialed MUST be
discarded without being requested. A metadata document that was retrieved and
refused MUST NOT be re-requested through another route. Every resolved document
MUST still name the server being dialed as its resource.

#### Scenario: Metadata published outside a well-known path

- **WHEN** a server publishes no metadata at either well-known URL and names its
  document in a `WWW-Authenticate` challenge on the MCP endpoint
- **THEN** sign-in reads that document and completes

#### Scenario: A challenge pointing at another host

- **WHEN** a server's challenge names a metadata URL on a different origin
- **THEN** that host receives no request and sign-in reports discovery failed

#### Scenario: A refused document is not retried

- **WHEN** a well-known document is retrieved and names a different resource
- **THEN** sign-in reports discovery failed without consulting the challenge
