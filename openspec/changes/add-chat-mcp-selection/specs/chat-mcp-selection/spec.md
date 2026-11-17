## ADDED Requirements

### Requirement: A chat has one immutable MCP selection

The application SHALL offer All servers and each configured MCP server in the
chat composer. All SHALL be the default for new chats and legacy conversations.
Native code SHALL bind a new conversation to its selection and normalized server
endpoint. Changing selection SHALL start a fresh chat and preserve the old one.

#### Scenario: Select one server

- **WHEN** a user selects a server and sends the first message
- **THEN** the conversation records that server's ID and native-resolved endpoint
- **AND** every run in that conversation uses that binding

#### Scenario: Change an existing chat's selection

- **WHEN** the user chooses another server or All
- **THEN** a fresh chat starts with the new selection
- **AND** no history, summaries, result handles, drafts or attachments are inherited
- **AND** the original conversation remains available with its original selection

### Requirement: Specific server selection fails closed

The selected server SHALL be the only MCP server whose tools are declared or
executed. A selected server that is unavailable or whose endpoint changed SHALL
produce an error. Neither the bundled server nor All SHALL be substituted.
Malformed selection metadata or a renderer selection mismatch SHALL be rejected
before any history mutation or model request.

#### Scenario: Unselected server tool call

- **GIVEN** a chat bound to server A
- **WHEN** a model requests server B's tool
- **THEN** native policy refuses the call without contacting server B

#### Scenario: Selected endpoint changes

- **GIVEN** a chat bound to server A's original endpoint
- **WHEN** its registry row points to another endpoint
- **THEN** continuation fails without sending the old context to the new endpoint

#### Scenario: A server is renamed

- **GIVEN** a chat bound to server A
- **WHEN** registered servers are renamed or their display-name slugs change
- **THEN** server A retains a tool namespace derived from its immutable ID
- **AND** server B's earlier tool calls cannot become authorized as server A

#### Scenario: A saved summary belongs to another scope

- **GIVEN** a chat bound to one MCP server
- **WHEN** an authoritative context load returns an incompatible compaction or foreign tool history
- **THEN** the run fails before any model request
- **AND** no incompatible summary is replayed as plain text

#### Scenario: The account changes during admission

- **GIVEN** a selected chat is being submitted
- **WHEN** its account or session generation changes during validation
- **THEN** the submission fails before a different account's runtime can receive its context

### Requirement: Every chat entry point preserves selection

First send, attachment reservations, reopen, retry, edit and regenerate SHALL
preserve the conversation's binding. A run SHALL capture its selected runtime and
keep it unchanged through provider rounds, reconnects and approval pauses.

#### Scenario: Retry after reopen

- **GIVEN** a chat bound to one server and a failed run
- **WHEN** the chat is reopened and the run is retried
- **THEN** the same binding governs tool declaration and execution

#### Scenario: Attach before first send

- **GIVEN** a new chat with a specific server selected
- **WHEN** an attachment reserves its conversation
- **THEN** the reservation records that same selection
