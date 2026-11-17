# chat-run-activity — Spec Delta

> Base requirements were introduced by `refine-run-feedback-ux` (pending
> archive). That change MUST be archived before this one so the MODIFIED
> block below applies onto the synced `chat-run-activity` spec.

## MODIFIED Requirements

### Requirement: The card auto-expands during the run and auto-collapses after

The activity card SHALL auto-expand when a run starts. It SHALL
auto-collapse to its one-line header ~400ms after the first answer token
of the run's response arrives (attention moves to the answer); for runs
that end without any answer token (tool-only runs, failures), it SHALL
collapse when the run reaches a terminal state. Auto-collapse MUST be
suppressed while the pointer is over the card, and MUST be disabled for
the rest of the run once the user toggles the card manually (a manual
choice wins until the next run starts). While collapsed during an active
run, the header SHALL keep a live working indicator so activity is never
invisible.

#### Scenario: Auto-collapse on first answer token

- **GIVEN** the activity card is auto-expanded during a streaming run
- **WHEN** the first answer token renders
- **THEN** the card collapses to its header shortly after, while the
  header keeps its working indicator until the run ends
- **AND** clicking the header re-expands it

#### Scenario: Hover holds the card open

- **GIVEN** the pointer is over the expanded activity card
- **WHEN** the first answer token renders
- **THEN** the card stays expanded until the pointer leaves

#### Scenario: Manual expand after completion

- **GIVEN** a finished conversation whose run invoked tools
- **WHEN** the user clicks the collapsed activity header
- **THEN** the full invocation list is shown until the user collapses it or
  a new run starts

#### Scenario: Tool-only run falls back to terminal collapse

- **GIVEN** a run that invokes tools but streams no answer text
- **WHEN** the run reaches a terminal state
- **THEN** the card collapses to its header

## ADDED Requirements

### Requirement: The card header summarizes run state

The activity card header SHALL present a live summary instead of a static
label: a working state with spinner while the run is active, "N tool
call(s) · <duration>" when tools completed, and a failure state when any
invocation failed. The header label MUST update from typed events only
(no polling timers).

#### Scenario: Header after a successful run

- **GIVEN** a run that invoked two tools and completed
- **WHEN** the collapsed card renders
- **THEN** the header reads "2 tool calls" with the run's elapsed duration

#### Scenario: Header during the run

- **WHEN** a run is executing a tool
- **THEN** the header shows a spinner and a working label

### Requirement: Row status changes crossfade and completed rows show duration

The activity rows SHALL render status transitions as a glyph crossfade in
a fixed-size slot (no layout shift), using the shared status glyph table
(pending, running spinner, success check with suppressed label, error,
cancelled). When an invocation completes with a known elapsed time, a
duration badge SHALL appear once with a one-shot pop animation and MUST
NOT re-animate on later renders.

#### Scenario: Running to completed transition

- **GIVEN** a row in running state (spinner)
- **WHEN** its completion event arrives
- **THEN** the spinner crossfades to a success check and a duration badge
  pops in once

#### Scenario: No replay on re-render

- **GIVEN** a completed row with its duration badge
- **WHEN** the card re-renders (expand/collapse, unrelated event)
- **THEN** the badge does not re-animate
