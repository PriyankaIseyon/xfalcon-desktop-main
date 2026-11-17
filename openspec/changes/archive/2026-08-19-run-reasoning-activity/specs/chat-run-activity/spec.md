# chat-run-activity — Spec Delta

## MODIFIED Requirements

### Requirement: Pre-final streamed text lives inside the live activity card

The live activity card MUST contain, while a tool-bearing run is in
flight, every piece of assistant text streamed so far: each completed
round's prose as a quiet thinking segment (in capture order, interleaved
with that round's tool rows) and the currently-streaming text as its
last block, carrying the streaming caret. Once the run's first tool call
is requested, the transcript MUST NOT grow a prose tail for that run —
narration never renders as answer prose. Tool-free runs keep streaming
as transcript prose and materialize no card.

#### Scenario: Narration moves into the card at the first tool call

- **GIVEN** an active run that streamed "I'll check what data sources…"
- **WHEN** the run's first tool invocation is requested
- **THEN** that text renders inside the live activity card as a thinking
  segment and the transcript prose tail disappears

#### Scenario: Later rounds stream inside the open card

- **GIVEN** a run whose first tool round completed
- **WHEN** the model streams further text before requesting more tools
- **THEN** the text streams inside the card (with the caret), not as
  transcript prose

### Requirement: The card opens while streaming and closes when the run ends

The live card SHALL auto-open on the run's rising edge, remain open
while the run streams (there is no collapse on the first answer token —
the answer streams inside the card), and auto-close on the run's falling
edge. A manual toggle takes precedence for the remainder of the run.
Elapsed time on the label is client-observed and advances only with
event/render activity, never a ticking interval.

#### Scenario: No mid-stream collapse

- **GIVEN** the open card with text streaming inside
- **WHEN** more answer tokens arrive
- **THEN** the card stays open until the run reaches a terminal state
