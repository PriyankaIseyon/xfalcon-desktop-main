# chat-run-activity Specification

## Purpose

The surface a run occupies while it is in flight, and the moment it hands over
to the durable transcript. This capability owns three questions and no others:
WHERE a run's content renders while it streams, WHICH state the store keeps for
it, and WHEN that state is cleared.

It does not own what the content looks like once settled (`chat-thread-layout`,
`chat-tool-trail`, `chat-reasoning-panel`), nor the prompt the run was assembled
from (`run-prompt-assembly`), nor the cost of rendering it (`chat-streaming-performance`).

## Scope

Files this capability owns. A change to any of them belongs to a proposal
carrying a `chat-run-activity` delta.

| File                                         | What it owns here                                                                |
| -------------------------------------------- | -------------------------------------------------------------------------------- |
| `src/features/chat/LiveRunTimeline.tsx`      | the in-flight run's block sequence, rebuilt from live state                      |
| `src/features/chat/runSurface.ts:49`         | `liveRunHasContent` — the one predicate for "the timeline has something to draw" |
| `src/features/chat/runState.ts:14`           | `PENDING_RUN_STATES` and `isRunPending`                                          |
| `src/features/chat/chatStore.ts:37`          | `ChatStoreSnapshot`'s live-run fields and their retirement                       |
| `src/features/chat/chatStore.ts:149`         | `admittedInvocations` — the scoping set                                          |
| `src/features/chat/chatStore.ts:207`         | `mapConversationView` / `applyConversation` — the retirement site                |
| `src/features/chat/chatStore.ts:320`         | `applyToolInvocation` — admission and the round-boundary capture                 |
| `src/features/chat/chatReduce.ts`            | inference and run event reduction, sequence watermarks                           |
| `src/features/chat/useTranscript.ts`         | the live-run half of the store, selected field by field                          |
| `src/features/chat/ChatSurface.tsx:101`      | the timeline-or-indicator choice                                                 |
| `src/features/chat/ThinkingPill.tsx`         | the indicator that fills the place when nothing has arrived                      |
| `src/features/chat/ToolActivityCard.tsx:317` | `LiveRunActivity` — the live tool card and its open/close choreography           |

## Boundaries

- **Row anatomy, grouping, and naming** belong to `chat-tool-trail`. This
  capability decides that a contiguous stretch of live tool rows renders as one
  card; what a row looks like inside it does not.
- **The reasoning disclosure's content, cap, and label** belong to
  `chat-reasoning-panel`. This capability owns only the fact that reasoning is a
  live-only block and that it is exempt from snapshot retirement.
- **The ribbon, the accent rail, and prose typography** belong to
  `chat-thread-layout`.
- **The system prompt, the tool inventory, the visualization contract, and the
  skills advertisement** belong to `run-prompt-assembly`. They were filed here
  by `port-web-system-prompts`, `user-skills` and `chat-reports-tab` because they
  concern a run; they are not the run's surface.
- **Recovery notices and failure copy** belong to `chat-run-recovery`.
- **The composer's send/stop affordance** belongs to `chat-composer`, which reads
  `isRunPending` from here rather than defining its own predicate.
- **Backend run lifecycle, journaling, and cancellation** belong to
  `run-execution`. `RunState` reaches this capability as a contract value only.

## Requirements

### Requirement: Streamed content renders where its durable form lives

Assistant content MUST render, from its first token, on the surface that will
hold it once the run has settled. No surface may take content away from another
surface while a run is in flight.

- Assistant prose — a round's narration and the final answer alike — is durably
  an assistant message, so it MUST render as a transcript prose row, with the
  message typography and the full markdown pipeline.
- Tool calls are durably `tool_calls` on that message, so they MUST render as
  tool rows inside a tool card.
- Reasoning is never durable, so it MUST render only on a live surface and MUST
  NOT be mistaken for either of the above.

The surface MUST NOT be chosen by whether the run has called a tool, or by any
other fact that is unknown when the first token arrives. Such a rule is revised
after the content is on screen, and every revision is a relocation the reader
watches.

