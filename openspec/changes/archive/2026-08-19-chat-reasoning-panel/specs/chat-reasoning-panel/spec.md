# chat-reasoning-panel — Spec Delta

## ADDED Requirements

### Requirement: Reasoning that arrives is shown; reasoning that never arrives leaves no trace

The system MUST show the reasoning streamed for the conversation on screen once
at least one fragment has arrived for its run, and MUST NOT render any card,
header, or placeholder on account of reasoning for a run that produced none.

A run that reasons but calls no tools MUST still get a card. The card is the
run's account of itself, not a tool inventory, and a run whose whole visible
work was thinking would otherwise have nowhere to show it.

#### Scenario: A model that volunteers its thinking

- **GIVEN** a run whose provider streams reasoning
- **WHEN** the first fragment arrives
- **THEN** the run's activity card is in the transcript, carrying that text

#### Scenario: A model that streams none

- **GIVEN** a run that streams an answer, calls no tools, and reasons none
- **THEN** no activity card is rendered at any point in that run, before,
  during, or after

#### Scenario: Thinking without tools still has a home

- **GIVEN** a run that streams reasoning and calls no tools
- **THEN** its card exists and is named for the reasoning it holds, not for a
  count of calls that were never made

### Requirement: One card per run, holding everything it thought and did in the order it happened

The system MUST render a run's reasoning, its rounds' narration, and its tool
rows inside ONE disclosure, ordered as the run produced them. A stretch of
reasoning that arrived after a tool result MUST render below that result's row,
and a stretch that preceded it MUST render above.

Reasoning MUST NOT be given a card of its own. A reader following a run follows
one sequence; a second card forces them to correlate two, and — because
reasoning accumulates while tool rows arrive beside it — placed a later round's
thinking above the row it was reacting to, which is not the order anything
happened in.

Ordering MUST be derived from a single position that both the in-flight card
and the settled trail can compute: how many tool rows the run had produced when
the segment was captured. Two surfaces deriving position from state only one of
them has is how the same run comes to read differently before and after it
settles.

Where reasoning and narration were captured at the same position, reasoning
MUST come first — within a round the model reasons, then writes the prose that
announces what it is about to call.

Fragments MUST be ordered and de-duplicated on a ledger of their own, never the
answer's. Reasoning and text deltas are numbered from one run-wide sequence
counter, so a reasoning fragment admitted through the answer's ledger would
advance it and silently swallow the next answer delta.

#### Scenario: A multi-round run reads as one sequence

- **GIVEN** a run that reasoned, called a tool, then reasoned again
- **THEN** exactly one card exists, and inside it the first stretch of
  reasoning is above the tool row and the second is below it

#### Scenario: Reasoning and narration from one round

- **GIVEN** a round whose model reasoned and then streamed prose before
  calling a tool
- **THEN** the reasoning renders above that prose, and both above the row

#### Scenario: A replayed fragment cannot cost answer text

- **GIVEN** a run whose reasoning fragment arrives twice
- **THEN** the text appears once in the card
- **AND** every answer delta that followed is still in the answer

### Requirement: Reasoning is admitted only for the run on screen

The system MUST admit a reasoning fragment only when it belongs to the run
bound to the conversation being shown, on the same scoping rule as every
other inference event. The inference channel is a broadcast: a run streaming
for another conversation, or a stale superseded run, emits here too, and its
thinking must never be attributed to the conversation the reader is looking
at.

The settled run's reasoning MUST NOT reach an earlier run's trail. Reasoning
belongs to the newest run in the conversation, and filing it under a previous
answer's tools would attribute one run's thinking to another's work.

#### Scenario: A background run's thinking stays out

- **GIVEN** a conversation on screen with its own run
- **WHEN** a reasoning fragment arrives for a different run
- **THEN** the card is unchanged

#### Scenario: An earlier answer keeps its own trail

- **GIVEN** a transcript with two answers, each with a tool trail
- **WHEN** the newest run's reasoning is drawn
- **THEN** only the newest trail carries it

