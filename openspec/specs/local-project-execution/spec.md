# local-project-execution Specification

## Purpose

The device as the sole authority over a trusted project root: who may resolve a path,
what a write must preserve, and how containment survives every path a mutation takes.

## Scope

| File                                    | What it owns here                                      |
| --------------------------------------- | ------------------------------------------------------ |
| `src-tauri/src/projects/trust.rs`       | trusted roots and their identity                       |
| `src-tauri/src/projects/path_policy.rs` | path validation and its bounds                         |
| `src-tauri/src/projects/writer.rs`      | the atomic write and its permission handling           |
| `src-tauri/src/projects/patch.rs`       | plans, their bounds, confirmation thresholds, rollback |
| `src-tauri/src/projects/patch.rs:39`    | `MAX_OPERATIONS_PER_PLAN`                              |
| `src-tauri/src/projects/patch.rs:57`    | the second-confirmation thresholds                     |
| `src-tauri/src/projects/patch.rs:63`    | `DEFAULT_PLAN_TTL`                                     |
| `src-tauri/src/projects/index.rs`       | the incremental index and its bounds                   |
| `src-tauri/src/projects/search.rs:67`   | `MAX_LIVE_WALK_FILES` — the uncached fallback          |
| `src-tauri/src/projects/read.rs`        | reads, sniffing, and metadata-only outcomes            |
| `src-tauri/src/projects/sensitive.rs`   | the sensitivity classifier                             |
| `src-tauri/src/projects/platform.rs`    | per-platform handle and link behaviour                 |

## Boundaries

- **The tool schemas and their argument validation** belong to
  `project-tool-contract`.
- **The review surface** belongs to `project-file-patch-inspector`, and the
  no-review save path to `project-file-editing`.
- **Where a root is stored and what stays on device** belong to
  `project-data-handling`.
- **Which conversation a project is bound to** belongs to
  `conversation-project-binding`.
- **The audit records a mutation emits** belong to `audit-trail`.

## Requirements

### Requirement: Local execution authority

The desktop runtime SHALL be the sole authority for trusted local project roots,
filesystem identity, indexing, reads, searches, and writes. The data plane and the
model provider MUST NOT select or authenticate a local project.

#### Scenario: Bound run starts

- **WHEN** a run starts for a conversation bound to an available trusted project
- **THEN** the runtime revalidates the root and freezes that project into the run
  context
- **AND** every project tool call in the run uses that frozen context

#### Scenario: Root is unavailable

- **WHEN** the bound project's root is missing or its identity changed
- **THEN** the conversation remains readable
- **AND** project tools fail closed without substituting another active project

### Requirement: Search index readiness

Before a run advertises project search tools, the desktop SHALL complete a bounded
incremental index pass for the bound project, off the async and UI core.

A direct read of an exact relative path MUST stay independent of index state.

#### Scenario: First run after trust

- **WHEN** the first project-bound run is assembled
- **THEN** the local encrypted index is refreshed before a search tool can be called

#### Scenario: Direct read before index

- **WHEN** a reader browses or reads an exact relative path
- **THEN** that operation is independent of search-index state

### Requirement: Search covers files the index has not reached

Search SHALL treat the local index as a fast path rather than the source of truth, so
a file that exists but is not yet indexed is still found.

A bounded live walk MUST supplement the index, and reaching its bound MUST be
reported, so "nothing matched" is distinguishable from "not everything was looked at".

The exclusions that apply to an indexed candidate MUST apply to a walked one.

#### Scenario: Search in a freshly trusted project

- **WHEN** a search runs before the index has covered the project
- **THEN** candidates include a bounded live walk
- **AND** the result does not report an empty match set merely because the index was
  cold

#### Scenario: Live walk hits its bound

- **WHEN** the live walk reaches its scan bound
- **THEN** the outcome reports that it was bounded

#### Scenario: Sensitive and non-text files in the live walk

