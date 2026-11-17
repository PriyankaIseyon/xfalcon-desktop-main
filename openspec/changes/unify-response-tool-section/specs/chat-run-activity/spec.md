## MODIFIED Requirements

### Requirement: Streamed content renders as one activity section and one answer

An active response SHALL render at most one activity section followed by at
most one assistant answer. The activity section SHALL contain all live tool
rows and any enabled reasoning. Prose captured at a tool-round boundary SHALL
NOT render as a separate transcript row. The current streaming prose SHALL
render through the message typography and markdown pipeline as the response's
answer.

The durable transcript SHALL use the same order. Settlement MUST NOT create a
second tool section or reveal intermediate tool-round narration.

#### Scenario: A multi-round run stays one response

- **GIVEN** an active run that narrates, calls one tool, narrates again, calls
  three tools, and then streams its answer
- **WHEN** the run renders
- **THEN** one activity section contains all four tool rows
- **AND** one streaming answer renders after it
- **AND** neither intermediate narration renders as a message row

#### Scenario: Settlement preserves the composition

- **GIVEN** one live activity section followed by one streaming answer
- **WHEN** the durable snapshot lands
- **THEN** one durable activity section followed by one durable answer replaces
  it without duplication or reordering

#### Scenario: A run with no tools needs no activity section

- **GIVEN** a run with no tool calls and no displayed reasoning
- **WHEN** its answer streams
- **THEN** only the assistant answer renders
