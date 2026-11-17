## ADDED Requirements

### Requirement: Reading A Number Is Structural

A cell SHALL be read as a number from the arrangement of its own separators, not
from the machine's locale. The same cell SHALL read as the same number on every
machine.

#### Scenario: Both separators are present

- **WHEN** a cell is `1.234,56`
- **THEN** it reads as 1234.56, because the last separator is the decimal
- **AND** a cell of `1,234.56` reads as 1234.56 for the same reason

#### Scenario: One separator, and the trailing group is not three digits

- **WHEN** a cell is `1,5` or `1.5`
- **THEN** it reads as 1.5

#### Scenario: A repeated separator groups

- **WHEN** a cell is `1,234,567` or `1.234.567`
- **THEN** it reads as 1234567, because a decimal mark cannot repeat

#### Scenario: A space or apostrophe groups

- **WHEN** a cell is `1 234,5`, `1 234,5` (no-break or thin space) or `1'234.5`
- **THEN** it reads as 1234.5

#### Scenario: A head that cannot be a group

- **WHEN** a cell is `0,500` or `1234,567`
- **THEN** it reads as 0.5 and 1234.567, because a group has a nonzero head of at
  most three digits

#### Scenario: Indian grouping

- **WHEN** a cell is `12,34,567`
- **THEN** it reads as 1234567

#### Scenario: No separator

- **WHEN** a cell is `1234`
- **THEN** it reads as 1234

#### Scenario: The same data on two machines

- **WHEN** the same table is read on machines with different host locales
- **THEN** every cell reads as the same number on both

### Requirement: Sign, Currency And Percent Are Part Of The Number

A cell with a sign, a currency mark, or a percent sign SHALL read as the number
it shows. A cell that classifies a column as a measure SHALL read as a number:
classifying and reading SHALL NOT be separate rules that can disagree.

#### Scenario: A negative amount

- **WHEN** a cell is `-$1,200`, `$-1,200` or `−$1,200` (U+2212)
- **THEN** it reads as -1200

#### Scenario: A signed amount

- **WHEN** a cell is `+$5`
- **THEN** it reads as 5

#### Scenario: A trailing currency mark

- **WHEN** a cell is `1.234,56 €`
- **THEN** it reads as 1234.56

#### Scenario: A percent is not rescaled

- **WHEN** a cell is `12%`
- **THEN** it reads as 12, the number the source wrote

#### Scenario: A negative amount is drawn

- **WHEN** a column holds `$1,200` and `-$300`
- **THEN** a chart of it draws two points, the second below the zero line
- **AND** it draws no gap

#### Scenario: A malformed sign or currency

- **WHEN** a cell is `-$-5`, `$5€`, `$$5` or `- 5`
- **THEN** it does not read as a number

### Requirement: An Ambiguous Cell Defers To Its Column, Then Its Table

An ambiguous cell SHALL be read using the convention held by the unambiguous
cells of its own column. A cell is ambiguous when it has one separator, exactly
three digits after it, and a head that can be a group. A column with no
unambiguous cell SHALL use the convention the rest of its table agrees on. A
column with no evidence in either place SHALL read comma as a group separator
when its ambiguous cells use a comma, and SHALL NOT be numeric when any of them
uses a dot.

#### Scenario: The column has an unambiguous cell

- **WHEN** a column holds `1,234` and `9,876.5`
- **THEN** the column's convention is comma-as-group
- **AND** `1,234` reads as 1234

#### Scenario: The column's convention is the other one

- **WHEN** a column holds `1.234` and `9.876,5`
- **THEN** the column's convention is dot-as-group
- **AND** `1.234` reads as 1234

#### Scenario: The column writes its decimal with commas

- **WHEN** a column holds `1,234` and `2,5`
- **THEN** `1,234` reads as 1.234

#### Scenario: The table decides

- **WHEN** a column holds only `1,234` and `5,678`, and another column of the same
  table holds `99.25`
- **THEN** the first column reads as 1234 and 5678

#### Scenario: Nothing decides, comma

- **WHEN** a column holds only `$1,200`, `$3,400` and `$12,500`, and no other
  column of its table shows a decimal
- **THEN** the column reads as 1200, 3400 and 12500
- **AND** it is offered as a chart measure

#### Scenario: Nothing decides, dot

- **WHEN** a column holds only `4.064`, `-810.553` and `55.719`, and nothing in
  its table shows a decimal
- **THEN** the column is not numeric, because it is as likely German thousands as
  English three-decimal values

### Requirement: An Unreadable Column Is Not A Measure

A column SHALL NOT be treated as numeric when any non-blank cell is not a number,
or when its unambiguous cells disagree. Such a column SHALL NOT offer numeric
sort and SHALL NOT be offered as a chart measure. Blank cells SHALL be ignored.

#### Scenario: The unambiguous cells disagree

- **WHEN** a column holds `1.234,5` and `9,876.5`, or `1,5` and `1.5`
- **THEN** the column is not numeric

#### Scenario: A non-measure is still refused

- **WHEN** a cell is `-`, `+`, `North`, `2026-03`, `2026-03-04`, `10:30`, `1.2.3`,
  `v2`, `0x10`, `1e3`, `3.5M` or `(1,234)`
- **THEN** it does not read as a number
- **AND** its column is not numeric

#### Scenario: Blanks are ignored

- **WHEN** a column holds `5`, an empty cell and `7`
- **THEN** the column is numeric and the empty cell reads as no value

### Requirement: A Chart Is Drawn From Read Numbers

A chart built from a table SHALL be given numbers, read once with the table's
resolved formats, and `null` where a cell has no value. The chart SHALL NOT parse
text.

#### Scenario: A German-formatted column

- **WHEN** a chart is built from a column of `1.234,56` and `2.000,00`
- **THEN** it draws the same domain as a chart of `1,234.56` and `2,000.00`

#### Scenario: A missing value

- **WHEN** a measure cell is empty
- **THEN** the chart draws a gap, not a zero

### Requirement: One Display Formatter

Every surface that shows a number to a person SHALL use one shared formatter,
which follows the host locale. Two parts of one view SHALL NOT format the same
value differently.

#### Scenario: A chart's axis and its tooltip

- **WHEN** a chart draws an axis tick and a tooltip for the same value
- **THEN** both show that value with the same separators

#### Scenario: The host locale groups differently

- **WHEN** the host locale writes 1234.5 as `1.234,5`
- **THEN** every surface showing that value shows `1.234,5`

#### Scenario: A file size

- **WHEN** a file size is shown
- **THEN** its number comes from the shared formatter
- **AND** its unit word is unchanged

### Requirement: Export Carries What Arrived

CSV export SHALL write each cell as it arrived. It SHALL NOT re-render a number
into the host locale.

#### Scenario: A German-formatted measure is exported

- **WHEN** a cell is `1.234,56` and the grid is saved as CSV
- **THEN** the file holds `1.234,56`
