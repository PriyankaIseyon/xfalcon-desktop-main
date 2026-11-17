# chat-motion-system — Spec Delta

## ADDED Requirements

### Requirement: Streamed text commits at most once per frame

Streamed assistant text arrives one token at a time. The store SHALL
write each delta to state synchronously — every reader, including the
round-boundary capture that turns a round's prose into a thinking
segment, must see exact values — while notifying subscribers at most
once per animation frame, so the transcript re-renders (and re-parses
its markdown) once per frame rather than once per token. This mirrors
xfalcon-web's `scheduleDeltaFlush`.

Any non-delta state change MUST notify immediately and absorb a pending
coalesced notification, so a run's terminal event and the durable
transcript that follows it are never delayed behind the token stream.

#### Scenario: A burst of tokens costs one render

- **GIVEN** several text deltas arriving within one animation frame
- **THEN** subscribers are notified once, with the accumulated text

#### Scenario: A round boundary reads the exact streamed text

- **GIVEN** deltas that have not yet been flushed to subscribers
- **WHEN** the round's first tool invocation is requested
- **THEN** the captured thinking segment carries all of the streamed
  text

#### Scenario: Terminal events are not delayed behind the stream

- **GIVEN** a pending coalesced notification
- **WHEN** the run's inference `completed` event arrives
- **THEN** subscribers are notified immediately
