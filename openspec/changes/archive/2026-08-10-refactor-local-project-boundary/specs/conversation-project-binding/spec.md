## ADDED Requirements

### Requirement: Conversation-scoped local binding

The desktop SHALL persist at most one local project binding per account and DA conversation ID. The binding SHALL remain local and SHALL survive process restart.

#### Scenario: Project conversation is created

- **WHEN** DA creates a conversation while a trusted project is active
- **THEN** Rust SHALL persist a local binding from that conversation ID to the active project

#### Scenario: Existing binding conflicts

- **WHEN** code attempts to bind a conversation already bound to a different project
- **THEN** the operation SHALL fail without changing the existing binding

#### Scenario: Conversation is reopened

- **WHEN** a bound conversation is opened
- **THEN** the desktop SHALL restore its project as the UI selection
- **AND** subsequent runs SHALL use the persisted binding rather than a prior sidebar selection

#### Scenario: Unbound conversation is opened

- **WHEN** a conversation has no local project binding
- **THEN** the desktop SHALL clear project selection for that conversation
- **AND** SHALL advertise no project tools

### Requirement: Binding lifecycle

Bindings SHALL be removed when their local conversation association or trusted project is deleted, without deleting the user's source files.

#### Scenario: Project trust is revoked

- **WHEN** a project is revoked
- **THEN** all local conversation bindings to that project SHALL be removed
- **AND** DA conversations SHALL remain intact

#### Scenario: Conversation is deleted

- **WHEN** a DA conversation is successfully deleted
- **THEN** its local project binding SHALL be removed
