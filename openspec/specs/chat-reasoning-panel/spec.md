# chat-reasoning-panel Specification

## Purpose

The model's own deliberation, as the reader encounters it: a collapsed disclosure
in the run's sequence, at the position the thinking was produced, holding inert
text, bounded, and offered only when the reader has asked to see it.

Reasoning is the one kind of run content that is never persisted. That single fact
drives every requirement here — where it renders, when it is cleared, and why it
cannot be treated like prose or like a tool row.

## Scope

| File                                         | What it owns here                                                       |
| -------------------------------------------- | ----------------------------------------------------------------------- |
| `src/features/chat/ToolActivityCard.tsx:194` | `ReasoningDisclosure` — the collapsed disclosure and its label          |
| `src/features/chat/ToolActivityCard.tsx:161` | `ReasoningBlock` — inert text rendering                                 |
| `src/features/chat/ToolActivityCard.tsx:172` | `ReasoningTrimmedNotice`                                                |
| `src/features/chat/chatReduce.ts:39`         | `MAX_REASONING_CHARS`                                                   |
| `src/features/chat/chatReduce.ts:75`         | `appendReasoning` — segmentation, cap, surrogate-safe cut               |
| `src/features/chat/chatReduce.ts:23`         | `lastReasoning` — reasoning's own de-duplication watermark              |
| `src/app/appearance.ts:127`                  | `useShowReasoning` — the reader's preference as a primitive             |
| `src/features/chat/ChatSurface.tsx:184`      | the handover: the transcript draws reasoning once the live tail is gone |

## Boundaries

- **Where in the run's sequence a block lands, and when live state is cleared**
  belong to `chat-run-activity`. This capability owns the ordering RULE (reasoning
  precedes prose captured at the same position) and the exemption from snapshot
  retirement; the retirement mechanism itself lives there.
- **The disclosure shell, its chevron, and its remeasure contract** belong to
  `chat-tool-trail`. Reasoning renders through that shell and does not own it.
- **Persisting the preference and the settings control that edits it** belong to
  `settings-appearance`. This capability owns only the reading of it.
- **Asking a provider for reasoning, and the shape of the fragments that arrive**
  belong to `openrouter-request-shape` and the provider adapters.
- **The coalesced commit path** is owned by `chat-streaming-performance`;
  reasoning is required here to use it, not to define it.

## Requirements

### Requirement: Reasoning that arrives is shown; reasoning that never arrives leaves no trace

The system MUST show the reasoning streamed for the conversation on screen once at
least one fragment has arrived for its run, and MUST NOT render any card, header,
or placeholder on account of reasoning for a run that produced none.

Reasoning MUST be shown behind a disclosure that is COLLAPSED by default,
including while it streams. It is the model's own words at a length no reader
asked for, and an answer preceded by four paragraphs of deliberation buries the
two lines that matter. Collapsed, it is one quiet row that says thinking happened
and can be opened.

A run that reasons but calls no tools MUST still get its disclosure. A run whose
whole visible work was thinking would otherwise have nowhere to show it, and would
fall back to a live indicator that says less than the run already told us.

Reasoning display MUST be a reader preference, persisted per device and shown by
default. It is a verbosity question — the cost of showing it is the reader's
attention — so the reader is who can weigh it, unlike a tool's arguments and
results, which quote project data and stay a build decision.

When the preference is off, every surface MUST behave as though the run produced
no reasoning: no disclosure, no empty card, and the thinking indicator holds its
place. The predicate for "the live run has something to draw" MUST therefore READ
the preference rather than assume it.

A disclosure asked to render empty text MUST render nothing.

#### Scenario: A model that volunteers its thinking

- **GIVEN** a run whose provider streams reasoning
- **WHEN** the first fragment arrives
- **THEN** a collapsed reasoning disclosure is in the transcript flow, carrying
  that text

#### Scenario: A model that streams none

- **GIVEN** a run that streams an answer, calls no tools, and reasons none
- **THEN** no reasoning disclosure and no activity card is rendered at any point
  in that run, before, during, or after

#### Scenario: Thinking without tools still has a home

- **GIVEN** a run that streams reasoning and calls no tools
- **THEN** its disclosure exists and is named for the thinking it holds, not for a
  count of calls that were never made

#### Scenario: Withheld reasoning leaves no empty box

- **GIVEN** an active run with reasoning, no tool events, and the reasoning
  preference off
- **THEN** no disclosure and no activity card is rendered
- **AND** the thinking indicator is on screen

#### Scenario: An install that predates the preference still shows reasoning

- **GIVEN** a stored appearance document written before the preference existed
- **WHEN** it is read
- **THEN** reasoning is shown

### Requirement: Reasoning is a block of the run's one sequence

