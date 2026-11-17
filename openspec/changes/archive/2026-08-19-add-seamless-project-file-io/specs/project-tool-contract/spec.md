# project-tool-contract Spec Delta

## MODIFIED Requirements

### Requirement: Typed patch proposal operations

`project.propose_patch` SHALL advertise a discriminated union whose required
fields match each operation kind. Model-authored content SHALL be UTF-8 text
and all paths SHALL be project-relative. The union SHALL include anchored edit
kinds so an existing file can be changed without re-sending its whole content.

#### Scenario: Create or modify

- **WHEN** the model proposes a create or modify operation
- **THEN** `kind`, `path`, and `content` SHALL be required

#### Scenario: Rename

- **WHEN** the model proposes a rename operation
- **THEN** `kind`, `from`, and `to` SHALL be required

#### Scenario: Delete

- **WHEN** the model proposes a delete operation
- **THEN** `kind` and `path` SHALL be required

#### Scenario: Replace

- **WHEN** the model proposes a replace operation
- **THEN** `kind`, `path`, and a non-empty `edits` array SHALL be required
- **AND** each edit SHALL require `old_string` and `new_string` and MAY carry
  `replace_all`

#### Scenario: Insert

- **WHEN** the model proposes an insert operation
- **THEN** `kind`, `path`, `insert_line`, and `content` SHALL be required
- **AND** `insert_line` `0` SHALL mean the top of the file

## ADDED Requirements

### Requirement: Anchored edits resolve against current file bytes

An anchored `replace`/`insert` operation SHALL be resolved by the desktop
runtime against the file's current on-disk bytes and lowered to a whole-file
modification before any mutation plan exists, so the reviewed diff and the
written bytes are the same bytes.

#### Scenario: Unique anchor

- **WHEN** an edit's `old_string` occurs exactly once in the file
- **THEN** that occurrence SHALL be replaced by `new_string`
- **AND** the resulting plan SHALL contain one whole-file modification for
  that path

#### Scenario: Anchor matches nothing

- **WHEN** an edit's `old_string` does not occur in the file
- **THEN** the tool call SHALL fail without writing
- **AND** the failure SHALL name the path and the edit's position and state
  that the text must match exactly, including whitespace

#### Scenario: Anchor is ambiguous

- **WHEN** an edit's `old_string` occurs more than once and `replace_all` is not
  set
- **THEN** the tool call SHALL fail without writing
- **AND** the failure SHALL report the number of occurrences and ask for more
  surrounding context or `replace_all`

#### Scenario: Sequential edits in one operation

- **WHEN** an operation carries several edits
- **THEN** they SHALL be applied in order to one working buffer, so a later
  edit MAY anchor on text an earlier edit wrote

#### Scenario: Edit changes nothing

- **WHEN** an edit's `new_string` equals its `old_string`, or a whole-file
  `content` equals the file's current bytes
- **THEN** the tool call SHALL fail as a no-op rather than write and report
  success

#### Scenario: File is not text

- **WHEN** an anchored edit targets a file that does not classify as UTF-8
  text
- **THEN** the tool call SHALL fail without writing

### Requirement: Line-ending-tolerant anchors

An anchor SHALL match a file whose lines end in CRLF even when the anchor
itself uses LF, and inserted or replacing text SHALL be written using the
file's own line ending.

#### Scenario: LF anchor against a CRLF file

- **WHEN** an anchor containing `\n` finds no exact match and the file
  contains `\r\n`
- **THEN** matching SHALL be retried with the anchor's line endings rewritten
  to `\r\n`
- **AND** `new_string` SHALL be written with `\r\n` line endings

#### Scenario: Mixed line endings

- **WHEN** the file contains both `\r\n` and bare `\n` line endings
- **THEN** only exact anchor matching SHALL be attempted

### Requirement: Whole-file modification is bounded by what a read can show

A whole-file `modify` SHALL be refused for a file larger than one
`project.read_file` call can return, because the model cannot have seen the
content it would be discarding.

#### Scenario: Oversized whole-file modify

- **WHEN** the model sends a `modify` whose target's current size exceeds the
  single-read ceiling
- **THEN** the tool call SHALL fail without writing
- **AND** the failure SHALL name `replace` as the way to edit that file

### Requirement: A bounded read carries a cursor

A read SHALL report how to obtain the rest of a file it did not fully return,
so a file of any size is reachable in pages rather than refused.

#### Scenario: Truncated page

- **WHEN** a read returns fewer lines than the file contains
- **THEN** the result SHALL carry the next line number to request
- **AND** a read that returned the rest of the file SHALL carry none

#### Scenario: A page always fits in a tool result

- **WHEN** a model-facing read returns content
- **THEN** the page SHALL be small enough to survive the structured-result
  ceiling, which discards an oversized result whole rather than truncating it

#### Scenario: File is longer than one read returns

- **WHEN** a file is longer than one read can return
- **THEN** the read SHALL return its first page rather than refusing it
- **AND** the result SHALL name the call that returns the next page

#### Scenario: Open-ended range

- **WHEN** the model gives `start_line` without `end_line`
- **THEN** one page beginning at that line SHALL be returned, bounded by the
  same per-call limits

#### Scenario: End without start

- **WHEN** the model gives `end_line` without `start_line`
- **THEN** the call SHALL be rejected as ambiguous

### Requirement: Bounds are stated against the resource that constrains them

Each project file bound SHALL be justified by the resource it protects, and an
operation that does not consume a resource SHALL NOT inherit that resource's
bound.

#### Scenario: Identity of a large file

- **WHEN** a file's hash is computed
- **THEN** it SHALL be computed by streaming, without holding the file in
  memory

#### Scenario: Rename or delete of a large file

- **WHEN** a plan renames or deletes a file whose content is never needed
- **THEN** the operation SHALL NOT be refused for exceeding a content-read
  bound

#### Scenario: Anchored edit of a large file

- **WHEN** an anchored edit targets a file larger than a single read returns to
  the model but within the editable-content bound
- **THEN** the edit SHALL be allowed, because its content never enters the
  model's context

### Requirement: Project instructions match write behavior

The project system-prompt section SHALL describe writing as it actually
behaves and SHALL point the model at anchored edits for existing files.

#### Scenario: Bound run assembles its prompt

- **WHEN** a run is bound to a project
- **THEN** the prompt SHALL NOT say a change waits for human approval
- **AND** it SHALL say that editing an existing file uses anchored edits
  rather than re-sending the whole file
- **AND** it SHALL say that a long file is read in pages
