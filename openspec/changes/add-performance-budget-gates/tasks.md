## 1. Fixtures

- [ ] 1.1 `benchmarks/canvas/generate-fixture.mjs` writing a deterministic wide
      and tall table. Follow `benchmarks/chat/generate-fixture.mjs` — same shape,
      same committed-fixture approach.
- [ ] 1.2 A fixture for a many-point multi-series chart.
- [ ] 1.3 A fixture for a long answer with many fences, mixing the languages the
      registry covers.
- [ ] 1.4 `benchmarks/canvas/README.md` stating each budget, its number, and
      that jsdom numbers are a relative signal and not a product SLA — the
      qualification `benchmarks/chat/README.md` already makes.

## 2. Grid budget

- [ ] 2.1 Mount budget for a 10,000-row, 12-column table.
- [ ] 2.2 Bounded mounted-row assertion, following `render.bench.tsx`'s
      under-64 pattern rather than inventing a new one.
- [ ] 2.3 Sort budget for a numeric column. Set it after
      `add-locale-aware-formatting` lands, so it measures the parser being kept.
- [ ] 2.4 Scroll to the end and assert the row count is still bounded.

## 3. Chart budget

- [ ] 3.1 Render budget and node-count bound per chart kind at 5,000 points.
- [ ] 3.2 The maximum supported series count at that point count.
- [ ] 3.3 If a budget fails, add a point cap with a stated limit and a visible
      notice of how many points arrived. Do not make the loop faster and leave
      the surface unbounded.

## 4. Markdown budget

- [ ] 4.1 Settled render budget for the long answer.
- [ ] 4.2 Per-batch budget for the same answer as a streaming tail.
- [ ] 4.3 Confirm the module-level lowlight instance is still built once. A
      per-render construction would show here as a linear cost in batch count.

## 5. Verify

- [ ] 5.1 Every budget fails when its number is lowered by a small margin. A
      budget that cannot fail is not a gate.
- [ ] 5.2 `vitest run --pool=threads` green, and check the `Errors` line — a
      worker timeout drops a file silently while the run still exits 0.
- [ ] 5.3 Add the canvas benchmarks to the same script that runs the chat ones,
      so they are not a directory nobody executes.
