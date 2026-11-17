# chat-conversation-open Specification

## Purpose

What happens between a click on a sidebar row and a readable transcript: how many
cloud requests it costs, what an offline device shows instead, and how a row whose
conversation no longer exists stops being clickable forever.

## Scope

| File                                 | What it owns here                                                |
| ------------------------------------ | ---------------------------------------------------------------- |
| `src-tauri/src/chat/service.rs:449`  | `conversation_open` — one reconcile, and the not-found self-heal |
| `src-tauri/src/chat/service.rs:1103` | `open_and_emit` — the reconcile that IS the access check         |
| `src-tauri/src/chat/service.rs:1969` | the error mapping that keeps not-found typed                     |
| `src/features/chat/useChat.ts:209`   | `openConversation` — the client half of the self-heal            |
| `src-tauri/src/clients/da/mod.rs`    | the request-rejected WARN log and its body snippet               |

## Boundaries

- **The local encrypted projection and its purge** belong to
  `conversation-projection`. This capability decides WHEN a purge is ordered.
- **The sidebar's rows, grouping and ordering** belong to `sidebar-hierarchy`.
- **Which view is on screen** belongs to `app-shell-material`.
- **The stale-copy banner's placement** belongs to `chat-thread-layout`; the
  availability value that drives it is set here.
- **Creating a conversation** is lazy and belongs to `run-execution`: a row is
  minted on first send, never on "New chat".

## Requirements

### Requirement: Opening a conversation performs exactly one cloud reconcile

The open command SHALL issue exactly one cloud bootstrap request per invocation.
The reconcile inside the open path IS the access check; no separate pre-flight
reconcile may run on that path.

Mutating commands keep their own pre-flight check, because they must not run against
a foreign or dead id. Only the open path is exempt, and only because its own
reconcile already answers the same question.

#### Scenario: Single bootstrap request per sidebar click

- **WHEN** the reader clicks a conversation row in the sidebar
- **THEN** the conversation bootstrap endpoint is called at most once for that click

### Requirement: Stale fallback preserves the original cloud error class

The open path SHALL fall back to the local snapshot, marked stale, when the cloud
bootstrap fails with an unavailable or not-found result AND a local snapshot exists.

When no local snapshot exists, the open MUST fail with the original error class:
not-found stays not-found and is never collapsed into unavailable. The two lead to
opposite client behaviour, so collapsing them would make a network blip delete a
sidebar row.

A stale view MUST report its staleness to the reader rather than presenting an
offline copy as current.

#### Scenario: Offline open from local copy

- **GIVEN** a conversation with a readable local projection row
- **WHEN** the cloud bootstrap returns not-found or is unreachable
- **THEN** the conversation opens from the local copy
- **AND** the view reports stale availability

#### Scenario: NotFound without local copy is typed

- **GIVEN** a conversation with no local projection row
- **WHEN** the cloud bootstrap returns not-found
- **THEN** the command fails with the dedicated not-found variant, distinguishable
  by the frontend without matching prose

### Requirement: Authoritative NotFound self-heals the workspace

When an open fails with the not-found variant, the system SHALL self-heal rather
than leave a permanently dead sidebar row:

- the service purges the conversation's local projection row, so the dead id cannot
  resurface as a stale open;
- the service clears the active-conversation pointer and unbinds the conversation's
  project;
- the client reports the outcome to the reader;
- the client refreshes the workspace conversation list from the cloud;
- if the dead conversation was the one on screen, the view resets to the new-chat
  state.

Transient failures MUST NOT trigger any of this. The row and the state are kept so
the reader can retry, and the failure is reported inline.

The typed variant MUST be recognizable by a stable machine token rather than by
user-facing prose, so the copy can change without changing the behaviour.

#### Scenario: Clicking a deleted conversation

- **GIVEN** a sidebar row whose conversation no longer exists on the server and has
  no local copy
- **WHEN** the reader clicks it
- **THEN** the reader is told it no longer exists
- **AND** the sidebar list is re-fetched
- **AND** the chat surface is not left in a partial or errored state

#### Scenario: Network blip is not treated as deletion

- **GIVEN** the server is unreachable
- **WHEN** the reader clicks a conversation with no local copy
- **THEN** the inline error reports the failure and the row remains in the sidebar
  for retry

#### Scenario: A dead conversation not on screen does not reset the view

- **GIVEN** the reader is in another conversation
- **WHEN** they click a dead row
- **THEN** the list refreshes and the conversation on screen is untouched

### Requirement: Opening binds the conversation's project, or gives it up loudly

An open SHALL restore the conversation's bound project when that binding is usable,
and SHALL clear the active project when it is not, with the reason logged.

A conversation whose bound project cannot be reopened MUST still open. The
transcript stays readable and local tools fail closed; a missing folder is not a
reason to refuse a conversation.

#### Scenario: A bound project is restored

- **GIVEN** a conversation bound to a trusted project folder that is present
- **WHEN** it is opened
- **THEN** that project is the active one

#### Scenario: An unreachable folder does not block the open

- **GIVEN** a conversation whose bound folder is on a disconnected drive
- **WHEN** it is opened
- **THEN** the transcript renders, the active project is cleared, and the reason is
  logged

### Requirement: Selecting a conversation lands on the chat view

Selecting a conversation from the sidebar SHALL switch the main surface to the chat
view when another view is active.

#### Scenario: Open from the Settings view

- **GIVEN** the Settings view is on screen
- **WHEN** the reader clicks a conversation row
- **THEN** the chat view becomes active and shows that conversation

### Requirement: DA rejection logs carry a body snippet

The data-plane client's request-rejected warning SHALL include a bounded snippet of
the response body alongside the status and the path, so a not-found is diagnosable
as route-missing versus row-missing from the desktop log alone.

#### Scenario: 404 from a missing row

- **WHEN** the server rejects a request with a JSON error body
- **THEN** the warning line includes the leading bytes of that body

## Provenance

Consolidated from the delta of `fix-conversation-open-web-parity`, reconciled
against the shipped code.

The project-binding requirement is new to the baseline: `conversation_open` restores
or clears the active project on every open, and no delta stated it. It is separated
from `conversation-project-binding`, which owns the binding itself; what is here is
only what an OPEN does to it.
