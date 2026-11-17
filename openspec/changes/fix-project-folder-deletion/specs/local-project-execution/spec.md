## MODIFIED Requirements

### Requirement: Local execution authority

The desktop runtime SHALL be the sole authority for trusted local project roots,
filesystem identity, indexing, reads, searches, and writes. The data plane and the
model provider MUST NOT select or authenticate a local project.

#### Scenario: Bound run starts

- **WHEN** a run starts for a conversation bound to an available trusted project
- **THEN** the runtime revalidates the root and freezes that project into the run context
- **AND** every project tool call in the run uses that frozen context

#### Scenario: Root is unavailable

- **WHEN** the bound project's root is missing or its identity changed without definitive deletion evidence
- **THEN** the conversation remains readable
- **AND** project tools fail closed without substituting another active project

## ADDED Requirements

### Requirement: Project availability presentation

Project names, status, and actions SHALL remain within the project row without overlapping adjacent content at supported sidebar sizes.

#### Scenario: Availability error wraps

- **WHEN** an unavailable folder produces multi-line status text
- **THEN** the project row SHALL grow to contain the text below its title
- **AND** the heading and adjacent project rows SHALL remain unobscured

### Requirement: Project lifecycle revalidation

The desktop SHALL revalidate projects on startup, window focus, and visibility restoration. Each successful conversation deletion SHALL publish an account-scoped removal event that clears the matching list entry and conversation view, even if later project cleanup fails.

#### Scenario: Folder is deleted outside the app

- **WHEN** revalidation completes confirmed deletion cleanup
- **THEN** the project and deleted chats SHALL leave the UI without requiring an app restart
