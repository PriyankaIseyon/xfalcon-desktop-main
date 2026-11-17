## ADDED Requirements

### Requirement: The Value Axis Has Round Ticks

A chart's value axis SHALL place its ticks on multiples of a step that is 1, 2 or
5 times a power of ten, and SHALL extend its domain to the nearest ticks. The
domain SHALL contain zero.

#### Scenario: A series that tops out between ticks

- **WHEN** a chart's largest value is 13,800 and its smallest is 0
- **THEN** its ticks are `0, 2k, 4k, 6k, 8k, 10k, 12k, 14k`

#### Scenario: A series that crosses zero

- **WHEN** a chart's values run from -300 to 1,200
- **THEN** zero is a tick and the zero line is drawn

#### Scenario: A fractional range

- **WHEN** a chart's values run from 0 to 1
- **THEN** its ticks are `0, 0.2, 0.4, 0.6, 0.8, 1`
- **AND** no tick shows floating-point noise

#### Scenario: An axis and its tooltip

- **WHEN** a tick and a tooltip show the same value
- **THEN** they use the same separators

### Requirement: A Time Series Is Drawn By Time

When every x label of a line, area or scatter chart is a date of one granularity,
the chart SHALL position each point by its time, draw the points in
chronological order, and label the axis on calendar boundaries.

#### Scenario: Uneven days

- **WHEN** the labels are `2024-01-01`, `2024-01-02` and `2024-01-30`
- **THEN** the second point is one day after the first and the third is 28 days
  after the second, in proportion

#### Scenario: Unsorted rows

- **WHEN** the rows arrive as `2024-03`, `2024-01`, `2024-02`
- **THEN** a line connects them in January, February, March order

#### Scenario: Monthly data

- **WHEN** the labels are twelve consecutive `YYYY-MM` months
- **THEN** the points are evenly spaced
- **AND** the axis is labelled on month boundaries

#### Scenario: Calendar-boundary labels

- **WHEN** a daily series spans several months
- **THEN** its ticks fall on the first of a month, or on Mondays when it spans
  weeks

#### Scenario: A date in any timezone

- **WHEN** the same chart is drawn on machines in different timezones
- **THEN** it is identical

### Requirement: Only A Clear Date Column Gets A Time Axis

A chart SHALL use a categorical axis unless every label parses as a date of one
granularity and the labels have at least two distinct values.

#### Scenario: Mixed granularity

- **WHEN** the labels are `2024-01` and `2024-01-15`
- **THEN** the axis is categorical

#### Scenario: An ambiguous date

- **WHEN** a label is `01/02/2024`
- **THEN** the axis is categorical

#### Scenario: A number that is not a year

- **WHEN** the labels are `5000` and `5001`
- **THEN** the axis is categorical

#### Scenario: A bar chart

- **WHEN** a chart with date labels is drawn as bars or as a pie
- **THEN** its axis is categorical, in row order