### Requirement: Reasoning renders inert, labelled, and never carries the answer

Reasoning MUST render as inert plain text in secondary ink: no markdown
execution, no HTML preview or table cards, no syntax highlighting. Reasoning
is untrusted model output that routinely contains half-finished fenced
blocks and partial markup, and executing it would let a mid-thought fragment
mint a preview frame or a table out of syntax the model has not finished
writing.

Reasoning MUST be distinguishable from a round's narration where the two sit
in one body. They are different things: narration is answer text the model
chose to emit and that its turn keeps durably, while reasoning is never
persisted and is never part of an answer. Presenting them identically would
make raw thinking read as something the model said.

The card MUST NOT carry the run's final answer. Which surface owns the answer
is already decided elsewhere, and a card that also showed it would put the
same answer on screen twice.

#### Scenario: A fenced block in mid-thought stays text

- **GIVEN** reasoning containing an unterminated fenced HTML block
- **THEN** it renders as visible text and no preview frame or table is
  created

#### Scenario: Thinking is marked as thinking

- **GIVEN** a card holding both a round's narration and a stretch of
  reasoning
- **THEN** the reasoning is labelled as such and the narration is not

#### Scenario: The card is not an answer surface

- **GIVEN** a settled run
- **THEN** its answer renders outside the card

### Requirement: Reasoning is bounded and its trimming is admitted

The system MUST cap the reasoning it retains for a run and SHALL drop from
the oldest end when that cap is passed, keeping the most recent text — that
is what a reader following a live run is watching. A long-lived window must
not be able to grow this state without limit.

Dropping MUST shed whole spent segments before cutting into a survivor, and a
cut MUST NOT split a surrogate pair. When text has been dropped the card MUST
say so: a body that silently begins mid-sentence reads as a bug in the model
rather than a bound in the app.

#### Scenario: A very long reasoning stream

- **GIVEN** a run whose reasoning passes the retention cap
- **THEN** the newest reasoning is still shown, the oldest is dropped, and
  the card states that it was trimmed

### Requirement: One live indicator per run

The generic thinking indicator MUST NOT be shown while the run's card is on
screen. The pill means "a run is in flight and nothing has streamed yet"; once
reasoning is streaming, something has, and showing both tells the reader the
same thing twice in two voices — the defect this transcript has been repaired
for repeatedly.

#### Scenario: Reasoning supersedes the pill

- **GIVEN** a run in flight showing the thinking pill
- **WHEN** its first reasoning fragment arrives
- **THEN** the pill is gone and the card is what reports the run

### Requirement: The card does not announce itself token by token

The card MUST NOT be an assistive-technology live region. A screen reader
narrating every chain-of-thought token would bury the answer under the
thinking, and the reader could not get past it.

The card MUST be a disclosure with an accessible name whose expanded state
is exposed programmatically, so a reader using assistive technology can open
the reasoning deliberately and hear it on request.

#### Scenario: The answer is what gets announced

- **GIVEN** a run streaming reasoning
- **THEN** no reasoning text is announced automatically

#### Scenario: The reasoning is reachable on purpose

- **GIVEN** the collapsed card
- **THEN** it is exposed as a named, expandable control

### Requirement: Reasoning commits at most once per frame

Reasoning MUST reach the view through the coalesced commit path that text
deltas use. It arrives at token cadence, and committing synchronously per
fragment re-renders the whole transcript — markdown re-parse of the growing
answer included — once per token, which is precisely the cost already
removed for text.

A terminal inference event MUST still commit immediately, so the handover to
the settled transcript is not delayed by a frame boundary.

#### Scenario: A burst of fragments is one render

- **GIVEN** several reasoning fragments arriving inside one animation frame
- **THEN** subscribers are notified once

#### Scenario: The run's end is not deferred

- **GIVEN** reasoning fragments followed by the run's terminal event
- **THEN** that terminal event is applied without waiting for a frame
