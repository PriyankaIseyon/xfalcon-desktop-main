## RENAMED Requirements

- FROM: `### Requirement: A saved report is cached locally, sealed under the account, and cleared with it`
- TO: `### Requirement: Desktop keeps no copy of a report`

## MODIFIED Requirements

### Requirement: A report is captured from the stored message, never from the renderer

The system MUST save a document into a project folder from a source identity — the
conversation, the message, and the ordinal of the HTML document within that message —
together with a name the reader chose. The renderer MUST NOT send the document's markup, and
the backend MUST NOT accept it if sent.

The backend SHALL load the named message from the encrypted local projection, re-scan its
text for fenced HTML documents, and take the document at the given ordinal. Re-reading
from what was persisted is what makes the path unusable for turning renderer-supplied
markup into a stored file.

Desktop MUST NOT upload a report to DA by this path or any other of its own: server reports
are created by the assistant through DA's tools.

Three conditions MUST refuse the save, reported as outcomes of a successful call rather
than as operation failures: the message does not belong to the named conversation, the
message is not an assistant turn, the message holds no document at that ordinal.

#### Scenario: A dashboard is saved into the project

- **GIVEN** an assistant message in a project conversation holding one fenced HTML document
- **WHEN** the reader names it and saves it
- **THEN** a file in the project's `reports/` folder holds the document's bytes as persisted

#### Scenario: A message from another conversation

- **GIVEN** a save naming a message that belongs to a different conversation
- **THEN** it is refused as not found, telling the caller nothing about whether that
  message exists

#### Scenario: A message with no such document

- **GIVEN** a save naming an ordinal past the last HTML document in the message
- **THEN** it is refused with that reason, and nothing is written

#### Scenario: The renderer cannot supply the bytes

- **GIVEN** the save command's inputs
- **THEN** none of them carries document markup

#### Scenario: Desktop has no upload of its own

- **GIVEN** the commands this capability registers
- **THEN** none of them sends a report's pages or definitions to DA

### Requirement: Report HTML never crosses the IPC boundary

The system MUST NOT return a report's markup to the renderer. Opening a report SHALL
fetch the assembled page from DA in the backend, register it with the bounded
preview-document store, and return only the content-addressed id, the page name, and the
list of page names. The viewer SHALL load that document from the preview origin.

An assembled page larger than the preview ceiling MUST be refused as too large to open,
never truncated.

There SHALL be one preview path in the app. The viewer reuses the contract the chat
preview card already honours rather than introducing a second one.

#### Scenario: Opening a report

- **WHEN** the reader opens a saved report
- **THEN** the call returns an opaque document id, the page name, and the page names, and
  no markup, rendering from its own non-app origin

#### Scenario: A multi-page report

- **GIVEN** a report with three pages
- **WHEN** the reader picks the second
- **THEN** that page opens under a new document id

#### Scenario: The document has no reach into the app

- **GIVEN** a report whose HTML attempts to read app storage or call the app's IPC
- **THEN** it cannot, because the frame is sandboxed without same-origin access

### Requirement: A report is named, pinned, renamed, and deleted from its list

The reader MUST be able to name a report and, when the account owns it, rename it, edit its
description, and delete it. Any reader MUST be able to pin and unpin a report for
themselves; a pin is the reader's own and changes nothing for anyone else.

The list SHALL show pinned reports first, then most recently updated, searchable over names
and descriptions. Pinning SHALL be expressed as intent.

The default view SHALL present three disjoint zones — pinned as cards, recent as cards,
the remainder in a table — each a filter over one ordering rule, never re-sorted
independently. The reader SHALL also be able to filter to pinned reports, choose a list
layout, and sort by last update, creation time or name without mutating the stored list.
Each report SHALL appear once in the current presentation.

Rename and delete controls MUST be disabled, with a stated reason, for a report the account
does not own. DA's refusal (HTTP 403) MUST surface as `Forbidden`.

