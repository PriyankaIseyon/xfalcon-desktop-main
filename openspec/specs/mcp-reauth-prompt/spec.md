# mcp-reauth-prompt Specification

## Purpose

An MCP credential a server has refused stops looking like a transport failure, gets
evicted so it cannot be replayed, and produces one persistent prompt the reader
resolves with a single reconnect.

## Scope

| File                                   | What it owns here                                          |
| -------------------------------------- | ---------------------------------------------------------- |
| `src-tauri/src/mcp/oauth/challenge.rs` | reading a 401 challenge, distinguishing insufficient-scope |
| `src-tauri/src/mcp/oauth/store.rs`     | evicting a refused grant                                   |
| `src/features/mcp/McpReauthPrompt.tsx` | the persistent prompt                                      |
| `src/features/mcp/mcpReauthCopy.ts`    | its copy                                                   |

## Boundaries

- **Registering a server and discovering its tools** belongs to
  `mcp-server-registry`.
- **The Settings card a server shows once reconnected** belongs to
  `settings-mcp-servers`.
- **The authorization flow the prompt launches** belongs to `mcp-oauth-flow`.

## Requirements

### Requirement: A refused credential keeps its name

The system MUST carry a credential rejection through the tool-execution path as
itself: which server was refused, and that the refusal was of the credential rather
than of the request or the network. It MUST NOT report such a failure as a transport
failure, and MUST NOT require a caller to read a message string to recover what the
type already knew.

Authentication failures MUST remain classified during session establishment,
tool discovery, explicit connection checks, and tool calls. A missing grant alone
MUST NOT be treated as evidence of an expired session: a registered server may
never have been signed in on this device. Storage and network failures do not
require sign-in. A rejection while connecting for a requested tool call and a
rejection answering that call MUST arrive as the same condition.

A 403 insufficient-scope is carried the same way. The grant is real and too narrow
rather than refused outright, but the action it needs is the same authorization
flow, so it MUST NOT be reported as a transport failure either.

#### Scenario: A call is answered 401

- **GIVEN** a run whose MCP server answers a tool call with 401 and a challenge
  header
- **THEN** the failure names that server and says its authorization was refused

#### Scenario: The connection is what was refused

- **GIVEN** a run whose MCP server refuses the saved credential when the session is
  established
- **THEN** the failure names that server and says its authorization was refused,
  rather than being flattened into a generic failure message

#### Scenario: A grant that is too narrow

- **GIVEN** a server that answers 403 insufficient-scope
- **THEN** the same condition is reported, because signing in again is the action
  either way

### Requirement: A refused grant is evicted, not replayed

The system MUST delete the stored grant for a server that has refused it. The next
attempt on that server SHALL run the authorization flow rather than attach a
credential the server has already rejected.

The system MUST NOT retry the refused call with the same credential — a retry cannot
succeed, so it spends a turn to arrive where it already was, hiding the real reason
behind an attempt count.

#### Scenario: The stored grant is dropped

- **GIVEN** a server that refuses the saved credential
- **THEN** that server's stored grant is removed

#### Scenario: No retry of a refused call

- **GIVEN** a tool call refused for its credential
- **THEN** it is not attempted again inside the same run

### Requirement: The reader is prompted at once, and the prompt persists

The system MUST prompt for fresh authorization when a requested tool call or an
explicit connection check encounters an OAuth authorization failure, naming the
server. The reader MUST NOT have to open Settings to discover that the requested
operation requires sign-in.

Registry scans and background tool discovery MUST NOT raise a reconnect modal.
They examine all eligible servers before the run selects any tool, so a missing
or refused grant there does not establish that the requested work needs that
server. Discovery failures remain classified and logged; explicit connection
checks and actual tool calls retain their recovery prompts.

The prompt MUST persist until the reader acts on it. It MUST NOT be a transient
notification, and clicking away MUST NOT dismiss it — a refused grant blocks every
tool call on that server, and a missable prompt leaves the run silently less capable
than it looks.

One prompt per server: concurrent failures MUST NOT duplicate an open prompt.
Distinct servers MUST be queued without replacing the server being authorized.
No server may trigger a prompt merely because it was examined during run preparation.
Browser sign-in MUST only be offered for registered OAuth servers.

#### Scenario: An expired grant during a run

- **GIVEN** a run whose server refuses its credential
- **THEN** a prompt naming that server appears without the reader navigating
  anywhere

#### Scenario: An unused server has no grant

- **GIVEN** an enabled OAuth server that has no grant on this device
- **WHEN** a query prepares its available tools
- **THEN** registry eligibility checks do not prompt for that server
- **AND** an explicit connection check can still offer sign-in

#### Scenario: Background discovery fails while the requested work succeeds

- **GIVEN** a usable MCP server and an unrelated server whose handshake or tool listing rejects its grant
- **WHEN** the run discovers tools and successfully calls the usable server
- **THEN** no reconnect modal interrupts the query
- **AND** a later requested tool call that encounters an authorization failure still raises a prompt

#### Scenario: Several failures, one prompt

- **GIVEN** a run that fails three tool calls on the same server for its credential
- **THEN** the reader is prompted once

#### Scenario: The prompt is not dismissed by accident

- **GIVEN** the prompt is showing
- **WHEN** the reader clicks outside it
- **THEN** it stays

### Requirement: Reconnecting is one action from the prompt

The system MUST offer the authorization flow from the prompt itself. On success the
server SHALL be returned to service and re-tested, so the prompt closes against a
verified connection, and Settings agrees without a reload.

A reconnect the reader abandons, or that fails, MUST leave the server visibly refused
rather than appearing to have worked.

#### Scenario: Reconnect succeeds

- **GIVEN** the prompt for a refused server
- **WHEN** the reader reconnects and authorizes
- **THEN** the server is connected again and the prompt closes

#### Scenario: Reconnect is abandoned

- **GIVEN** the prompt for a refused server
- **WHEN** the reader cancels the authorization
- **THEN** the server is still reported as refused

#### Scenario: Reconnection requires an operator-issued client

- **WHEN** automatic reconnection reports that dynamic client registration is unavailable
- **THEN** the prompt offers the same client ID and optional client secret fields as Settings
- **AND** sign-in failures remain visible inside the dialog
- **AND** reconnection is complete only after the server connection check succeeds
- **AND** entered client details are cleared before another server is presented

### Requirement: Dismissing hides the prompt, not the state

The system MUST let the reader dismiss the prompt without reconnecting, and
dismissal MUST NOT be recorded as a resolution. The server SHALL remain visibly
refused wherever its state is shown.

#### Scenario: Dismissed, still refused

- **GIVEN** a prompt the reader dismissed without reconnecting
- **THEN** that server is still shown as refused in Settings

## Provenance

From the delta of `add-mcp-reauth-prompt`.
