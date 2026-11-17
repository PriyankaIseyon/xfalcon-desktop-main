# conversation-project-binding Specification

## Purpose

Which local project a conversation runs against, how that survives a restart, and
what happens when the folder behind it does not.

## Scope

| File                                | What it owns here                         |
| ----------------------------------- | ----------------------------------------- |
| `src-tauri/src/projects/mod.rs`     | the per-account, per-conversation binding |
| `src-tauri/src/chat/service.rs:471` | restoring or clearing the binding on open |

## Boundaries

- **What an open does to the binding** is stated by `chat-conversation-open`.
- **Resolving a path under the bound root** belongs to `local-project-execution`.
- **What stays on the device** belongs to `project-data-handling`.
- **The sidebar's project tree** belongs to `sidebar-hierarchy`.

## Requirements

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