Deleting MUST require explicit confirmation.

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
- **THEN** it lists with its owner's name, can be opened and pinned, and cannot be renamed
  or deleted

#### Scenario: Deleting

- **WHEN** the reader deletes a report
- **THEN** confirmation is required first

#### Scenario: An empty list says what to do

- **GIVEN** an account that has no reports
- **THEN** the surface says that reports are made by asking the assistant to save a dashboard

### Requirement: A report outlives the conversation it came from

The system MUST keep a report readable after its source conversation or message is deleted.
The source conversation id is DA's record and the surface does not depend on it.

#### Scenario: The source conversation is deleted

- **GIVEN** a saved report whose source conversation is deleted
- **THEN** the report still lists and opens

### Requirement: Desktop keeps no copy of a report

The system MUST keep no copy of a saved report's title, description, or document on this
device. The Reports surface reads DA. Clearing an account's local data MUST remove the
legacy `reports/` directory that earlier builds wrote, through its registered handler, and
MUST remove `local-projections/`, which holds the earlier reports projection.

The system MUST NOT delete, rewrite, or read the legacy store itself. Rows in it that
earlier builds never shipped stay on disk until that clear.

#### Scenario: Clearing local data

- **GIVEN** an account whose device holds the legacy reports directory
- **WHEN** the reader clears local data
- **THEN** the directory is gone and the clear is not partial

#### Scenario: No report content on this device

- **GIVEN** a report listed from DA
- **THEN** no file under the account's local data holds its title or document

### Requirement: A report must carry a name a reader can recognize it by

The system MUST require a non-empty name of at most 200 characters, with no control
characters, on rename and on a project save, trimmed of surrounding whitespace before it
is judged, and MUST refuse an unusable one as an invalid input before any request to DA.
An optional description is trimmed and at most 4000 characters; a longer one MUST be
refused rather than cut, and an empty one clears the description on DA.

An invalid name is an operation failure, not a save outcome.

#### Scenario: A blank name

- **WHEN** a report is renamed to a name that is empty or only whitespace
- **THEN** the rename is refused as an invalid name

#### Scenario: A name with surrounding space

- **WHEN** a report is renamed to a name padded by spaces
- **THEN** the stored name is the trimmed one

#### Scenario: A name with a line break

- **WHEN** a report is renamed to a name containing a newline
- **THEN** the rename is refused as an invalid name

#### Scenario: A description past the bound

- **WHEN** a rename carries a description longer than 4000 characters
- **THEN** it is refused, and the old description stands

### Requirement: An unknown report id and someone else's are the same answer

The system MUST fold three cases into one refusal: a report that does not exist, one the
account cannot see, and an id that is not well-formed (DA issues UUIDs; anything else is
refused before a request leaves the device). The refusal MUST be `not_found` and identical
across every id-scoped operation — open, rename, pin, delete, refresh.

A report the account can see but may not change is a different case and is not folded: DA
answers 403, which MUST surface as `forbidden` on rename, delete, and refresh. The account already
knows the report exists, so this discloses nothing.

#### Scenario: An id that was never issued

- **WHEN** a report is opened by an id no report carries
- **THEN** it is refused as not found

#### Scenario: A malformed id

- **WHEN** any id-scoped operation receives an id that is not a UUID
- **THEN** it is refused as not found without a request to DA

#### Scenario: Another account's private report

- **GIVEN** a private report owned by a different account
- **WHEN** it is opened
- **THEN** DA reports it as not found, indistinguishable from absent

#### Scenario: A shared report the account does not own

- **GIVEN** an organization-shared report owned by someone else
- **WHEN** the account renames or deletes it
- **THEN** the operation is refused as forbidden, and the report is unchanged

### Requirement: Saving a report grants no filesystem write authority

No command SHALL accept a path from the renderer, and the forbidden write commands
remain forbidden. No working download, export, or save-to-disk affordance SHALL be
offered for a report or for the chat document it came from: bytes move between stores
the app already owns, and no path crosses the boundary. The viewer's Export action is a
labeled preview that creates no file (see "Report action previews"). When DA exposes an
export, the file it returns SHALL be written by the backend through its own save path,
never to a path the renderer names.

