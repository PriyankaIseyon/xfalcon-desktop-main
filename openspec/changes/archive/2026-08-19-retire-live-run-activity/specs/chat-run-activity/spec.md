# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: Live run state is retired by its durable record

The live activity card MUST show a given tool round exactly once in the
transcript. Live run state — the tool-invocation feed, the captured
round narration, and the admitted-invocation set that scopes both —
belongs to an in-flight run and SHALL be retired when a conversation
snapshot for the conversation on screen arrives while no run is in
flight. That snapshot is the run's durable record; its tool rounds
render as the umbrella above the answer, so the live card must not
also draw them below it.

Retirement SHALL be keyed to the snapshot's arrival, never to the run's
falling edge: the run's terminal event precedes the reconcile that
produces the snapshot, so clearing on the falling edge would leave the
transcript with no tool record for that gap. A run whose snapshot never
arrives keeps its live card, which is then the only record of the round.

A late tool-lifecycle event for a retired run MUST NOT resurrect the
card.

#### Scenario: A finished run's tools appear once, above the answer

- **GIVEN** a completed run whose tool round and final answer arrive in
  a conversation snapshot
- **WHEN** the snapshot is applied
- **THEN** the transcript shows the user message, then one tool
  umbrella, then the answer — and no live activity card below it

#### Scenario: The card survives the reconcile gap

- **GIVEN** a run that has just reached a terminal state
- **WHEN** its durable snapshot has not arrived yet
- **THEN** the live activity card stays on screen

#### Scenario: A mid-run snapshot leaves the live card alone

- **GIVEN** an in-flight run with an approval prompt in its activity
  feed
- **WHEN** the same conversation is re-projected (a mid-run reconcile or
  a first-message rename)
- **THEN** the activity feed and the prompt survive

#### Scenario: The previous run's card does not ride into the next send

- **GIVEN** a conversation whose last run used tools and has settled
- **WHEN** the next message is sent and its post-submit snapshot arrives
- **THEN** the settled run's live card is gone before the new run starts

#### Scenario: A straggling tool event cannot resurrect a retired run

- **GIVEN** a run retired by its durable snapshot
- **WHEN** a lifecycle event for one of its invocations arrives late
- **THEN** the activity feed stays empty

### Requirement: One predicate decides whether a run is still working

The system MUST decide "this run is still working" in exactly one place,
read by the composer's send/stop affordance, the Escape-cancels binding,
the live card's open-while-running choreography, and the retirement rule
above. That predicate MUST count the run states `draft`, `queued`,
`running`, `awaiting_approval`, and `recovering` as working, and every
other state as settled.

A run parked on a tool approval is working, not finished. An interrupted
run is settled here even though the Rust journal counts it as in flight
to keep it resumable — the UI hands that run to the recovery notice and
returns the composer.

#### Scenario: A run parked on an approval still reads as working

- **GIVEN** a run that has entered `awaiting_approval`
- **THEN** the composer keeps its stop affordance and the live activity
  card keeps its running spinner and open state

#### Scenario: An interrupted run releases the composer

- **GIVEN** a run that reached `interrupted`
- **THEN** the recovery notice is shown and the composer accepts input
