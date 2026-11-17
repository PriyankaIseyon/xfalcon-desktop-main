# dynamic-canvas-artifacts Specification

## Purpose

The side panel a transcript artifact expands into, and what becomes possible there
that is not possible in a chat card: sorting and filtering ten thousand rows, drawing
the same table five ways, and reading a document at full width.

The panel shares the workspace with the chat rather than replacing it. That is the
whole reason it exists as a third track instead of a modal.

## Scope

| File                                     | What it owns here                                          |
| ---------------------------------------- | ---------------------------------------------------------- |
| `src/features/canvas/canvasStore.ts`     | the tab state, the artifact union, the tab cap             |
| `src/features/canvas/canvasStore.ts:103` | `MAX_CANVAS_TABS`                                          |
| `src/features/canvas/CanvasPanel.tsx`    | the tab strip, its keyboard model, the panel chrome        |
| `src/features/canvas/CanvasSlot.tsx`     | the workspace slot the panel occupies                      |
| `src/features/canvas/DataGrid.tsx`       | the virtualized grid, its sort, its filter, its exports    |
| `src/features/canvas/ChartCanvas.tsx`    | the hand-drawn chart, its five types, its palette          |
| `src/features/canvas/HtmlCanvas.tsx`     | the full-width document frame and its re-registration      |
| `src/features/canvas/tabularExport.ts`   | CSV serialization, formula neutralization, numeric reading |

## Boundaries

- **The offer that opens a tab** belongs to `chat-rich-content`, which also computes
  the measure classification this capability consumes rather than re-deriving.
- **The preview origin and its document policy** belong to `preview-protocol`. The
  panel loads a document the same way a transcript card does and defines no second
  contract.
- **The workspace's three-track layout and the shortcut that toggles the panel** belong
  to `app-shell-material`.
- **Writing a file to disk** belongs to `lean-report-export`, whose save command the
  grid and the document frame both call.
- **Locale-aware number parsing** is proposed by `add-locale-aware-formatting` and is
  not part of this baseline; the numeric reader here assumes a dot decimal.

## Requirements

### Requirement: A side panel holds artifacts beside the conversation, in tabs

The system MUST provide a side panel that opens alongside the active conversation when
an artifact is expanded, and MUST NOT replace or unmount the chat transcript when it
does.

The panel MUST support multiple tabs when a conversation produces several artifacts,
with the active one indicated.

Opening an artifact whose id is already open MUST replace that tab's content in place
and select it, rather than adding a second tab for the same artifact.

The number of tabs MUST be capped. Every table tab retains its full row set, so an
unbounded strip would let one long conversation pin an arbitrary amount of data in
memory. Opening past the cap drops the OLDEST tab that is not the one being opened.

Closing a tab MUST select a neighbour rather than nothing, and closing the last tab
MUST close the panel and leave full width to the chat, because a shell still laid out
for an empty panel squeezes the transcript against an empty column.

Changing conversation MUST drop every tab. An artifact belongs to the conversation that
produced it; leaving one open across a switch would show one chat's data on another
chat's screen, and after a sign-out it would show the previous account's.

#### Scenario: Opening an artifact does not replace the chat

- **GIVEN** an assistant message carrying a table
- **WHEN** the reader expands it into the panel
- **THEN** the panel opens beside the transcript and the transcript is still readable

#### Scenario: Re-opening the same artifact reuses its tab

- **GIVEN** an artifact already open in a tab
- **WHEN** it is opened again
- **THEN** that tab is selected and no second tab appears

#### Scenario: The strip is bounded

- **GIVEN** the maximum number of tabs open
- **WHEN** a further artifact is opened
- **THEN** the oldest tab is dropped and the new one is selected

#### Scenario: Closing the last tab closes the panel

- **GIVEN** one open tab
- **WHEN** it is closed
- **THEN** the panel closes, full width returns to the chat, and expanded mode is
  cleared

#### Scenario: Switching conversation clears the panel

- **GIVEN** an open panel with tabs
- **WHEN** another conversation is opened
- **THEN** the panel is closed and holds no tabs

### Requirement: The tab strip behaves as a tablist, not merely as buttons

The strip MUST be a real tablist: arrow keys and Home/End move the selection, a key
closes the focused tab, and only the selected tab is in the tab order.

A strip of eight focus stops between the transcript and the artifact is a thicket the
reader has to tab their way out of, which is why reachability alone is not enough.

Each tab MUST name its artifact, and its close control MUST name the artifact it
closes. The panel body MUST be a tabpanel named for the active artifact.

#### Scenario: Arrow keys move between tabs

- **GIVEN** focus on a tab in a strip of several
- **WHEN** the reader presses the arrow keys
- **THEN** the selection moves and the newly selected tab takes focus

#### Scenario: One focus stop for the whole strip

- **GIVEN** a strip of eight tabs
- **WHEN** the reader tabs through the panel
- **THEN** the strip costs one stop, not eight

