# chat-streaming-performance Specification

## Purpose

What a streamed answer is allowed to cost. Every requirement here bounds work
against the thing that changed — one turn, one block, one field — rather than
against the conversation, the transcript, or the window's lifetime.

The capability spans both sides of the IPC boundary, because the cost is shared:
the core decides how often text crosses it, and the frontend decides how much
re-render each crossing buys.

## Scope

| File                                                                             | What it owns here                                            |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `src-tauri/src/agent/stream.rs:35`                                               | `CoalesceConfig` — the interval and byte bounds on a forward |
| `src-tauri/src/agent/stream.rs:298`                                              | `meets_a_bound` — whichever bound is reached first           |
| `src/features/chat/chatStore.ts:186`                                             | `setStateCoalesced` — one notification per frame             |
| `src/features/chat/chatStore.ts:164`                                             | `setState` — an empty patch commits nothing                  |
| `src/features/chat/chatStore.ts:386`                                             | `useChatSelector` — per-field subscription                   |
| `src/features/chat/composerStore.ts`                                             | the draft, outside the transcript store                      |
| `src/features/chat/useTranscript.ts`                                             | the streaming half of the store, selected field by field     |
| `src/features/chat/MessageList.tsx:253`                                          | the two-half transcript build at the run boundary            |
| `src/features/chat/MessageList.tsx:100`                                          | `runStartIndex` — the split point                            |
| `src/features/chat/richContent/splitMarkdown.ts`                                 | the incremental block split and its reuse guards             |
| `src/features/chat/AssistantMessage.tsx`                                         | the per-message split cache                                  |
| `benchmarks/chat/README.md`                                                      | every budget number, its measurement, and what it replaced   |
| `benchmarks/chat/stream.bench.tsx`                                               | the streaming render budgets                                 |
| `benchmarks/chat/split.bench.ts`                                                 | the cost-per-byte ratio gate                                 |
| `benchmarks/chat/storeApply.perf.test.ts`                                        | the reducer-apply budget                                     |
| `benchmarks/chat/render.bench.tsx`, `render-10k.bench.tsx`, `open-10k.bench.tsx` | mount and open budgets with bounded mounted-row counts       |
| `vitest.config.ts:10`                                                            | the pinned pool, so a dropped file fails the run             |
| `tsconfig.bench.json`                                                            | the second typecheck pass over benchmarks and tests          |

## Boundaries

- **What renders, and where** belongs to `chat-run-activity`,
  `chat-thread-layout`, `chat-tool-trail` and `chat-rich-content`. This capability
  bounds the cost of those decisions and makes none of them.
- **The remeasure contract** belongs to `chat-tool-trail`; the reduced-motion rule
  behind it to `chat-motion-system`.
- **Provider transport limits** — the response ceiling, the SSE line bound, the
  idle read timeout — belong to `provider-adapters`. This capability consumes
  `MAX_SSE_LINE_BYTES` as the ceiling a single delta is bounded by.
- **The run-wide output byte budget** belongs to `run-execution`.
- **Canvas and table render budgets** are proposed by `add-performance-budget-gates`
  and are not part of this baseline; `benchmarks/canvas/` does not exist.

## Requirements

### Requirement: Progressive Answer Flush

The desktop core SHALL forward buffered answer text to the presentation channel
while the provider stream is still open. A turn MUST NOT hold its whole answer
until a terminal event.

Coalescing exists because a provider emits one delta per token and each forwarded
delta costs a JSON serialization and an IPC hop. Holding text is only ever a means
to that end, so it bounds two things at once: the interval between forwards, and
the number of bytes one forward may carry. Whichever bound is reached first
triggers the forward. A configuration that holds text until the turn ends is not a
coalescer, it is a buffer, and a buffer is what makes an answer arrive all at once.

The interval SHALL match the frontend's own per-frame commit cadence. A shorter
interval buys a forward the reader cannot see; a longer one drops frames the reader
can.

The byte bound is checked after the incoming delta is appended, so one forwarded
delta carries at most that bound plus one incoming delta — both bounded, the second
by the transport's own line limit.