#### Scenario: A tool-bearing run's answer streams as the answer

- **GIVEN** an active run that has already completed a tool round
- **WHEN** its answer streams
- **THEN** the text renders as a transcript prose row with the streaming caret,
  in message typography, through the markdown pipeline
- **AND** no copy of it is drawn inside any activity card

#### Scenario: The settle changes nothing about the answer

- **GIVEN** a streamed answer on screen
- **WHEN** its durable row lands
- **THEN** the text is in the same place, in the same typography, and neither
  blanks nor moves

#### Scenario: Round prose is never withdrawn

- **GIVEN** an active run that streamed "I'll check the sales table."
- **WHEN** its first tool invocation is requested
- **THEN** that prose remains on screen as a prose row
- **AND** the round's tool rows render below it, not above it
- **AND** the text is still on screen after the durable snapshot lands

### Requirement: The run in flight renders in the tail, outside the virtualized window

The in-flight run MUST render after the virtualized window, in the same scroll
flow, at the position where its durable turns will land. The virtualized window
MUST hold the durable conversation only.

The tail's block sequence MUST be rebuilt from live state on the same ordering
axis the durable transcript uses — the number of tool rows the run had produced
when a segment was captured — so the settle replaces each live block with an
identical durable one.

The tail needs no virtualization of its own because the run budgets bound it: at
most `MAX_TOOL_ROUNDS` prose rows and tool cards, one reasoning disclosure per
captured stretch, and one arriving prose row.

#### Scenario: A delta does not touch the virtual window

- **WHEN** a text delta arrives during a run
- **THEN** the virtualizer's row count and every row key are unchanged

#### Scenario: The tail sits where the turn will land

- **GIVEN** a run in flight below a settled transcript
- **THEN** its blocks render after the last durable row and before the composer

### Requirement: A run's answer is on exactly one surface at a time

A run's answer text SHALL be owned by the store through its whole life and drawn
by exactly one surface at every moment of it.

Two surfaces can render a run's answer, and they are distinguished by
durability, not by timing:

|     | Surface                                            | Source                               |
| --- | -------------------------------------------------- | ------------------------------------ |
| A   | the synthetic `stream-` prose row in the live tail | `streamingText`, then `settlingText` |
| B   | the durable transcript row                         | `messages`                           |

`streamingText` is a run still producing tokens. `settlingText` is that same
text after the terminal inference event and before the durable row lands — a gap
that genuinely exists, because the run's finalization reconciles after that
event, with retries. A owns the text while either is non-empty; B owns it
afterwards; the handover is the durable snapshot's arrival, which clears both.

The live tool card MUST NOT be a third surface for answer text. It holds tool
rows, and nothing that a durable message will carry.

#### Scenario: Answer text is never drawn twice

- **GIVEN** any moment of any run, with or without tool calls
- **WHEN** the transcript renders
- **THEN** the run's answer text appears exactly once

#### Scenario: The activity card carries no answer text

- **GIVEN** an active run with tool events and streaming answer text
- **WHEN** the activity card renders
- **THEN** its body contains tool rows only

#### Scenario: The gap between the last token and the durable row is covered

- **GIVEN** a run whose terminal inference event has landed
- **WHEN** its durable row has not arrived yet
- **THEN** the answer is still on screen, unchanged

#### Scenario: A dead run holds nothing

- **GIVEN** a run that failed mid-answer
- **THEN** its partial text is dropped rather than held, and the failure notice
  takes the surface instead

### Requirement: Every live field of a run retires on one signal

Every live field of a run MUST be cleared by the same condition — the streamed
text, the settling text, the activity rows, the captured round prose, and the
admitted-invocation set alike. That condition is a durable conversation snapshot
arriving for a conversation whose run is no longer in flight. Retiring them
separately is what let a streamed answer and its durable row be on screen at the
same time.

"In flight" MUST admit a run whose state has not yet been observed: a
conversation opened onto a run that was already running has a bound run id and a
null run state, and that run's live state has to survive the open.

