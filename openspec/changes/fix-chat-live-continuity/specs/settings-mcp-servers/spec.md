## ADDED Requirements

### Requirement: MCP catalogs belong to a connection generation

Cached tool schemas SHALL be reused only for the session that discovered them.
Reconnect and capability changes MUST refresh the catalog. Concurrent refreshes
MUST NOT restore a stale catalog over a current one.

#### Scenario: A server reconnects with the same identity

- **WHEN** a new session connects with the previous server identity and protocol
- **THEN** its tool list and validators are discovered again

#### Scenario: An old session reports failure after its replacement connects

- **WHEN** a delayed failure requests invalidation of a session that has already been replaced
- **THEN** the replacement session and its catalog remain available
- **AND** invalidation of the current session still forces reconnect and discovery

#### Scenario: A replaced session's refused call can recover

- **WHEN** a delayed tool call reports refused credentials from an already replaced session
- **THEN** the call receives a retryable disconnect outcome instead of a terminal authorization refusal
- **AND** a subsequent call can succeed through the current connection without a stale sign-in prompt

#### Scenario: Catalog discovery is slow during invalidation

- **WHEN** a catalog refresh is waiting for the old session's tool listing
- **THEN** session invalidation completes without waiting for that listing
- **AND** replacement authentication can start before the old listing completes
- **AND** the replacement discovers and validates its own schemas

#### Scenario: A current generation reports refusal after a transport failure

- **WHEN** one call clears the current session after a transport failure
- **AND** another call from that same connection generation reports refused credentials
- **THEN** the refused credential is forgotten and reauthorization is requested
- **AND** a reconnect cannot renew or store a replacement credential before that cleanup completes

#### Scenario: A replaced generation reports refusal after the replacement is cleared

- **WHEN** a newer connection generation has succeeded and is subsequently cleared
- **AND** a call from the older generation reports refused credentials
- **THEN** the older refusal does not erase the newer generation's credential or request stale reauthorization
- **AND** the older refused call remains retryable

### Requirement: Tool timeout cancels its execution scope

A tool timeout SHALL cancel the call's child scope and allow bounded protocol
cleanup before recording failure. A round timeout SHALL cancel and drain all calls
in that round. Per-call timeout MUST NOT cancel the parent run or unrelated calls.

#### Scenario: An MCP call times out

- **WHEN** an MCP call reaches its execution deadline
- **THEN** its cancellation notification and progress cleanup are attempted within the cleanup bound
- **AND** the existing timeout outcome is recorded

#### Scenario: The round deadline expires

- **WHEN** a round expires while calls are pending
- **THEN** each pending call receives cancellation before the bounded cleanup window ends
