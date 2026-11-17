# Chat performance budgets

Deterministic fixtures and enforced budgets for the chat store, the transcript
render, and the streaming hot path.

A jsdom millisecond is a **relative signal, not a product service level** — the
rule `docs/contributing.md` sets for timing
claims applies to every number here. Where a budget can lean on a structural
count instead of a clock, it does: a count of renders is deterministic, and it is
what actually regressed.

## Generate fixture

```powershell
node benchmarks/chat/generate-fixture.mjs
```

Writes `benchmarks/chat/fixture-10k.json` with 10,000 mixed-length messages.

## Streaming budget

```powershell
npm test -- benchmarks/chat/stream.bench.tsx
```

Streams a 6.8 KB answer with 12 fences in 80 steps into an empty transcript
and below 500 settled turns, then measures the transcript's own per-frame cost
with the answer's markdown excluded.

| measure                         | budget | now    | before `refactor-chat-streaming-hot-path` |
| ------------------------------- | ------ | ------ | ----------------------------------------- |
| fence-card renders, 0 prior     | < 120  | 60     | 500                                       |
| fence-card renders, 500 prior   | < 120  | 60     | 500                                       |
| transcript ms/stream, 500 prior | < 750  | 34-120 | ~910                                      |
| transcript ms/stream, 0 prior   | none   | 5-15   | —                                         |
| settled tool-card renders       | <= 8   | 1      | 80                                        |
| composer renders, 80 deltas     | <= 2   | 0      | 80 (see note)                             |
| editing-row renders, 20 keys    | > 0    | 20     | 20 (see note)                             |
| other-row renders, 20 keys      | 0      | 0      | 780 (see note)                            |

The **fence-card render count** is the budget that matters. 12 is the floor —
each fence renders once when its block completes. 500 was one render of every
settled fence on every frame, because the whole message re-parsed per token.
It is now IDENTICAL at 0 and 500 prior turns, which is the property: the cost
of an answer belongs to the answer, not to what is above it.

The **settled tool-card render count** is the third structural gate: a round's
card must not re-render while a later answer streams. It was rebuilt once per
token by a memo keyed on the streaming text, which re-derived and re-measured
every row of it.

The **composer render count** streams 80 deltas through the real `ChatPane` and
the real chat store, and counts renders of the real `Composer`. The composer's
inputs do not change while text streams, so the budget is a small constant that
covers the commit that starts the run. The **turn-edit** rows type 20 characters
into an open turn editor with 40 settled rows mounted. The editing row re-renders
once per keystroke, with its editor. No other row may re-render.

The "before" column for these three rows is the code before this change. The
composer rendered once per delta because `ComposerDock` was not memoized. Every
row rendered once per keystroke because `MessageRow` read the whole turn-edit
state, not only the id being edited. Those before figures are measured on that
code, not on the `refactor-chat-streaming-hot-path` baseline the column is named
for.

The **transcript-overhead** scenario streams a trivial tail instead of the real
answer, so what it measures is `MessageList`'s own per-frame work. Before
`refactor-run-timeline-classification` this isolation came for free — the
in-flight row was a virtual row that jsdom's fake scrolling kept outside the
window — but that case cannot happen now the row is in the live tail, and it
never happened in the app either: the answer being written is exactly where the
reader is looking. 9 ms with one row mounted and 74 ms with a full window is
the mounted-window size, not the conversation length; the window is bounded by
`overscan`.

The corresponding core-side budgets are
`src-tauri/src/agent/tests/stream.rs::forwards_more_than_once_and_fewer_times_than_it_received`
and its reasoning twin. Both fail in both directions: one forward for a whole
stream is the defect that shipped, and one forward per token is the cost
coalescing exists to remove.

## Store apply budget

```powershell
npm test -- benchmarks/chat/storeApply.perf.test.ts
```

Asserts applying projection-sized batches through the chat reducers stays under
the MVP budget (500 ms in jsdom).

## Document fence budget

```powershell
npm test -- benchmarks/chat/markdownDocument.bench.tsx
```

Renders a 60 KB HTML document fence through the chat path and counts what the
highlighter returns for it. A document fence is drawn as a preview card and its
highlighted source is never shown, so tokenising it was per-delta work thrown
away. The chat path now skips it (`DOCUMENT_AWARE_REHYPE_PLUGINS`); the bare
`REHYPE_PLUGINS` row of the same bench still highlights it, which shows the
counter is live.

| measure                         | budget | now    | before `markdownDocument` change |
| ------------------------------- | ------ | ------ | -------------------------------- |
| highlighted fences, chat path   | 0      | 0      | 1                                |
| highlight tokens, chat path     | 0      | 0      | 13,543                           |
| highlight tokens, bare pipeline | > 0    | 13,543 | 13,543                           |

## Open and render budgets

```powershell
npm test -- benchmarks/chat/render.bench.tsx
npm test -- benchmarks/chat/render-10k.bench.tsx
npm test -- benchmarks/chat/open-10k.bench.tsx
```

500-row and 10k-row `MessageList` mounts, and a 10k-message conversation open,
each with a bounded mounted-item count that proves virtualization is working
rather than assuming it.

The 10k-row test also streams 80 live-tail updates with the durable history
unchanged. The budget is **zero re-estimates of settled offscreen rows** after
mount. An unstable virtualizer key callback caused 80 re-estimates of the sampled
row before this fix. A real history replacement still changes the callback and
refreshes measurements.

## Production browser profile

```powershell
npm run build
npx playwright test -c tests/e2e/playwright.production.config.ts tests/e2e/chat-performance.spec.ts --workers=1
```

The real frontend opens 10,000 saved messages, streams a 120-paragraph answer
with simulated 4x CPU slowdown, and accepts a composer draft during the stream.
The test enforces a bounded history window, complete final text, preserved draft,
and retired caret. It prints and attaches frame-gap percentiles and long tasks
as diagnostics, without a machine-dependent timing assertion.

Use the production configuration for frame measurements. React's development
checks add substantial overhead under CPU throttling. Native IPC is mocked in
this browser scenario; its timings do not measure Windows WebView or provider
latency.

## These files are typechecked

`tsconfig.bench.json` covers `benchmarks/` and `tests/`, and `npm run typecheck`
runs it as a second pass. Before that, `storeApply.perf.test.ts` had drifted five
fields behind `ChatStoreSnapshot` and nothing reported it — the gate was
measuring a shape the store no longer had.

## Manual paint trace

1. Open a conversation with the generated fixture loaded in dev tools.
2. Record a Performance profile while scrolling, and while an answer streams.
3. The streamed frames should show one block re-parsing, not the whole message.
