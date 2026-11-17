# project-data-handling Specification

## Purpose

What about a local project never leaves the device, and where the line between
local and uploaded is drawn.

## Scope

| File                              | What it owns here                                       |
| --------------------------------- | ------------------------------------------------------- |
| `src-tauri/src/projects/trust.rs` | canonical roots and filesystem identity, held locally   |
| `src-tauri/src/projects/index.rs` | the local encrypted index                               |
| `src-tauri/src/uploads/`          | the separate, explicit path by which a file is uploaded |

## Boundaries

- **Reading, writing, and searching under a root** belong to
  `local-project-execution`.
- **Which conversation a project is bound to** belongs to
  `conversation-project-binding`.
- **The encrypted store the index and snapshots live in** belongs to
  `encrypted-local-store`.
- **The upload pipeline itself** belongs to `attachment-uploads`.

## Requirements

### Requirement: Local project data boundary

Canonical roots, filesystem identities, local project identifiers, indexes, and source-file copies SHALL remain on the device. The desktop SHALL NOT create or depend on a DA local-project table or endpoint.

#### Scenario: Local project is trusted

- **WHEN** the user trusts a folder
- **THEN** no project row or folder path SHALL be sent to DA

#### Scenario: Tool context is persisted

- **WHEN** a bounded project excerpt is used in a conversation
- **THEN** it MAY be sent to the selected model and persisted through existing chat/tool records
- **AND** it SHALL remain subject to local sensitive-file policy and existing payload caps

### Requirement: Explicit upload separation

A source file SHALL become a DA-managed asset only through the explicit upload workflow, never by trusting or indexing a local project.

#### Scenario: Project indexing completes

- **WHEN** the local indexer processes project files
- **THEN** no file SHALL be created in DA attachment or artifact storage
