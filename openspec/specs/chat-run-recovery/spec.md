# chat-run-recovery Specification

## Purpose

What a reader is told when a run dies, and what the app is allowed to do about a
run whose fate it is not sure of. Two failures this capability exists to prevent: a
death reported twice in two voices, and an answer executed twice because the first
execution's outcome was lost in the ack window.

## Scope

| File                                        | What it owns here                                           |
| ------------------------------------------- | ----------------------------------------------------------- |
| `src-tauri/src/chat/service.rs:1765`        | `run_failure_report` — the failure vocabulary and its codes |
| `src-tauri/src/agent/recovery.rs:24`        | `decide` — the recovery scenario table                      |
| `src-tauri/src/agent/recovery.rs:34`        | `decide_from_run_state`                                     |
| `src-tauri/src/agent/recovery.rs:61`        | the startup notice strings                                  |
| `src-tauri/src/agent/recovery.rs:98`        | notice strength, so the strongest notice wins               |
| `src-tauri/src/agent/recovery.rs:133`       | `is_in_flight_run_state` — the journal's own predicate      |
| `src-tauri/src/chat/cold_start_recovery.rs` | boot classification against the data plane                  |
| `src-tauri/src/agent/journal_recovery.rs`   | the journal tail a boot reads                               |
| `src/features/chat/recoveryPolicy.ts`       | the frontend copy, kept aligned with the Rust policy        |
| `src/features/chat/RunNotice.tsx`           | the one card                                                |
| `src/features/chat/ChatSurface.tsx:128`     | the error-wins-over-notice rule                             |
| `src/features/chat/chatReduce.ts:172`       | which run states raise or clear a notice                    |

## Boundaries

- **Where the card renders in the scroll flow** belongs to `chat-thread-layout`;
  the rule that it renders beside the turn that died is stated here because it is a
  recovery requirement, and the layout capability owns the placement mechanism.
- **The run lifecycle, its journal, and cancellation** belong to `run-execution`.
  This capability reads run states and classifies them; it does not drive them.
- **The provider's own error taxonomy** belongs to `provider-failure-recovery`.
  This capability maps an engine error to reader-facing copy.
- **The composer returning after a failure** belongs to `chat-composer`, gated by
  `isRunPending` in `chat-run-activity`.
- **Retiring a dead run's live state** belongs to `chat-run-activity`.

## Requirements

### Requirement: A failed run names what ran out

The message shown when a run dies MUST name the cause and the move that would change
the outcome. A single catch-all sentence for every failure is not acceptable: it
names nothing the reader can act on, leaving retrying blind as the only option.

The system MUST distinguish at least: the tool-step budget exhausted, a tool that
exceeded its time bound, any other tool-call failure, the turn's own time budget, an
answer that outgrew what one turn can hold, a stream that could not be assembled,
and a provider that stopped answering.

Every one of these is retryable — none is a permanent state of the world, and the
run's own inputs are still on screen — so each MUST be reported as retryable.

The classification MUST be exhaustive over the engine's error type, so a new error
variant cannot fall through to a message that does not fit it.

A failure that names a provider problem MUST carry the provider-unavailable code
rather than the internal one, because the two lead the reader to different places.

#### Scenario: The tool-step budget runs out

- **GIVEN** a run that used every tool round without reaching an answer
- **THEN** the notice says how many steps were used and suggests asking for a
  narrower slice

#### Scenario: A tool times out

- **GIVEN** a round abandoned on its wall-clock bound
- **THEN** the notice says a tool took too long, and offers a retry

#### Scenario: An answer outgrows the turn

- **GIVEN** a run whose output passed the run-wide byte budget
- **THEN** the notice says the answer grew past what one turn holds

#### Scenario: The provider stops answering

- **GIVEN** an executor failure
- **THEN** the notice points at the provider key in Settings
- **AND** the event carries the provider-unavailable code

#### Scenario: A new engine error cannot borrow a wrong message

- **WHEN** a variant is added to the engine's error type
- **THEN** the classification fails to compile rather than falling through

### Requirement: One failure surface, beside the turn that died

A run's death MUST be reported once. The inference failure and the recovery notice
are two reports of one event; the surface SHALL render a single card carrying the
failure's own account when it has one and the recovery copy otherwise.

The card MUST render in the transcript's scroll flow, after the run's activity —
beside the turn that died — never pinned above the scroll region, where it describes
a failure the reader must scroll away from to see and stays on screen over every
turn that follows.

The card's tone MUST follow which field fed it: a run that died is an alert, a run
that can be resumed is a quiet status.

Dismissing the card MUST clear both underlying fields, so it cannot immediately
return carrying the other one. Retrying MUST clear both before the new run starts.

A retry affordance MUST be offered only when there is a specific run to replay.
Without one bound to the view, the button would do nothing.

#### Scenario: An interrupted run shows one card

- **GIVEN** a run that raised both an error and a recovery notice
- **THEN** exactly one card is shown, carrying the error's account

#### Scenario: A run that died without an account

