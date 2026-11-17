## ADDED Requirements

### Requirement: A run another client is driving is followed, not ignored

When a reconcile finds an in-flight run in the open conversation that this
device does not drive and holds no journal for, the app SHALL follow it by
reading the run's durable event log from the last sequence the projection
holds, and SHALL project those events into the transcript in the same shape a
locally executed run produces.

A followed run's text and thinking SHALL reach the reader as they are written,
not only when the run terminates.

#### Scenario: A turn started on the web continues on the desktop

- **GIVEN** a run started by another client that is still streaming
- **WHEN** the desktop opens that conversation
- **THEN** the answer already produced is shown
- **AND** further text arrives without reopening the conversation

#### Scenario: A followed transcript is indistinguishable from a local one

- **GIVEN** a followed run and a locally executed run with the same events
- **THEN** the projected transcript is the same for both

### Requirement: Following is bounded to what the reader is looking at

The app SHALL poll a followed run at 500 ms while its conversation is open and
the window is focused, and at 2 s while the window is unfocused. The app SHALL
NOT poll when no foreign run is in flight in the open conversation.

Following SHALL stop on the run reaching a terminal state, on the conversation
closing, on the account changing, and on sign-out.

#### Scenario: No foreign run, no polling

- **GIVEN** an open conversation whose runs are all terminal
- **THEN** no run-event request is made

#### Scenario: Following stops with the session

- **GIVEN** a followed run in progress
- **WHEN** the user signs out
- **THEN** no further run-event request is made

### Requirement: A followed run is live without being owned

A followed run SHALL present as live: the transcript streams and the composer
is held as it is for a local run. The app MUST NOT report it as owned by this
process — the local drive check MUST continue to answer false for it, so boot
recovery never reclaims it.

Stop SHALL remain available for a followed run, because cancelling one's own
run from either client is legitimate. Retry and revise SHALL remain unavailable
until the run reaches a terminal state.

#### Scenario: A followed run is never reclaimed as abandoned

- **GIVEN** a run being followed
- **WHEN** a workspace bootstrap runs
- **THEN** the run is not cancelled and not projected as interrupted

#### Scenario: Stop reaches the data plane

- **GIVEN** a followed run
- **WHEN** the reader presses Stop
- **THEN** the cancellation is sent for that run and the transcript settles

### Requirement: A terminal followed run settles on the durable message

When a followed run reaches a terminal state, the app SHALL reconcile the
conversation once, and the durable assistant message SHALL replace the followed
stream rather than appear beside it.

#### Scenario: The streamed text is not duplicated

- **GIVEN** a followed run whose streamed text is on screen
- **WHEN** the run completes and the conversation reconciles
- **THEN** exactly one assistant message for that run is present

### Requirement: A stalled foreign run is reported, never cancelled

A followed run that stops producing events without reaching a terminal state
SHALL be reported to the reader as stalled, with Stop offered. The app SHALL
NOT cancel it automatically.

#### Scenario: Silence is surfaced, not resolved

- **GIVEN** a followed run that produces no further events
- **THEN** the reader is told it has stalled
- **AND** no cancellation is sent unless the reader asks for one
