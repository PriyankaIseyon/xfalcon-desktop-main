## ADDED Requirements

### Requirement: Every active run has a strict wall-clock deadline

An actively executing run MUST fail no later than 30 minutes after execution
starts, including while a provider stream, MCP call, tool round, or persistence
operation is waiting. A boundary check between rounds alone is insufficient.
The timeout MUST cancel outstanding work, finalize the durable run as failed,
release local ownership, and offer retry with the existing time-budget message.

#### Scenario: A blocked operation crosses the deadline

- **GIVEN** a run blocked inside one provider or tool operation
- **WHEN** 30 minutes elapse
- **THEN** the operation is cancelled and the run becomes failed
- **AND** Stop is no longer shown and retry is available

#### Scenario: Work completes before the deadline

- **WHEN** a run reaches a terminal result before 30 minutes
- **THEN** the deadline does not change that result

#### Scenario: MCP authorization expires mid-run

- **GIVEN** an MCP server refuses a run's expired or revoked authorization
- **WHEN** the refusal invalidates that server session
- **THEN** the run becomes failed before another model turn starts
- **AND** the user can reconnect the MCP server and retry

## MODIFIED Requirements

### Requirement: Stop reconciles completion without reviving a finished run

DA `error` runs MUST project as failed, and `streaming` runs MUST project as
running. A missing local cancellation handle MUST NOT by itself establish a
cancellation failure. The system MUST verify access to the durable run,
preserve terminal results, and refresh the conversation after settling a stale
cancellation request. A terminal-write conflict counts as successful Stop only
after a read confirms completion, failure, or cancellation. Network and
authorization failures remain errors.

An authoritative conversation snapshot with no active run MUST retire the
renderer's previous running or cancelled run and every live-only surface for
it. The renderer MUST NOT preserve a stale run ID merely because the same
conversation is still open. A failed/interrupted run MAY retain its id only as
the target of the existing retry action and MUST NOT be treated as active.

#### Scenario: A failed run is refreshed

- **GIVEN** DA reports the run status as `error`
- **WHEN** the conversation is reconciled
- **THEN** the run is failed and Stop is no longer offered

#### Scenario: Completion wins the cancellation race

- **GIVEN** completion has removed the local run handle
- **WHEN** the user stops that run
- **THEN** its authorized durable state is reconciled without overwriting the
  result
- **AND** a late cancellation failure does not replace a settled response with
  an error

#### Scenario: Stop receives a terminal snapshot

- **GIVEN** the open conversation still holds a stale running ID
- **WHEN** Stop reconciles a snapshot whose `active_run_id` is null
- **THEN** the stale ID and Stop control are removed
- **AND** the durable transcript replaces live-only run state