### Requirement: The panel resizes while retaining chat and navigation

The panel SHALL provide a pointer- and keyboard-operable vertical divider. Its
width SHALL range from 20% to 70% of the app width. The expanded panel SHALL leave
the remaining 30% for chat and navigation, less the two 2px pane gutters;
an open desktop sidebar SHALL shrink to 15% at the maximum.
Narrow windows SHALL retain their existing overlay navigation behavior.

The expand control SHALL select 70%; restoring or pressing Escape SHALL return to
the previous split. The divider SHALL support Left/Right arrows, Home/End, and a
reset to its default with Enter or double-click. Resizing SHALL not remount the
artifact, intercept iframe input after a drag, or leave a selection lock behind.
The chosen split SHALL survive hiding and reopening the panel in the same shell.

It MUST NOT offer a device-viewport picker.

#### Scenario: Maximum split

- **WHEN** the reader drags past the maximum or activates Expand panel
- **THEN** the panel stops at 70% and chat remains visible

#### Scenario: Keyboard adjustment

- **WHEN** the focused divider receives Left/Right or Home/End
- **THEN** it adjusts within the same bounds and announces its current percentage

#### Scenario: Restore the split

- **GIVEN** the expanded panel and no dialog open
- **WHEN** the reader presses Escape
- **THEN** the previous split is restored

### Requirement: A table renders as a virtualized grid with sorting and filtering

A table artifact MUST render as a grid with client-side sorting, a free-text filter
across all columns, and a bounded number of mounted rows regardless of the row count.

Sorting MUST cycle through ascending, descending, and unsorted, so a reader can return
to the artifact's own order.

Blank values MUST sink in both sort directions. A column sorted descending should not
begin with the rows that have no value in it.

Numeric sorting MUST use the measure classification the producing card carried, not a
guess from the cell's runtime type. A chat table's cells are all strings, so a grid
left to infer would sort a column of numbers as text and place 100 before 50. An absent
classification means nothing is known, and everything sorts as text.

The filter MUST match on any cell of a row, case-insensitively, and the visible row and
column counts MUST be announced politely as they change.

#### Scenario: Sorting cycles and returns

- **GIVEN** a table in the panel
- **WHEN** the reader activates a column header three times
- **THEN** the rows sort ascending, then descending, then return to the artifact's own
  order

#### Scenario: A numeric column sorts numerically

- **GIVEN** a column the producing card classified as measures, holding string cells
- **WHEN** it is sorted ascending
- **THEN** 50 precedes 100

#### Scenario: Blanks sink both ways

- **GIVEN** a column with some empty cells
- **WHEN** it is sorted in either direction
- **THEN** the empty cells are last

#### Scenario: Ten thousand rows stay bounded

- **GIVEN** a table of 10,000 rows
- **WHEN** it renders and is scrolled
- **THEN** the mounted row count stays bounded

### Requirement: A table leaves the panel as CSV, safely, through a dialog

A grid MUST offer its data as CSV to the clipboard and as a file.

The file MUST be written through the native save dialog, never through an anchor with a
download attribute. A blob anchor writes with no destination step, which is the download
affordance this build deliberately refuses; the person names the file and the backend
never takes a path.

A dismissed dialog is an answer, not a failure, and MUST be reported as neither.

Every CSV field MUST be quoted, so a comma, a quote, or a newline inside a value cannot
end it, and a literal quote MUST be doubled. A null or absent cell MUST be a quoted
empty field, so a reader scanning the file finds no unquoted value.

A cell whose text begins with a character a spreadsheet reads as a formula MUST be
neutralized with a leading apostrophe — the documented way to force a cell to text in
every major spreadsheet, and one that survives a round trip. Quoting does NOT stop
evaluation. The leading character MUST NOT be stripped: deleting content to make it
safe would silently change the data, and a negative number is the common case.

Line endings MUST be CRLF, which is what the format specifies and what the likely
destination reads without complaint.

The exported rows MUST be the rows the grid is showing — filtered and sorted as the
reader left them — not the artifact's raw order.

#### Scenario: A formula-shaped cell exports as text

- **GIVEN** a cell whose value begins with an equals sign
- **WHEN** the table is exported
- **THEN** the field carries a leading apostrophe and the original characters

#### Scenario: A negative number keeps its sign

- **GIVEN** a cell holding a negative number
- **THEN** the exported field carries the apostrophe and the sign, with no character
  removed

#### Scenario: Saving goes through the dialog

- **WHEN** the reader saves the table
- **THEN** the native save dialog names the file, and no anchor download occurs

#### Scenario: A dismissed dialog says nothing

- **WHEN** the reader cancels the save dialog
- **THEN** no success and no failure is reported

#### Scenario: The export matches what is on screen

- **GIVEN** a filtered and sorted grid
- **WHEN** it is exported
- **THEN** the CSV holds the filtered rows in the sorted order

