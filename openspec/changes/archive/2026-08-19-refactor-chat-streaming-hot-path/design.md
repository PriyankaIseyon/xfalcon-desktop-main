## Context

The chat surface has a complete streaming design on the frontend and a core that
never streams. Recon established the facts below by reading the code, not by
inference.

Path of one provider token today:

```
provider SSE chunk
  └─ stream_turn                             src-tauri/src/agent/local_executor.rs:498
       └─ inference_event_for -> TextDelta   local_executor.rs:760
            └─ presentation.emit(event)      stream.rs:87  -> buffered, NOT forwarded
                 ...                        no clock is ever read
            └─ Completed arrives             stream.rs:99  -> flush_pending, one blob
                 └─ app.emit(channel, ev)    src-tauri/src/app/events.rs:39
                      └─ JS listen -> applyInference -> rAF -> React
```

`min_coalesce` is consulted once, at `stream.rs:95`, and only to ask whether it
is zero. Production always passes 16 ms, so production always takes the buffered
branch. Nothing else flushes text.

Measured frontend cost, `benchmarks/chat/stream.bench.tsx`, jsdom, 6.8 KB answer,
12 fences, 80 stream steps:

- empty transcript: 500 fence-card renders, 1472 ms of React work, 18.4 ms per
  frame.
- 500 prior turns with the stream row outside the virtual window: 0 fence-card
  renders, 1931 ms of React work, 24.1 ms per frame.

jsdom numbers are a relative signal, not a product service level. The
`docs/contributing.md` rule on timing claims
applies. The structural count — how many times a settled fence renders — is
deterministic and is the number the budget leans on.

## Goals / Non-Goals

Goals:

- The reader sees text appear as the model produces it.
- Work per streamed frame is proportional to what changed, not to the size of the
  transcript or the size of the answer so far.
- The budgets that prove both are in the repository and fail when broken.

Non-Goals:

- No change to the inference, run, or projection contracts.
- No change to the DA durable mirror or its batching.
- No general `AppShell` refactor. Only its chat subscription moves.
- No new virtualization strategy. `@tanstack/react-virtual` stays. CSS
  `content-visibility` is not added; the transcript is already virtualized, so it
  would bound nothing that is not already bounded.
- No streaming markdown library. The repository already has the parser that
  `react-markdown` itself uses.

## Decisions

### D1. The core flushes on both a clock and a byte cap

`CoalesceConfig` gains `max_coalesce_bytes`. `InferencePresentationSink` gains
`pending_since: Option<Instant>` and a public `flush()`.

`emit` flushes a buffered text delta when either condition holds:

- `pending_since` is older than `min_coalesce`; or
- `pending_text.len()` has reached `max_coalesce_bytes`.

`stream_turn` adds a second tick to its `select!` at `min_coalesce`, which calls
`presentation.flush()`. The existing DA tick keeps its own interval.

Why both. The clock inside `emit` only fires when the next delta arrives, so a
provider that sends three tokens and then stalls would hold them. The tick covers
that. The byte cap covers the opposite case: a burst that arrives faster than the
tick would otherwise grow one delta without bound.

Alternatives considered:

- A timer inside the sink. The sink is synchronous and takes an `FnMut`, and it
  is constructed in four places. A timer would make it async and would change all
  four. The caller already owns a `select!` loop, so the tick belongs there.
- `tauri::ipc::Channel` instead of `app.emit`. A channel is created per invoke,
  and a run outlives its invoke — startup recovery and background runs both
  depend on that. A channel would break run adoption to buy an IPC saving that is
  not the measured bottleneck.
- Scoping the emit to the conversation that owns the run. Rejected for now. The
  frontend owns the view-to-conversation mapping, and the frontend already drops a
  foreign delta with one string compare. The payload still crosses IPC either
  way, so scoping would move the cost, not remove it. Coalescing reduces the same
  cost for every run, foreign ones included.

`pending_sequence` keeps the FIRST buffered delta's sequence. The frontend
watermark at `chatReduce.ts:122` drops `sequence <= last`, and first-sequences
increase across batches, so this stays correct. It is stated here because it is a
decision, not an accident.

### D2. Selector subscriptions, not a state library

`chatStore` keeps the project's module-store pattern (`openspec/project.md`:
"module stores, not Redux/zustand"). It gains:

```ts
useChatSelector<T>(select: (state: ChatStoreSnapshot) => T): T
```

built on `useSyncExternalStore(subscribeChat, () => select(state))`.

`useSyncExternalStore` calls `getSnapshot` on every notification and compares by
reference. A selector that builds a new object or array each call therefore loops
forever. The rule is: **a selector returns a primitive, or a reference the store
itself owns.** A unit test asserts that every selector shipped in this change
returns a stable reference for unchanged state.

Alternative rejected: adding zustand. It is a dependency for a capability the
current set already provides, and the project convention forbids it.

### D3. `composerDraft` leaves the transcript store

Draft text is not transcript state. It moves to `composerStore.ts` with its own
listener set. `useChat` stops returning `setComposerDraft`; `snapshot.composerDraft`
is removed. Every call site moves in the same pass — no shim, no deprecated
alias.

### D4. The synthetic stream row leaves `useChat`

