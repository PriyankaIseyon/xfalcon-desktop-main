## ADDED Requirements

### Requirement: A revision's changes can be reviewed before it is restored

The system MUST let a person see, on request, a line diff from a stored revision
to the current file, side by side.

The diff MUST be loaded only when the person asks for it, and MUST apply the file
view's sensitivity gate before either side is read.

The system MUST refuse, rather than truncate, a diff when either side is larger
than 1 MiB, when the edit changes more than 2,000 lines, or when the rendered diff
is larger than `MAX_DIFF_BYTES`, and MUST say which limit applied.

#### Scenario: Show a revision's changes

- **GIVEN** a file with a stored revision
- **WHEN** the person selects Show changes on that revision
- **THEN** a side-by-side diff shows the revision on the left and the current file on the right

#### Scenario: Show one change alone

- **GIVEN** a revision with a newer revision after it
- **WHEN** the person chooses This change only
- **THEN** the diff compares the revision with the next newer revision instead of the current file

#### Scenario: A diff is too large

- **GIVEN** a revision or current file over 1 MiB, or an edit over 2,000 lines
- **WHEN** the person selects Show changes
- **THEN** an alert says the diff is too large to show
- **AND** restoring the revision stays available

#### Scenario: A sensitive path is withheld

- **GIVEN** a revision of a path classified as sensitive
- **WHEN** the person selects Show changes
- **THEN** neither side is read and the panel says the changes are withheld
