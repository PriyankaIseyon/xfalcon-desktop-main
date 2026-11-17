# project-file-patch-inspector — Spec Delta

## ADDED Requirements

### Requirement: Visual diff preview before executing file writes or updates

The system MUST present a visual side-by-side or unified diff preview whenever an assistant or MCP tool proposes modifying an existing local project file.

The diff view MUST highlight additions, deletions, and unchanged context lines, allowing the user to review the exact patch before approving the action.

#### Scenario: Reviewing a CSV data modification

- **GIVEN** an agent proposing an update to `sales_summary.csv`
- **WHEN** the approval prompt appears
- **THEN** it renders the visual diff showing modified rows with clear red/green highlighting

### Requirement: Automated shadow backups and single-click file restoration

The system MUST automatically capture a shadow snapshot of any local project file prior to executing an overwrite or update operation.

Users MUST be able to view past revisions of any project file and revert the file to any previous snapshot with a single confirmation.

Shadow snapshots MUST be compressed, encrypted with the current account's
device-local key, and stored inside the account-scoped project-data resource so
clear-local-data removes them.

The system MUST retain at most 50 revisions per file and at most 512 MiB of
compressed snapshot content per account, evicting the oldest revisions first.

#### Scenario: Restoring an overwritten file

- **GIVEN** a project file that was modified by an agent
- **WHEN** the user opens the file history and clicks "Restore this revision"
- **THEN** the file on disk is safely restored to that exact snapshot state and a new backup is recorded

#### Scenario: Account cleanup removes revisions

- **GIVEN** an account with stored project-file revisions
- **WHEN** local data is cleared for that account
- **THEN** every revision for that account is removed with the project-data resource

#### Scenario: Retention remains bounded

- **GIVEN** snapshot creation would exceed a per-file or per-account retention bound
- **WHEN** the new snapshot is stored
- **THEN** the oldest eligible revisions are removed until both bounds hold

### Requirement: Binary and bounded diff states remain actionable

The system MUST show an explicit binary or truncated state when it cannot show
the complete line diff, and MUST still allow the reviewed mutation to be
approved or declined.

#### Scenario: Binary file changes

- **GIVEN** a proposed mutation whose existing or proposed content is binary
- **WHEN** the approval sheet appears
- **THEN** it states that the binary files differ and keeps both decision actions available

#### Scenario: A diff exceeds the display ceiling

- **GIVEN** a proposed text diff larger than the backend display ceiling
- **WHEN** the approval sheet appears
- **THEN** it shows the truncation marker and keeps both decision actions available
