# local-project-execution Spec Delta

## ADDED Requirements

### Requirement: Missing parent directories are created under containment

A write SHALL create the missing intermediate directories inside the trusted
root when its target's parent does not exist yet — for a create operation, or
for a rename whose destination is in a new directory. Each directory SHALL be
created through its parent's own capability handle, and a symlink, junction, or
reparse point encountered on any existing component SHALL still fail closed.

#### Scenario: Create in a new directory

- **WHEN** the model creates `src/feature/new/mod.rs` and `src/feature/new`
  does not exist
- **THEN** the missing directories SHALL be created inside the trusted root
- **AND** the file SHALL be written at that path

#### Scenario: Link on the existing prefix

- **WHEN** an existing component of the path is a symlink, junction, or
  reparse point
- **THEN** the operation SHALL fail closed and create nothing

#### Scenario: Plan rolls back after directories were created

- **WHEN** an operation fails after this plan created directories
- **THEN** those directories SHALL be removed in reverse creation order while
  they are still empty
- **AND** a directory that is no longer empty SHALL be left alone

### Requirement: Edits fail closed on content that changed underneath them

The bytes an anchored edit was computed against SHALL be pinned by hash, and
the mutation plan's independently captured base hash MUST equal that hash.

#### Scenario: File changes between edit and plan

- **WHEN** the file's content changes after an anchored edit was computed and
  before the plan captured its base hash
- **THEN** the tool call SHALL fail without writing
- **AND** the failure SHALL tell the model to read the file again and retry