The system MUST render a run's reasoning and its tool rows as blocks of ONE
sequence, ordered as the run produced them. A stretch of reasoning that arrived
after a tool result MUST render below that result's row, and a stretch that
preceded it MUST render above.

Reasoning MUST NOT be given a surface PARALLEL to the run's sequence — a side
panel, a separate column, or a card the reader has to correlate with the tool rows
by timestamp. It renders as its own block AT its own position in the one
sequence, which is a different thing: the reader still follows a single
top-to-bottom order, and a later round's thinking cannot appear above the row it
was reacting to.

Reasoning MUST NOT share a disclosure body with tool rows. They are different
kinds of content — one is the model's private deliberation, the other is what the
app did on the user's behalf — and one body holding both means one label has to
name both, one expand reveals both, and a flag that withholds one empties a card
built for the other.

Assistant prose MUST NOT appear in either. It is durably part of an assistant
message and renders as a prose row.

Ordering MUST be derived from a single position that both the in-flight surface
and the settled transcript can compute: how many tool rows the run had produced
when the segment was captured. Two surfaces deriving position from state only one
of them has is how the same run comes to read differently before and after it
settles.

Where reasoning and prose were captured at the same position, reasoning MUST come
first — within a round the model reasons, then writes the prose that announces
what it is about to call.

A fragment MUST extend the last segment while no tool row has appeared since that
segment began, and MUST open a new segment once one has. That is what puts a later
round's thinking below the tools it reacted to instead of back at the top of one
growing block.

#### Scenario: A multi-round run reads as one sequence

- **GIVEN** a run that reasoned, called a tool, then reasoned again
- **THEN** the first stretch of reasoning renders above the tool row and the
  second below it, in one top-to-bottom order

#### Scenario: Reasoning and prose from one round

- **GIVEN** a round whose model reasoned and then streamed prose before calling a
  tool
- **THEN** the reasoning disclosure renders above that prose row, and both above
  the tool row

#### Scenario: Reasoning and tools do not share a body

- **GIVEN** a run that reasoned and called a tool
- **WHEN** the reader expands the tool umbrella
- **THEN** it reveals tool rows only, and the reasoning stays behind its own
  disclosure

#### Scenario: The live and settled orders agree

- **GIVEN** a run whose reasoning, prose and tool rows are on screen live
- **WHEN** its durable snapshot lands
- **THEN** the blocks are in the same order, drawn by the transcript instead of
  the tail

### Requirement: Reasoning is admitted only for the run on screen, on its own ledger

The system MUST admit a reasoning fragment only when it belongs to the run bound
to the conversation being shown, on the same scoping rule as every other inference
event. The inference channel is a broadcast: a run streaming for another
conversation, or a stale superseded run, emits here too, and its thinking must
never be attributed to the conversation the reader is looking at.

Fragments MUST be ordered and de-duplicated on a ledger of their own, never the
answer's. Reasoning and text deltas are numbered from one run-wide sequence
counter, so a reasoning fragment admitted through the answer's ledger would
advance it and silently swallow the next answer delta.

Reasoning belongs to the newest run in the conversation. It MUST NOT reach an
earlier run's trail, and adopting a new run MUST clear it — filing one run's
thinking under a previous answer's tools would attribute one run's work to
another's.

#### Scenario: A background run's thinking stays out

- **GIVEN** a conversation on screen with its own run
- **WHEN** a reasoning fragment arrives for a different run
- **THEN** nothing on screen changes

#### Scenario: An earlier answer keeps its own trail

- **GIVEN** a transcript with two answers, each with a tool trail
- **WHEN** the newest run's reasoning is drawn
- **THEN** only the newest response carries it

#### Scenario: A replayed fragment cannot cost answer text

- **GIVEN** a run whose reasoning fragment arrives twice
- **THEN** the text appears once
- **AND** every answer delta that followed is still in the answer

### Requirement: Reasoning renders inert, labelled, and never carries the answer

Reasoning MUST render as inert plain text in secondary ink: no markdown execution,
no HTML preview or table cards, no syntax highlighting, preserving its own line
breaks. Reasoning is untrusted model output that routinely contains half-finished
fenced blocks and partial markup, and executing it would let a mid-thought
fragment mint a preview frame or a table out of syntax the model has not finished
writing.

The disclosure MUST name itself for what it holds, and the name MUST distinguish
thinking in progress from thinking that is over — the label reads as an activity
while the run is working and as a record once it is not.

Reasoning MUST be distinguishable from a round's prose. They are different things:
prose is answer text the model chose to emit and that its turn keeps durably,
while reasoning is never persisted and is never part of an answer. Presenting them
identically would make raw thinking read as something the model said.

The disclosure MUST NOT carry the run's final answer. Which surface owns the answer
is decided by `chat-run-activity`, and a disclosure that also showed it would put
the same answer on screen twice.

