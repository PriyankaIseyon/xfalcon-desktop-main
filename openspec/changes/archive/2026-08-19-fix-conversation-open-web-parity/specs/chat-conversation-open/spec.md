# chat-conversation-open — Spec Delta

## ADDED Requirements

### Requirement: Opening a conversation performs exactly one cloud reconcile

`chat_conversation_open` SHALL issue exactly one cloud bootstrap request per
invocation. The reconcile inside `open_and_emit` is the access check; no
separate pre-flight reconcile runs on the open path.

#### Scenario: Single bootstrap request per sidebar click

- **WHEN** the user clicks a conversation row in the sidebar
- **THEN** the DA endpoint `/api/v1/chat/bootstrap/conversations/{id}` is
  called at most once for that click

### Requirement: Stale fallback preserves the original cloud error class

The open path SHALL fall back to the local snapshot (marked stale) when the
cloud bootstrap fails with Unavailable or NotFound and a local snapshot
exists. When no local snapshot exists, the open MUST fail with the original
error class: NotFound stays NotFound and is never collapsed into Unavailable.

#### Scenario: Offline open from local copy

- **GIVEN** a conversation with a readable local projection row
- **WHEN** the cloud bootstrap returns 404 or is unreachable
- **THEN** the conversation opens from the local copy
- **AND** the view reports stale availability ("Offline copy — reconnect to sync")

#### Scenario: NotFound without local copy is typed

- **GIVEN** a conversation with no local projection row
- **WHEN** the cloud bootstrap returns 404
- **THEN** `chat_conversation_open` fails with the dedicated not-found error
  variant (distinguishable by the frontend without matching prose)

### Requirement: Authoritative NotFound self-heals the workspace

When an open fails with the not-found variant, the client SHALL self-heal
instead of leaving a permanently dead sidebar row (parity with
xfalcon-web `useMessageLoader`'s `Conversation not found` branch):

- the service purges the conversation's local projection row,
- the UI shows an error toast naming the outcome,
- the workspace conversation list is refreshed from the cloud,
- if the dead conversation was on screen, the view resets to the new-chat state.

Transient failures (`Unavailable`, network) SHALL NOT trigger any of this —
the row and state are kept so the user can retry.

#### Scenario: Clicking a deleted conversation

- **GIVEN** a sidebar row whose conversation no longer exists on DA and has no
  local copy
- **WHEN** the user clicks it
- **THEN** a toast reports the conversation no longer exists
- **AND** the sidebar list is re-fetched
- **AND** the chat surface is not left in a partial or errored state

#### Scenario: Network blip is not treated as deletion

- **GIVEN** the DA host is unreachable
- **WHEN** the user clicks a conversation with no local copy
- **THEN** the inline error reports the failure and the row remains in the
  sidebar for retry

### Requirement: Selecting a conversation lands on the chat view

Selecting a conversation from the sidebar SHALL switch the main surface to the
chat view when another view (Settings, Files) is active.

#### Scenario: Open from the Settings view

- **GIVEN** the Settings view is on screen
- **WHEN** the user clicks a conversation row
- **THEN** the chat view becomes active and shows that conversation

### Requirement: DA rejection logs carry a body snippet

The DA client's request-rejected WARN log SHALL include a bounded snippet of
the response body alongside status and path, so a 404 is diagnosable as
route-missing versus row-missing from the desktop log alone.

#### Scenario: 404 from a missing row

- **WHEN** DA rejects a request with a JSON error body
- **THEN** the WARN line includes the leading bytes of that body
