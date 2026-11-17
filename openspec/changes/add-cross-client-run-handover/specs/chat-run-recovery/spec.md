## MODIFIED Requirements

### Requirement: Boot recovery classifies against the server before interrupting

Boot recovery SHALL patch a journal-tail run — one whose local transition log ends
non-terminal — to interrupted ONLY when the server also reports the run
non-terminal.

When the server reports it terminal, the projection MUST adopt the terminal state via
reconcile and no recovery banner may appear for that run.

This device is NOT the only client that executes runs against a conversation.
An in-flight run for which this device holds **no** local transition log was
started by another client and may be streaming right now. Such a run SHALL NOT
be cancelled on the data plane, SHALL NOT be projected as cancelled, and SHALL
NOT raise a recovery notice. Boot recovery MAY finalize only a run this device
can prove it once drove.

The journal's own in-flight predicate is deliberately broader than the view's: it
counts an interrupted run as in flight so the run stays resumable, while the view
calls it settled and returns the composer. Neither may be used in the other's place.

When more than one recovery notice is candidate at boot, the strongest MUST win, so
a weaker notice cannot mask a stronger one.

#### Scenario: Kill between server finalize and local ack

- **GIVEN** a run whose server status is completed but whose local journal tail is
  running
- **WHEN** the app boots
- **THEN** the run is reconciled to completed, no interrupted patch is written, and
  no recovery banner is shown for it

#### Scenario: A genuinely interrupted run is patched

- **GIVEN** a run the server also reports non-terminal
- **WHEN** the app boots
- **THEN** it is patched to interrupted and the recovery notice is shown

#### Scenario: Another client's live run survives a bootstrap

- **GIVEN** an in-flight run this device holds no journal entry for
- **WHEN** a workspace bootstrap reconciles its conversation
- **THEN** no run-status request is sent for it
- **AND** it is not projected as cancelled
- **AND** no recovery notice is raised for it

#### Scenario: An abandoned run is still finalized beside a foreign one

- **GIVEN** one in-flight run this device journaled and one it did not, in the
  same conversation
- **WHEN** a workspace bootstrap reconciles that conversation
- **THEN** exactly one cancellation is sent, for the journaled run

#### Scenario: The strongest notice survives

- **GIVEN** two candidate recovery notices at boot
- **THEN** the stronger one is the one shown
