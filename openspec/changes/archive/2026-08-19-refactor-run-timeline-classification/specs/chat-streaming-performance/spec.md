# chat-streaming-performance — Spec Delta

## MODIFIED Requirements

### Requirement: Transcript Work Is Proportional To What Changed

A streamed text delta SHALL cost work proportional to the turn in progress,
not to the length of the transcript.

The virtualized window SHALL hold the durable conversation only. The
in-flight run — its reasoning disclosure, its tool cards, its settled round
prose and its streaming prose row — renders in the live tail after that
window, so a streamed delta changes neither the virtual row count nor any
row key, and the virtualizer performs no keying, estimating or measuring on
account of it.

The live tail needs no virtualization because it is bounded by the run
budgets: at most `MAX_TOOL_ROUNDS` prose rows and tool cards, one reasoning
disclosure, and one streaming row.

#### Scenario: The durable message array keeps its identity

- **WHEN** a text delta arrives during a run
- **THEN** the array of durable messages the transcript receives is the same
  reference it received before the delta

#### Scenario: Settled turns are not rebuilt

- **WHEN** a text delta or a reasoning fragment arrives during a run
- **THEN** the transcript rebuilds display items only for the turns after the
  last user message

#### Scenario: The virtual window does not react to a delta

- **WHEN** a text delta arrives during a run
- **THEN** the virtualizer's row count and every row key are unchanged

#### Scenario: A long transcript does not tax a stream

- **WHEN** an answer streams in 80 steps below a transcript of 500 turns
- **THEN** React work per step stays within the budget recorded in
  `benchmarks/chat/README.md`

### Requirement: Settled Prose Renders Once

Model prose SHALL be split into top-level markdown blocks, and a block whose
source has not changed MUST NOT be parsed or rendered again.

The SPLIT itself must not re-read what has settled either. A streamed answer is
append-only, so only its last block can change; re-parsing the whole message to
find the boundaries is quadratic over a stream, and measurably the dominant
per-frame cost of a long answer. Cost per byte MUST stay flat as an answer grows.

Reusing a settled prefix is only sound while the incremental result is
byte-identical to parsing the message whole, so that equivalence MUST be held by
a differential test over every prefix of a corpus that includes the constructs
where block boundaries depend on context — indented code, loose lists, late
setext headings, reference and footnote definitions, raw HTML.

#### Scenario: A complete fence stops re-rendering

- **WHEN** a fenced code block is complete and later text arrives in the same
  answer
- **THEN** that fence's card does not render again

#### Scenario: The fence-render count stays bounded

- **WHEN** an answer with 12 fences streams in 80 steps
- **THEN** the number of fence-card renders stays within the budget recorded in
  `benchmarks/chat/README.md`

#### Scenario: Parsing does not scale with what has already settled

- **WHEN** an answer four times as long streams in four times as many steps
- **THEN** the split's cost per byte is no worse than it was for the shorter one

#### Scenario: The incremental split renders the same page

- **GIVEN** any message and any prefix of it
- **THEN** the blocks the incremental split produces are identical to the blocks
  produced by parsing that prefix whole

#### Scenario: Splitting does not change what is rendered

- **WHEN** a message contains a top-level raw HTML block, or a reference or
  footnote definition
- **THEN** the whole message renders as one block
- **AND** the rendered output matches the output of rendering the message unsplit
