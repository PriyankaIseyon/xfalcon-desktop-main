## MODIFIED Requirements

### Requirement: Opening a conversation is local-first and server-authoritative

Selecting a conversation SHALL synchronously set the active conversation ID and
its known sidebar title before any native request resolves. The client SHALL then
start a read of the existing account-bound encrypted projection and the
authoritative open concurrently.

A local projection hit SHALL render immediately with stale availability. A local
miss SHALL render a compact transcript skeleton until an admitted view arrives.
The prior conversation's transcript MUST NOT render under the newly selected
title.

The local preview SHALL perform no cloud request, mutation, active-pointer
change, project-binding change, event emission, or deletion. The authoritative
open remains the only path that establishes server truth.

#### Scenario: Local hit renders before the server

- **GIVEN** the selected conversation has an encrypted local projection
- **WHEN** its local preview resolves while the server reconcile is still in flight
- **THEN** the selected row and title are already active
- **AND** the local transcript renders with stale availability
- **AND** the server reconcile continues in the background

#### Scenario: Local miss uses a compact skeleton

- **GIVEN** the selected conversation has no local projection
- **WHEN** the authoritative open is still in flight
- **THEN** the selected row and title are already active
- **AND** the transcript region reports busy and renders a compact skeleton
- **AND** the real sidebar and titlebar remain visible

#### Scenario: Authoritative view replaces local preview

- **GIVEN** a stale local preview is visible
- **WHEN** the authoritative open returns a live view
- **THEN** that view replaces the preview
- **AND** the opening state clears

### Requirement: Opening a conversation performs exactly one cloud reconcile

The authoritative open command SHALL issue exactly one cloud bootstrap request
per admitted navigation. The concurrent local preview SHALL issue zero cloud
requests. The reconcile inside the authoritative open IS the access check; no
separate pre-flight reconcile may run on that path.

Repeated selection of the same conversation while its open is already in flight
SHALL reuse that navigation and SHALL NOT issue another authoritative open.

#### Scenario: Single bootstrap request per sidebar click

- **WHEN** the reader selects a conversation row
- **THEN** the local preview performs no cloud request
- **AND** the authoritative conversation bootstrap endpoint is called at most once

#### Scenario: Repeated click is coalesced

- **GIVEN** a conversation open is in flight
- **WHEN** the reader selects the same row again
- **THEN** no second authoritative open starts

### Requirement: The latest conversation selection wins

Conversation opens SHALL use an account-scoped monotonically increasing
navigation generation. Only the latest generation may apply a preview, an
authoritative view, an error, a rollback, or a follow-up refresh to the visible
conversation.

Native projection events for another conversation MUST NOT replace the current
view.

#### Scenario: B wins over a slow A

- **GIVEN** conversation A is opening
- **WHEN** the reader selects conversation B before A settles
- **THEN** B becomes active immediately
- **AND** a later A preview, success, event, or failure does not change B

### Requirement: Failed optimistic navigation restores complete state

Before optimistic selection, the client SHALL capture the complete chat-store
snapshot, copied collection fields, and the admitted invocation set. A transient
failure for the still-current navigation SHALL restore that exact state and
report the failure. A failure from an obsolete navigation SHALL do nothing.

Typed not-found handling SHALL retain its self-heal rules and MUST NOT be
collapsed into transient rollback behavior.

#### Scenario: Current transient failure rolls back exactly

- **GIVEN** conversation A is visible with live activity and admitted invocations
- **WHEN** conversation B has no local preview and its authoritative open fails transiently
- **THEN** every captured field and admitted invocation for A is restored
- **AND** the failure is reported

#### Scenario: Obsolete failure cannot roll back a newer chat

- **GIVEN** A fails after B has become the latest selection
- **THEN** B remains unchanged

### Requirement: Stale fallback preserves the original cloud error class

The authoritative open SHALL fall back to the local snapshot, marked stale,
when cloud bootstrap fails with an unavailable or not-found result and a local
snapshot exists.

When no local snapshot exists, the open MUST fail with the original error class.
Not-found stays not-found and is never collapsed into unavailable. A stale view
MUST report its staleness.

#### Scenario: Offline open from local copy

- **GIVEN** a conversation with a readable local projection
- **WHEN** cloud bootstrap returns not-found or is unreachable
- **THEN** the conversation remains open from the local copy
- **AND** the view reports stale availability

#### Scenario: NotFound without local copy is typed

- **GIVEN** a conversation with no local projection row
- **WHEN** cloud bootstrap returns not-found
- **THEN** the command fails with the dedicated not-found variant

### Requirement: Authoritative NotFound self-heals the workspace

When an open fails with the not-found variant, the service SHALL purge the local
projection row, clear the active pointer, unbind the project, and return the
stable typed error. The client SHALL report the outcome and refresh the
conversation list. It SHALL restore a prior different conversation only when
that navigation is still current and the prior conversation remains valid.

Transient failures MUST NOT purge data or remove the sidebar row.

#### Scenario: Clicking a deleted conversation

- **GIVEN** a sidebar row has no server row and no local projection
- **WHEN** the reader selects it
- **THEN** the reader is told it no longer exists
- **AND** the list is refreshed
- **AND** no partial dead view remains

#### Scenario: Network blip is not treated as deletion

- **GIVEN** the server is unreachable and no local projection exists
- **WHEN** the reader selects the row
- **THEN** prior state is restored
- **AND** the row remains retryable

#### Scenario: A dead conversation not on screen does not reset the view

- **GIVEN** a valid prior conversation remains available
- **WHEN** a selected dead row returns authoritative not-found
- **THEN** the list refreshes
- **AND** the valid prior conversation is restored rather than reset

### Requirement: Opening binds the conversation's project, or gives it up loudly

The authoritative open SHALL restore a usable bound project and SHALL clear an
unusable active project with the reason logged. The local preview MUST NOT
change project state. An unreachable project folder MUST NOT block transcript
display.

#### Scenario: Project changes only after authoritative open

- **GIVEN** a local preview is visible for a project conversation
- **WHEN** authoritative open has not settled
- **THEN** preview rendering has not changed active project state
- **WHEN** authoritative open succeeds
- **THEN** the bound usable project becomes active

#### Scenario: A bound project is restored

- **GIVEN** the conversation is bound to a present trusted project folder
- **WHEN** authoritative open succeeds
- **THEN** that project is active

#### Scenario: An unreachable folder does not block the open

- **GIVEN** the bound project folder is unavailable
- **WHEN** authoritative open succeeds
- **THEN** the transcript remains readable
- **AND** the active project is cleared and the reason is logged

### Requirement: Selecting a conversation lands on the chat view

Selecting a conversation from any sidebar or palette surface SHALL immediately
switch the main surface to the chat view. The selected row and title SHALL
change in the same interaction before local preview or server reconcile settles.

#### Scenario: Open from the Settings view

- **GIVEN** Settings is visible
- **WHEN** the reader selects a conversation
- **THEN** the chat view, active row, and known title update immediately

### Requirement: DA rejection logs carry a body snippet

The data-plane client's request-rejected warning SHALL include a bounded snippet
of the response body alongside status and path.

#### Scenario: 404 from a missing row

- **WHEN** the server rejects a request with a JSON error body
- **THEN** the warning includes the leading bytes of that body
