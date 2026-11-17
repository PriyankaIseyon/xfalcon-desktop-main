## 1. Reading a number

- [x] 1.1 `src/lib/readNumber.ts`: one grammar for sign, currency, digits and
      percent; classify a cell as plain, definite (names the decimal) or
      ambiguous, from its own separators.
- [x] 1.2 Resolve a column's convention from its unambiguous cells, then the
      table's. Disagreement inside a column refuses the column, not a majority
      vote.
- [x] 1.3 `readNumber(cell, format)` reads a cell under a resolved format and
      returns `null` for a cell that contradicts it.
- [x] 1.4 Keep the refusals: empty, bare sign, non-finite, date-like,
      version-like, text, hex, exponent, magnitude suffix, parenthesised negative.
- [x] 1.5 A trailing percent does not divide by a hundred; stated in the module
      comment.
- [x] 1.6 A column with nothing deciding it: `d,ddd` reads as comma-groups,
      `d.ddd` is refused.

## 2. Wiring the call sites

- [x] 2.1 `tableChart.ts`: `numberFormatsFor` replaces `measureColumns` and
      `MEASURE_RE`; `chartSpecFor` writes numbers (and `null`) into chart data.
- [x] 2.2 `TableArtifact.numberFormats` replaces `numericColumns`; `numericValue`
      is removed.
- [x] 2.3 `DataGrid` reads each cell once per sort (decorate, sort, undecorate).
      A non-measure column sorts as text.
- [x] 2.4 `ChartCanvas` and the chart offer in `cards.tsx` treat a non-measure as
      a non-measure; `LargeTableView` uses `numberFormatsFor`.
- [x] 2.5 Decided: no refused-column tooltip here. A refused column is visible by
      the absence of numeric sort and chart; recorded in `design.md`.

## 3. Showing a number

- [x] 3.1 `src/lib/formatNumber.ts`: cached `Intl.NumberFormat`, one per digit
      setting, plus a step-aware `formatTick`.
- [x] 3.2 `DataGrid`, `cards.tsx` counts, and `ChartCanvas` tooltips use
      `formatNumber`. Other `toLocaleString` call sites are deliberately left.
- [x] 3.3 The axis tick uses `formatTick`, so axis and tooltip agree.
- [x] 3.4 `formatBytes` formats its number through `formatNumber` and keeps its
      unit words.
- [x] 3.5 `formatExportTimestamp` is unchanged (reason in its comment).

## 4. Tests

- [x] 4.1 `numericValue("1,234")` coverage restated in `readNumber.test.ts`, by
      shape.
- [x] 4.2 A table of cells per shape, including the sign, currency and grouping
      cases, and every refusal (`readNumber.test.ts`).
- [x] 4.3 A dot-ambiguous column is not numeric; a comma-ambiguous column falls
      back to comma-groups; the table decides when it can.
- [x] 4.4 A column with disagreeing unambiguous cells is not numeric.
- [x] 4.5 A German-formatted column builds the same chart data as the same
      quantities in English (`tableChart.test.ts`).
- [x] 4.6 With a forced `de-DE` formatter, axis and tooltip use the same
      separators (`ChartCanvas.locale.test.tsx`).
- [x] 4.7 CSV export of `1.234,56` and `-$1,200` is byte-identical to the input
      apart from the existing formula guard (`tabularExport.test.ts`).

## 5. Verify

- [x] 5.1 `vitest run`, `tsc -b`, `eslint`, `prettier --check`, `check:styles`,
      `check:tooltips` green; 156 files, 2,405 tests.
- [ ] 5.2 Live: a table of European-formatted measures sorts and charts
      correctly in the running app.
