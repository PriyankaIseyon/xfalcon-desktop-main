# Change: Make the answer stream continuously, and make each frame cheap

## Why

Two defects sit on the same path. Together they are the reason the chat surface
does not feel finished.

**The answer does not stream.** `InferencePresentationSink` buffers text deltas
and flushes them only when a non-text event arrives, when the turn reaches a
terminal event, or at `finish()`. `CoalesceConfig::min_coalesce` is a
`Duration`, and no code reads a clock. Every production path builds the sink
with `CoalesceConfig::for_delta_coalescing()` (16 ms), so every production path
takes the buffered branch and never flushes on time. A plain text answer
therefore arrives as ONE `text_delta` at the end of the turn. The reader sees
dead air, then the whole answer at once.

`src-tauri/src/agent/tests/stream.rs::inference_presentation_sink_coalesces_adjacent_text_deltas`
asserts `forwarded.len() == 1` for two deltas, so the test suite locks the
defect in place rather than catching it. A reasoning model streams by accident,
because each `ReasoningSummary` flushes the pending text; a non-reasoning model
does not stream at all.

Everything the frontend builds for streaming — the rAF commit coalescing in
`chatStore`, `settlingText`, the streaming caret, the stick-to-bottom growth
observer — has nothing to do, because the core delivers one blob.

**Each streamed frame costs more than it should.** A new benchmark,
`benchmarks/chat/stream.bench.tsx`, measures a 6.8 KB answer with 12 fences over
80 stream steps in jsdom:

| scenario                               | fence-card renders | React ms per frame |
| -------------------------------------- | ------------------ | ------------------ |
| empty transcript                       | 500                | 18.4               |
| 500 prior turns, stream row off-screen | 0                  | 24.1               |

The second row is the important one. It costs 24 ms per frame to draw nothing
new, because the transcript rebuilds itself completely on every frame. Three
causes:

- `useChat` builds a fresh `displayMessages` array on every render, so
  `MessageList`'s `buildDisplayItems` walks the whole transcript and allocates
  two id `Set`s of every message, once per frame. `AppShell`'s
  `chatFiles(chat.messages)` memo re-runs for the same reason.
- No component in `src/` is memoized. `grep -r "memo("` returns nothing.
  `react-markdown` parses in its render body, so every mounted row re-runs
  remark, sanitize, and highlight on every frame.
- `chatStore` publishes one snapshot object, and `AppShell` subscribes to all of
  it. `composerDraft` lives in that snapshot, so a keystroke re-renders the whole
  application, and a token re-renders 844 lines of `AppShell`.

The 500 fence-card renders match the number LibreChat measured before it fixed
the same defect (PR #13576: 500 renders and 388 ms became 62 renders and 172 ms).
`references/LibreChat/client/src/components/Chat/Messages/Content/splitMarkdown.ts`
holds the approach.

Two hygiene gaps let this rot unseen:

- `tsconfig.app.json` is `include: ["src"]`, so `benchmarks/` and `tests/` never
  typecheck. `benchmarks/chat/storeApply.perf.test.ts` builds a
  `ChatStoreSnapshot` that is missing five required fields, and nothing reports
  it. The performance gate has silently drifted from the store it guards.
- `vitest.config.ts` sets no `pool`. A worker timeout drops a test file and the
  run still exits 0.

## What Changes

Core, first, because the render work is pointless until the stream is real:

- `CoalesceConfig` gains a byte cap, and `min_coalesce` becomes a value that is
  actually enforced. `InferencePresentationSink` flushes buffered text when the
  interval has elapsed or the cap is reached, and exposes `flush()` so a caller
  can flush on a timer.
- `stream_turn` flushes the presentation sink on its own tick, so a provider that
  stalls with text buffered still shows that text. The existing DA batch tick is
  left alone; the two cadences are independent.
- **BREAKING** (internal): `inference_presentation_sink_coalesces_adjacent_text_deltas`
  asserted the defect. It is rewritten to assert progressive flushing.

Frontend, upstream first:

- `chatStore` gains selector subscriptions. A subscriber names the fields it
  reads and re-renders only when those change.
- `composerDraft` leaves the transcript store for its own module store. Typing
  never notifies a transcript subscriber. **BREAKING**: `useChat` no longer
  returns `setComposerDraft`, and `snapshot.composerDraft` is removed.
- The synthetic in-flight row leaves `useChat`. `MessageList` takes the streamed
  text as its own prop and builds the tail item itself, so the durable message
  array keeps its identity for the whole stream.
- `buildDisplayItems` splits at the run boundary. The turns before the last user
  message are memoized on `messages`; only the current run's turns rebuild when
  reasoning or streamed text arrives.
- `AssistantMessage` splits model prose into top-level markdown blocks and
  memoizes each one on its own source slice. A settled fence stops re-parsing.
- `MessageRow`, `AssistantMessage`, `UserMessage`, and `DurableToolTrail` are
  memoized.
- The chat subtree moves into a `ChatPane` component that owns the hot
  subscription, so a token does not re-render `AppShell`.

Gates:

- `benchmarks/chat/stream.bench.tsx` becomes a budget with stated numbers, not a
  report.
- `benchmarks/` and `tests/` join the typecheck. The drifted
  `ChatStoreSnapshot` in `storeApply.perf.test.ts` is repaired.
- `vitest.config.ts` pins `pool: "threads"`, so a dropped file fails the run.

## Impact

- Affected specs: `chat-streaming-performance` (new)
- Affected code:
  - `src-tauri/src/agent/stream.rs`, `src-tauri/src/agent/local_executor.rs`,
    `src-tauri/src/agent/tests/stream.rs`
  - `src/features/chat/chatStore.ts`, `composerStore.ts` (new), `useChat.ts`,
    `MessageList.tsx`, `MessageRow.tsx`, `AssistantMessage.tsx`,
    `UserMessage.tsx`, `ToolActivityCard.tsx`, `ChatSurface.tsx`,
    `markdownBlocks.tsx` (new), `splitMarkdown.ts` (new)
  - `src/app/AppShell.tsx`, `src/features/chat/ChatPane.tsx` (new)
  - `benchmarks/chat/`, `tsconfig.app.json`, `vitest.config.ts`
- No contract change. `text_delta.text` is already an arbitrary string and
  `sequence` stays monotonic, so `contracts.ts` is untouched and no Windows
  contract regeneration is needed.
- Not affected: the DA durable mirror. `stream_turn`'s batch records already
  flush on their own interval, and only the presentation path changes.
- Relation to `add-performance-budget-gates`: that change flags the per-token
  markdown re-parse as unmeasured and proposes measuring it on the canvas
  surfaces. This change measures and fixes it on the chat surface. The two do not
  overlap in code, and neither blocks the other.