### Requirement: A chart is drawn in the app, and its colours survive leaving it

A chart artifact MUST render as a chart the reader can switch between bar, line, area,
scatter, and pie, over one category axis and one or more measure series.

The chart MUST be drawn by this app rather than a charting library. The artifact needs a
handful of types over a handful of series, the bundle is already large, and the export
has to be a standalone file — which rules out any renderer whose output depends on its
own runtime stylesheet.

Series, axis, and grid colours MUST be literal values, NOT theme tokens. The same values
have to work in the panel and inside an exported vector file opened in a viewer that has
none of this app's stylesheets, where a token-referencing fill renders as black — losing
exactly the information the colours carry.

A point MUST respond to hover by distinguishing itself from the other series, so a
reader can tell which series a value belongs to.

The drawing MUST scale with the panel rather than being fixed to a pixel size.

An axis label longer than the readable bound MUST be elided rather than overlapping its
neighbours into an unreadable band.

A series key MUST be synthetic, with the header carried separately as a display label.
Header text cannot be a key: two columns may share a name, which would collapse two
series into one, and a column named like the axis key would collide with it.

#### Scenario: Five chart types over one artifact

- **GIVEN** a chart artifact in the panel
- **WHEN** the reader switches type
- **THEN** the same data draws as a bar, line, area, scatter, or pie chart

#### Scenario: An exported chart keeps its colours

- **GIVEN** a chart saved as a vector file
- **WHEN** it is opened in a viewer with none of this app's styles
- **THEN** its series colours are the colours the panel showed

#### Scenario: Hover distinguishes a series

- **WHEN** the reader hovers a point
- **THEN** that point's series is emphasized and the others recede

#### Scenario: Two columns with the same name stay two series

- **GIVEN** a table with two measure columns sharing a header
- **THEN** the chart draws two series

### Requirement: A document reads at full width and survives its preview being evicted

A document artifact MUST render in the panel through the same preview origin and the
same sandbox contract a transcript card uses, and MUST NOT introduce a second preview
path.

The panel MUST hold the document's own bytes and re-register them on mount rather than
trusting a stored id. A preview id is not durable — the store is bounded with
least-recently-served eviction and nothing survives a restart — and the frame cannot
report the loss, because the scheme answers an unknown id with a not-found response
whose body the frame LOADS. The failure would present as a silently blank panel.

Registration MUST be retried before it is reported as failed, so one transient refusal
does not blank a document the reader just opened.

The panel MUST carry the document's kind, because the same card serves both an HTML
document and a standalone SVG and the scheme answers each under a different content type
and policy. Re-registering an SVG as HTML would hand it the wrong one.

The document's source MUST be copyable and saveable, through the same dialog path the
grid uses, under the format its kind implies.

#### Scenario: A document opens at full width

- **GIVEN** an HTML document expanded into the panel
- **THEN** it renders in a frame on the preview origin with the same sandbox the
  transcript card uses

#### Scenario: An evicted document still renders

- **GIVEN** a document whose preview id was evicted while another was open
- **WHEN** its tab is shown again
- **THEN** the content is registered again and the document renders

#### Scenario: An SVG keeps its own kind

- **GIVEN** an SVG expanded into the panel
- **THEN** it is registered and served as an image document

### Requirement: An artifact's identity separates two artifacts that look alike

An artifact's id MUST distinguish two artifacts a conversation can plausibly produce
together, and MUST stay stable across the re-renders of one.

A title and a column set are not enough: an answer routinely renders the same query for
two periods, producing two tables with identical titles and identical headers. Sharing
an id would make the second silently replace the first.

A document's id MUST be derived from the content-addressed preview id, so re-opening the
same document reuses its tab and two documents differing by a byte get two.

#### Scenario: Two tables of the same query, two tabs

- **GIVEN** an answer rendering the same query for two periods
- **WHEN** both tables are expanded
- **THEN** two tabs open

#### Scenario: The same document reuses its tab

- **GIVEN** a document already open
- **WHEN** the same document is expanded again
- **THEN** its existing tab is selected

## Provenance

Consolidated from the delta of `add-dynamic-canvas-artifacts`, reconciled against the
shipped code. That delta was written at a coarser grain than the implementation, and
three of its statements do not describe what shipped:

- "charts MUST dynamically adapt their background, grid lines, and label colors
  according to the active app theme" is false, and deliberately so. The palette is
  literal hex precisely so an exported vector file keeps its colours outside the app.
- "using a vector chart renderer" implies a library. The chart is hand-drawn SVG, for
  the reasons stated above.
- "bar, line, pie, scatter" undercounts: area is a fifth type.

Most of this spec is new to the baseline. The tab cap, the tablist keyboard model, the
conversation-switch clear, the neighbour-selection-on-close rule, the CSV formula
neutralization, the carried measure classification, the artifact-identity rule, and the
document re-registration all shipped without a delta stating them.