The one place the app MAY write a report to disk is the trusted project that its source
conversation is bound to. That write MUST go through the same path as a person's own
save in the project file viewer, and the file name MUST be derived in the backend.

#### Scenario: The forbidden set is unchanged

- **GIVEN** this capability's commands
- **THEN** none of them takes a path

#### Scenario: No working download appears

- **GIVEN** an open report and an HTML preview card in chat
- **THEN** neither offers a working way to write the document to a location the reader
  picks

#### Scenario: A project chat writes only into its own project

- **GIVEN** a conversation bound to a trusted project
- **WHEN** a dashboard in it is saved
- **THEN** the only file written is inside that project's folder

### Requirement: Writing a report requires an active session

Listing, opening, renaming, pinning, unpinning, deleting, and refreshing a report each
MUST require a usable DA session, because each is a call to DA, and MUST be refused as
storage unavailable, before any request is sent, when the device has no signed-in
session or the session has ended. There is no offline copy to fall back on. Saving a
report into a project is local and needs only the signed-in account.

#### Scenario: Acting while signed out

- **GIVEN** no active account is signed in
- **WHEN** the reader renames, pins, deletes, or refreshes a report
- **THEN** the operation is refused and no request leaves the device

#### Scenario: Listing with an ended session

- **GIVEN** a session that has ended
- **WHEN** the reader opens Reports
- **THEN** the list fails to load, and offers Retry, rather than showing an empty library

#### Scenario: A project save needs no DA

- **GIVEN** a signed-in account and a conversation bound to a project
- **WHEN** a dashboard is saved to the project
- **THEN** it is written without any request to DA

### Requirement: A useful reporting workspace

The library SHALL display counts derived from the list, searchable report names and
descriptions, accessible layout and ordering controls, and a short instruction to ask the
assistant to save a dashboard in a chat. A count MUST NOT be shown while the list is not
known (loading, or a failed read). Metadata SHALL distinguish saved document timestamps
from business-data freshness. Report cards SHALL display a scaled preview of the actual
saved document through the isolated preview origin. Only visible cards SHALL load
previews. Thumbnail frames SHALL be inert, unfocusable, and sandboxed without
same-origin, top-navigation, popup or app privileges; document HTML MUST NOT cross IPC.
Unavailable previews SHALL be labeled without preventing report actions.

#### Scenario: Browse a large library

- **WHEN** the reader filters, searches, sorts or switches layouts
- **THEN** the visible reports follow all selected controls without duplicate report identities

#### Scenario: The assistant makes reports

- **WHEN** the reader visits Reports
- **THEN** the banner explains that the assistant saves reports from a chat, without New
  report or Sync library buttons

#### Scenario: Accessible responsive presentation

- **WHEN** the workspace uses a narrow window, dark theme or reduced motion
- **THEN** navigation and report actions remain operable with keyboard and pointer without page overflow

### Requirement: Report action previews