- **GIVEN** a recovery notice and no error
- **THEN** the card shows the recovery copy as a quiet status, not an alert

#### Scenario: Dismissing clears the whole card

- **GIVEN** a card fed by both fields
- **WHEN** it is dismissed
- **THEN** it does not return

#### Scenario: No run, no retry button

- **GIVEN** a startup recovery notice with no run bound to the view
- **THEN** the card offers dismissal and no retry

### Requirement: A dead run's partial text is dropped, and its history is kept

A run that failed MUST NOT have its partial answer held across the reconcile gap: a
partial answer is not an answer, and the notice takes the surface instead.

The rounds the run completed before it died MUST be kept. Those rounds happened,
their durable record will carry them, and deleting the account of what the run did
before it died leaves the notice explaining a failure with no visible history behind
it.

The event that reports the failure MUST carry the reader-facing reason. Dropping it
leaves a sent message with no visible outcome at all.

#### Scenario: A failure clears the answer and keeps the trail

- **GIVEN** a run that streamed prose, called a tool, then failed
- **THEN** the partial answer is gone, the completed round's prose and tool rows are
  still on screen, and the card names the failure

### Requirement: A completed run is never re-executed

The system SHALL enforce at-most-once answer execution per user submission. Before
re-submitting a run's input — retry, recovery retry, or any resubmission path — the
run's authoritative server status MUST be consulted, and a run in a
terminal-completed state MUST NOT be re-executed.

The path instead reconciles the conversation so the durable answer is on screen, and
reports success without minting a new run.

A run the server reports as errored or cancelled MUST be re-submitted as a new run.

#### Scenario: Retry of a completed run reconciles instead of re-running

- **GIVEN** a run whose finalize landed on the server
- **AND** the local projection still shows it in flight, because the process died
  before the local acknowledgement
- **WHEN** the reader triggers retry for that run
- **THEN** no new run is created and no user message is appended
- **AND** the conversation is reconciled so the existing answer renders exactly once

#### Scenario: Retry of a genuinely failed run re-submits

- **GIVEN** a run the server reports as errored or cancelled
- **WHEN** the reader triggers retry
- **THEN** the input is re-submitted as a new run

### Requirement: Boot recovery classifies against the server before interrupting

Boot recovery SHALL patch a journal-tail run — one whose local transition log ends
non-terminal — to interrupted ONLY when the server also reports the run
non-terminal.

When the server reports it terminal, the projection MUST adopt the terminal state via
reconcile and no recovery banner may appear for that run.

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

#### Scenario: The strongest notice survives

- **GIVEN** two candidate recovery notices at boot
- **THEN** the stronger one is the one shown

### Requirement: User-message creation is idempotent per submission

A submission SHALL create its user message through the idempotent create with a
stable per-submission key, so a transport-level retry of the same submission cannot
write the user row twice.

#### Scenario: Lost response, client retry

- **GIVEN** a user-message create whose response is lost after the server committed
  the row
- **WHEN** the same submission is retried with the same key
- **THEN** exactly one user message row exists for that submission

### Requirement: The recovery copy is one policy, stated on both sides

The reader-facing recovery strings SHALL be defined once per side and kept aligned,
with each side naming the other. Two independently worded accounts of the same
recovery state give the reader two different stories about one event.

Which run states raise a notice, and which clear one, SHALL be decided in one place
per side: an interrupted or failed run raises the interrupted notice; a completed or
cancelled run clears it.

#### Scenario: The two sides agree

- **GIVEN** a run that reaches the interrupted state
- **THEN** the notice the frontend shows is the copy the Rust policy defines for
  that state

#### Scenario: A completed run clears the notice

- **GIVEN** a conversation showing a recovery notice
- **WHEN** a run completes
- **THEN** the notice is cleared

### Requirement: Stop reconciles completion without reviving a finished run

DA `error` runs MUST project as failed, and `streaming` runs MUST project as running.
A missing local cancellation handle MUST NOT by itself establish a cancellation
failure. The system MUST verify access to the durable run, preserve terminal
results, and refresh the conversation after settling a stale cancellation request.
A terminal-write conflict counts as successful Stop only after a read confirms
completion, failure, or cancellation. Network and authorization failures remain errors.

#### Scenario: A failed run is refreshed

- **GIVEN** DA reports the run status as `error`
- **WHEN** the conversation is reconciled
- **THEN** the run is failed and Stop is no longer offered

#### Scenario: Completion wins the cancellation race

- **GIVEN** completion has removed the local run handle
- **WHEN** the user stops that run
- **THEN** its authorized durable state is reconciled without overwriting the result
- **AND** a late cancellation failure does not replace a settled response with an error

## Provenance

Consolidated from the deltas of `graceful-run-failure` and
`fix-duplicate-run-answers`, reconciled against the shipped code.

Three requirements are new to the baseline, all describing shipped behaviour no delta
stated: the dead-run text rule (which `chat-motion-system` states from the motion
side and is restated here from the recovery side), the strongest-notice rule, and the
two-sided copy alignment.
