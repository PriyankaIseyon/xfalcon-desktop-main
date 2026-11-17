# chat-thread-layout — Spec Delta

## MODIFIED Requirements

### Requirement: Turn Actions Appear On Hover In A Reserved Slot

A turn's actions MUST occupy a slot whose height is reserved whether or not the
actions are visible, so revealing them never moves the text around them.

The slot now carries more than a copy button: a user turn also offers edit, and
the last prose row of a response also offers regenerate. Every action in the slot
is reachable by keyboard and names itself to a screen reader, because a hover-only
affordance is no affordance at all for a reader who does not hover.

#### Scenario: The transcript does not move when actions appear

- **WHEN** the pointer enters a turn
- **THEN** no text in the transcript changes position

#### Scenario: Every action is reachable without a pointer

- **WHEN** the reader moves focus through a turn
- **THEN** each of its actions takes focus and is announced by its own name

### Requirement: A Row Whose Height Changes Is Remeasured

A row that changes height in place MUST tell the transcript to remeasure it.

The virtualizer's measurement cache is grow-only and keyed by row, so a row that
grows without reporting it leaves every row below it positioned against the old
height — which is the overlapping-text failure this surface is measured against.
Opening and closing the turn editor is such a change, and takes the same
remeasure path a disclosure does.

#### Scenario: Opening the editor does not overlap the rows below

- **WHEN** the reader opens the editor on a turn in a long transcript
- **THEN** the transcript remeasures that row
- **AND** no row overlaps another

#### Scenario: Closing the editor remeasures too

- **WHEN** the reader cancels an edit
- **THEN** the transcript remeasures that row