The viewer SHALL offer Schedule a refresh, Share via email and Export as previews. They
are placeholders for capabilities DA is expected to expose (scheduled refresh, sharing,
and export), so until it does they SHALL be explicitly labeled as demo mode and SHALL
perform no API calls, email delivery, scheduling, report mutation or file export.
Confirming valid input SHALL show a demo success message inside the dialog and a toast.
Refresh itself is not a preview: it is a real call to DA (see "A report refreshes on DA,
from this surface or from chat").

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

### Requirement: A project conversation saves reports into its project folder

The system MUST save a dashboard from a conversation bound to a project as an HTML file
in that project's folder, under `reports/`. Desktop has no server save: a report on DA is
made by the assistant with DA's tools, and the chat card never sends a document to DA.

The save MUST take the conversation, the message, the document's ordinal, and a name. It
MUST NOT take markup, a path, or a project id. The backend SHALL re-read the document
from the stored message and SHALL take the project from the conversation's own binding.

A conversation with no binding MUST be refused as an outcome (`not_in_project`), and its
card MUST NOT offer a save.

#### Scenario: A dashboard in a project chat

- **GIVEN** a conversation bound to a trusted project and a finished answer holding a
  fenced HTML document
- **WHEN** the reader names it "Q3 revenue" and saves it
- **THEN** `reports/Q3 revenue.html` exists in the project folder, holding the
  document's bytes as persisted, and no DA report is created

#### Scenario: An ordinary chat

- **GIVEN** a conversation with no project binding
- **THEN** the card offers no save, and a save command naming it is refused as not in a project

#### Scenario: The binding is gone

- **GIVEN** a project save naming a conversation that is not bound to a project
- **THEN** the save is refused as not in a project, and nothing is written anywhere

#### Scenario: The renderer cannot choose the folder

- **GIVEN** the project save command's inputs
- **THEN** none of them carries markup, a path, or a project id

## ADDED Requirements

### Requirement: Reports are created, changed, and deleted by the assistant through DA

The assistant MUST create, change, refresh, share, and delete saved reports by calling DA's
report tools, and the system prompt MUST say so: DA stores reports, the app offers no save
control for them, and the assistant makes the calls.

For a save, the prompt MUST specify the arguments DA needs: the page exactly as shown (it
begins with a document root marker and keeps its dataset carriers), a manifest holding the
`window.XF_MANIFEST` marker, definitions carrying each dataset's SQL, source, columns and
parameters, and no payload rows; and it MUST specify one refresh call after the save so DA
fills the data from the stored SQL. It MUST tell the assistant to look for an existing
report before rebuilding one the user names, to delete only after the user confirms, and to
send the user to the Reports page to open a saved report, since the app cannot render DA's
inline viewer.

#### Scenario: Asked to keep a dashboard

- **GIVEN** a dashboard the assistant built in this chat
- **WHEN** the reader asks to save it as a report
- **THEN** the assistant saves it with DA's save tool and refreshes it once, and the report
  appears in the Reports list

#### Scenario: Asked to keep it for the team

- **WHEN** the reader asks to share a saved dashboard with the organization
- **THEN** the assistant saves it with the organization scope, and the report lists with a
  shared label for its owner

### Requirement: A report moves between a project folder and DA through the assistant

In a conversation bound to a project, when the reader asks to bring a saved DA report into
the project, the assistant MUST recall it from DA and write its pages, manifest, and
definitions into the project's `reports/<name>/` folder with the project write tool, and
MUST tell the reader that the copy keeps the rows the pages were saved with and that DA
holds the live data. When the reader asks to put a report from the project on DA, the
assistant MUST read the file with the project read tool and save it with DA's save tool,
and MUST say so when the report has no query definitions and so cannot be refreshed.

#### Scenario: A kit is downloaded into the project

- **GIVEN** a project conversation and a saved report on DA
- **WHEN** the reader asks to download it into the project
- **THEN** the project's `reports/` folder holds the report's pages, manifest, and
  definitions, and the reply states what the copy contains

#### Scenario: A local report is uploaded to DA

- **GIVEN** an HTML report file in the project that has no query definitions
- **WHEN** the reader asks to put it on DA
- **THEN** it is saved to DA, and the reply says it is a static copy that cannot be refreshed

### Requirement: A report refreshes on DA, from this surface or from chat

The system MUST offer Refresh on an open report when DA states the viewer `can_refresh` it,
and MUST NOT offer it otherwise. Refresh SHALL ask DA to re-run the report's stored
datasets, SHALL NOT send SQL or rows from the device, and SHALL reload the page when DA
answers.

A dataset DA could not refresh MUST be named with DA's reason, and the report MUST keep its
last good data for that dataset. While the call runs the control MUST be disabled and
announce that it is working. A report refreshed in chat through DA's refresh tool MUST show
the new data the next time it is opened, because nothing is cached.

#### Scenario: A refresh that succeeds

- **GIVEN** an open report the reader can refresh
- **WHEN** the reader chooses Refresh
- **THEN** the page reloads with the new data

#### Scenario: One dataset fails

- **GIVEN** a report with two datasets, one whose source is unreachable
- **WHEN** the reader chooses Refresh
- **THEN** the failing dataset is named with DA's reason, and the other shows new data

#### Scenario: A report with no stored queries

- **GIVEN** a report DA saved without queries (a static copy)
- **WHEN** DA refuses the refresh because it has nothing to re-run
- **THEN** the surface says the report cannot be refreshed (`not_refreshable`), and the
  page is unchanged

#### Scenario: A report the viewer may not refresh

- **GIVEN** a shared report for which DA states `can_refresh` is false
- **THEN** no Refresh control is shown

#### Scenario: Refreshed in chat

- **GIVEN** a report refreshed by the assistant in a chat
- **WHEN** the reader opens it from Reports
- **THEN** it shows the refreshed data

### Requirement: The library shows dashboard kits and nothing else

The system MUST list only kits of kind `dashboard`. A workbook or presentation kit made
elsewhere MUST NOT appear. The list MUST read every page DA returns, up to a fixed page cap,
so a long library is not silently cut short. A library that cannot be read whole (the cap
runs out, DA promises more kits than it returns, or DA's total changes between pages or the kits read do
not add up to it, on every one of three passes, because the library kept changing) MUST be an error, never a
shorter list.

#### Scenario: A workbook kit exists

- **GIVEN** an account with one dashboard kit and one workbook kit
- **THEN** the library lists one report

#### Scenario: A library longer than one page

- **GIVEN** 250 dashboard kits
- **THEN** all 250 are listed

#### Scenario: A kit moves between two pages

- **GIVEN** a rename that moves a kit behind the cursor while the list is read
- **THEN** the list is read again, and the result holds every kit exactly once

#### Scenario: A library that never adds up

- **GIVEN** a library that changes under every pass
- **THEN** the read fails and the surface offers Retry, and no partial list is shown

### Requirement: A failed read says so; there is no stale copy

The system MUST show a loading, ready, or error state for the library. An error state MUST
offer Retry and MUST NOT show a previous list as though it were current.

#### Scenario: DA is unreachable

- **WHEN** the library is opened and DA cannot be reached
- **THEN** the surface says so and offers Retry, and lists no report

## REMOVED Requirements

### Requirement: A report that could not be opened is never saved

**Reason**: Desktop no longer stores reports. DA's limits on report count and size are DA's
and are stated by DA. A DA page larger than the preview ceiling is refused when opened
(`too_large_to_open`), not at save time.

**Migration**: None.

### Requirement: A cache row that cannot be decrypted is skipped, recorded, and repaired by the next reconcile

**Reason**: Desktop keeps no local copy of reports any more, so there is no row to decrypt.
DA owns the data.

**Migration**: None. A report that earlier builds saved only in the local store and never
shipped to DA is not shipped now (the route is gone) and is removed when local data is
cleared (see "A saved report is sealed under the account and cleared with it").

### Requirement: Reports read locally first and reconcile against the data plane

**Reason**: There is no local cache to read first. The list is read from DA each time, and a
failed read is an error with Retry, never an offline copy.

**Migration**: None.

### Requirement: Reports saved before this capability existed migrate to the data plane once

**Reason**: The route the migration shipped rows to is gone (DA replaced chat reports with
report kits and did not migrate its own data), so no row can be shipped now.

**Migration**: None. A report that earlier builds saved only locally is not readable in the
app and is removed when local data is cleared.

### Requirement: Saving the same document twice is a choice, not an error

**Reason**: The server save it described is gone. The project case is "Saving the same
document into a project twice is a choice", which stays as it is.

**Migration**: None.
