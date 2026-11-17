# Change: Draw a chart's axes honestly

## Why

`ChartCanvas` draws two things that look like measurements and are not.

**The x axis is by index.** A line over `2024-01-01`, `2024-01-02`, `2024-01-30`
puts the three points an equal distance apart, so a four-week gap reads as a
one-day gap. On a trend line the slope between two points is the claim, and the
slope is wrong whenever the dates are uneven (weekends removed, missing months,
a late first reading). A time series drawn by position is a chart of the order
of rows, not of time.

**The y ticks are not round.** The domain is `min/max ± 8%` and the five
gridlines are five equal fractions of it, so a series topping out at 13,800
draws ticks at `0, 3.7k, 7.4k, 11.1k, 14.9k`. A reader cannot say what a bar
between two gridlines is worth, and no two charts share a scale.

Both are rendering accuracy, not taste: the reader takes values and rates of
change off the axes.

## What Changes

- The value axis uses a "nice" scale: a step of 1, 2 or 5 times a power of ten,
  ticks on multiples of it, and a domain that reaches the nearest ticks, always
  including zero (the existing rule that a bar chart is honest about losses).
- When every x label is a date of one granularity (year, quarter, month, day,
  or a timestamp), line, area and scatter charts place points by time, sort them
  chronologically, and label the axis on calendar boundaries. Bar and pie
  charts stay categorical.
- Axis labels come from the shared number formatter (see
  `add-locale-aware-formatting`), so an axis tick and its tooltip agree.
- A new pure module `src/features/canvas/chartScale.ts` owns the scale maths so
  it is tested without a DOM.

## Impact

- Affected specs: `chart-axes` (new)
- Affected code: `src/features/canvas/ChartCanvas.tsx`, and the new
  `src/features/canvas/chartScale.ts`
- Depends on: `add-locale-aware-formatting` (chart data are numbers; `formatTick`)
- Not affected: the exported SVG's self-contained rule, the palette, the Rust
  side, the data a chart receives.
- Visual baselines: none compares a `ChartCanvas` image, so none changes.
