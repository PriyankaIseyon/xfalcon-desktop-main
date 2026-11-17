## ADDED Requirements

### Requirement: The Grid Has A Budget

A large table SHALL mount and sort inside a stated budget, and the number of
mounted rows SHALL stay bounded regardless of table height.

#### Scenario: A large table mounts

- **WHEN** a table of at least 10,000 rows and 12 columns is mounted
- **THEN** the mount completes inside the stated budget
- **AND** the number of mounted row elements stays under the stated bound

#### Scenario: A large table is sorted

- **WHEN** a numeric column of that table is sorted
- **THEN** the sort completes inside the stated budget

#### Scenario: Scrolling does not accumulate rows

- **WHEN** the table is scrolled from the first row to the last
- **THEN** the number of mounted row elements is still under the bound

### Requirement: The Chart Has A Budget

A many-point series SHALL render inside a stated budget for every chart kind,
and the node count SHALL stay bounded.

#### Scenario: A many-point series renders

- **WHEN** a chart of at least 5,000 points is rendered
- **THEN** the render completes inside the stated budget for that chart kind
- **AND** the rendered node count stays under the stated bound

#### Scenario: Several series render

- **WHEN** a chart draws the maximum supported number of series at that point
  count
- **THEN** the render completes inside the stated budget

### Requirement: The Markdown Pipeline Has A Budget

A long answer with many fences SHALL render inside a stated budget, both settled
and while streaming.

#### Scenario: A long settled answer

- **WHEN** an answer of at least 40,000 characters holding at least 20 fences is
  rendered
- **THEN** the render completes inside the stated budget

#### Scenario: A streaming tail

- **WHEN** the same answer is rendered as a streaming tail, growing by batches
- **THEN** each batch completes inside the stated per-batch budget

### Requirement: A Cap Is Visible

Where a surface caps its input to hold a budget, the cap SHALL be stated in the
code and SHALL be visible to the person. A truncated result SHALL NOT be
presentable as a complete one.

#### Scenario: A chart is capped

- **WHEN** a series holds more points than the chart draws
- **THEN** the chart states how many points it drew and how many arrived

#### Scenario: The input is within the cap

- **WHEN** a series is inside the cap
- **THEN** no notice is shown

### Requirement: A Budget States Its Conditions

Every budget SHALL record the environment it was measured in and SHALL NOT be
presented as a product guarantee.

#### Scenario: A budget is read

- **WHEN** a budget's number is read in code or in its README
- **THEN** the environment it applies to is stated beside it

#### Scenario: A budget fails

- **WHEN** a budget is exceeded
- **THEN** the failure names the measured value, the budget, and the case
- **AND** it does not retry
