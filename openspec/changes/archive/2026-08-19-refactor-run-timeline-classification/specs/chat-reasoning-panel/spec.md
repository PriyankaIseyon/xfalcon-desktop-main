# chat-reasoning-panel — Spec Delta

## MODIFIED Requirements

### Requirement: Reasoning that arrives is shown; reasoning that never arrives leaves no trace

The system MUST show the reasoning streamed for the conversation on screen
once at least one fragment has arrived for its run, and MUST NOT render any
card, header, or placeholder on account of reasoning for a run that produced
none.

Reasoning MUST be shown behind a collapsed disclosure, not expanded in the
reading flow. It is the model's own words at a length no reader asked for,
and an answer preceded by four paragraphs of deliberation buries the two
lines that matter. Collapsed, it is one quiet row that says thinking
happened and can be opened.

A run that reasons but calls no tools MUST still get its disclosure. A run
whose whole visible work was thinking would otherwise have nowhere to show
it, and would fall back to a live indicator that says less than the run
already told us.

Reasoning display MUST be a reader preference, persisted per device and
shown by default. It is a verbosity question — the cost of showing it is the
reader's attention — so the reader is who can weigh it, unlike a tool's
arguments and results, which quote project data and stay a build decision.

When the preference is off, every surface MUST behave as though the run
produced no reasoning: no disclosure, no empty card, and the thinking
indicator holds its place. The predicate for "the live run has something to
draw" MUST therefore READ the preference rather than assume it, so turning it
off leaves a coherent surface rather than a bordered empty box.

#### Scenario: A model that volunteers its thinking

- **GIVEN** a run whose provider streams reasoning
- **WHEN** the first fragment arrives
- **THEN** a collapsed reasoning disclosure is in the transcript flow,
  carrying that text

#### Scenario: A model that streams none

- **GIVEN** a run that streams an answer, calls no tools, and reasons none
- **THEN** no reasoning disclosure and no activity card is rendered at any
  point in that run, before, during, or after

#### Scenario: Thinking without tools still has a home

- **GIVEN** a run that streams reasoning and calls no tools
- **THEN** its disclosure exists and is named for the thinking it holds, not
  for a count of calls that were never made

#### Scenario: Withheld reasoning leaves no empty box

- **GIVEN** an active run with reasoning, no tool events, and the reasoning
  preference off
- **THEN** no disclosure and no activity card is rendered
- **AND** the thinking indicator is on screen

#### Scenario: An install that predates the preference still shows reasoning

- **GIVEN** a stored appearance document written before the preference
  existed
- **WHEN** it is read
- **THEN** reasoning is shown

### Requirement: One card per run, holding everything it thought and did in the order it happened

The system MUST render a run's reasoning and its tool rows as blocks of ONE
sequence, ordered as the run produced them. A stretch of reasoning that
arrived after a tool result MUST render below that result's row, and a
stretch that preceded it MUST render above.

Reasoning MUST NOT be given a surface PARALLEL to the run's sequence — a
side panel, a separate column, or a card the reader has to correlate with
the tool rows by timestamp. It renders as its own block AT its own position
in the one sequence, which is a different thing: the reader still follows a
single top-to-bottom order, and a later round's thinking cannot appear above
the row it was reacting to.

Reasoning MUST NOT share a disclosure body with tool rows. They are
different kinds of content — one is the model's private deliberation, the
other is what the app did on the user's behalf — and one body holding both
means one label has to name both, one expand reveals both, and a flag that
withholds one empties a card built for the other.

Assistant prose MUST NOT appear in either. It is durably part of an
assistant message and renders as a prose row (spec: chat-run-activity,
"Streamed content renders where its durable form lives").

Ordering MUST be derived from a single position that both the in-flight
surface and the settled transcript can compute: how many tool rows the run
had produced when the segment was captured. Two surfaces deriving position
from state only one of them has is how the same run comes to read
differently before and after it settles.

Where reasoning and prose were captured at the same position, reasoning
MUST come first — within a round the model reasons, then writes the prose
that announces what it is about to call.

Fragments MUST be ordered and de-duplicated on a ledger of their own, never
the answer's. Reasoning and text deltas are numbered from one run-wide
sequence counter, so a reasoning fragment admitted through the answer's
ledger would advance it and silently swallow the next answer delta.

#### Scenario: A multi-round run reads as one sequence

- **GIVEN** a run that reasoned, called a tool, then reasoned again
- **THEN** the first stretch of reasoning renders above the tool row and the
  second below it, in one top-to-bottom order

#### Scenario: Reasoning and prose from one round

- **GIVEN** a round whose model reasoned and then streamed prose before
  calling a tool
- **THEN** the reasoning disclosure renders above that prose row, and both
  above the tool row

#### Scenario: Reasoning and tools do not share a body

- **GIVEN** a run that reasoned and called a tool
- **WHEN** the reader expands the tool umbrella
- **THEN** it reveals tool rows only, and the reasoning stays behind its own
  disclosure

#### Scenario: A replayed fragment cannot cost answer text

- **GIVEN** a run whose reasoning fragment arrives twice
- **THEN** the text appears once
- **AND** every answer delta that followed is still in the answer
