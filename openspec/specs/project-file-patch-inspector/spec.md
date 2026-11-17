# project-file-patch-inspector Specification

## Purpose

What a person sees before an agent's change lands, and how they get the old bytes back
afterwards. Two surfaces: a diff at the decision point, and a bounded encrypted history
of revisions.

## Scope

| File                                                   | What it owns here                               |
| ------------------------------------------------------ | ----------------------------------------------- |
| `src/features/approvals/ProjectPatchApprovalSheet.tsx` | the review sheet and its decisions              |
| `src/features/projects/FileDiffViewer.tsx`             | the rendered diff                               |
| `src/features/projects/diff.ts`                        | the diff model                                  |
| `src-tauri/src/projects/backup.rs`                     | shadow snapshots, their format, their retention |
| `src-tauri/src/projects/backup.rs:23`                  | `MAX_REVISIONS_PER_FILE`                        |
| `src-tauri/src/projects/backup.rs:24`                  | `MAX_BACKUP_ACCOUNT_BYTES`                      |
| `src-tauri/src/projects/patch.rs:67`                   | `MAX_DIFF_BYTES` — the display ceiling          |

## Boundaries

- **The write the review gates** belongs to `local-project-execution`.
- **A person's own save**, which has no review, belongs to `project-file-editing`.
- **Approving a tool call as a call** belongs to `tool-approval`; this capability owns
  the diff shown inside that decision.
- **Clearing an account's local data** belongs to `encrypted-local-store`; this
  capability requires only that revisions live inside the resource that clearing
  removes.

## Requirements

### Requirement: A proposed change to an existing file is shown as a diff

The system MUST present a visual diff whenever an agent or an MCP tool proposes
modifying an existing local project file.

The diff MUST distinguish additions, deletions, and unchanged context, so the exact
patch is reviewable before the action is approved.

#### Scenario: Reviewing a data modification

- **GIVEN** an agent proposing an update to an existing file
- **WHEN** the approval prompt appears
- **THEN** it renders the diff, marking modified lines distinctly

### Requirement: Binary and bounded diff states remain actionable

The system MUST show an explicit binary or truncated state when it cannot show the
complete line diff, and MUST still allow the reviewed mutation to be approved or
declined.

A reviewer who cannot see the whole change still has a decision to make, and removing
the decision would leave the run stuck rather than informed.

#### Scenario: Binary file changes

- **GIVEN** a proposed mutation whose existing or proposed content is binary
- **WHEN** the approval sheet appears
- **THEN** it states that the binary files differ and keeps both decisions available

#### Scenario: A diff exceeds the display ceiling

- **GIVEN** a proposed text diff larger than the backend display ceiling
- **WHEN** the approval sheet appears
- **THEN** it shows the truncation marker and keeps both decisions available

### Requirement: A shadow snapshot is captured before an overwrite

The system MUST capture a shadow snapshot of a local project file before executing an
overwrite or update of it.

Snapshots MUST be compressed and encrypted with the account's device-local key, and
stored inside the account-scoped project-data resource, so clearing local data removes
them.

#### Scenario: An overwrite is preceded by a snapshot

- **WHEN** an agent's plan overwrites an existing file
- **THEN** the previous bytes are captured before the write

#### Scenario: Account cleanup removes revisions

- **GIVEN** an account with stored project-file revisions
- **WHEN** local data is cleared for that account
- **THEN** every revision for that account is removed with the project-data resource

### Requirement: A revision can be restored, and restoring is itself recorded

The system MUST let a person view past revisions of a project file and restore any of
them with a single confirmation.

A restore MUST record a new snapshot of what it replaced, so the restore is itself
undoable and no state is unreachable.

#### Scenario: Restoring an overwritten file

- **GIVEN** a project file that was modified by an agent
- **WHEN** the person opens the file history and restores a revision
- **THEN** the file on disk becomes that snapshot and a new backup is recorded

### Requirement: Retention is bounded per file and per account

The system MUST retain at most a fixed number of revisions per file and at most a fixed
volume of compressed snapshot content per account, evicting the oldest eligible
revisions first.

#### Scenario: Retention remains bounded

- **GIVEN** snapshot creation would exceed a per-file or per-account bound
- **WHEN** the new snapshot is stored
- **THEN** the oldest eligible revisions are removed until both bounds hold

## Provenance

From the delta of `add-project-file-patch-inspector`. The restore-records-a-snapshot
rule is stated as its own requirement; the delta carried it inside a scenario, where it
read as an implementation detail rather than the reason a restore is safe.
