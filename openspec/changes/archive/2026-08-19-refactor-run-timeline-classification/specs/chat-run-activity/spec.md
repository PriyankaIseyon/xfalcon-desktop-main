# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: Streamed content renders where its durable form lives

Assistant content MUST render, from its first token, on the surface that
will hold it once the run has settled. No surface may take content away
from another surface while a run is in flight.

- Assistant prose — a round's narration and the final answer alike — is
  durably an assistant message, so it MUST render as a transcript prose
  row, with the message typography and the full markdown pipeline.
- Tool calls are durably `tool_calls` on that message, so they MUST render
  as tool rows.
- Reasoning is never durable, so it MUST render only on a live surface and
  MUST NOT be mistaken for either of the above.

The surface MUST NOT be chosen by whether the run has called a tool, or by
any other fact that is unknown when the first token arrives. Such a rule is
revised after the content is on screen, and every revision is a relocation
the reader watches.

#### Scenario: A tool-bearing run's answer streams as the answer

- **GIVEN** an active run that has already completed a tool round
- **WHEN** its answer streams
- **THEN** the text renders as a transcript prose row with the streaming
  caret, in message typography, through the markdown pipeline
- **AND** no copy of it is drawn inside any activity card

#### Scenario: The settle changes nothing about the answer

- **GIVEN** a streamed answer on screen
- **WHEN** its durable row lands
- **THEN** the text is in the same place, in the same typography, and
  neither blanks nor moves

#### Scenario: Round prose is never withdrawn

- **GIVEN** an active run that streamed "I'll check the sales table."
- **WHEN** its first tool invocation is requested
- **THEN** that prose remains on screen as a prose row
- **AND** the round's tool rows render below it, not above it
- **AND** the text is still on screen after the durable snapshot lands

### Requirement: A live surface never draws an empty card

The live activity card MUST be drawn only when it has a tool row or a
visible segment to draw. Whether it has one MUST be decided by a single
predicate that the thinking indicator reads as well, so the two can never
both be absent or both be present.

The card component MUST refuse to render a body with no visible blocks on
its own account, rather than relying on its caller to have checked. A card
whose content is withheld by a display flag draws nothing, which is a
bordered empty box, which reads as a broken app.

#### Scenario: A run that has only reasoned, with reasoning withheld

- **GIVEN** an active run with reasoning, no tool events, and reasoning
  display off
- **THEN** no activity card is rendered
- **AND** the thinking indicator is on screen

#### Scenario: A run that has only reasoned, with reasoning shown

- **GIVEN** an active run with reasoning, no tool events, and reasoning
  display on
- **THEN** a reasoning disclosure is on screen
- **AND** the thinking indicator is not, because something has arrived

## MODIFIED Requirements

### Requirement: A run's answer is on exactly one surface at a time

A run's answer text SHALL be owned by the store through its whole life and
drawn by exactly one surface at every moment of it.

Two surfaces can render a run's answer, and they are distinguished by
durability, not by timing:

|     | Surface                           | Source                       |
| --- | --------------------------------- | ---------------------------- |
| A   | the synthetic `stream-` prose row | `streamingText`, then a hold |
| B   | the durable transcript row        | `messages`                   |

`streamingText` is a run still producing tokens. `settlingText` is that
same text after the terminal inference event and before the durable row
lands — a gap that genuinely exists, because `finish_run` reconciles after
that event, with retries. A owns the text while either is non-empty; B owns
it afterwards; the handover is the durable snapshot's arrival, which clears
both.

The live activity card MUST NOT be a third surface for answer text. It
holds tool rows, and nothing that a durable message will carry.

#### Scenario: Answer text is never drawn twice

- **GIVEN** any moment of any run, with or without tool calls
- **WHEN** the transcript renders
- **THEN** the run's answer text appears exactly once

#### Scenario: The activity card carries no answer text

- **GIVEN** an active run with tool events and streaming answer text
- **WHEN** the activity card renders
- **THEN** its body contains tool rows only

### Requirement: Every live surface of a run retires on one signal

Every live field of a run MUST be cleared by the same condition — the
streamed text, the settling text, the activity rows, the captured round
prose and the admitted-invocation set alike. That condition is a durable
snapshot arriving for a conversation whose run is no longer in flight.
Retiring them separately is what let a streamed answer and its durable row
be on screen at the same time.

Reasoning is exempt and scoped to the conversation instead. The snapshot
retires what it carries, and it carries no reasoning; clearing it there
would delete the only copy that ever existed. A conversation switch still
drops it, so one run's thinking is never attached to another
conversation's answer.

#### Scenario: The durable snapshot retires the live surfaces together

- **GIVEN** a settled run whose live state is on screen
- **WHEN** the durable snapshot for its conversation arrives
- **THEN** the streamed text, the activity rows and the captured round
  prose are all cleared in the same commit

#### Scenario: A mid-run reconcile keeps them

- **GIVEN** a run still in flight
- **WHEN** the conversation on screen is re-applied (a rename, a reconcile)
- **THEN** the live surfaces are unchanged

## REMOVED Requirements

### Requirement: Pre-final streamed text lives inside the live activity card

**Reason**: The rule it states — that the transcript grows no prose tail
once a run has called a tool — is what moved the answer into the activity
card and what deleted a round's narration at the first tool event. It
decides the owning surface from `activity.length > 0`, a fact unknown when
the text starts arriving, so the decision is revised while the reader is
watching. Replaced by "Streamed content renders where its durable form
lives".

**Migration**: Round prose and answer text both render as transcript prose
rows. `ThinkingBlock` and the card's `liveText` prop are removed;
`liveThinking` becomes `roundProse` and is rendered as prose. The card
keeps its tool rows and its open/close choreography.

### Requirement: The card opens while streaming and closes when the run ends

**Reason**: Its "no collapse on the first answer token" clause exists only
because the answer streamed inside the card. With the answer in the
transcript, the card's content is tool rows, which are complete when the
round is.

**Migration**: The card keeps auto-open on the run's rising edge, auto-close
on the falling edge, and manual override for the rest of the run — all of
which are stated by "The card auto-expands during the run and auto-collapses
after" in `unify-tool-activity-ux`. Only the answer-token clause is dropped.
