## ADDED Requirements

### Requirement: Known live runs survive conversation switches

The frontend SHALL retain a bounded in-memory live snapshot for known adopted runs.
Background events MUST update only their retained run and MUST NOT change the visible
conversation. Returning to the conversation SHALL restore its live snapshot and
sequence watermarks. Account reset and conversation deletion MUST remove that state.

#### Scenario: A live answer continues while another conversation is open

- **WHEN** a known run receives answer, reasoning, and tool events after the user switches away
- **THEN** the visible conversation remains unchanged
- **AND** returning restores the retained state including those events

#### Scenario: A background run completes

- **WHEN** a retained background run completes and its durable snapshot arrives
- **THEN** returning shows its durable answer once
- **AND** late events cannot resurrect the retired synthetic answer

#### Scenario: Opening a live destination fails after it receives updates

- **WHEN** a cached destination receives answer, reasoning, and approval events while its authoritative open is pending
- **AND** the open fails and restores the source conversation
- **THEN** the destination retains those updates and its invocation ownership for the next open

#### Scenario: A failed source receives repeated snapshots before rollback

- **WHEN** a failed or interrupted source receives repeated authoritative snapshots while another conversation is opening
- **AND** the destination open fails
- **THEN** rollback preserves the source's latest transcript, retry target, recovery notice, and error
- **AND** a retired reasoning cache cannot overwrite those fields

#### Scenario: The active account is cleared

- **WHEN** the chat store resets for an account change
- **THEN** no previously retained run can restore text or activity into the new account
