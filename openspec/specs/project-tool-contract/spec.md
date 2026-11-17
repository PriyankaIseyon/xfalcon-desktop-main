# project-tool-contract Specification

## Purpose

The shape of the project tools as the model sees them: what a schema may ask for,
what it may never ask for, and how an existing file is changed without re-sending it.

The governing rule is that project authority is injected, never accepted. A model
names a project-relative path and nothing else.

## Scope

| File                                  | What it owns here                                            |
| ------------------------------------- | ------------------------------------------------------------ |
| `src-tauri/src/tools/dispatch.rs`     | the tool schemas, their arguments, and the outcome documents |
| `src-tauri/src/projects/domain.rs`    | the patch operation union                                    |
| `src-tauri/src/projects/edit.rs`      | anchored edits, their bounds, their failures                 |
| `src-tauri/src/projects/edit.rs:30`   | `MAX_EDITS_PER_OPERATION`                                    |
| `src-tauri/src/projects/edit.rs:33`   | `MAX_ANCHOR_BYTES`                                           |
| `src-tauri/src/projects/read.rs:53`   | `MAX_FILE_READ_BYTES`                                        |
| `src-tauri/src/projects/read.rs:70`   | `MAX_LINE_RANGE_LINES`                                       |
| `src-tauri/src/projects/read.rs:86`   | `MAX_PARTIAL_READ_BYTES` — the page a tool result can carry  |
| `src-tauri/src/projects/search.rs:53` | the search bounds                                            |
| `src-tauri/src/projects/search.rs:65` | `MAX_PATTERN_PROGRAM_BYTES`                                  |
| `src-tauri/src/projects/patch.rs:54`  | `MAX_EDITABLE_FILE_BYTES`                                    |
| `src-tauri/src/agent/prompt.rs`       | the project section of the system prompt                     |

## Boundaries

- **Executing a plan against the filesystem** belongs to `local-project-execution`.
- **The diff a person reviews** belongs to `project-file-patch-inspector`.
- **Saving from the viewer** belongs to `project-file-editing`.
- **The step summary a durable call carries** belongs to `chat-tool-call-detail`,
  which reads the outcome documents this capability authors.
- **Where the trusted root came from** belongs to `conversation-project-binding` and
  `project-data-handling`.
- **The rest of the system prompt** belongs to `run-prompt-assembly`.

## Requirements

### Requirement: Runtime-injected project authority

Provider-facing project tool schemas MUST NOT contain a project identifier, a root
path, a folder handle, or any other project-authority input. The runtime SHALL inject
the run-bound project context at dispatch.

#### Scenario: Model reads a file

- **WHEN** the model calls the read tool
- **THEN** it provides only a project-relative path and an optional line range
- **AND** the runtime resolves that path against the run-bound trusted root

#### Scenario: Injected identifier appears in content

- **WHEN** a prompt, file, or tool result contains another local project identifier
- **THEN** that content has no effect on the dispatcher's project context

### Requirement: Typed patch proposal operations

The patch tool SHALL advertise a discriminated union whose required fields match each
operation kind. Model-authored content MUST be UTF-8 text and all paths MUST be
project-relative.

The union SHALL include anchored edit kinds, so an existing file can be changed
without re-sending its whole content.

#### Scenario: Create or modify

- **WHEN** the model proposes a create or modify operation
- **THEN** the kind, the path, and the content are required

#### Scenario: Rename

- **WHEN** the model proposes a rename operation
- **THEN** the kind, the source, and the destination are required

#### Scenario: Delete

- **WHEN** the model proposes a delete operation
- **THEN** the kind and the path are required

#### Scenario: Replace

- **WHEN** the model proposes a replace operation
- **THEN** the kind, the path, and a non-empty edit list are required
- **AND** each edit requires its old and new text and may ask for every occurrence

#### Scenario: Insert

- **WHEN** the model proposes an insert operation
- **THEN** the kind, the path, an insert line, and the content are required
- **AND** insert line zero means the top of the file

### Requirement: Directory operations in the write union

The patch tool SHALL advertise directory rename and directory delete alongside its
file operations, with the same project-relative paths.

A directory delete MUST require the directory to be empty, so removing a populated
folder is expressed as deleting its files and then the folder.

An operation whose kind does not match what is on disk MUST fail and name the
operation to use instead.

#### Scenario: Rename a directory

- **WHEN** the model proposes a directory rename
- **THEN** the kind, the source, and the destination are required

#### Scenario: Delete a directory

- **WHEN** the model proposes a directory delete
- **THEN** the kind and the path are required, and the directory must be empty

#### Scenario: Directory operation names a file

- **WHEN** a directory operation targets a plain file
- **THEN** the call fails and names the file operation to use instead

#### Scenario: File operation names a directory

- **WHEN** a file operation targets a directory
- **THEN** the call fails and names the directory operation to use instead

### Requirement: Anchored edits resolve against current file bytes

An anchored edit SHALL be resolved by the runtime against the file's current on-disk
bytes and lowered to a whole-file modification before any mutation plan exists, so the
reviewed diff and the written bytes are the same bytes.

Edits in one operation MUST be applied in order to one working buffer, so a later edit
may anchor on text an earlier edit wrote.

#### Scenario: Unique anchor

- **WHEN** an edit's anchor occurs exactly once in the file
- **THEN** that occurrence is replaced
- **AND** the resulting plan contains one whole-file modification for that path

#### Scenario: Anchor matches nothing

- **WHEN** an edit's anchor does not occur in the file
- **THEN** the call fails without writing, naming the path and the edit's position and
  stating that the text must match exactly, whitespace included

#### Scenario: Anchor is ambiguous

