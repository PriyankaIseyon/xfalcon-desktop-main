## Context

Two different jobs are tangled together today, and they want opposite answers.

**Showing** a number is about the reader. It should follow the machine's locale,
which is what `toLocaleString()` already does.

**Reading** a number is about the data. A chat table's cells arrive as strings
from a model that is quoting a warehouse. The warehouse's formatting has nothing
to do with the reader's machine. A German analyst querying an American warehouse
gets `1,234.56`; an American analyst querying a German one gets `1.234,56`.
Parsing either by the host locale is wrong half the time.

## Goals / Non-Goals

- Goals: never return a wrong number; a cell that classifies as a measure always
  reads; no regression for the English tables that chart today; one display
  formatter.
- Non-Goals: a locale picker in Settings; translating the interface; parsing
  spelled-out numbers, magnitude suffixes (`3.5M`), scientific notation, or
  accounting parentheses; changing what CSV export writes.

## Decisions

- **Decision: one grammar classifies and reads.**

  `-$1,200` passed `MEASURE_RE` and then read as `null`, because the classifier
  and the reader were two different pieces of code that disagreed about where a
  sign may sit. A cell is now `sign? currency? sign? digits currency? %?` with
  at most one sign and one currency mark, and the same function decides whether
  a column is numeric and what each cell is. Signs: `-`, `+`, U+2212. Currency:
  `$ € £ ¥ ₹ ₩ ₽ ₺ ₪ ₫ ฿ ₱ ₦`, before or after (`5,00 €`). A trailing percent
  does not divide by a hundred — the axis shows the number the source wrote.
  No exponent, hex, magnitude suffix or parenthesised negative: those refuse.

- **Decision: parse by structure, then by column, then by table, then by
  default.**

  For the digits of one cell, in order:

  1. Both `,` and `.` present — the last one is the decimal and must appear
     once; the other groups. `1.234,56` is German, `1,234.56` is English.
  2. One mark repeated (`1,234,567`) — it groups; a decimal cannot repeat.
  3. Spaces, thin and no-break spaces, and apostrophes only ever group
     (`1 234,5`, `1'234.5`). With one of them present a single mark is the
     decimal.
  4. One mark once, tail not exactly three digits — decimal (`1,5`, `1.50`).
  5. One mark once, three-digit tail, but a head that cannot be a group (more
     than three digits, or a leading zero) — decimal (`0,500`, `1234,567`).
  6. One mark once, three-digit tail, plausible head (`1,234`) — AMBIGUOUS.
  7. No separator — plain.

  Groups are the usual 3-3-3 or the Indian 2-2-3 (`12,34,567`).

  An ambiguous cell takes the convention of its own column's unambiguous cells;
  failing that, of the rest of the TABLE, because one source formats one table
  one way. A column with no evidence at all is settled by its mark: `d,ddd`
  reads as comma-groups (English thousands is by far the likelier reading and the
  commonest table there is), `d.ddd` is refused (as likely German thousands as
  English three-decimal values, a factor of a thousand apart).

  - Alternative considered: use `Intl.NumberFormat().formatToParts()` to learn
    the host's separators and parse with those. Rejected: it answers "how would
    this machine write it", which is not the question, and it would make the
    same file sort differently on two laptops.
  - Alternative considered: refuse a column whose cells are all ambiguous (the
    first draft of this design). Rejected after implementation: `$1,200 /
$3,400 / $12,500` is the commonest table there is and charts today, so
    refusing it is a regression for the dominant case in order to protect a
    rare one (a European source whose every figure is a three-decimal value).
    The default is the convention SQL engines and models write and applies only
    when column and table give no evidence. The dot case is not defaulted: there
    the likelihoods are even, so it refuses.
  - Alternative considered: assume English always, as before. Rejected: that is
    the defect.

- **Decision: disagreement refuses the column.** A column whose unambiguous
  cells name two conventions (`1.234,5` beside `9,876.5`) is a broken source.
  Refusing it says so; a majority vote would hide it.

- **Decision: numbers are read once, at the boundary.** `chartSpecFor` resolves
  the formats and writes real numbers (and `null` for a missing value) into the
  chart's data, so `ChartCanvas` never parses text. A table artifact carries
  `numberFormats` for the grid. The grid reads each cell once per sort, not once
  per comparison.

- **Decision: one `formatNumber` wrapping cached `Intl.NumberFormat`s.**
  Constructing `Intl.NumberFormat` is not free and a chart axis formats on every
  render, so each instance is built once per digit setting — the same reasoning
  that keeps the lowlight instance out of the render path in
  `markdownPipeline.ts`. Axis ticks keep the English magnitude suffixes
  (`k M B T`); the numerals and separators follow the host locale.

- **Decision: leave `formatExportTimestamp` alone.**
  `YYYY-MM-DD HH:MM` is unambiguous everywhere and its comment already states
  the choice. Changing it to a locale format would make an exported document
  read differently depending on who exported it, which is worse for a file that
  gets mailed around.

## Risks / Trade-offs

- A column whose every cell is `d,ddd` is read as English thousands. From a
  European source whose every figure has exactly three decimals it is wrong by a
  factor of a thousand with no signal. Real, accepted, and bounded: a round-trip
  of 14,400 generated columns across sixteen locales read 87 columns (0.6%) this
  way, all of them columns where every single value had exactly three decimals,
  and refused another 985 visibly.
- A refused column does not sort numerically and is not offered as a chart
  measure. That is visible; a wrong reading is not.

## Migration Plan

No stored data changes. Nothing is persisted from either path — parsing happens
when a grid sorts or a chart draws. A build with this change reads an existing
conversation exactly as before for English-formatted data.

## Open Questions

- Should a column read under the default say so in its header tooltip? Not done
  here; the default only applies to data that charts correctly today.