Retirement MUST be keyed to the snapshot's arrival, never to the run's falling
edge. The run's terminal event precedes the reconcile that produces the
snapshot, so clearing on the falling edge would leave the transcript with no
record of the run for that gap. A run whose snapshot never arrives keeps its live
state, which is then the only record of what it did.

Reasoning is exempt and scoped to the conversation instead. The snapshot retires
what it carries, and it carries no reasoning; clearing it there would delete the
only copy that ever existed. A conversation switch still drops it, and adopting a
new run for the conversation drops it, so one run's thinking is never attached to
another run's or another conversation's answer.

#### Scenario: The durable snapshot retires the live surfaces together

- **GIVEN** a settled run whose live state is on screen
- **WHEN** the durable snapshot for its conversation arrives
- **THEN** the streamed text, the settling text, the activity rows and the
  captured round prose are all cleared in the same commit

#### Scenario: A mid-run reconcile keeps them

- **GIVEN** a run still in flight
- **WHEN** the conversation on screen is re-applied (a rename, a reconcile)
- **THEN** the live surfaces are unchanged, including a pending approval prompt
  in the activity feed

#### Scenario: A run with no snapshot keeps its answer on screen

- **GIVEN** a run that settled but whose reconcile never produced a snapshot
- **THEN** its answer is still readable on the live surface

#### Scenario: Reasoning survives the snapshot that retires everything else

- **GIVEN** a settled reasoning run whose durable snapshot has landed
- **THEN** the answer comes from the snapshot, and the reasoning is still
  readable, drawn once and by the transcript

#### Scenario: The next run does not inherit the last one's thinking

- **GIVEN** a conversation whose previous run reasoned
- **WHEN** the next message is sent and its run is adopted
- **THEN** no reasoning and no captured round prose is on screen before the new
  run streams anything

#### Scenario: Switching conversations drops it

- **GIVEN** a run's reasoning on screen
- **WHEN** another conversation is opened
- **THEN** no reasoning is carried into it

### Requirement: A retired run cannot be resurrected by a late event

A tool-lifecycle event arriving for a run whose live state has been retired MUST
NOT restore that state. The admitted-invocation set is cleared with the rest, so
a follow-up event for a retired invocation is not admitted and the activity feed
stays empty.

#### Scenario: A straggling tool event cannot resurrect a retired run

- **GIVEN** a run retired by its durable snapshot
- **WHEN** a lifecycle event for one of its invocations arrives late
- **THEN** the activity feed stays empty

#### Scenario: The previous run's card does not ride into the next send

- **GIVEN** a conversation whose last run used tools and has settled
- **WHEN** the next message is sent and its post-submit snapshot arrives
- **THEN** the settled run's live card is gone before the new run starts

### Requirement: Events are admitted only for the run bound to the view

The store MUST admit an event only when it belongs to the run bound to the
conversation on screen. The run, inference, and tool-invocation channels are
broadcasts: a run streaming for another conversation, or a stale superseded run,
emits on them too.

- The run id is bound by the submit result, and by the conversation-scoped
  `run_state_changed` projection, which is the adoption seam. Without the submit
  binding, every event a run emits before its first projection would be dropped
  as foreign.
- Run and inference events MUST be matched on that run id.
- Only a tool `requested` event names its run. Follow-up events carry the
  invocation id alone, so admission MUST be by membership of the
  admitted-invocation set, which a `requested` for the bound run populates.
- A `snapshot_replaced` projection for a conversation other than the one on
  screen MUST be ignored, and a snapshot whose highest message sequence is lower
  than what is on screen MUST be ignored.
- Text deltas and reasoning fragments MUST each be de-duplicated against a
  watermark of their own. Both are numbered from one run-wide sequence counter,
  so admitting a reasoning fragment through the answer's watermark would advance
  it past the next answer delta and silently swallow answer text.

An empty reduction result MUST NOT commit a new state object. A foreign run
produces one event per token, and committing for those would re-render the
transcript at that run's token rate.

#### Scenario: A background run does not disable this view's composer

