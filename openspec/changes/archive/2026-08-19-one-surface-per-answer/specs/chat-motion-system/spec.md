# chat-motion-system — Spec Delta

## MODIFIED Requirements

### Requirement: The answer does not blank between streaming and durable

The transcript MUST NOT blank or repaint the answer in the gap between a
run's terminal inference event and the arrival of its durable row. That gap
is real — the reconcile that produces the durable row runs after the
terminal event, with retries — so the streamed text SHALL be held across
it.

The hold MUST live in the store beside the text it continues, and MUST be
released by the arrival of the durable row rather than by a heuristic over
message counts or roles. A heuristic release can fire early (blank) or late
(two copies); both were observed.

A run that died MUST NOT have its partial text held: a partial answer is
not an answer, and the failure notice takes the surface instead.

#### Scenario: The gap is covered

- **GIVEN** a run whose terminal inference event has landed
- **WHEN** its durable row has not arrived yet
- **THEN** the answer is still on screen, unchanged

#### Scenario: The handover does not repaint

- **WHEN** the durable row arrives
- **THEN** the held text retires in the same commit, so the answer is never
  absent and never doubled

#### Scenario: A dead run holds nothing

- **GIVEN** a run that failed mid-answer
- **THEN** its partial text is dropped rather than held