#### Scenario: A fenced block in mid-thought stays text

- **GIVEN** reasoning containing an unterminated fenced HTML block
- **THEN** it renders as visible text and no preview frame or table is created

#### Scenario: Thinking is marked as thinking

- **GIVEN** a response holding both a round's prose and a stretch of reasoning
- **THEN** the reasoning is behind a disclosure that names it, and the prose is a
  prose row

#### Scenario: The label changes when the run ends

- **GIVEN** a reasoning disclosure while its run is working
- **THEN** its label reads as thinking in progress
- **WHEN** the run settles
- **THEN** its label reads as a completed thought process

#### Scenario: The disclosure is not an answer surface

- **GIVEN** a settled run
- **THEN** its answer renders outside the disclosure

### Requirement: Reasoning is bounded and its trimming is admitted

The system MUST cap the reasoning it retains for a run and SHALL drop from the
oldest end when that cap is passed, keeping the most recent text — that is what a
reader following a live run is watching. A long-lived window must not be able to
grow this state without limit.

The cap MUST bound retained characters rather than exact encoded bytes. Reasoning
arrives one fragment at a time, and re-encoding the whole string to measure it per
fragment would be quadratic on the render path.

Dropping MUST shed whole spent segments before cutting into a survivor, and a cut
MUST NOT begin the kept text on the tail half of a surrogate pair.

When text has been dropped the surface MUST say so, on the oldest surviving
stretch — which is where the missing text was. A body that silently begins
mid-sentence reads as a bug in the model rather than a bound in the app.

#### Scenario: A very long reasoning stream

- **GIVEN** a run whose reasoning passes the retention cap
- **THEN** the newest reasoning is still shown, the oldest is dropped, and the
  surface states that it was trimmed

#### Scenario: A cut does not split a character

- **GIVEN** a cap boundary that falls inside a surrogate pair
- **THEN** the kept text begins at the next whole character

#### Scenario: The notice sits where the text was lost

- **GIVEN** a trimmed run with three surviving stretches
- **THEN** only the oldest stretch carries the notice

### Requirement: Reasoning is announced on request, not as it arrives

The reasoning surface MUST NOT be an assistive-technology live region. A screen
reader narrating every chain-of-thought token would bury the answer under the
thinking, and the reader could not get past it.

It MUST be a disclosure with an accessible name whose expanded state is exposed
programmatically, and whose body is named, so a reader using assistive technology
can open the reasoning deliberately and hear it on request.

#### Scenario: The answer is what gets announced

- **GIVEN** a run streaming reasoning
- **THEN** no reasoning text is announced automatically

#### Scenario: The reasoning is reachable on purpose

- **GIVEN** the collapsed disclosure
- **THEN** it is exposed as a named, expandable control with a named body

### Requirement: Reasoning commits at most once per frame

Reasoning MUST reach the view through the coalesced commit path that text deltas
use. It arrives at token cadence, and committing synchronously per fragment
re-renders the whole transcript — markdown re-parse of the growing answer
included — once per token, which is precisely the cost already removed for text.

A terminal inference event MUST still commit immediately, so the handover to the
settled transcript is not delayed by a frame boundary.

#### Scenario: A burst of fragments is one render

- **GIVEN** several reasoning fragments arriving inside one animation frame
- **THEN** subscribers are notified once

#### Scenario: The run's end is not deferred

- **GIVEN** reasoning fragments followed by the run's terminal event
- **THEN** that terminal event is applied without waiting for a frame

### Requirement: Exactly one surface draws a run's reasoning

Exactly one surface MUST draw a run's reasoning at any moment: the live tail while
it is on screen, the transcript once that tail retires. The handover is exclusive —
the surface that is not drawing it is given nothing, or the same thinking is on
screen twice.

The handover MUST NOT be a second copy or a second retirement condition. It is one
value, read by whichever surface is currently drawing the run.

#### Scenario: The handover draws it once, not twice

- **GIVEN** a run whose reasoning is showing in the live tail
- **WHEN** the durable snapshot retires that tail
- **THEN** the reasoning appears in the transcript and nowhere else

#### Scenario: Neither surface loses it

- **GIVEN** a settled reasoning run
- **THEN** at every moment of the handover exactly one surface is drawing the
  reasoning

## Provenance

Consolidated from the deltas of `chat-reasoning-panel` and
`refactor-run-timeline-classification`, reconciled against the shipped code and
resolved in that order.

Resolved contradictions:

- "One card per run, holding everything it thought and did in the order it
  happened" is superseded: reasoning has its own disclosure, tool rows have
  theirs, and prose is a prose row. The ordering rule survives; the shared body
  does not.
- The clause placing a round's narration inside the reasoning body is removed.
  It is why withholding reasoning also withheld a piece of the answer.
