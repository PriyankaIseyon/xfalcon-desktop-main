## ADDED Requirements

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
