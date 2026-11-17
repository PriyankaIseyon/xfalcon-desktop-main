# chat-run-activity — Spec Delta

## MODIFIED Requirements

### Requirement: Every live surface of a run retires on one signal

The system MUST retire the streamed answer, the held answer, the activity
feed, and the captured round narration on one condition: a durable
conversation snapshot arriving for the conversation on screen while no run is
in flight. Retiring any of them on a different signal reintroduces a window
in which two surfaces disagree about who owns the answer.

A run whose durable snapshot never arrives MUST keep its live surfaces —
they are then the only record of the answer.

The run's reasoning is exempt, and the exemption follows from the same
premise. Each of the four fields above retires on the snapshot because the
snapshot carries that content: the answer becomes the durable row, the tool
rounds and their prose become the umbrella above it. Nothing in the snapshot
carries reasoning — it is not persisted and does not appear in the
conversation view — so retiring it there would delete the only copy that
ever existed, which is exactly why a snapshot-less run keeps its card.

Reasoning MUST therefore be handed from the live card to the durable umbrella
at that same moment, rather than kept on a surface of its own. Exactly one
surface draws a run's activity at any time; if both held reasoning the same
thinking would be on screen twice, and if neither did it would be lost at the
handover.

Reasoning SHALL be retired when the view stops being about the run that
produced it: a new run is adopted for this conversation, a different
conversation is shown, or the store is reset. It MUST NOT be inherited by the
next run, and it MUST NOT survive a conversation switch — either would attach
one run's thinking to another run's answer.

#### Scenario: The retirement is one moment

- **GIVEN** a settled run whose live surfaces are still showing
- **WHEN** its durable snapshot lands
- **THEN** the streamed answer, the held answer, the activity feed, and the
  captured narration all clear together

#### Scenario: A run with no snapshot keeps its answer on screen

- **GIVEN** a run that settled but whose reconcile never produced a snapshot
- **THEN** its answer is still readable on the live surface

#### Scenario: The reasoning survives the durable snapshot

- **GIVEN** a settled reasoning run whose durable snapshot has landed
- **THEN** the answer comes from the snapshot, and the reasoning is inside
  the durable umbrella above it, collapsed and reopenable

#### Scenario: The handover draws it once, not twice

- **GIVEN** a run whose reasoning is showing in its live card
- **WHEN** the durable snapshot retires that card
- **THEN** the reasoning appears in the umbrella and nowhere else

#### Scenario: The next run does not inherit the last one's thinking

- **GIVEN** a conversation whose previous run reasoned
- **WHEN** the next message is sent and its run is adopted
- **THEN** no reasoning is on screen before the new run streams anything

#### Scenario: Switching conversations drops it

- **GIVEN** a run's reasoning on screen
- **WHEN** another conversation is opened
- **THEN** no reasoning is carried into it

### Requirement: Pre-final streamed text lives inside the live activity card

The run's card MUST contain, while the run is in flight, every piece of what
the run has produced so far: each stretch of streamed reasoning, each
completed round's prose as a quiet narration segment, that round's tool rows,
and the currently-streaming text as its last block, carrying the streaming
caret — all in the order the run produced them. Once the run's first tool call
is requested, the transcript MUST NOT grow a prose tail for that run —
narration never renders as answer prose. A tool-free run that streams no
reasoning keeps streaming as transcript prose and materializes no card.

Streamed reasoning and a round's narration MUST be kept distinguishable inside
that card. They are different things from different events: a round's prose is
answer text the model chose to emit and is durably attached to that round's
message, while reasoning arrives on its own event, is never persisted, and is
never part of an answer.

#### Scenario: Narration moves into the card at the first tool call

- **GIVEN** an active run that streamed "I'll check what data sources…"
- **WHEN** the run's first tool invocation is requested
- **THEN** that text renders inside the card as a narration segment and the
  transcript prose tail disappears

#### Scenario: Later rounds stream inside the open card

- **GIVEN** a run whose first tool round completed
- **WHEN** the model streams further text before requesting more tools
- **THEN** the text streams inside the card (with the caret), not as
  transcript prose

#### Scenario: A tool-free run's thinking still gets a card

- **GIVEN** a run that streams reasoning and calls no tools
- **THEN** a card exists holding that reasoning, and the answer renders
  outside it
