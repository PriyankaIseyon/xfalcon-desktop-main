## ADDED Requirements

### Requirement: Local execution authority

The desktop Rust runtime SHALL be the sole authority for trusted local project roots, filesystem identity, indexing, reads, searches, and writes. DA and the model provider MUST NOT select or authenticate a local project.

#### Scenario: Bound run starts

- **WHEN** a run starts for a conversation bound to an available trusted project
- **THEN** Rust SHALL revalidate the root and freeze that project into the run context
- **AND** every project tool call in the run SHALL use that frozen context

#### Scenario: Root is unavailable

- **WHEN** the bound project's root is missing or its identity changed
- **THEN** the conversation SHALL remain readable
- **AND** project tools SHALL fail closed without substituting another active project

### Requirement: Search index readiness

Before a run advertises project search tools, the desktop SHALL complete the existing bounded incremental index pass for the bound project off the async/UI core.

#### Scenario: First run after trust

- **WHEN** the first project-bound run is assembled
- **THEN** the local encrypted index SHALL be refreshed before `project.search_files` or `project.search_text` can be called

#### Scenario: Direct read before index

- **WHEN** a user browses or reads an exact relative path
- **THEN** that operation SHALL remain independent of search-index state
