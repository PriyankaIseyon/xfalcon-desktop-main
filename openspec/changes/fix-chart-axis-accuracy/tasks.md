## 1. The scales

- [x] 1.1 `chartScale.ts`: `niceScale` (d3 step, nice loop, drift-free ticks).
- [x] 1.2 `chartScale.ts`: `resolveTimeAxis` with strict parsers, one granularity,
      UTC positions, calendar-boundary ticks.
- [x] 1.3 Tests: 200 seeded ranges hold every `niceScale` guarantee; the worked
      examples; each time parser and each rejection; the output is identical
      under `TZ=Asia/Kolkata` and `TZ=America/Los_Angeles`.

## 2. The chart

- [x] 2.1 `ChartCanvas` takes numbers from its data (no parsing) and draws the
      nice value axis.
- [x] 2.2 Line, area and scatter use the time axis when `resolveTimeAxis`
      answers, draw in chronological order, and keep input order for ties.
- [x] 2.3 Bar and pie stay categorical.
- [x] 2.4 Tick labels use `formatTick`; tooltips use `formatNumber`.
- [x] 2.5 The exported SVG is still self-contained (the export function and the
      presentation-attribute rule are unchanged; the new labels follow them).

## 3. Tests

- [x] 3.1 A chart of `13,800` draws the labelled ticks in the spec.
- [x] 3.2 Uneven days draw proportional x coordinates.
- [x] 3.3 Unsorted dates draw a left-to-right path.
- [x] 3.4 A negative value draws below the zero line; a missing value draws a gap.
- [x] 3.5 A categorical chart is unchanged.

## 4. Verify

- [x] 4.1 `vitest run --pool=threads`, `tsc -b`, `eslint` green. Check the
      `Errors` line, not only the exit code.
- [x] 4.2 No baseline compares a `ChartCanvas` image: `tests/visual/project.spec.ts`
      attaches its chart screenshots without comparing them, and
      `chart-preview.spec.ts` draws an HTML document, not this chart. Nothing to
      regenerate.