`useChat` returns `snapshot.messages` unchanged, and the streamed text as a
separate value. `MessageList` takes `streamingText` and `streamRunId` props and
builds the tail item itself.

This is what keeps the durable array's identity stable for a whole stream. It also
fixes `AppShell`'s `chatFiles(chat.messages)` memo, which re-ran per frame for the
same reason.

The tail item keeps the `stream-` id prefix, so `MessageRow`'s caret rule and
`AssistantMessage`'s `isDurableMessageId` check are unchanged. Entrance discipline
is unchanged: the tail id still appears and vanishes exactly as before.

### D5. `buildDisplayItems` splits at the run boundary

`runStart` is the index after the last user message. A tool group is always
flushed by a user message, so `runStart` is always a clean group boundary. That
makes the split safe:

- items for `[0, runStart)` are memoized on `messages` alone;
- items for `[runStart, end)` rebuild when reasoning or streamed text changes.

The function takes index bounds rather than slices, so neither half allocates a
copy of the transcript.

### D6. Block-memoized markdown

Port the LibreChat approach. `splitMarkdown.ts` parses the message with
`mdast-util-from-markdown` plus the GFM extension, then returns the exact source
slice of each top-level node. `MarkdownBlock` renders one slice through the
existing `REMARK_PLUGINS`/`REHYPE_PLUGINS` pipeline and is memoized on that slice.

Append-only streaming changes only the last slice, so every settled block skips
both the parse and the render.

Two documented escapes render the whole message as one block, because per-block
rendering would change the output:

- a top-level raw `html` node, since the sanitize schema escapes it to text and
  the separator between adjacent HTML blocks would be lost;
- any reference or footnote definition, which is document-scoped and would render
  as literal text once severed from its definition.

Our fence ladder needs no base-index threading. LibreChat seeds per-block
providers with prefix-summed counters because its code blocks are numbered by
mount order. `AssistantMessage` resolves an HTML document's ordinal by looking the
content up in `findHtmlDocuments(text)` (`htmlDocumentIndex`), which is
position-independent. `saveable` is already `null` while streaming, so it is a
constant across a stream and does not break block memoization.

Why mdast and not a blank-line scanner. A hand-rolled splitter is cheaper but
wrong: it splits a fenced block that contains a blank line, and it splits one
list into two lists. `react-markdown` is built on the same parser, so mdast
boundaries match the rendered output by construction.

New direct dependencies: `mdast-util-from-markdown`, `mdast-util-gfm`,
`micromark-extension-gfm`. All three are already present as transitive
dependencies of `react-markdown` and `remark-gfm`. They are declared directly
because this change imports them directly, and depending on another package's
tree is not a dependency contract.

The per-frame mdast parse is a tokenizer pass with no rehype, no sanitize, no
highlight, and no React. If the budget shows it dominating, the follow-up is an
append-only cache that re-parses from the last stable boundary. That is not built
now, because no measurement asks for it yet.

### D7. `ChatPane` owns the hot subscription

`AppShell` reads cold chat fields — conversation list, title, availability,
run state. The transcript fields change per token. A new `ChatPane` component
subscribes to the hot fields and renders `ChatSurface`. `AppShell` renders
`<ChatPane />`.

This is the smallest change that stops a token from re-rendering 844 lines. It is
not a rewrite of `AppShell`.

### D8. Memoize the row components

`memo` on `MessageRow`, `AssistantMessage`, `UserMessage`, and
`DurableToolTrail`. Every prop these take is a primitive or a store-owned
reference, so the default shallow comparison is correct and no custom comparator
is needed.

## Risks / Trade-offs

- **A flush cadence that is too eager loses the coalescing win.** Mitigation: the
  budget counts emitted deltas for a fixed answer, so a regression to
  per-token emission fails the gate.
- **Block splitting changes rendered output.** Mitigation: the two whole-message
  escapes in D6, plus a test that a message containing each construct renders
  identically split and unsplit.
- **A selector that returns a fresh object loops forever.** Mitigation: the
  stability test in D2. The failure is loud and immediate, not subtle.
- **Removing `snapshot.composerDraft` breaks call sites.** Mitigation: this is
  intended. `tsc` finds every one, and the same pass updates them.
- **Rewriting a test that asserted the defect looks like weakening a test.**
  Mitigation: the proposal names it explicitly, and the replacement asserts the
  stronger property — that text is forwarded before the turn ends.
- **jsdom timings are not browser timings.** Mitigation: the structural counts
  carry the gate; the timing numbers are recorded with that qualification, the
  same one `benchmarks/chat/README.md` already makes.

## Migration Plan

Order is upstream to downstream, and every step leaves a tree that builds and
passes tests.

1. Core: flush contract and its tests. Verified on Windows with `cargo.exe`.
2. Store: selectors, then the composer store, then every call site.
3. Transcript: stream row out of `useChat`, split item build, memoized rows.
4. Markdown: block split, then block memoization.
5. `ChatPane`.
6. Gates: budgets, typecheck coverage, vitest pool.

Rollback is per step. Each step is independent of the steps after it.

## Open Questions

None. The scope question was settled before this change was written: full stack,
core first.
