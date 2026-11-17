# local-project-execution Spec Delta

## ADDED Requirements

### Requirement: A write preserves the file's own permissions

A write that replaces an existing file SHALL preserve that file's permissions,
because the replacement is a different file on disk than the one it replaces.

#### Scenario: Executable file is edited

- **WHEN** an existing file's content is replaced on a platform with Unix mode
  bits
- **THEN** the replacement SHALL carry the target's previous mode
- **AND** an executable file SHALL remain executable

#### Scenario: Private file is edited

- **WHEN** a file readable only by its owner is edited
- **THEN** the replacement SHALL NOT be readable more widely than the original

#### Scenario: Read-only file is edited on Windows

- **WHEN** a read-only file's content is replaced on Windows
- **THEN** the write SHALL fail with the original file intact, because a
  read-only destination cannot be replaced
- **AND** no staged temporary file SHALL be left behind

#### Scenario: New file is created

- **WHEN** a file is created rather than replaced
- **THEN** it SHALL take the platform default, because there is no prior file
  to inherit from

#### Scenario: Permissions cannot be copied

- **WHEN** the permission copy itself fails
- **THEN** the operation SHALL fail closed with the original file unchanged,
  rather than landing new content under permissions nobody asked for

### Requirement: Search covers files the index has not reached

Search SHALL treat the local index as a fast path rather than the source of
truth, so a file that exists but is not yet indexed is still found.

#### Scenario: Search in a freshly trusted project

- **WHEN** a search runs before the index has covered the project
- **THEN** candidates SHALL include a bounded live walk of the project
- **AND** the result SHALL NOT report an empty match set merely because the
  index was cold

#### Scenario: Live walk hits its bound

- **WHEN** the live walk reaches its scan bound
- **THEN** the outcome SHALL report that it was bounded, so "nothing matched"
  is distinguishable from "not everything was looked at"

#### Scenario: Sensitive and non-text files in the live walk

- **WHEN** the live walk surfaces a classified-sensitive, binary, non-UTF-8, or
  oversized file
- **THEN** the same exclusions that apply to an indexed candidate SHALL apply

### Requirement: Directories can be moved and removed under containment

A directory SHALL be renamable and deletable through the same plan, approval,
audit, and containment path a file mutation uses.

#### Scenario: Directory is renamed

- **WHEN** a plan renames a directory
- **THEN** the move SHALL be performed against link-checked parent handles
- **AND** missing destination parents SHALL be created as they are for a file

#### Scenario: Empty directory is deleted

- **WHEN** a plan deletes a directory that is empty
- **THEN** the directory SHALL be removed

#### Scenario: Directory is not empty

- **WHEN** a plan deletes a directory that still holds an entry
- **THEN** the removal SHALL fail with the directory intact
- **AND** the failure SHALL say that its contents must be deleted first

#### Scenario: Directory gains a file after the plan was made

- **WHEN** a file appears in the directory between propose and execute
- **THEN** the removal SHALL fail rather than destroy it, because emptiness is
  enforced by the removal itself

#### Scenario: A directory operation and an operation inside it

- **WHEN** one plan both operates on a path and renames or deletes the
  directory containing that path
- **THEN** the plan SHALL be refused when it is proposed
- **AND** the refusal SHALL name the sequence that works: the contained
  operations first, then the directory

#### Scenario: Removing a populated directory

- **WHEN** a directory's files are deleted and the directory is then deleted in
  a following call
- **THEN** the directory SHALL be removed

#### Scenario: Directory delete needs a second confirmation

- **WHEN** a plan contains a directory delete
- **THEN** it SHALL require second confirmation, as every other delete does

#### Scenario: Link in place of a directory

- **WHEN** a directory operation's target is a symlink, junction, or reparse
  point
- **THEN** the operation SHALL fail closed rather than follow it