- **WHEN** the walk surfaces a classified-sensitive, binary, non-UTF-8, or oversized
  file
- **THEN** the same exclusions as for an indexed candidate apply

### Requirement: Reads report completeness and encountered binary content honestly

A bounded whole-file or streaming hash read SHALL return a complete hash only
after reaching end of file within its byte limit. A file that grows beyond that
limit MUST fail explicitly rather than return a prefix buffer or hash as complete.

A partial text read that encounters a NUL in a scanned line MUST return a binary
metadata outcome with no text content. This applies to returned lines, skipped
prefix lines, and the line used to check whether the requested page continues.

#### Scenario: File grows after its size check

- **GIVEN** a file passed the whole-read or hash size check
- **WHEN** the reader observes more bytes than its checked limit
- **THEN** the read fails without returning an incomplete buffer or hash
- **AND** empty and exact-limit files still return their complete hashes

#### Scenario: Binary content appears after the initial text sniff

- **GIVEN** the initial prefix looked like text
- **WHEN** a scanned line beyond that prefix contains a NUL
- **THEN** the outcome contains binary metadata and no text content
- **AND** requesting a later line does not ignore binary evidence already scanned
- **AND** bytes discarded beyond the line display bound still count as scanned binary evidence

### Requirement: A write preserves the file's own permissions

A write that replaces an existing file SHALL preserve that file's permissions, because
the replacement is a different file on disk than the one it replaces.

A permission copy that itself fails MUST fail the operation closed with the original
unchanged, rather than landing new content under permissions nobody asked for.

A newly created file takes the platform default, because there is no prior file to
inherit from.

#### Scenario: Executable file is edited

- **WHEN** an existing file's content is replaced on a platform with mode bits
- **THEN** the replacement carries the target's previous mode and stays executable

#### Scenario: Private file is edited

- **WHEN** a file readable only by its owner is edited
- **THEN** the replacement is not readable more widely than the original

#### Scenario: Read-only file is edited on a platform that refuses replacement

- **WHEN** a read-only file's content is replaced
- **THEN** the write fails with the original intact and no staged temporary left behind

#### Scenario: Permissions cannot be copied

- **WHEN** the permission copy fails
- **THEN** the operation fails closed with the original file unchanged

### Requirement: Missing parent directories are created under containment

A write SHALL create the missing intermediate directories inside the trusted root when
its target's parent does not exist — for a create, or for a rename whose destination is
in a new directory.

Each directory MUST be created through its parent's own capability handle, and a
symlink, junction, or reparse point on any existing component MUST still fail closed.

Directories this plan created MUST be removed in reverse creation order if the plan
rolls back, and a directory that is no longer empty MUST be left alone.

#### Scenario: Create in a new directory

- **WHEN** the model creates a file whose parent directories do not exist
- **THEN** the missing directories are created inside the trusted root and the file is
  written

#### Scenario: Link on the existing prefix

- **WHEN** an existing component of the path is a link or reparse point
- **THEN** the operation fails closed and creates nothing

#### Scenario: Plan rolls back after directories were created

- **WHEN** an operation fails after this plan created directories
- **THEN** those directories are removed in reverse order while still empty

### Requirement: Directories can be moved and removed under containment

A directory SHALL be renamable and deletable through the same plan, approval, audit,
and containment path a file mutation uses.

Emptiness MUST be enforced by the removal itself, not by a check at proposal time, so a
file appearing between propose and execute fails the removal rather than destroying it.

A plan that both operates on a path and renames or deletes the directory containing it
MUST be refused at proposal, naming the sequence that works.

A directory delete MUST require second confirmation, as every other delete does.

#### Scenario: Directory is renamed

- **WHEN** a plan renames a directory
- **THEN** the move is performed against link-checked parent handles, and missing
  destination parents are created as for a file

#### Scenario: Empty directory is deleted

- **WHEN** a plan deletes an empty directory
- **THEN** it is removed

#### Scenario: Directory is not empty

