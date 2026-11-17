# chat-run-recovery — Spec Delta

## ADDED Requirements

### Requirement: A completed run is never re-executed

The system SHALL enforce at-most-once answer execution per user
submission: before re-submitting a run's input (retry, recovery retry, or
any resubmission path), the run's authoritative DA status MUST be
consulted, and a run in a terminal-completed state MUST NOT be
re-executed. The path instead reconciles the conversation so the durable
answer is on screen, and reports success without minting a new run.

#### Scenario: Retry of a completed run reconciles instead of re-running

- **GIVEN** a run whose finalize landed on DA (status `completed`)
- **AND** the local projection still shows it in-flight (the process died
  before the local ack)
- **WHEN** the user triggers retry for that run
- **THEN** no new run is created and no user message is appended
- **AND** the conversation is reconciled so the existing answer renders
  exactly once

#### Scenario: Retry of a genuinely failed run re-submits

- **GIVEN** a run DA reports as `error` or `cancelled`
- **WHEN** the user triggers retry
- **THEN** the input is re-submitted as a new run (current behavior)

### Requirement: Boot recovery classifies against DA before interrupting

Boot recovery SHALL patch a journal-tail run (local transition log ends
non-terminal) to Interrupted only when DA also reports the run
non-terminal. When DA reports it terminal, the projection MUST adopt the
terminal state via reconcile and the recovery banner MUST NOT appear for
that run.

#### Scenario: Kill between DA finalize and local ack

- **GIVEN** a run whose DA status is `completed` but whose local journal
  tail is `Running` (killed in the ack window)
- **WHEN** the app boots
- **THEN** the run is reconciled to completed, no Interrupted patch is
  written, and no recovery banner is shown for it

### Requirement: User-message creation is idempotent per submission

`message_send` SHALL create the user message via the idempotent create
with a stable per-submission idempotency key, so a transport-level retry
of the same submission cannot write the user row twice.

#### Scenario: Lost response, client retry

- **GIVEN** a user-message create whose response is lost after the server
  committed the row
- **WHEN** the same submission is retried with the same idempotency key
- **THEN** exactly one user message row exists for that submission
