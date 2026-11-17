## ADDED Requirements

### Requirement: A report made in a project chat is linked to its project

The system MUST record, on this device, which trusted project a saved report was created in,
when an assistant's save call creates a kit in a conversation bound to that project. It MUST
record only a kit the call created, only once, and MUST NOT change an existing link. It MUST
record nothing for a failed save, a part of a chunked save, a redefine of an existing kit, or a
conversation with no project. A failure to record MUST NOT fail the tool call.

#### Scenario: A dashboard is saved in a project chat

- **GIVEN** a conversation bound to project Sales
- **WHEN** the assistant's save creates a kit
- **THEN** the library lists that report under project Sales

#### Scenario: A dashboard is saved in a regular chat

- **GIVEN** a conversation with no project
- **WHEN** the assistant's save creates a kit
- **THEN** the report has no project

#### Scenario: A report is redefined from a project chat

- **GIVEN** a report created in a regular chat
- **WHEN** the assistant redefines it from a conversation bound to a project
- **THEN** the report still has no project

#### Scenario: A project is removed

- **GIVEN** reports linked to a project
- **WHEN** the account stops holding that project
- **THEN** those reports have no project

### Requirement: The library filters by project

The library MUST offer a Project filter with _All reports_, _No project_, and one entry for each
project that has at least one listed report, each with its count. The filter MUST combine with
the relationship filter, the search, the sort and the hidden toggle. A card and a table row for
a linked report MUST name its project.

#### Scenario: Filter to one project

- **GIVEN** reports in projects Sales and Ops, and others with no project
- **WHEN** the reader chooses Sales
- **THEN** only Sales reports are listed

#### Scenario: Filter to reports with no project

- **WHEN** the reader chooses _No project_
- **THEN** only reports made outside a project chat, and those saved before the link existed,
  are listed

#### Scenario: The chosen project has nothing left

- **GIVEN** the reader chose Sales and hid its last report
- **THEN** the empty state names the project and offers _Show all reports_

### Requirement: A refresh keeps the report on screen

The viewer MUST keep showing the current page while a refresh re-reads it, and replace it only
when the new page has loaded. It MUST state the data's age in one status strip, list each
dataset's outcome after a refresh, and say plainly when a dataset failed.

#### Scenario: A refresh succeeds

- **WHEN** the reader refreshes a dashboard
- **THEN** the old page stays until the new one has loaded, and no skeleton is drawn over it

#### Scenario: A refresh partly fails

- **THEN** the strip names the datasets that failed and their reasons, and does not claim the
  data is current
