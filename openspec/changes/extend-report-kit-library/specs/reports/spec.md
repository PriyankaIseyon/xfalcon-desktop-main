## RENAMED Requirements

- FROM: `### Requirement: The library shows dashboard kits and nothing else`
- TO: `### Requirement: The library shows every kind of report`

## MODIFIED Requirements

### Requirement: The library shows every kind of report

The system MUST list every kit the account can open: a dashboard, a workbook, and a
presentation. It MUST ask DA for the reader's hidden kits too and mark each with its
`hidden` flag, so the surface can show them again. The list MUST read every page DA
returns, up to a fixed page cap, so a long library is not silently cut short. A library
that cannot be read whole (the cap runs out, DA promises more kits than it returns, or
DA's total changes between pages or the kits read do not add up to it, on every one of
three passes, because the library kept changing) MUST be an error, never a shorter list.

#### Scenario: A workbook kit exists

- **GIVEN** an account with one dashboard kit and one workbook kit
- **THEN** the library lists two reports, and the workbook carries its kind

#### Scenario: A hidden kit exists

- **GIVEN** a kit the reader hid
- **THEN** it is in the list with `hidden` set, and is read in the same walk as the others

#### Scenario: A library longer than one page

- **GIVEN** 250 kits
- **THEN** all 250 are listed

#### Scenario: A kit moves between two pages

- **GIVEN** a rename that moves a kit behind the cursor while the list is read
- **THEN** the list is read again, and the result holds every kit exactly once

#### Scenario: A library that never adds up

- **GIVEN** a library that changes under every pass
- **THEN** the read fails and the surface offers Retry, and no partial list is shown

### Requirement: A report is named, pinned, renamed, and deleted from its list

The reader MUST be able to name a report and, when the account owns it, rename it, edit its
description, and delete it. Any reader MUST be able to pin and unpin a report for
themselves, and to hide it from, and show it in, their own list; a pin and a hide are the
reader's own and change nothing for anyone else.

The list SHALL show pinned reports first, then most recently updated, searchable over names
and descriptions. Pinning and hiding SHALL be expressed as intent.

The default view SHALL present three disjoint zones — pinned as cards, recent as cards,
the remainder in a table — each a filter over one ordering rule, never re-sorted
independently. The reader SHALL also be able to filter to the reports they made, the
reports shared with them, or the reports they pinned, choose a list layout, and sort by last
update, creation time or name without mutating the stored list. Each report SHALL appear once
in the current presentation.

A hidden report MUST NOT appear in the list, the counts, or the command palette until the
reader asks to show hidden reports, and the surface MUST always offer that request while any
report is hidden, including when every report is. A hidden report that is shown MUST say it
is hidden and offer the way back.

Rename and delete controls MUST be disabled, with a stated reason, for a report the account
does not own. DA's refusal (HTTP 403) MUST surface as `Forbidden`.

Deleting MUST require explicit confirmation.

#### Scenario: Hiding

- **GIVEN** a listed report
- **WHEN** the reader hides it
- **THEN** it leaves their list and counts, no other reader's list changes, and it comes back
  when they choose to show hidden reports

#### Scenario: Every report is hidden

- **GIVEN** an account whose every report is hidden
- **THEN** the surface says all reports are hidden and offers to show them, rather than
  reading as empty

#### Scenario: Filtering by who made a report

- **GIVEN** reports the reader owns and reports others shared
- **WHEN** the reader chooses Mine, or Shared with me
- **THEN** only the matching reports are listed, and an empty result says what that filter means

#### Scenario: Renaming

- **GIVEN** a saved report the account owns
- **WHEN** the reader renames it
- **THEN** the list shows the new name, a failure restoring the old one

#### Scenario: Clearing a description

- **GIVEN** a report with a description
- **WHEN** the reader clears it
- **THEN** the stored description is empty, not unchanged

#### Scenario: Pinning

- **GIVEN** several reports
- **WHEN** one is pinned
- **THEN** it sorts ahead of the unpinned ones, for this reader only

#### Scenario: A report someone else shares

- **GIVEN** an organization-shared report owned by another person
- **THEN** it lists with its owner's name, can be opened, pinned, and hidden, and cannot be
  renamed, shared, or deleted

#### Scenario: Deleting

- **WHEN** the reader deletes a report
- **THEN** confirmation is required first

#### Scenario: An empty list says what to do

- **GIVEN** an account that has no reports
- **THEN** the surface says that reports are made by asking the assistant to save a dashboard

### Requirement: Report action previews