The sink can only observe the interval when the next delta arrives, so a caller
that wants buffered text released while the provider is quiet MUST drive the flush
on its own timer at that period.

Buffered reasoning SHALL take a byte bound of its own, derived from the text bound
and capped so that one assembled event still fits the contract's hard ceiling for a
reasoning event. A buffer that assembled more than the contract accepts could not
be forwarded at all, and a refusal MUST NOT be discarded silently.

#### Scenario: Text appears before the turn ends

- **WHEN** a provider stream sends text deltas for longer than the coalesce
  interval and sends no other event
- **THEN** the core forwards at least one `text_delta` before the turn reaches a
  terminal event

#### Scenario: A burst is bounded by bytes, not by time

- **WHEN** buffered text reaches the coalesce byte cap before the coalesce interval
  elapses
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

#### Scenario: An assembled reasoning event stays within the contract

- **WHEN** reasoning fragments accumulate past the reasoning byte bound
- **THEN** the forwarded event is within the contract's ceiling and no fragment is
  dropped without a report

### Requirement: Monotonic Sequence Across Coalesced Forwards

A coalesced `text_delta` SHALL carry a sequence number greater than the sequence of
every `text_delta` the core forwarded before it, so the frontend watermark admits
it.

#### Scenario: A later batch is admitted

- **WHEN** the core forwards one coalesced delta and then forwards a second
- **THEN** the second carries a higher sequence than the first
- **AND** the frontend reducer applies both

### Requirement: The View Commits At Most Once Per Frame

The store SHALL write a streamed delta to state synchronously and notify
subscribers at most once per animation frame. Text and reasoning both take that
path; both arrive at token cadence, and a synchronous notification per fragment
re-renders the whole transcript once per token.

State MUST still be written synchronously, because readers inside the same tick —
the round-boundary capture, a snapshot read — must see exact values rather than the
last flushed ones.

A non-delta change MUST notify immediately and absorb any pending coalesced
notification, so a terminal event and the durable transcript behind it are never
delayed by a frame boundary.

A reduction that produces no patch MUST NOT commit a new state object. A foreign
run's broadcast produces one event per token, and committing for those would
re-render the transcript at that run's token rate.

#### Scenario: A burst of deltas costs one render

- **GIVEN** several text deltas arriving within one animation frame
- **THEN** subscribers are notified once, with the accumulated text

#### Scenario: A synchronous reader sees unflushed text

- **GIVEN** deltas that have not yet been flushed to subscribers
- **WHEN** a same-tick reader reads the snapshot
- **THEN** it sees all of the accumulated text

#### Scenario: A foreign run's tokens cost nothing

- **GIVEN** a run streaming for another conversation
- **THEN** no state object is committed and no subscriber is notified

### Requirement: Transcript Work Is Proportional To What Changed

A streamed text delta SHALL cost work proportional to the turn in progress, not to
the length of the transcript.

The virtualized window holds the durable conversation only. Everything a streamed
token can move renders in the live tail after that window, so a delta changes
neither the virtual row count nor any row key, and the virtualizer performs no
keying, estimating or measuring on account of it.

The display items MUST be built in two halves, split at the run boundary — one past
the last user turn. Above that boundary nothing can change while a run streams, so
those items are built once per conversation snapshot; below it, only the run's own
turns rebuild. The boundary is also a clean tool-group boundary, because a user turn
always closes a group.

#### Scenario: The durable message array keeps its identity

- **WHEN** a text delta arrives during a run
- **THEN** the array of durable messages the transcript receives is the same
  reference it received before the delta

#### Scenario: Settled turns are not rebuilt

- **WHEN** a text delta or a reasoning fragment arrives during a run
- **THEN** the transcript rebuilds display items only for the turns after the last
  user message

#### Scenario: The virtual window does not react to a delta

- **WHEN** a text delta arrives during a run
- **THEN** the virtualizer's row count and every row key are unchanged

#### Scenario: A live answer does not re-estimate offscreen history

- **GIVEN** a mounted conversation with 10,000 durable messages
- **WHEN** its live answer changes over 80 renders without a new durable snapshot
- **THEN** the virtualizer SHALL perform no new height estimates for an offscreen
  history row