- **WHEN** an anchor occurs more than once and every-occurrence was not asked for
- **THEN** the call fails without writing, reporting the number of occurrences and
  asking for more surrounding context

#### Scenario: Sequential edits in one operation

- **WHEN** an operation carries several edits
- **THEN** they apply in order to one buffer

#### Scenario: Edit changes nothing

- **WHEN** an edit's replacement equals its anchor, or a whole-file content equals the
  file's current bytes
- **THEN** the call fails as a no-op rather than writing and reporting success

#### Scenario: File is not text

- **WHEN** an anchored edit targets a file that does not classify as UTF-8 text
- **THEN** the call fails without writing

### Requirement: Line-ending-tolerant anchors

An anchor SHALL match a file whose lines end in CRLF even when the anchor itself uses
LF, and replacing text SHALL be written using the file's own line ending.

A file with mixed line endings MUST be matched exactly only, because there is no one
ending to rewrite to.

#### Scenario: LF anchor against a CRLF file

- **WHEN** an anchor with LF endings finds no exact match and the file uses CRLF
- **THEN** matching is retried with the anchor's endings rewritten, and the replacement
  is written with the file's endings

#### Scenario: Mixed line endings

- **WHEN** the file contains both endings
- **THEN** only exact matching is attempted

### Requirement: Whole-file modification is bounded by what a read can show

A whole-file modify SHALL be refused for a file larger than one read call can return,
because the model cannot have seen the content it would be discarding.

The refusal MUST name anchored edits as the way to edit that file.

#### Scenario: Oversized whole-file modify

- **WHEN** the model sends a whole-file modify whose target exceeds the single-read
  ceiling
- **THEN** the call fails without writing and names the anchored path

### Requirement: A bounded read carries a cursor

A read SHALL report how to obtain the rest of a file it did not fully return, so a
file of any size is reachable in pages rather than refused.

A page MUST be small enough to survive the structured-result ceiling, which discards
an oversized result whole rather than truncating it.

#### Scenario: Truncated page

- **WHEN** a read returns fewer lines than the file contains
- **THEN** the result carries the next line number to request
- **AND** a read that returned the rest carries none

#### Scenario: File is longer than one read returns

- **WHEN** a file is longer than one read can return
- **THEN** the read returns its first page rather than refusing it, and names the call
  that returns the next

#### Scenario: Open-ended range

- **WHEN** the model gives a start line without an end line
- **THEN** one page beginning at that line is returned, bounded by the same per-call
  limits

#### Scenario: End without start

- **WHEN** the model gives an end line without a start line
- **THEN** the call is rejected as ambiguous

### Requirement: Bounds are stated against the resource that constrains them

Each project file bound MUST be justified by the resource it protects, and an
operation that does not consume a resource MUST NOT inherit that resource's bound.

#### Scenario: Identity of a large file

- **WHEN** a file's hash is computed
- **THEN** it is computed by streaming, without holding the file in memory

#### Scenario: Rename or delete of a large file

- **WHEN** a plan renames or deletes a file whose content is never needed
- **THEN** the operation is not refused for exceeding a content-read bound

#### Scenario: Anchored edit of a large file

- **WHEN** an anchored edit targets a file larger than a single read returns to the
  model but within the editable-content bound
- **THEN** the edit is allowed, because its content never enters the model's context

### Requirement: Content search accepts a pattern by opt-in

The text search tool SHALL accept a regular expression when the caller asks for one,
and SHALL treat a query as literal text otherwise.

A compiled pattern MUST be bounded in program size.

#### Scenario: Literal is the default

- **WHEN** no search mode is given
- **THEN** the query matches as literal text and metacharacters carry no special
  meaning

#### Scenario: Regex is requested

- **WHEN** the caller asks for regex mode
- **THEN** the query is compiled under a compiled-size bound

#### Scenario: Pattern does not compile

- **WHEN** a pattern fails to compile
- **THEN** the call fails with a message naming what is wrong with it

### Requirement: Path search accepts a glob by opt-in

The file search tool SHALL accept a glob pattern when the caller asks for one, and
SHALL rank by substring match otherwise.

A single star MUST NOT match across a path separator.

#### Scenario: Glob is requested

- **WHEN** the caller asks for glob mode with a directory-spanning pattern
- **THEN** only paths matching that glob are returned

#### Scenario: A single star does not cross a directory boundary

- **WHEN** a glob contains a single star
- **THEN** it does not match across a path separator

#### Scenario: Pattern does not compile

- **WHEN** a glob fails to compile
- **THEN** the call fails with a message naming what is wrong with it

### Requirement: Project-aware instructions match write behavior

A project-bound run SHALL include a bounded system-prompt section that names the
project for orientation and explains relative-path use, without exposing its absolute
path or local identifier.

That section MUST describe writing as it actually behaves: it MUST NOT say a change
waits for human approval, it MUST point at anchored edits for an existing file, and it
MUST say that a long file is read in pages.

It MUST NOT ask the model to invent or provide a project identifier.

#### Scenario: Project prompt assembled

- **WHEN** the run has a valid project binding
- **THEN** the system prompt says project tools already target that project, and asks
  for no identifier

#### Scenario: The prompt describes the write it will get

- **WHEN** a run is bound to a project
- **THEN** the prompt does not promise human approval, names anchored edits for
  existing files, and says long files are read in pages

## Provenance

The base requirements come from the archived `refactor-local-project-boundary`, with
the deltas of `add-seamless-project-file-io` and `complete-project-file-crud` folded
in. Bounds are referenced by their named constants rather than by value, because the
constants are the authority and a spec repeating a number adds a second place to
update.
