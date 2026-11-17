# project-tool-contract Spec Delta

## ADDED Requirements

### Requirement: Content search accepts a pattern by opt-in

`project.search_text` SHALL accept a regular expression when the caller asks
for one, and SHALL treat a query as literal text otherwise.

#### Scenario: Literal is the default

- **WHEN** no search mode is given
- **THEN** the query SHALL match as literal text
- **AND** regex metacharacters in it SHALL carry no special meaning

#### Scenario: Regex is requested

- **WHEN** the caller asks for regex mode
- **THEN** the query SHALL be compiled as a regular expression under a
  compiled-size bound

#### Scenario: Pattern does not compile

- **WHEN** a regex fails to compile
- **THEN** the call SHALL fail with a message naming what is wrong with the
  pattern, so the caller can correct it

### Requirement: Path search accepts a glob by opt-in

`project.search_files` SHALL accept a glob pattern when the caller asks for
one, and SHALL rank by substring match otherwise.

#### Scenario: Glob is requested

- **WHEN** the caller asks for glob mode with `src/**/*.rs`
- **THEN** only paths matching that glob SHALL be returned

#### Scenario: A single star does not cross a directory boundary

- **WHEN** a glob contains `*`
- **THEN** it SHALL NOT match across a path separator

#### Scenario: Pattern does not compile

- **WHEN** a glob fails to compile
- **THEN** the call SHALL fail with a message naming what is wrong with the
  pattern

### Requirement: Directory operations in the write union

`project.propose_patch` SHALL advertise directory rename and directory delete
alongside its file operations, with the same project-relative paths.

#### Scenario: Rename a directory

- **WHEN** the model proposes a directory rename
- **THEN** `kind`, `from`, and `to` SHALL be required

#### Scenario: Delete a directory

- **WHEN** the model proposes a directory delete
- **THEN** `kind` and `path` SHALL be required
- **AND** the directory SHALL be required to be empty, so removing a populated
  folder is expressed as deleting its files and then the folder

#### Scenario: Directory operation names a file

- **WHEN** a directory operation targets a path that is a plain file
- **THEN** the call SHALL fail and name the file operation to use instead

#### Scenario: File operation names a directory

- **WHEN** a file operation targets a path that is a directory
- **THEN** the call SHALL fail and name the directory operation to use instead