- **AND** a real durable snapshot change SHALL still refresh the row keys and
  estimates

#### Scenario: A settled tool card does not re-render while an answer streams

- **WHEN** an answer streams below a completed tool round
- **THEN** that round's card renders within the budget recorded in
  `benchmarks/chat/README.md`

#### Scenario: A long transcript does not tax a stream

- **WHEN** an answer streams in 80 steps below a transcript of 500 turns
- **THEN** React work per step stays within the budget recorded in
  `benchmarks/chat/README.md`

#### Scenario: The cost of an answer belongs to the answer

- **WHEN** the same answer streams into an empty transcript and below 500 settled
  turns
- **THEN** the structural render counts are identical

### Requirement: Settled Prose Renders Once

Model prose SHALL be split into top-level markdown blocks, and a block whose source
has not changed MUST NOT be parsed or rendered again.

Boundaries MUST come from the same parser the renderer uses. A hand-rolled
blank-line scan cuts a fenced block that contains a blank line and turns one list
into two; matching the renderer's own parse is the only thing that makes the split
invisible.

Inter-block whitespace belongs to no node's span and is dropped. That is safe
because every block-level element carries its own margins.

The SPLIT itself MUST NOT re-read what has settled. A streamed answer is
append-only, so only its last block can change; re-parsing the whole message to
find the boundaries is quadratic over a stream, and measurably the dominant
per-frame cost of a long answer. Cost per byte MUST stay flat as an answer grows.

Reuse of a settled prefix MUST be refused unless every condition holds:

- the previous parse produced real block boundaries — a whole-message escape has
  none to reuse;
- the new source actually begins with the previous source, so a caller that
  replaces the text rather than appending to it gets a full parse;
- the tail begins at a line start. An indented code block's node can start after
  its indentation, and a slice taken there parses as a paragraph rather than as
  code.

Two constructs MUST fall back to rendering the whole message as one block, because
rendering their slices independently would not produce the same page:

- a raw HTML node — the sanitize schema escapes it to text, and the separator
  between two adjacent HTML blocks lives in the gap between their slices, which
  slicing drops;
- a reference or footnote definition, at any depth — it is document-scoped, so a
  reference in another block would render as literal text once the definition is in
  a different render pass.

A node with no position MUST also fall back, because guessing its bounds would
silently drop or duplicate text.

Reusing a settled prefix is only sound while the incremental result is
byte-identical to parsing the message whole, so that equivalence MUST be held by a
differential test over EVERY prefix of a corpus that includes the constructs where
block boundaries depend on context — indented code, loose lists, late setext
headings, reference and footnote definitions, raw HTML, CRLF, multibyte text.

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

#### Scenario: A replaced source is not treated as an append

- **GIVEN** a previous split of one message
- **WHEN** the split is called with a different message that does not extend it
- **THEN** the new message is parsed whole

#### Scenario: Splitting does not change what is rendered

- **WHEN** a message contains a top-level raw HTML block, or a reference or
  footnote definition
- **THEN** the whole message renders as one block
- **AND** the rendered output matches the output of rendering the message unsplit

### Requirement: Streaming presentation catches up without a fixed character queue

The visual reveal SHALL scale with the outstanding source length so a large
provider burst does not accumulate a seconds-long queue behind a fixed character
ceiling. It MUST preserve exact source order and complete UTF-16 surrogate pairs.
Terminal events, hidden documents, and reduced motion MUST expose the complete
authoritative source immediately.

#### Scenario: Large bursts converge while the run remains active

- **WHEN** a long answer arrives faster than the normal visual reveal
- **THEN** the reveal accelerates with the backlog and catches up before a
  terminal event is required
- **AND** the final rendered content matches the saved answer when reopened

### Requirement: Highlighting configuration is reused across streamed blocks

Creating a markdown processor MUST NOT rebuild the registered language grammars.
Processors SHALL share the configured highlight transformer while retaining
sanitize-first ordering and explicit-language-only highlighting.

#### Scenario: A later streamed block reuses the language registry

