# dynamic-canvas-artifacts — Spec Delta

## ADDED Requirements

### Requirement: Dedicated sidecar canvas surface for interactive artifacts

The system MUST provide an expandable sidecar Canvas surface that opens alongside the active conversation when an interactive artifact (table, chart, or sandboxed document) is selected.

The Canvas MUST support multi-tab navigation when multiple artifacts are produced within a single conversation, with an indicator of the active artifact.

#### Scenario: Opening a generated data artifact

- **GIVEN** an assistant message containing a structured dataset or chart specification
- **WHEN** the user clicks "Open in Canvas" or selects the artifact chip
- **THEN** the Canvas sidecar opens without replacing the chat timeline, displaying the interactive artifact in full detail

#### Scenario: Closing or collapsing the canvas

- **GIVEN** an open Canvas sidecar
- **WHEN** the user clicks the collapse button or presses the Canvas toggle shortcut
- **THEN** the Canvas smoothly collapses, returning full width to the chat timeline while retaining artifact state

### Requirement: Interactive tabular data grid with sorting and filtering

The system MUST render tabular datasets inside the Canvas with interactive client-side column sorting, multi-column search filtering, and clipboard export.

The grid MUST virtualize rendering for datasets containing up to 10,000 rows without UI stutter or memory leaks.

#### Scenario: Sorting dataset columns

- **GIVEN** a tabular dataset loaded in the Canvas
- **WHEN** the user clicks a column header
- **THEN** the rows are sorted ascending by that column, and clicking again reverses the sort to descending

#### Scenario: Exporting table data to CSV

- **GIVEN** a table view in the Canvas
- **WHEN** the user clicks the "Export CSV" action
- **THEN** a formatted CSV representation is saved or copied to the clipboard

### Requirement: Sandboxed chart rendering with theme integration

The system MUST render structured chart specifications (bar, line, pie, scatter) using a vector chart renderer inside the Canvas.

The charts MUST dynamically adapt their background, grid lines, and label colors according to the active app theme (light or dark mode).

#### Scenario: Displaying a revenue trend chart

- **GIVEN** a chart artifact spec received from an assistant turn or MCP tool
- **WHEN** the chart is rendered in the Canvas
- **THEN** it displays tooltips on hover, responds to window resizing, and uses the active palette theme
