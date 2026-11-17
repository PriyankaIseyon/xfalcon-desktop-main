## ADDED Requirements

### Requirement: Confirmed deleted-folder cleanup

The desktop SHALL delete conversations locally bound to a project whose folder is definitively deleted, then revoke that project's trust. Missing paths alone MUST NOT authorize this cleanup.

#### Scenario: Root deletion is confirmed

- **WHEN** a retained trusted directory handle proves deletion or a durable prior confirmation exists
- **THEN** the desktop SHALL delete only conversations bound to that project in the same account
- **AND** SHALL clear matching active conversation state and revoke trust after all conversation cleanup succeeds

#### Scenario: Cleanup partially fails

- **WHEN** a conversation deletion fails
- **THEN** unfinished bindings and project trust SHALL remain available for retry
- **AND** the UI SHALL expose the failure
- **AND** a retry of an already-deleted conversation SHALL accept the server's not-found result

#### Scenario: Account changes during cleanup

- **WHEN** the account changes while cleanup awaits a remote operation
- **THEN** subsequent remote deletion SHALL stop
- **AND** local effects SHALL remain scoped to the original account

#### Scenario: Root is moved or temporarily inaccessible

- **WHEN** a root is renamed, moved, disconnected, inaccessible, or missing without definitive deletion evidence
- **THEN** project trust and conversations SHALL remain intact

#### Scenario: Associated run is still stopping

- **WHEN** a conversation has an active local run
- **THEN** cleanup SHALL request cancellation and wait for the run to finish before deleting cloud or local conversation data
- **AND** a bounded wait failure SHALL retain the project and unfinished bindings for retry

#### Scenario: Windows deletion is pending

- **WHEN** Windows reports pending deletion on the retained root handle
- **THEN** the desktop SHALL release its observer handle without deleting chats or revoking trust
- **AND** SHALL NOT treat pending deletion or a zero link count as proof of permanent Windows deletion

#### Scenario: A delayed response arrives after deletion

- **WHEN** cloud or local snapshot work completes after a conversation was durably deleted
- **THEN** an account-scoped persistent deletion fence SHALL reject writes that recreate the conversation
- **AND** the frontend SHALL reject stale list, view, and rollback state for that conversation

#### Scenario: Later cleanup fails after one chat was deleted

- **WHEN** one conversation was deleted but a later binding, project, or conversation operation fails
- **THEN** its account-scoped removal event SHALL clear that chat immediately
- **AND** unfinished cleanup SHALL remain retryable

#### Scenario: A parked run exceeds the cleanup wait

- **WHEN** cleanup times out while a parked run is finalizing
- **THEN** finalization SHALL continue independently of that wait
- **AND** deletion SHALL remain blocked until teardown finishes

#### Scenario: A different conversation is selected during deletion or approval

- **WHEN** a background conversation is deleted or its parked run resumes
- **THEN** the operation SHALL use its owning account and conversation
- **AND** SHALL preserve an unrelated selected conversation