- **GIVEN** a conversation on screen with no run of its own
- **WHEN** a run for another conversation changes state
- **THEN** this view's run state and composer are unchanged

#### Scenario: A background conversation's approval prompt stays out

- **GIVEN** a run in another conversation whose tool call requires approval
- **THEN** no prompt reaches this view's activity feed

#### Scenario: Another conversation's snapshot does not teleport the reader

- **GIVEN** a background run finalizing in another conversation
- **WHEN** its `snapshot_replaced` broadcast arrives
- **THEN** the conversation on screen is unchanged

#### Scenario: A replayed reasoning fragment cannot cost answer text

- **GIVEN** a run whose reasoning fragment arrives twice
- **THEN** the text appears once
- **AND** every answer delta that followed is still in the answer

### Requirement: One predicate decides whether a run is still working

The system MUST decide "this run is still working" in exactly one place, read by
the composer's send/stop affordance, the cancel binding, the live card's
open-while-running choreography, and the retirement rule above. That predicate
MUST count the run states `draft`, `queued`, `running`, `awaiting_approval`, and
`recovering` as working, and every other state as settled.

A run parked on a tool approval is working, not finished. An interrupted run is
settled here even though the Rust journal counts it as in flight to keep it
resumable — the UI hands that run to the recovery notice and returns the
composer.

#### Scenario: A run parked on an approval still reads as working

- **GIVEN** a run that has entered `awaiting_approval`
- **THEN** the composer keeps its stop affordance and the live tool card keeps
  its running spinner and open state

#### Scenario: An interrupted run releases the composer

- **GIVEN** a run that reached `interrupted`
- **THEN** the recovery notice is shown and the composer accepts input

### Requirement: A live surface never draws an empty card

The live timeline MUST be drawn only when it has a block to draw. Whether it has
one MUST be decided by a single predicate that the thinking indicator reads as
well, so the two can never both be absent or both be present. That predicate
MUST read the reasoning display preference rather than assume it.

A component that can draw nothing MUST refuse to render on its own account,
rather than relying on its caller to have checked. A card whose content is
withheld by a display flag draws nothing, which is a bordered empty box, which
reads as a broken app.

#### Scenario: A run that has only reasoned, with reasoning withheld

- **GIVEN** an active run with reasoning, no tool events, and reasoning display
  off
- **THEN** no activity card and no reasoning disclosure is rendered
- **AND** the thinking indicator is on screen

#### Scenario: A run that has only reasoned, with reasoning shown

- **GIVEN** an active run with reasoning, no tool events, and reasoning display
  on
- **THEN** a reasoning disclosure is on screen
- **AND** the thinking indicator is not, because something has arrived

#### Scenario: A card with no rows refuses itself

- **GIVEN** a tool card asked to render an empty event list
- **THEN** it renders nothing at all, not a bordered empty body

### Requirement: The live tool card holds tool rows, and its open state follows its own stretch

A contiguous stretch of a run's live tool rows MUST render as one disclosure
card, in the order the rows were requested, holding tool rows and nothing a
durable message carries. A run MAY produce more than one such card: a prose row
or a reasoning disclosure between two tool rows means those rows are not
contiguous.

Open-state choreography MUST follow the stretch, not the run: the card auto-opens
while its own stretch is still working and auto-closes on that stretch's falling
edge. Only the last card of a run can still be working; the cards above it belong
to rounds that finished. A manual toggle MUST win for the rest of the run, and a
new run MUST return the card to automatic.

The header MUST summarize state from typed events only, with no polling timer: a
working label with a spinner and the run's client-observed elapsed time while the
stretch is active, a failure label when any row in it failed, and a count of tool
calls otherwise. Elapsed time MUST advance only with event or render activity,
never on an interval.

#### Scenario: A card per contiguous stretch

- **GIVEN** a run of two calls, then prose, then three calls
- **THEN** two tool cards render, labelled for two and three calls, with the
  prose row between them

#### Scenario: Only the last card is working

- **GIVEN** a run whose second stretch is executing a tool
- **THEN** the first card is closed and unspinnered, and the second shows the
  spinner and is open

