## ADDED Requirements

### Requirement: A local file opens at any size up to 10 GiB

The system MUST open a project text file of up to 10 GiB in the file view. It MUST NOT
hold a file larger than 64 MiB in memory at once; such a file SHALL be shown a window
at a time, read-only, with its byte position. A file over 10 GiB SHALL show its size and
MUST NOT be read.

File bytes SHALL reach the webview as raw bytes over IPC, in ranges of at most 8 MiB,
never as a JSON string.

#### Scenario: A 64 MiB log

- **GIVEN** a 64 MiB text file
- **THEN** it opens whole and scrolls with only the visible lines in the DOM

#### Scenario: A 5 GiB export

- **GIVEN** a 5 GiB text file
- **THEN** it opens at its start in a windowed view, and the reader can move to any
  position without the whole file being loaded

#### Scenario: Over the cap

- **GIVEN** a 12 GiB file
- **THEN** the view states its size and that it is too large to open, and reads nothing

### Requirement: Large tables page from the backend

The system MUST render a CSV or TSV file of up to 16 MiB as the chat's table. A larger
one SHALL be read by the backend a window of rows at a time, through one delimited
parser that honours quoted delimiters and newlines, with a sparse row index so a later
window does not re-read the file from its start. The backend's memory MUST stay bounded
whatever the file holds: a row spanning more than 64 MiB of the file SHALL be refused
rather than held, and a window SHALL stop early past 16 MiB of cells, saying where the
next one starts.

#### Scenario: A 2 GiB CSV

- **GIVEN** a 2 GiB CSV
- **THEN** its header and first rows show promptly, and later rows are reached by
  paging

#### Scenario: An unclosed quote

- **GIVEN** a large CSV whose quote near the start is never closed
- **WHEN** its rows are paged
- **THEN** the read is refused as a row too long, and the file is not held in memory

### Requirement: Staleness is detected by stat

The system MUST detect that a file changed on disk by comparing its size and modified
time with those it was read at, and MUST NOT re-read the file to find out.

#### Scenario: An external edit

- **GIVEN** an open file changed by another program
- **WHEN** the window regains focus
- **THEN** the view offers a reload without replacing the content on screen

### Requirement: A person's own save is not bounded by what the model can read

The system MUST let a person save an edit to a file of any size the editor holds (up to
16 MiB). The whole-file-replace refusal for files over one read MUST apply only to a
model's proposal.

#### Scenario: Saving a 5 MiB file

- **GIVEN** a 5 MiB text file edited in the viewer
- **WHEN** it is saved
- **THEN** it is written, with no refusal about the file's size

#### Scenario: The model replacing a large file whole

- **WHEN** the model proposes a whole-file replace of a file larger than one read
- **THEN** the proposal is refused and names anchored edits
