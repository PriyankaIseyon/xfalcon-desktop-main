# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: Tool activity is one in-transcript card

Live tool activity SHALL render as a single disclosure card inside the
transcript flow, after the last message of the conversation — not as a
floating pill and not below the composer. The card lists one row per tool
invocation (tool id, lifecycle status, elapsed time, safe error text),
sourced exclusively from typed `ToolInvocationEvent`s.

#### Scenario: Card position

- **GIVEN** a run that has invoked at least one tool
- **WHEN** the transcript renders
- **THEN** exactly one activity card appears, in the message flow after the
  last message, and no activity UI exists below the composer

### Requirement: The card auto-expands during the run and auto-collapses after

While the run is active the card SHALL be expanded (its header shows a
working indicator). When the run reaches a terminal state the card SHALL
collapse to its one-line header. The user MAY toggle it manually at any
time; a manual choice wins over the automatic state until the next run
starts.

#### Scenario: Auto-collapse on completion

- **GIVEN** the activity card is auto-expanded during a streaming run
- **WHEN** the run completes and the response is rendered
- **THEN** the card collapses to its header
- **AND** clicking the header re-expands it

#### Scenario: Manual expand after completion

- **GIVEN** a finished conversation whose run invoked tools
- **WHEN** the user clicks the collapsed activity header
- **THEN** the full invocation list is shown until the user collapses it or a
  new run starts