#### Scenario: The card closes when its stretch ends

- **GIVEN** an open card whose stretch is working
- **WHEN** that stretch reaches its falling edge
- **THEN** the card collapses to its header

#### Scenario: A manual toggle wins until the next run

- **GIVEN** a card the reader collapsed during a run
- **WHEN** further events for that stretch arrive
- **THEN** the card stays collapsed
- **WHEN** a new run starts
- **THEN** the card is expanded again by the automatic rule

#### Scenario: A failed stretch says so on its header

- **GIVEN** a settled stretch in which one row failed
- **THEN** its collapsed header reads as a failure

### Requirement: A round's prose is captured at its round boundary

A round's streamed prose MUST be moved into the run's captured round prose, and
the streaming text emptied, when that round's first `requested` event arrives and
the streamed text is not blank. That event arrives exactly after the round's prose
finished streaming.

The capture MUST be positioned against the number of tool rows drawn BEFORE this
round's first call, which is the same axis the round's reasoning is placed on, so
the two land in the order the model produced them.

The capture MUST read exact state, not a coalesced view of it, so a burst of
deltas not yet flushed to subscribers is captured whole. Parallel calls in one
round are safe because only the round's first `requested` sees non-blank text.

#### Scenario: A round boundary reads the exact streamed text

- **GIVEN** deltas that have not yet been flushed to subscribers
- **WHEN** the round's first tool invocation is requested
- **THEN** the captured prose carries all of the streamed text

#### Scenario: A second call in the same round captures nothing

- **GIVEN** a round that has already captured its prose
- **WHEN** a further `requested` for the same round arrives
- **THEN** no second capture is made

### Requirement: Live activity is bounded

Retained tool-invocation events MUST be capped, dropping the oldest first. A
long-lived window must not be able to grow this array without limit.

The cap MUST be large enough to hold several complete runs, and the surface MUST
key rows by invocation and show the latest event per invocation, so dropping the
oldest entries only ever loses whole finished invocations and never half of a
live one.

#### Scenario: A long session does not grow the feed without limit

- **GIVEN** a window that has run many tool-bearing runs
- **THEN** the retained event count stays at or below the cap

### Requirement: A run in flight with nothing on screen shows an indicator

The transcript MUST show a thinking indicator while a run is working and the live
timeline has nothing to draw, so a sent message is never followed by dead air. The
indicator MUST NOT be shown while the timeline is drawing anything: the indicator
means "a run is in flight and nothing has arrived", and showing both tells the
reader the same thing twice in two voices.

#### Scenario: A sent message is acknowledged

- **GIVEN** a submitted message whose run is queued
- **THEN** the thinking indicator is on screen

#### Scenario: The first arriving content supersedes the indicator

- **GIVEN** a run in flight showing the indicator
- **WHEN** its first reasoning fragment, tool row, or prose token arrives
- **THEN** the indicator is gone and the timeline is what reports the run

## Provenance

Consolidated from the deltas of `refine-run-feedback-ux`,
`run-reasoning-activity`, `unify-tool-activity-ux`, `chat-reasoning-panel`,
`one-surface-per-answer`, `retire-live-run-activity` and
`refactor-run-timeline-classification`, reconciled against the shipped code and
resolved in that order.

Resolved contradictions, all in favour of the code:

- "Tool activity is one in-transcript card" (`refine-run-feedback-ux`) is
  superseded: a run renders one card per contiguous tool stretch.
- "Pre-final streamed text lives inside the live activity card"
  (`run-reasoning-activity`, `chat-reasoning-panel`) is removed: prose renders as
  a prose row from its first token.
- "The card opens while streaming and closes when the run ends" and the
  "auto-collapse ~400ms after the first answer token" and hover-suppression
  clauses (`unify-tool-activity-ux`) are removed: the card's open state follows
  its own stretch's active edge, and no hover suppression is wired.
- The prompt-assembly requirements of `port-web-system-prompts`, `user-skills`
  and `chat-reports-tab` are moved to `run-prompt-assembly`.
