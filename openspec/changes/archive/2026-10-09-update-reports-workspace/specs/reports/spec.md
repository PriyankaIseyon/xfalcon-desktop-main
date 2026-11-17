## MODIFIED Requirements

### Requirement: A report is named, pinned, renamed, and deleted from its list

The reader MUST be able to name a report on save and afterward rename it, edit its
description, pin and unpin it, and delete it — the whole action set.

The list SHALL show pinned reports first, then most recently updated, searchable over
names and descriptions. Pinning SHALL be expressed as intent and resolved to a
timestamp in the backend.

The default view SHALL present three disjoint zones — pinned as cards, recent as cards,
the remainder in a table. The reader SHALL also be able to filter to pinned reports,
choose a list layout, and sort by last update, creation time or name without mutating
the stored catalog. Each report SHALL appear once in the current presentation.

Deleting MUST require explicit confirmation.

#### Scenario: Renaming

- **GIVEN** a saved report
- **WHEN** the reader renames it
- **THEN** the list shows the new name, a failure restoring the old one

#### Scenario: Pinning

- **GIVEN** several reports
- **WHEN** one is pinned
- **THEN** it sorts ahead of the unpinned ones, the pin time from the backend

#### Scenario: Deleting

- **WHEN** the reader deletes a report
- **THEN** confirmation is required first

#### Scenario: An empty list says what to do

- **GIVEN** an account that has saved no reports
- **THEN** the surface says how one is made

## ADDED Requirements

### Requirement: A useful reporting workspace

The library SHALL display counts derived from the catalog, searchable report names and descriptions, accessible layout and ordering controls, and a short instruction to use Save as report in chat. Metadata SHALL distinguish saved document timestamps from business-data freshness. Report cards SHALL display a scaled preview of the actual saved document through the isolated preview origin. Only visible cards SHALL load previews. Thumbnail frames SHALL be inert, unfocusable, and sandboxed without same-origin, top-navigation, popup or app privileges; document HTML MUST NOT cross IPC. Unavailable previews SHALL be labeled without preventing report actions.

#### Scenario: Browse a large library

- **WHEN** the reader filters, searches, sorts or switches layouts
- **THEN** the visible reports follow all selected controls without duplicate report identities

#### Scenario: Recover from stale library state

- **GIVEN** the cached library is stale
- **WHEN** the library is reloaded and the server answers
- **THEN** the current catalog replaces the stale copy and mutations become available
- **AND** a failed synchronization preserves the cached content

#### Scenario: Save a report from chat

- **WHEN** the reader visits Reports
- **THEN** the banner explains Save as report in chat without New report or Sync library buttons

#### Scenario: Accessible responsive presentation

- **WHEN** the workspace uses a narrow window, dark theme or reduced motion
- **THEN** navigation and report actions remain operable with keyboard and pointer without page overflow

### Requirement: Report action previews

The viewer SHALL offer Refresh now, Schedule a refresh, Share via email and Export. These actions SHALL be explicitly labeled as demo mode and SHALL perform no API calls, email delivery, scheduling, report mutation or file export. Confirming valid input SHALL show a demo success message inside the dialog and a toast.

#### Scenario: Schedule preview

- **WHEN** the reader configures a schedule
- **THEN** once, hourly, daily, weekly, monthly and custom five-field numeric cron options are available with calendar, time, timezone and optional end date
- **AND** invalid dates, times, timezone, empty weekly selection and invalid cron block confirmation
- **AND** a readable summary states the schedule in words

#### Scenario: Share and export previews

- **WHEN** the reader shares by email
- **THEN** one to ten addresses and a nonempty subject are required
- **WHEN** the reader exports
- **THEN** PDF, HTML and PNG presentation choices are available without generating or claiming a valid exported file

#### Scenario: Safe report rendering

- **WHEN** a demo action is confirmed or cancelled
- **THEN** the isolated report iframe and stored report remain unchanged
