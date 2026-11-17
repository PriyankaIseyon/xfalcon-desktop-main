## ADDED Requirements

### Requirement: A turn cut off by a limit is never taken for a finished turn

The system MUST read why each provider turn ended. A tool call that was
unfinished when the turn ended MUST NOT run. The model MUST be told, in the next
request, that its output was cut off and to write less per call.

#### Scenario: A file body is cut off inside a tool call

- **GIVEN** the provider ends a turn at its output limit while a tool call's arguments are incomplete
- **WHEN** the turn is read
- **THEN** no tool call runs for it
- **AND** the next request carries a notice naming the output limit and asking for one file per call

#### Scenario: Repeated cut-offs

- **GIVEN** three turns in a row are cut off inside a tool call
- **WHEN** the fourth is cut off
- **THEN** the run fails with a message naming the output limit and suggesting fewer files per request

#### Scenario: A prose answer is cut off

- **GIVEN** a turn with no tool call is cut off by the output limit
- **WHEN** continuations are exhausted
- **THEN** the saved answer ends with a fixed sentence saying it was cut off

#### Scenario: Corrupt arguments without a limit

- **GIVEN** a tool call whose arguments do not parse and whose turn was not cut off
- **WHEN** the turn is read
- **THEN** the turn fails as an invalid response

### Requirement: A reply that leaves the plan unfinished is sent back to work

The system MUST track the model's latest checklist. A reply with no tool call
and open items MUST be answered with a notice naming them, at most three times
per run.

#### Scenario: Stopping early

- **GIVEN** a checklist with one item pending
- **WHEN** the model replies with no tool call
- **THEN** it is asked to continue and the reply it wrote stays in the saved answer

#### Scenario: Every item done

- **GIVEN** a checklist with every item completed
- **WHEN** the model replies with no tool call
- **THEN** the run finishes

### Requirement: A refusal or a full window ends the run with a reason

#### Scenario: A refusal

- **WHEN** the provider ends a turn with a refusal and no tool call
- **THEN** the run fails with a message that the model declined
