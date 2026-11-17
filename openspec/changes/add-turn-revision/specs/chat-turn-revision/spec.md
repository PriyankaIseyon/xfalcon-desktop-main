# chat-turn-revision — Spec Delta

## ADDED Requirements

### Requirement: A Reader Can Revise Their Own Turn

A user turn in a durable conversation SHALL offer an edit action, and submitting
an edit MUST replace that turn and everything after it with the edited text.

The turn's own text opens the editor. Submitting an unchanged or empty text
cancels instead of submitting, because neither is a revision.

The action is absent — not disabled — on a turn with no durable id. An optimistic
row has not landed at the server, so the server cannot resolve a cutoff for it,
and an action that could only fail must not be offered.

#### Scenario: The edited turn replaces the old one

- **WHEN** the reader edits a user turn and submits
- **THEN** that turn and every message after it is gone from the conversation
- **AND** the edited text is the last user turn
- **AND** a run is in flight for it

#### Scenario: An unchanged edit is not a submission

- **WHEN** the reader opens the editor and submits without changing the text
- **THEN** the editor closes
- **AND** no message is removed and no run starts

#### Scenario: An optimistic turn offers no edit

- **WHEN** a user turn is still the optimistic row added at send time
- **THEN** no edit action is present on it

### Requirement: A Reader Can Ask For Another Answer

An assistant response SHALL offer a regenerate action, and using it MUST remove
everything after the user turn that prompted it and run that turn again.

The user turn itself stays, and no second copy of it is written.

Exactly one regenerate action is offered per response, on its last prose row. A
response can hold several prose rows — a round's narration, then the answer — and
regenerating from any of them means the same thing.

#### Scenario: The response is replaced and the question is not

- **WHEN** the reader regenerates a response
- **THEN** every message after the user turn above it is gone
- **AND** that user turn is still present exactly once
- **AND** a run is in flight for it

#### Scenario: One action per response

- **WHEN** a response consists of narration, a tool round, and an answer
- **THEN** exactly one regenerate action is present for that response

### Requirement: A Revision Is Instant Before It Is Durable

A revision SHALL redraw the transcript on the frame of the click: the removed
turns leave and the thinking indicator appears BEFORE any command is issued.

The command that follows is ONE call. Cutting the conversation and starting the
run in two calls leaves a window in which the reader's turn is gone and nothing
is running, and a failure in that window loses the turn.

#### Scenario: The transcript changes before the backend answers

- **WHEN** the reader submits an edit
- **THEN** the edited turn and everything after it is off screen, and a thinking
  indicator is on screen, before the command resolves

#### Scenario: A failed revision restores the conversation

- **WHEN** the command reports a failure, or the runtime does not answer
- **THEN** every removed turn is back, in its original order
- **AND** the reason is reported to the reader
- **AND** no thinking indicator remains

### Requirement: The Cutoff Is The Server's To Decide

The revision commands SHALL take a message id and resolve the cutoff from the
conversation the server reconciled, never from an index or boundary supplied by
the caller.

An id that names no message, names a turn of the wrong role, or has no user turn
above it MUST be refused with nothing removed.

Message ids cross a trust boundary on their way in. A boundary computed in the
renderer can be stale, and the damage a stale boundary does is to the reader's own
transcript.

#### Scenario: An unknown id changes nothing

- **WHEN** a revision names a message id that is not in the conversation
- **THEN** the command fails
- **AND** no message is removed and no run starts

#### Scenario: An edit must name a user turn

- **WHEN** an edit names an assistant turn
- **THEN** the command fails with nothing removed

#### Scenario: A regenerate needs a question above it

- **WHEN** a regenerate names an assistant turn with no user turn before it
- **THEN** the command fails with nothing removed

### Requirement: A Revision Resolves Before It Destroys

A revision SHALL resolve everything it can fail on cheaply before it cuts: the
session, the model list, the provider credential, and access to the conversation
are all settled while the conversation is still whole.

A revision that cannot produce an answer must not first remove the answer that is
already there.

#### Scenario: No usable key leaves the conversation whole

- **WHEN** a revision is submitted on a device with no usable provider credential
- **THEN** the command fails with the credential's own message
- **AND** the conversation still holds every message it held before

### Requirement: A Revision Stops The Run It Supersedes

A revision SHALL cancel every run of that conversation this process owns before
it cuts.

A run left streaming finalizes into a conversation whose tail has been removed,
which writes an answer to a question that is no longer there.

#### Scenario: An in-flight run is cancelled by a revision

- **WHEN** the reader revises a turn while a run is streaming
- **THEN** that run is cancelled
- **AND** the revision's own run is the only one in flight afterwards

#### Scenario: A removed run leaves no journal behind

- **WHEN** a revision removes runs from the conversation
- **THEN** the local event journal for each removed run is purged
