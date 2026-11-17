# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: A run's answer is on exactly one surface at a time

The system MUST render a run's answer on exactly one surface at any
moment. Three can carry it — the live activity card, the in-flight
transcript row, and the durable transcript row — and which one owns it
SHALL be decided by state the store holds, never inferred from message
counts, elapsed time, or event arrival order.

While the run is producing or settling its answer: a run that has made tool
calls owns it in the activity card, and a tool-free run owns it in the
in-flight transcript row. Once the durable row carries it, neither live
surface may.

#### Scenario: A tool-free run hands its answer to the transcript

- **GIVEN** a run with no tool calls that has streamed an answer
- **WHEN** its durable row arrives
- **THEN** the in-flight row is gone and the answer appears once

#### Scenario: A tool-bearing run hands its answer to the transcript

- **GIVEN** a run whose answer streamed inside the activity card
- **WHEN** its durable row arrives
- **THEN** the card no longer carries the answer and the answer appears
  once, below the card

#### Scenario: No surface is left holding a stale copy

- **GIVEN** any completed run
- **THEN** the streamed answer, the held answer, the activity feed, and the
  captured narration are all empty

### Requirement: Every live surface of a run retires on one signal

The system MUST retire the streamed answer, the held answer, the activity
feed, and the captured round narration on one condition: a durable
conversation snapshot arriving for the conversation on screen while no run
is in flight. Retiring any of them on a different signal reintroduces a
window in which two surfaces disagree about who owns the answer.

A run whose durable snapshot never arrives MUST keep its live surfaces —
they are then the only record of the answer.

#### Scenario: The retirement is one moment

- **GIVEN** a settled run whose live surfaces are still showing
- **WHEN** its durable snapshot lands
- **THEN** all of its live state clears together

#### Scenario: A run with no snapshot keeps its answer on screen

- **GIVEN** a run that settled but whose reconcile never produced a
  snapshot
- **THEN** its answer is still readable on the live surface
