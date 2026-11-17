## ADDED Requirements

### Requirement: Runtime-injected project authority

Provider-facing project tool schemas MUST NOT contain `project_id`, root paths, folder handles, or any other project-authority input. Rust SHALL inject the run-bound project context at dispatch.

#### Scenario: Model reads a file

- **WHEN** the model calls `project.read_file`
- **THEN** it SHALL provide only a project-relative path and optional complete line range
- **AND** Rust SHALL resolve that path against the run-bound trusted root

#### Scenario: Injected identifier appears in content

- **WHEN** a prompt, file, or tool result contains another local project identifier
- **THEN** that content SHALL have no effect on the dispatcher project context

### Requirement: Typed patch proposal operations

`project.propose_patch` SHALL advertise a discriminated union whose required fields match each operation kind. Model-authored content SHALL be UTF-8 text and all paths SHALL be project-relative.

#### Scenario: Create or modify

- **WHEN** the model proposes a create or modify operation
- **THEN** `kind`, `path`, and `content` SHALL be required

#### Scenario: Rename

- **WHEN** the model proposes a rename operation
- **THEN** `kind`, `from`, and `to` SHALL be required

#### Scenario: Delete

- **WHEN** the model proposes a delete operation
- **THEN** `kind` and `path` SHALL be required

### Requirement: Project-aware instructions

A project-bound run SHALL include a bounded system-prompt section that names the project for user/model orientation and explains relative-path use, without exposing its absolute path or local identifier.

#### Scenario: Project prompt assembled

- **WHEN** the run has a valid project binding
- **THEN** the system prompt SHALL say project tools already target that project
- **AND** SHALL NOT ask the model to invent or provide a project ID
