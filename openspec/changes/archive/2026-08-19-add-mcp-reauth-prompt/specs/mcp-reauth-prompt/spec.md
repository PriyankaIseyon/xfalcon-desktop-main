# mcp-reauth-prompt — Spec Delta

## ADDED Requirements

### Requirement: A refused credential keeps its name

The system MUST carry a credential rejection through the tool-execution
path as itself: which server was refused, and that the refusal was of the
credential rather than of the request or the network. It MUST NOT report
such a failure as a transport failure, and MUST NOT require a caller to
read a message string to recover what the type already knew.

Both routes count. A run reaches a server twice — once to connect, once
per call — and either can be the thing that is refused. A rejection while
connecting and a rejection answering `tools/call` are the same condition
and MUST arrive as the same one.

A 403 `insufficient_scope` is carried the same way. The grant is real and
too narrow rather than refused outright, but the action it needs is the
same authorization flow, so it MUST NOT be reported as a transport failure
either.

#### Scenario: A call is answered 401

- **GIVEN** a run whose MCP server answers `tools/call` with 401 and a
  `WWW-Authenticate` challenge
- **THEN** the failure names that server and says its authorization was
  refused, rather than reporting a disconnection

#### Scenario: The connection is what was refused

- **GIVEN** a run whose MCP server refuses the saved credential when the
  session is established
- **THEN** the failure names that server and says its authorization was
  refused, rather than being flattened into a generic failure message

#### Scenario: A grant that is too narrow

- **GIVEN** a server that answers 403 `insufficient_scope`
- **THEN** the same condition is reported, because signing in again is the
  action either way

### Requirement: A refused grant is evicted, not replayed

The system MUST delete the stored grant for a server that has refused it.
The next attempt on that server SHALL run the authorization flow rather
than attach a credential the server has already rejected.

The system MUST NOT retry the refused call with the same credential. A
retry cannot succeed — the same token is refused the same way — so it
spends a turn to arrive where it already was, and hides the real reason
behind an attempt count.

#### Scenario: The stored grant is dropped

- **GIVEN** a server that refuses the saved credential
- **THEN** that server's stored grant is removed, so nothing replays it

#### Scenario: No retry of a refused call

- **GIVEN** a tool call refused for its credential
- **THEN** it is not attempted again inside the same run

### Requirement: The user is prompted at once, and the prompt persists

The system MUST prompt for a fresh authorization as soon as a grant is
refused during a run, naming the server. The user MUST NOT have to open
Settings to discover that a sign-in is what is wanted.

The prompt MUST persist until the user acts on it — reconnecting or
dismissing it. It MUST NOT be a transient notification, and clicking away
from it MUST NOT dismiss it: a refused grant blocks every tool call on that
server, so a prompt that can be missed leaves the run silently less capable
than it looks, which is the failure this change exists to end.

One prompt per server. A run may fail several calls against the same
server, and each MUST NOT raise its own prompt.

#### Scenario: An expired grant during a run

- **GIVEN** a run whose server refuses its credential
- **THEN** a prompt naming that server appears without the user navigating
  anywhere

#### Scenario: Several failures, one prompt

- **GIVEN** a run that fails three tool calls on the same server for its
  credential
- **THEN** the user is prompted once

#### Scenario: The prompt is not dismissed by accident

- **GIVEN** the prompt is showing
- **WHEN** the user clicks outside it
- **THEN** it stays, because only an explicit action may close it

### Requirement: Reconnecting is one action from the prompt

The system MUST offer the authorization flow from the prompt itself. On
success the server SHALL be returned to service and re-tested, so the
prompt closes against a verified connection rather than an assumed one,
and the Settings panel agrees without a reload.

A reconnect the user abandons, or that fails, MUST leave the server
visibly refused rather than appearing to have worked.

#### Scenario: Reconnect succeeds

- **GIVEN** the prompt for a refused server
- **WHEN** the user reconnects and authorizes
- **THEN** the server is connected again and the prompt closes

#### Scenario: Reconnect is abandoned

- **GIVEN** the prompt for a refused server
- **WHEN** the user cancels the authorization
- **THEN** the server is still reported as refused, and nothing claims
  otherwise

### Requirement: Dismissing hides the prompt, not the state

The system MUST let the user dismiss the prompt without reconnecting, and
dismissal MUST NOT be recorded as a resolution. The server SHALL remain
visibly refused wherever its state is shown, so a dismissed prompt does not
become a server that looks healthy and is not.

#### Scenario: Dismissed, still refused

- **GIVEN** a prompt the user dismissed without reconnecting
- **THEN** that server is still shown as refused in Settings, with the
  sign-in it needs still stated