- **WHEN** a plan deletes a directory that still holds an entry
- **THEN** the removal fails with the directory intact, saying its contents must be
  deleted first

#### Scenario: Directory gains a file after the plan was made

- **WHEN** a file appears in the directory between propose and execute
- **THEN** the removal fails rather than destroying it

#### Scenario: A directory operation and an operation inside it

- **WHEN** one plan both operates on a path and renames or deletes its containing
  directory
- **THEN** the plan is refused at proposal, naming the working sequence

#### Scenario: Link in place of a directory

- **WHEN** a directory operation's target is a link or reparse point
- **THEN** the operation fails closed rather than following it

### Requirement: Edits fail closed on content that changed underneath them

The bytes an anchored edit was computed against SHALL be pinned by hash, and the
mutation plan's independently captured base hash MUST equal that hash.

Destructive file operations MUST also recheck base hashes immediately before
apply. Directory operations MUST recheck the staged stable filesystem identity
through a revalidated capability handle; creation timestamps alone are
insufficient. A rename to an absent destination MUST refuse an entry that appears
before the final check. These checks do not eliminate the final check-to-syscall
race with an external process.

#### Scenario: File changes between edit and plan

- **WHEN** the file's content changes after an anchored edit was computed and before
  the plan captured its base hash
- **THEN** the call fails without writing, telling the model to read the file again

#### Scenario: Content changes after staging

- **GIVEN** a modify, delete, or file rename has finished staging
- **WHEN** a source or an originally existing destination changes before apply
- **THEN** the writer refuses that operation on its base hash
- **AND** the failure reports the applied prefix and cleans unused staged files

#### Scenario: A rename destination appears after staging

- **GIVEN** a file or directory rename targets an absent destination
- **WHEN** that destination appears before the final apply check
- **THEN** the writer refuses the rename without replacing the new entry

#### Scenario: A staged directory is replaced

- **WHEN** a directory entry changes between staging and the final apply check
- **THEN** directory rename or deletion fails on its captured filesystem identity
- **AND** matching creation time and attributes do not bypass the check

#### Scenario: A durability check fails after one mutation lands

- **GIVEN** a multi-operation plan has staged all of its changes
- **WHEN** a mutation lands but its parent-directory flush cannot be confirmed
- **THEN** the writer reports that mutation in the applied prefix with unconfirmed durability
- **AND** it removes unused temporary files and empty staging-created directories
- **AND** later operations leave their original files unchanged

### Requirement: Local writes keep the runtime responsive and report actual outcomes

The desktop SHALL perform local resolution, backup capture, filesystem mutation,
and terminal approval persistence in one owned blocking job. Local mutation jobs
MUST be serialized, including jobs bound to duplicate or nested project roots.
Reads and proposal preparation SHALL remain independent of this mutation lock.

A queued agent write MUST observe cancellation before claiming or changing files.
Once a local mutation starts, Stop or a tool deadline MUST drain its actual result
before finalizing the invocation. A round deadline SHALL still terminate the run
after its completed mutation results have been recorded. This does not create a
cross-file transaction.

#### Scenario: Stop while waiting for another writer

- **WHEN** an agent write is cancelled while waiting for the mutation lock
- **THEN** its plan remains pending and it changes no file

#### Scenario: Stop or timeout after mutation starts

- **WHEN** Stop, a call deadline, or a round deadline occurs during an admitted write
- **THEN** the writer finishes its local decision and persists its terminal status
- **AND** the invocation records the real result rather than a timeout for files
  that were successfully written

#### Scenario: Async caller is dropped

- **WHEN** an async resolution caller disappears after its blocking job starts
- **THEN** the owned job retains the mutation lock through terminal persistence
- **AND** it does not leave a completed write at claiming solely due to that drop

## Provenance

The base requirements come from the archived `refactor-local-project-boundary`, with
the deltas of `complete-project-file-crud` and `add-seamless-project-file-io` folded
in.
