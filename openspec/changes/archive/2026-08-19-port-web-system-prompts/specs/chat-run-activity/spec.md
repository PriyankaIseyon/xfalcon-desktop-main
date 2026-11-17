# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: Every run carries a system prompt

The system MUST place a system message at the head of every run's context.
Its content SHALL be the conversation's own system prompt when it declares
one, and xfalcon-web's `DEFAULT_SYSTEM_PROMPT` otherwise. A conversation
prompt REPLACES the default; it is never appended to it.

The prompt bodies MUST be byte-for-byte copies of web's, so the two
surfaces answer the same question the same way. Tests SHALL pin their size
and their section markers, so a truncated or edited copy fails rather than
degrades silently.

#### Scenario: A plain question

- **GIVEN** a run whose conversation declares no system prompt
- **THEN** its context opens with a system message carrying the default
  prompt

#### Scenario: A conversation prompt replaces the default

- **GIVEN** a conversation that declares its own system prompt
- **THEN** the run carries that prompt alone

### Requirement: A visualization request carries the output contract

The system MUST append the `<visualization_contract>` block when a turn is
a visualization request, using the same gate as web: a loaded skill whose
name matches the BI pattern makes every turn one; otherwise the message
decides. The gate SHALL be a pure function of the message and the loaded
skills, so the same request always takes the same route.

Without that block, a request to build a dashboard or a report has nothing
telling it what those are in this product, and it is answered with SQL.

#### Scenario: Build a dashboard

- **GIVEN** the message "build a dashboard of sales by region"
- **THEN** the run's system prompt contains the visualization contract

#### Scenario: A plain question does not carry it

- **GIVEN** the message "what tables do i have"
- **THEN** the run's system prompt is the base prompt alone

#### Scenario: A visualization skill gates every turn

- **GIVEN** a loaded skill named `ida-dashboard`
- **THEN** every turn carries the contract regardless of its message

### Requirement: The tool inventory reaches the model

The system MUST append the `<available_tools>` block whenever a run has
tools, listing every tool grouped under its server, servers ordered
alphabetically and tools ordered within each server. A tool that declares
no description SHALL be listed as `(no description)`.

A run with no tools MUST NOT carry the block, or an empty inventory would
tell the model tools exist when none do.

#### Scenario: Tools are grouped and ordered

- **GIVEN** tools from two servers
- **THEN** the inventory lists both servers alphabetically, each with its
  own tools in order

#### Scenario: No tools, no block

- **GIVEN** a run that resolved no tools
- **THEN** the system prompt ends before `<available_tools>`