The viewer SHALL offer Schedule a refresh and Share via email as previews. They are
placeholders for capabilities DA is expected to expose (scheduled refresh and email), so
until it does they SHALL be explicitly labeled as demo mode and SHALL perform no API calls,
email delivery, scheduling, or report mutation. Confirming valid input SHALL show a demo
success message inside the dialog and a toast. Refresh and Export are not previews: each is
a real action (see "A report refreshes on DA, from this surface or from chat" and "A report
exports to a file the reader names").

#### Scenario: Schedule preview

- **WHEN** the reader configures a schedule
- **THEN** once, hourly, daily, weekly, monthly and custom five-field numeric cron options are available with calendar, time, timezone and optional end date
- **AND** invalid dates, times, timezone, empty weekly selection and invalid cron block confirmation
- **AND** a readable summary states the schedule in words

#### Scenario: Email preview

- **WHEN** the reader shares by email
- **THEN** one to ten addresses and a nonempty subject are required
- **AND** confirming claims no email was sent

#### Scenario: Safe report rendering

- **WHEN** a demo action is confirmed or cancelled
- **THEN** the isolated report iframe and stored report remain unchanged

## ADDED Requirements

### Requirement: The owner chooses who can open a report

The system MUST let the owner of a report, when DA says the account can manage it, change the
report between private and the organization, and MUST NOT offer the change to anyone else.
Each change MUST be confirmed first, and the confirmation MUST say who can then open the
report and that they can refresh it under their own access. No public link exists and none
is offered. DA's refusal for lack of ownership MUST be told to the reader in the dialog that
asked, and any other failure MUST leave the report as it was.

#### Scenario: Sharing

- **GIVEN** a private report the account owns
- **WHEN** the owner confirms sharing with the organization
- **THEN** the request carries only the new scope, and the report lists as shared with the organization

#### Scenario: Cancelling

- **WHEN** the owner cancels the confirmation
- **THEN** nothing is sent

#### Scenario: A reader who does not own it

- **GIVEN** a report another person shared
- **THEN** no sharing control is offered

#### Scenario: DA refuses

- **GIVEN** a sharing change DA refuses for lack of ownership
- **THEN** the dialog says only the owner can change who can open it, and the list is unchanged

### Requirement: A workbook or presentation opens as a file

The system MUST open a workbook or presentation as a file card, never as a page: it MUST NOT
request an assembled page for one, and it MUST NOT frame one. The card MUST name the kind and
offer Download and, when DA says the account can refresh it, Refresh. A refresh of a file MUST
NOT try to put a page back.

#### Scenario: Opening a workbook

- **GIVEN** a workbook report
- **WHEN** the reader opens it
- **THEN** the viewer shows its kind and a Download control, and no page was requested

### Requirement: A dashboard says when its data was stored

The viewer MUST state when DA last stored each dataset behind the page on screen, using the
time DA stamped, and MUST say plainly when a report with stored queries has never been
refreshed, in which case the page shows the rows it was saved with. With several datasets it
MUST state the oldest time. It MUST NOT claim the data is current, and MUST state nothing for
a report with no stored queries. After a refresh it MUST list, for each dataset, its rows, how
long it took, and the source it ran against, or DA's stated reason for a dataset that failed.

#### Scenario: Datasets stored at different times

- **GIVEN** two datasets, one stored an hour ago and one never refreshed
- **THEN** the viewer states the stored time and that one dataset has not been refreshed yet

#### Scenario: Never refreshed

- **GIVEN** a report with stored queries and no stored data
- **THEN** the viewer says the saved data has not been refreshed yet

#### Scenario: A refresh with one failure

- **WHEN** a refresh finishes and one dataset failed
- **THEN** the viewer lists each dataset's outcome, and the failed one with DA's reason

### Requirement: A report exports to a file the reader names

The system MUST write a report to a file only at a place the reader chose in a save dialog
that the backend opens. The command MUST take a report id and a page name and nothing that
names a path or carries bytes. A dashboard MUST be written as its assembled, self-contained
page in one `.html` file. A workbook or presentation MUST be written as the `.xlsx` or `.pptx`
DA built: the backend MUST follow DA's redirect for it at most once, only to an `https`
address with no credentials in it, without sending the bearer to that address, and MUST write
it to a temporary file beside the destination and replace the destination only when every
byte arrived within the byte ceiling. A dismissed dialog MUST be an answer, not a failure.
Any failure MUST leave an existing file at the destination as it was and MUST leave no
temporary file behind. A page MUST be fetched before the dialog opens, and the account MUST be
checked again after each wait.

#### Scenario: Exporting a dashboard

- **WHEN** the reader exports the page on screen
- **THEN** the dialog opens in the backend, and the chosen file holds that page's assembled HTML

#### Scenario: Dismissing the dialog

- **WHEN** the reader dismisses the dialog
- **THEN** nothing is written and no failure is shown

#### Scenario: A download that cannot finish

- **GIVEN** a workbook whose download is cut short, comes back empty, or passes the ceiling
- **THEN** the export fails, the existing file is untouched, and no temporary file remains

#### Scenario: The redirect target is not trusted

- **GIVEN** DA's redirect names a plain-`http` address, an address with credentials, or a second redirect
- **THEN** it is refused and nothing is requested from that address

#### Scenario: The renderer cannot aim the write

- **GIVEN** the export command's inputs
- **THEN** none of them names a path or carries markup

### Requirement: A library report copies into a project folder

The system MUST let the reader copy the dashboard page on screen into `reports/` of a trusted
project they pick, and MUST offer it only when the account has such a project. The command MUST
take a report id, a page name, a project id, and a name, and MUST resolve the project to a
trusted root in the backend and fetch the page from DA itself, so no markup crosses the boundary.
The write MUST be the one function the chat card's project save uses, so the file name, the
duplicate rule, the size ceiling, and the approvals write are the same. The copy MUST be
described as a snapshot of the page as of its last refresh that keeps no link to the report.
A project the account does not hold MUST be refused as unavailable and nothing written.

#### Scenario: Copying into the working project

- **GIVEN** a reader with two projects, one of them active
- **WHEN** they save a dashboard to a project
- **THEN** the active project is offered first, and `reports/<name>.html` holds the assembled page

#### Scenario: Copying twice

- **GIVEN** a page already copied into the project
- **WHEN** it is copied again
- **THEN** the result names the existing file, and a copy on purpose is written as a numbered sibling

#### Scenario: A workbook

- **GIVEN** a workbook report
- **THEN** no project copy is offered, because it has no page