- **WHEN** additional markdown processors render changed blocks in an answer
- **THEN** grammar registration occurs only once per module instance
- **AND** supported languages highlight, unknown languages remain plain, and raw
  HTML remains inert

### Requirement: Subscribers Re-render Only For Fields They Read

The chat store SHALL offer selector subscription. A component that selects a field
MUST NOT re-render when an unrelated field changes.

A selector MUST return a primitive or a reference the store owns. React calls it on
every notification and compares the result by reference, so a selector that builds
a fresh object or array reports a change every time and re-renders forever. A
caller that needs a stable empty list MUST use a module-level constant rather than a
literal.

State that changes while a run streams MUST be selected separately from state that
changes once per conversation or once per run, so the shell above the transcript is
not subscribed to the stream.

#### Scenario: An unrelated field does not wake a subscriber

- **WHEN** a store field changes
- **THEN** only the subscribers whose selected value changed are notified

#### Scenario: A selector returns a stable reference

- **WHEN** a selector runs twice against unchanged state
- **THEN** it returns the identical value or reference

#### Scenario: A withheld list does not re-key a memo

- **GIVEN** a surface that withholds a list by passing an empty one
- **THEN** the empty list is the same reference on every render

### Requirement: Composer Typing Is Independent Of The Transcript

Composer draft text SHALL live outside the transcript store. Typing MUST NOT notify
a transcript subscriber.

An identical write MUST NOT notify at all, so a controlled input re-emitting the
same value wakes nobody.

Resetting the chat store MUST still clear the draft, so signing out leaves none of
the previous account's typing behind — the guarantee the draft had while it was a
field of the chat snapshot.

#### Scenario: A keystroke does not re-render the transcript

- **WHEN** the reader types one character in the composer
- **THEN** no transcript subscriber runs

#### Scenario: An identical write notifies nobody

- **WHEN** the draft is set to the value it already holds
- **THEN** no subscriber is notified

#### Scenario: Signing out clears the draft

- **WHEN** the chat store is reset
- **THEN** the composer draft is empty

### Requirement: Streaming Budgets Are Enforced, Not Reported

The repository SHALL hold benchmarks that FAIL when their budget is exceeded, and
the budget files SHALL be typechecked.

A budget SHALL lean on a structural count rather than a clock wherever it can. A
count of renders is deterministic and is what actually regressed; a jsdom
millisecond is a relative signal, not a product service level.

A budget whose absolute value would flake on a loaded machine or pass a quadratic
implementation on a fast one SHALL be expressed as a RATIO instead.

Every budget number, what it measures, and what it replaced SHALL be recorded
alongside the benchmarks, so a later change can tell a regression from a
renegotiation.

The test pool SHALL be pinned rather than left to the default, because a pool that
lets a worker time out, drop its whole file, and still exit zero produces a green
run that tested less than it claimed.

#### Scenario: A tightened budget fails

- **WHEN** any streaming budget number is lowered by a small margin
- **THEN** the benchmark fails

#### Scenario: A drifted fixture is a compile error

- **WHEN** a benchmark builds a store snapshot that is missing a required field
- **THEN** the typecheck reports it

#### Scenario: A dropped test file fails the run

- **WHEN** a test worker times out and its file does not run
- **THEN** the test command exits non-zero

#### Scenario: A virtualization budget proves the window is bounded

- **WHEN** a 10,000-message conversation mounts
- **THEN** the mounted row count is asserted bounded rather than assumed

## Provenance

Consolidated from the deltas of `refactor-chat-streaming-hot-path` and
`refactor-run-timeline-classification`, reconciled against the shipped code.

`add-turn-revision` proposes one further requirement here — "Editing A Turn Is
Independent Of The Transcript" — which stays in that change until it ships.

Documentation gap, recorded rather than fixed: `benchmarks/chat/README.md` has
sections for the streaming, store-apply, and render budgets but none for
`split.bench.ts`. The ratio gate and its measured comparison against the
whole-message split live in that file's own header comment. The requirement above
says budgets are recorded alongside the benchmarks, which this satisfies, but the
README is where a reader looks first.
