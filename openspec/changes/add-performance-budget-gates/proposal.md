# Change: Give the canvas surfaces the budgets chat already has

## Why

`benchmarks/chat/` establishes the pattern and enforces it: a 10,000-message
fixture, a 500ms store-apply budget, a 2s open budget, a 2s 500-row mount, a 4s
10k-row mount, and a bounded mounted-item count that proves virtualization is
working rather than assuming it. `benchmarks/projects/` adds a 100,000-file index
smoke test. `docs/contributing.md` even rules on
what a timing number is allowed to claim.

The surfaces added by `add-dynamic-canvas-artifacts` and
`add-lean-report-export` have none of this. `DataGrid` virtualizes and sorts,
`ChartCanvas` builds an SVG per series, and `markdownPipeline` runs sanitize and
highlight over model prose, and no fixture or budget covers any of them. The
tests that exist assert what they do, not what they cost.

Three of those have a cost that grows with input and a plausible bad shape:

- `DataGrid` sorting calls `numericValue` per comparison, so a wide sort is
  `O(n log n)` parses. Nothing asserts the mounted row count stays bounded the
  way `MessageList`'s bench does.
- `ChartCanvas` draws every point of every series into the DOM as SVG nodes.
  There is no cap on point count and no budget on the render.
- `markdownPipeline` sanitizes and highlights on every token of a stream. A long
  answer with many fences is the common case in this product, and it is
  unmeasured.

This is a gap the two recent changes should have closed and did not. It is the
lowest priority of the five proposals written in this pass, and it is the one
most likely to catch a regression in code that is a week old.

## What Changes

- A deterministic fixture generator for a wide, tall table, matching how
  `benchmarks/chat/generate-fixture.mjs` works.
- A `DataGrid` budget: mount, sort, and a bounded mounted-row count under a
  large table.
- A `ChartCanvas` budget: render time and a bounded node count for a many-point
  series, per chart kind.
- A `markdownPipeline` budget: one long answer with many fences, measured once
  for a settled render and once for a streaming tail.
- Each budget states its number and the fact that it is a jsdom number, the same
  qualification `benchmarks/chat/README.md` already makes.
- Where a budget reveals an unbounded surface, the fix is a cap with a stated
  limit and a visible notice, not a faster loop. **A silently truncated chart
  must never read as a complete one.**

## Impact

- Affected specs: `performance-budgets` (new)
- Affected code: `benchmarks/canvas/` (new), and whichever of
  `src/features/canvas/DataGrid.tsx`, `src/features/canvas/ChartCanvas.tsx`, and
  `src/features/canvas/tabularExport.ts` a budget shows needs a cap
- Not affected: the existing chat budgets. This follows their pattern and does
  not restate or renumber them.
- Depends on: `add-locale-aware-formatting` touches the same parse path that the
  sort budget measures. Sequence that change first so the budget is set against
  the parser being kept, not the one being replaced.

## Non-Goals

- A native-shell paint budget. `benchmarks/chat/README.md` lists that as a manual
  trace, and automating it belongs to `add-real-binary-verification`, not here.
- Reworking the chat budgets or their numbers.
- Optimizing anything a budget does not show to be a problem. A budget that
  passes ends the work.
