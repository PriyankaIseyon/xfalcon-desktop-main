## ADDED Requirements

### Requirement: Progressive Answer Flush

The desktop core SHALL forward buffered answer text to the presentation channel
while the provider stream is still open. A turn MUST NOT hold its whole answer
until a terminal event.

Coalescing bounds two things at once: the interval between forwards, and the
number of bytes one forward may carry. Whichever bound is reached first triggers
the forward.

#### Scenario: Text appears before the turn ends

- **WHEN** a provider stream sends text deltas for longer than the coalesce
  interval and sends no other event
- **THEN** the core forwards at least one `text_delta` before the turn reaches a
  terminal event

#### Scenario: A burst is bounded by bytes, not by time

- **WHEN** buffered text reaches the coalesce byte cap before the coalesce
  interval elapses
- **THEN** the core forwards that text at once

#### Scenario: A stalled stream does not hide the text it already sent

- **WHEN** a provider stream sends text and then sends nothing for longer than the
  coalesce interval
- **THEN** the core forwards the buffered text within that interval

#### Scenario: Buffered text precedes a terminal event

- **WHEN** a terminal inference event follows buffered text
- **THEN** the core forwards the buffered text first, then the terminal event

#### Scenario: Coalescing still reduces the forward count

- **WHEN** a provider stream sends many small text deltas inside one coalesce
  interval
- **THEN** the core forwards fewer `text_delta` events than it received

### Requirement: Monotonic Sequence Across Coalesced Forwards

A coalesced `text_delta` SHALL carry a sequence number that is greater than the
sequence of every `text_delta` the core forwarded before it, so the frontend
watermark admits it.

#### Scenario: A later batch is admitted

- **WHEN** the core forwards one coalesced delta and then forwards a second
- **THEN** the second carries a higher sequence than the first
- **AND** the frontend reducer applies both

### Requirement: Transcript Work Is Proportional To What Changed

A streamed text delta SHALL cost work proportional to the turn in progress, not
to the length of the transcript.

#### Scenario: The durable message array keeps its identity

- **WHEN** a text delta arrives during a run
- **THEN** the array of durable messages the transcript receives is the same
  reference it received before the delta

#### Scenario: Settled turns are not rebuilt

- **WHEN** a text delta or a reasoning fragment arrives during a run
- **THEN** the transcript rebuilds display items only for the turns after the last
  user message

#### Scenario: A long transcript does not tax a stream

- **WHEN** an answer streams in 80 steps below a transcript of 500 turns
- **THEN** React work per step stays within the budget recorded in
  `benchmarks/chat/README.md`

### Requirement: Settled Prose Renders Once

Model prose SHALL be split into top-level markdown blocks, and a block whose
source has not changed MUST NOT be parsed or rendered again.

#### Scenario: A complete fence stops re-rendering

- **WHEN** a fenced code block is complete and later text arrives in the same
  answer
- **THEN** that fence's card does not render again

#### Scenario: The fence-render count stays bounded

- **WHEN** an answer with 12 fences streams in 80 steps
- **THEN** the number of fence-card renders stays within the budget recorded in
  `benchmarks/chat/README.md`

#### Scenario: Splitting does not change what is rendered

- **WHEN** a message contains a top-level raw HTML block, or a reference or
  footnote definition
- **THEN** the whole message renders as one block
- **AND** the rendered output matches the output of rendering the message unsplit

### Requirement: Subscribers Re-render Only For Fields They Read

The chat store SHALL offer selector subscription. A component that selects a
field MUST NOT re-render when an unrelated field changes.

#### Scenario: An unrelated field does not wake a subscriber

- **WHEN** a store field changes
- **THEN** only the subscribers whose selected value changed are notified

#### Scenario: A selector returns a stable reference

- **WHEN** a selector runs twice against unchanged state
- **THEN** it returns the identical value or reference

### Requirement: Composer Typing Is Independent Of The Transcript

Composer draft text SHALL live outside the transcript store. Typing MUST NOT
notify a transcript subscriber.

#### Scenario: A keystroke does not re-render the transcript

- **WHEN** the user types one character in the composer
- **THEN** no transcript subscriber runs

#### Scenario: The draft survives a conversation switch the same way it did before

- **WHEN** the user types a draft and opens another conversation
- **THEN** the draft behaves as it did before the store split

### Requirement: Streaming Budgets Are Enforced, Not Reported

The repository SHALL hold a streaming benchmark that fails when its budget is
exceeded, and the budget files SHALL be typechecked.

#### Scenario: A tightened budget fails

- **WHEN** any streaming budget number is lowered by a small margin
- **THEN** the benchmark fails

#### Scenario: A drifted fixture is a compile error

- **WHEN** a benchmark builds a store snapshot that is missing a required field
- **THEN** `npm run typecheck` reports it

#### Scenario: A dropped test file fails the run

- **WHEN** a test worker times out and its file does not run
- **THEN** the test command exits non-zero
