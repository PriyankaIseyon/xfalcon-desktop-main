## MODIFIED Requirements

### Requirement: A bounded read carries a cursor

A read SHALL report how to obtain the rest of a file it did not fully return, so a
file of any size is reachable in pages rather than refused.

A page MUST be sized from the run's tool-result budget and MUST NOT exceed it: the
page stops before a line that would overflow it, and a single line longer than the
per-line bound is cut and marked as cut.

#### Scenario: Truncated page

- **WHEN** a read returns fewer lines than the file contains
- **THEN** the result carries the next line number to request
- **AND** a read that returned the rest carries none

#### Scenario: File is longer than one read returns

- **WHEN** a file is longer than one read can return
- **THEN** the read returns its first page rather than refusing it, and names the call
  that returns the next

#### Scenario: Open-ended range

- **WHEN** the model gives a start line without an end line
- **THEN** one page beginning at that line is returned, bounded by the same per-call
  limits

#### Scenario: End without start

- **WHEN** the model gives an end line without a start line
- **THEN** the call is rejected as ambiguous

#### Scenario: Long lines do not overshoot the page

- **GIVEN** a file whose lines are each near the per-line bound
- **THEN** every page's content stays within the page budget

## ADDED Requirements

### Requirement: A directory listing pages

A directory listing SHALL return its entries in a stable name order, from an `offset`,
as many as fit the run's budget or the requested `limit`, together with the total entry
count and the next offset when entries remain.

#### Scenario: A large directory

- **GIVEN** a directory with 5,000 entries
- **WHEN** the model lists it
- **THEN** it receives the first entries that fit, `total` 5,000, and `next_offset`
- **AND** listing again from `next_offset` continues in the same order
