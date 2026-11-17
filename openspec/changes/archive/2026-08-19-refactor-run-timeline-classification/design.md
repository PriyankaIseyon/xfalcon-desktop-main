# Design

## The rule that replaces the proxies

> A piece of assistant content renders on the surface where its durable
> form lives, from its first token, and no other surface ever draws it.

Every relocation the reader currently sees comes from breaking that rule:
narration is durably an assistant message (so it belongs in the
transcript) but is drawn in the card; the answer is durably an assistant
message (so it belongs in the transcript) but is drawn in the card while
it streams. Reasoning is the one kind that is never durable, so it is the
one kind that belongs to a live surface only — and `chatStore` already
exempts it from the durable-snapshot retirement for exactly that reason.

The rule is decidable at the first token, which is what makes it a rule
rather than a guess. It needs no knowledge of whether a tool call follows.

| content     | durable form                      | surface              |
| ----------- | --------------------------------- | -------------------- |
| reasoning   | none, ever                        | live disclosure only |
| round prose | assistant message with tool calls | transcript prose row |
| tool calls  | `tool_calls` on that message      | tool umbrella        |
| answer      | assistant message, no tool calls  | transcript prose row |

## Two surfaces, split by durability, not by time

- **The virtualizer draws `messages`.** Its row count and keys change only
  when the durable conversation changes. A streamed token cannot touch it.
- **The tail draws the in-flight run.** Bounded by construction:
  `MAX_TOOL_ROUNDS` (8) rounds, `MAX_TOOL_CALLS_PER_RUN` (24) calls, one
  reasoning disclosure, one streaming row. It needs no virtualization and
  gets none.

The synthetic `stream-` row moves from the first surface to the second.
This is required by ordering — the streaming answer has to render _below_
the run's tool cards, and the tail renders after every virtual row — and it
is also strictly cheaper: `getItemKey`, `estimateSize` and
`measureElement` stop being called for a row whose height changes every
frame.

`listRef` already spans the virtualizer _and_ the tail, so the existing
`ResizeObserver` that re-pins the bottom keeps following the stream with no
change. That observer, not `scrollToIndex`, is what follows content growth
today.

## The live timeline is the durable timeline, one frame early

`LiveRunTimeline` renders the same block sequence `buildDisplayItems`
produces for a settled run, from live state instead of from `messages`:

```
reasoning (afterCallCount ≤ 0)      →  <ReasoningDisclosure>
roundProse[0]                       →  prose row
activity rows for round 0           →  <LiveRunActivity events={slice}>
reasoning (afterCallCount ≤ 1)      →  <ReasoningDisclosure>
roundProse[1]                       →  prose row
activity rows for round 1           →  <LiveRunActivity events={slice}>
streamingText || settlingText       →  prose row, streaming caret
```

`afterCallCount` stays the single ordering axis both surfaces read, which
is what makes the settle invisible: the durable build walks `tool_calls`
counts and the live build walks drawn rows, and a segment lands after the
same number in either.

One consequence, accepted deliberately: an umbrella now closes at every
prose row and every reasoning disclosure, so a multi-round run shows one
card per round rather than one card for the run. That is the ordering
requirement, not a separate decision — a single card cannot have prose in
the middle of it and also be a single card. With eight rounds as the hard
bound, the worst case is eight collapsed one-line cards, and each is
labelled with its own call count.

## Why `roundProse` and not `liveThinking`

The field holds prose the model chose to emit and that its turn keeps
durably. Calling it thinking is what let it be rendered with the thinking
component, gated by the thinking flag, and deleted with the thinking. The
rename is the fix; the render change alone would leave the next reader of
this code the same trap.

## The empty-card rule

One predicate, read by the pill and by the card, so they cannot disagree:

```
the live run has something to draw  ⇔  activity.length > 0
                                    ∨  roundProse.length > 0
                                    ∨  liveText.length > 0
                                    ∨  (showReasoning ∧ reasoning.length > 0)
```

It lives in `runSurface.ts` with `responseAlreadyOpen`, because each has two
readers and a predicate with two readers and two definitions is exactly what
produced defect 3. `showReasoning` is a PARAMETER rather than an assumption,
so a reader who turns the preference off gets a coherent surface instead of
an empty box. `LiveRunActivity` additionally refuses to render a card with
no rows, in the same shape `DurableToolTrail` already refuses — a component
that can draw nothing must say so itself, not rely on its caller.

## Why the reasoning switch is a preference and the payload switches are not

Whether the model's own words are readable is a verbosity question: the cost
of showing them is a reader's attention, and the reader is the one who can
weigh it. It is therefore theirs — `AppearanceSettings.show_reasoning`,
persisted per device, shown by default because a model that was asked to
think produced the text and an install that has never opened Settings should
be able to see what it paid for. `bool::default()` is `false`, which would
have silently hidden it on every existing install, so the contract names the
default rather than deriving it.

Whether a tool's arguments and results are readable is a different question
with a different owner. Those quote project data — a tool's error text
routinely echoes the rows it was given — so the decision is the build's, not
the reader's, and `SHOW_TOOL_PAYLOADS` stays a constant.

## Alternatives rejected

**Keep the answer in the card and stop the leaks.** Preserves
`run-reasoning-activity` exactly and is a much smaller change. Rejected:
the answer still streams as `text-xs` inert text inside a collapsible and
then re-renders as markdown prose, so the largest jump — the one on the
answer itself — survives. It also keeps the rule that cannot be decided at
the first token, so the next feature that reads it re-derives the same
proxy.

**Stream everything into the card, promote on settle.** Uniform, no
mid-stream relocation. Rejected: it makes the answer's streaming form
worse for the common case (a tool-free run) to make the rare case
consistent, and every artifact the answer contains — a table, a preview, a
code card — cannot render inside an inert block.

**Render narration but withhold it durably.** Keeps the transcript clean
after the fact. Rejected: content that exists while a run streams and is
gone once it settles is the defect this change removes, not a design.
