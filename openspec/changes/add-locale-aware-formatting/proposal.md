# Change: Read and show a number the same way everywhere

## Why

`numericValue` in `src/features/canvas/tabularExport.ts:73` strips every comma
before it reads a cell as a number. That is correct for `1,234` written in
English and wrong for the same quantity written in most of Europe:

| Cell            | Means   | `numericValue` returns |
| --------------- | ------- | ---------------------- |
| `1,234` (en)    | 1234    | 1234                   |
| `1,5` (de)      | 1.5     | **15**                 |
| `1.234,56` (de) | 1234.56 | **1.23456**            |

This is not a display complaint. `numericValue` decides the sort order in
`DataGrid` and the axis domain in `ChartCanvas`, so a warehouse that returns
European-formatted measures produces a grid sorted wrongly and a chart drawn to
the wrong scale, with no error and nothing on screen to suggest either. A wrong
number that looks right is the worst failure this product can produce, because
the whole point of it is answering questions with numbers.

The display side has a smaller, verifiable inconsistency. A chart tooltip calls
`toLocaleString()` (`ChartCanvas.tsx:270`) while the axis tick beside it calls
`Number(value.toFixed(1)).toString()` (`ChartCanvas.tsx:529`). On a German
machine the same value reads `1.234,5` in the tooltip and `1234.5` on the axis,
in one image. `src/lib/formatBytes.ts` hand-builds its number the same way.

To be clear about what is already right: `toLocaleString()` is used in the grid,
the cards, and the chart tooltips, and it follows the host locale correctly.
`formatExportTimestamp` writes `YYYY-MM-DD HH:MM`, which is unambiguous in every
locale and is left alone deliberately. This change is not a sweep for its own
sake.

## What Changes

- Reading a number becomes structural, not locale-guessed, and ONE grammar both
  classifies a column as a measure and reads its values. A cell is parsed by the
  shape of its separators; a shape that is genuinely ambiguous is resolved from
  the rest of its column, then of its table, then by a stated default.
- Sign, currency and percent are part of the grammar, so a cell that classifies
  as a measure always reads. Today `-$1,200` and `+$5` pass the classifier in
  `tableChart.ts` and then read as `null` in `numericValue`, so a chart draws a
  gap where a loss belongs and the grid sorts them as text.
- A chart's data carries real numbers, parsed once at the boundary
  (`chartSpecFor`), not strings re-parsed on every draw.
- One display formatter for numbers, used by the grid, the cards, the chart
  tooltips, and the chart axes, so two parts of one image cannot disagree.
- `formatBytes` uses that formatter for its number and keeps its unit words.
- **BREAKING** `numericValue` and `TableArtifact.numericColumns` are removed.
  Reading moves to `src/lib/readNumber.ts` (`resolveNumberFormats`, `readNumber`)
  and `TableArtifact` carries `numberFormats`. Every call site is in this
  repository and is updated here.

## Impact

- Affected specs: `locale-formatting` (new)
- Affected code: `src/features/canvas/tabularExport.ts`,
  `src/features/canvas/canvasStore.ts`, `src/features/canvas/DataGrid.tsx`,
  `src/features/canvas/ChartCanvas.tsx`, `src/features/chat/richContent/cards.tsx`,
  `src/features/chat/richContent/tableChart.ts`,
  `src/features/projects/LargeTableView.tsx`, `src/lib/formatBytes.ts`, and the
  new `src/lib/readNumber.ts` and `src/lib/formatNumber.ts`
- Not affected: the Rust side. No number crosses the boundary pre-formatted;
  `document_export_save` takes text that is already composed.
- Not affected: CSV export. `toCsv` writes the cell as it arrived, which is
  correct — the file must carry what the model produced, not a re-rendering of
  it.
