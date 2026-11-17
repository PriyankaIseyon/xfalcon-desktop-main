## Context

The chart is hand-drawn SVG (see `dynamic-canvas-artifacts`) with a fixed
620 x 360 viewBox, so every scale is a function from a number to a coordinate.
Two scales were implicit in the drawing code: `yFor`, driven by a padded min/max,
and `xFor`, driven by an index. This change makes each an explicit, tested
function.

## Decisions

- **Decision: d3's tick step and nice loop, written here.** The step is the
  1-2-5 value nearest `(max - min) / count` by the usual thresholds
  (`sqrt(50)`, `sqrt(10)`, `sqrt(2)`), the domain is widened to multiples of it,
  and the step is recomputed until stable. Ticks are integer multiples of the
  step, rounded to 12 significant digits, so `0.1 + 0.2` never reaches a label.
  Not added as a dependency: it is forty lines, the chart already declines a
  charting library, and the result has to land in a standalone SVG.

- **Decision: zero stays in the domain.** `ChartCanvas` already forces the
  domain to contain zero so a loss draws below the axis. The nice scale is
  computed over that range, so the guarantee is unchanged. The old 8% headroom
  is no longer needed: the domain reaches the next tick above the data.

- **Decision: a time axis only when every label parses, as one granularity.**
  Strict, anchored parsers for year (1900-2100 only, so a code like `5000` is
  not a year), quarter, month, day and timestamp, in ISO and common English
  spellings. Anything else — mixed granularity, a bare `Jan`, `01/02/2024`
  (day-first or month-first, unknowable), a single point, one repeated date —
  stays categorical. A wrong time axis is worse than no time axis.

- **Decision: positions are in the granularity's own unit.** Years as the year,
  months and quarters as months since year 0, days as days since the epoch,
  timestamps as milliseconds. A monthly series is therefore evenly spaced
  rather than 28-to-31-days uneven, and a daily one is spaced by days. Every
  computation is UTC, so a chart reads the same in every timezone.

- **Decision: ticks on calendar boundaries, from a ladder.** The smallest step
  with at most seven ticks: days, 7 days aligned to Monday, months, quarters,
  years, decades; minutes to hours for timestamps. A range that fits no rung in
  two or more ticks labels its own two endpoints.

- **Decision: bar and pie stay categorical.** A bar's slot is its category; a
  time-proportional bar axis needs a bar width rule this change does not set.
  Switching a time series from line to bar therefore re-spaces it, which is the
  honest consequence of bars being categories.

- **Decision: a chronological sort for time charts only.** An unsorted time
  series drawn by position zigzags back across itself. Points are drawn in time
  order; ties keep input order.

## Risks / Trade-offs

- A date label that parses but means something else (an order id that looks like
  `2024-01`) gets a time axis. Accepted: the strict grammar makes it rare, and
  the axis is still labelled with the label's own value.
- Month-to-month spacing is by month, not by calendar day. A series mixing
  `2024-01` and `2024-01-15` is mixed granularity and stays categorical.

## Migration Plan

No stored data changes. A chart draws differently on the next render.
