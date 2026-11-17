## ADDED Requirements

### Requirement: A write result names what it did

The result of `project.propose_patch` MUST list every operation with its index,
kind, path and whether it applied, and MUST list files left alone because they
already held the content sent.

#### Scenario: A retry after a partial write

- **GIVEN** one file of a plan already holds the content the plan would write
- **WHEN** the plan is sent again with another file that still differs
- **THEN** the changed file is written and the matching file is reported as unchanged

#### Scenario: Nothing to write

- **GIVEN** every modification in a plan already matches
- **WHEN** the plan is sent
- **THEN** it is refused once, naming the files

### Requirement: Every bad operation is reported at once

#### Scenario: Two bad paths

- **GIVEN** a plan with two modifications of files that do not exist
- **WHEN** it is proposed
- **THEN** one error names both, states that nothing was written, and is retryable

### Requirement: A partly applied plan says what landed

#### Scenario: A write fails part way

- **GIVEN** the second of three operations fails to apply
- **WHEN** the error is reported
- **THEN** it names the first as applied and the second and third as not applied
- **AND** it tells the model to resend only the operations that did not apply

### Requirement: The model cannot change credentials or repository state

The system MUST refuse an agent operation whose path, or either path of a
rename, is classified as a secret or lies under `.git` or `.xfalcon`. A checked-in
template such as `.env.example` MUST NOT be classified as a secret.

#### Scenario: Renaming a secret

- **WHEN** the model asks to rename `.env` to `env.txt`
- **THEN** the operation is refused and nothing moves

### Requirement: A path has one spelling

#### Scenario: Two spellings of one file

- **WHEN** a plan changes `sub/inner.html` and `sub//inner.html/`
- **THEN** it is refused as two operations on one target

### Requirement: A whole-file replace needs the whole file to have been readable

#### Scenario: A file larger than one read

- **GIVEN** a file larger than one read returns
- **WHEN** the model replaces its whole content
- **THEN** the operation is refused with advice to use anchored edits

### Requirement: A file keeps its line endings

#### Scenario: Rewriting a Windows file

- **GIVEN** a file that is CRLF throughout
- **WHEN** the model rewrites it with bare line feeds, or edits one line with a multi-line replacement
- **THEN** the file is still CRLF throughout

#### Scenario: A viewer save

- **GIVEN** a CRLF file opened in the viewer
- **WHEN** the person edits and saves
- **THEN** the file is written with CRLF

### Requirement: Unsaved edits survive navigation

#### Scenario: Opening another file

- **GIVEN** unsaved edits to a file
- **WHEN** another file is opened and the first is opened again unchanged on disk
- **THEN** the edits are back in the editor

#### Scenario: A file that changed meanwhile

- **WHEN** the file changed on disk after the edits were left
- **THEN** the edits are not applied to it

### Requirement: An unedited open file follows the disk

#### Scenario: An agent edits the open file

- **GIVEN** a file open for reading
- **WHEN** it changes on disk
- **THEN** the view shows the new content without a prompt

#### Scenario: Someone is typing

- **WHEN** the file changes on disk while the person has edits open
- **THEN** their text is kept and the changed-on-disk notice appears

### Requirement: An edit anchor survives what a model cannot see

When `old_string` matches nowhere as written, the system MUST read it line by
line and accept a match that differs only in trailing whitespace, indentation,
or typographic quotes, dashes, ellipses and no-break spaces. The first tolerance
that finds the lines decides. The lines MUST be found in exactly one place
unless `replace_all` is set. New text MUST be moved to the file's indentation
and line ending. A file that mixes line endings MUST get exact matching only.

#### Scenario: An anchor copied without the file's indentation

- **GIVEN** a file whose lines are indented by four spaces
- **WHEN** an edit anchors on those lines without the indentation
- **THEN** the edit applies and the new text is indented like the file

#### Scenario: Two places match loosely

- **WHEN** a tolerant anchor finds its lines in two places and `replace_all` is not set
- **THEN** the edit is refused, naming the line numbers of the places

### Requirement: A failed anchor says how to fix it

A "no match" error MUST name the line of the file the anchor most resembles when
one resembles it, and MUST say so when the anchor carries line-number prefixes.
An ambiguity error MUST list the lines where the matches are.

#### Scenario: A misspelt anchor

- **WHEN** an anchor differs from a line of the file by a few characters
- **THEN** the error quotes that line and its number

### Requirement: Placeholder content is refused

Content written by a create, a whole-file modify or an edit MUST NOT contain a
comment that stands in for omitted content, such as `// ... rest unchanged`,
unless the file already held that line.

#### Scenario: A shortened rewrite

- **WHEN** a whole-file rewrite contains `// ... rest of the file unchanged`
- **THEN** the call is refused, the file is untouched, and the error says to send the complete text

### Requirement: A capped search says it is incomplete

A search result that hit its cap, or that ran over a partial index, MUST carry a
`detail` saying the list is not complete and what to do.

#### Scenario: Two hundred matches

- **WHEN** a content search stops at its cap
- **THEN** the result's `detail` says more matches exist and to narrow the query
