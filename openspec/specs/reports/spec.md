# reports Specification

## Purpose

A report is a dashboard kept on DA: its pages, its datasets, and the SQL that refreshes
them. The assistant creates and changes reports with DA's tools; this capability lists,
opens, renames, pins, deletes, and refreshes them over DA's REST API, and saves a chat
dashboard into a project folder as a file. Desktop keeps no copy of a report, and the
renderer never supplies a report's bytes.

## Scope

| File                                          | What it owns here                                |
| --------------------------------------------- | ------------------------------------------------ |
| `src-tauri/src/clients/da/kits_api.rs`        | the DA report-kit REST client                    |
| `src-tauri/src/reports/kit_flow.rs`           | list, open, rename, pin, delete, refresh         |
| `src-tauri/src/reports/kit_view.rs`           | mapping DA kits to views, name and id checks     |
| `src-tauri/src/reports/capture.rs`            | scanning a stored message for its HTML documents |
| `src-tauri/src/reports/project.rs`            | saving a chat dashboard into a project folder    |
| `src-tauri/src/reports/commands.rs`           | the seven report commands                        |
| `src/features/reports/reportsStore.ts`        | client-side list state                           |
| `src/features/reports/ReportViewer.tsx`       | opening and refreshing via the preview origin    |
| `src/features/reports/SaveAsReportAction.tsx` | the chat card's "Save to project" action         |
| `src/features/reports/ReportsView.tsx`        | the destination surface                          |

## Boundaries

- **The preview origin the viewer loads from** belongs to `preview-protocol`.
- **The detection ladder both scanners must agree on** belongs to `chat-rich-content`,
  which owns the frontend scanner; this capability owns only the Rust-side resolver
  and the fixture pin between them.
- **The workspace destination shell** — titlebar naming, current-destination marking —
  belongs to `app-shell-material`.
- **Clearing local data** belongs to `encrypted-local-store`.

## Requirements

### Requirement: A report is captured from the stored message, never from the renderer

The system MUST capture a report from a source identity — the conversation, the
message, and the ordinal of the HTML document within that message — together with a
name the reader chose. The renderer MUST NOT send the document's markup, and the
backend MUST NOT accept it if sent.

The backend SHALL load the named message from the encrypted local projection,
re-scan its text for fenced HTML documents, and take the document at the given
ordinal. Re-reading from what was persisted is what makes the capture path unusable
for turning renderer-supplied markup into a stored report. Once resolved, the
document SHALL be submitted to the DA data plane as the durable copy of record; the
renderer's inputs remain limited to identity and name regardless of where the bytes
are ultimately stored.

Three conditions MUST refuse the capture, reported as outcomes of a successful call
rather than as operation failures: the message does not belong to the named
conversation, the message is not an assistant turn, the message holds no document at
that ordinal.

#### Scenario: A dashboard is saved

- **GIVEN** an assistant message in the open conversation holding one fenced HTML
  document
- **WHEN** the reader names it and saves it
- **THEN** a report is stored on the data plane whose bytes are the document's bytes
  as persisted, and it is readable from this device immediately

#### Scenario: A message from another conversation

- **GIVEN** a capture naming a message that belongs to a different conversation
- **THEN** the capture is refused as not found, telling the caller nothing about
  whether that message exists

#### Scenario: A message with no such document

- **GIVEN** a capture naming an ordinal past the last HTML document in the message
- **THEN** the capture is refused with that reason, and no row is written

#### Scenario: The renderer cannot supply the bytes

- **GIVEN** the capture command's inputs
- **THEN** none of them carries document markup

### Requirement: Both fence scanners MUST agree on which document was saved

The system MUST count HTML documents inside a message identically in the renderer,
which supplies the ordinal, and in the backend, which resolves it. Display-time
detection stays in the frontend and resolution stays in Rust, so two implementations
exist and MUST be pinned to each other by a shared fixture set exercised from both
sides.

Divergence is the worst failure this capability can have: the reader saves a document
other than the one they clicked, and nothing in the result says so.

#### Scenario: A message with several documents

- **GIVEN** a message holding three fenced HTML documents separated by prose
- **THEN** both scanners report the same three, in the same order

#### Scenario: The fixtures are shared, not duplicated

- **GIVEN** the fixture set of HTML-in-prose cases
- **THEN** the frontend test and the Rust test read the same file

### Requirement: Report HTML never crosses the IPC boundary

The system MUST NOT return a report's markup to the renderer. Opening a report SHALL
decrypt the snapshot in the backend, register it with the bounded preview-document
store, and return only the content-addressed id. The viewer SHALL load that document
from the preview origin.

There SHALL be one preview path in the app. The viewer reuses the contract the chat
preview card already honours rather than introducing a second one.

#### Scenario: Opening a report

- **WHEN** the reader opens a saved report
- **THEN** the call returns an opaque document id and no markup, rendering from its
  own non-app origin

#### Scenario: The document has no reach into the app

- **GIVEN** a report whose HTML attempts to read app storage or call the app's IPC
- **THEN** it cannot, because the frame is sandboxed without same-origin access

### Requirement: The viewer re-opens a report rather than caching its document id

The system MUST resolve a report to a fresh preview document id on every mount of the
viewer. The preview store is in-memory and bounded with least-recently-served
eviction, so an id is not durable.

A frame that fails to load MUST re-open once before reporting an error, because the
overwhelmingly likely cause is an evicted id and not a lost report.

#### Scenario: A report opened after a restart

- **GIVEN** a report open before the app restarted
- **WHEN** the reader opens it again
- **THEN** it renders, because the viewer asked for a new document id

#### Scenario: An evicted id

- **GIVEN** a document id the preview store has evicted
- **WHEN** the frame fails to load it
- **THEN** the report is re-opened once and renders, and only a second failure is
  shown

### Requirement: A saved report is cached locally, sealed under the account, and cleared with it

The DA data plane is the copy of record. A local encrypted cache MAY hold a
report's rows, including its document bytes once fetched, so that reads work
offline — the same local-first shape `chat-conversation-open`'s cache already
has. Only what ordering and dedup need SHALL be plaintext in the cache;
title, description, and document SHALL live in one AEAD ciphertext under its
own authenticated-data family, following the encrypted chat projection.

Clearing an account's local data MUST remove the cache. This clears what this
device holds; it does not delete the account's reports from the data plane —
"clear local data" and "delete a report" remain two different actions with
two different scopes, and clearing local data MUST NOT be capable of deleting
anything the data plane would otherwise still answer for after the next sign-in.

#### Scenario: Clearing local data

- **GIVEN** an account with cached reports
- **WHEN** the reader clears local data
- **THEN** the cache is gone, and the clear is not partial

#### Scenario: Reports survive a local clear

- **GIVEN** an account that cleared its local data
- **WHEN** the reader signs in again and opens Reports
- **THEN** every report the data plane holds for that account is listed again

#### Scenario: Nothing readable is left on disk

- **GIVEN** a cached report
- **THEN** its title and document are absent from the plaintext columns

### Requirement: A cache row that cannot be decrypted is skipped, recorded, and repaired by the next reconcile

The system MUST skip a cache row it cannot decrypt when listing, rather than
failing the list, and MUST record that this happened. Unlike the store this
capability used before this change, an undecryptable row is not a lost report
— the data plane still holds it, and the next successful reconcile
overwrites the row with a freshly sealed one.

#### Scenario: One unreadable cache row

- **GIVEN** three cached reports, one of whose rows cannot be decrypted
- **THEN** the list returns two immediately and the third is repaired on the
  next successful reconcile with the data plane

#### Scenario: What the reader is told about an unreadable row

- **WHEN** the list reports rows it could not decrypt
- **THEN** the surface says they return on the next sync, and MUST NOT
  describe them as lost or their saved copies as unavailable
- **AND** where no row could be read at all, the surface offers the sync that
  repairs them

### Requirement: Saving a report grants no filesystem write authority

No command SHALL accept a path from the renderer, and the forbidden write commands
remain forbidden. No download, export, or save-to-disk affordance SHALL be offered for
a report or for the chat document it came from — save-as-report is the replacement:
bytes move between stores the app already owns, no path crosses the boundary.

The one place a report MAY be written to disk is the trusted project that its source
conversation is bound to. That write MUST go through the same path as a person's own
save in the project file viewer, and the file name MUST be derived in the backend.

#### Scenario: The forbidden set is unchanged

- **GIVEN** this capability's commands
- **THEN** none of them takes a path

#### Scenario: No download button appears

- **GIVEN** an open report and an HTML preview card in chat
- **THEN** neither offers to write the document to a location the reader picks

#### Scenario: A project chat writes only into its own project

- **GIVEN** a conversation bound to a trusted project
- **WHEN** a dashboard in it is saved
- **THEN** the only file written is inside that project's folder

### Requirement: A report that could not be opened is never saved

The system MUST refuse to store a document larger than the preview store will serve —
the per-document ceiling SHALL equal the preview ceiling, so a report that stores is a
report that opens. This ceiling is enforced locally regardless of what the data
plane's own limit is, because a document under the data plane's ceiling but over
this one would store there and then be permanently unopenable here.

The per-account count and total-bytes limits are the data plane's to enforce; the
system MUST surface its answer as a capture outcome stating the limit reached, not
attempt an independent local count.

#### Scenario: A document over the ceiling

- **GIVEN** a document larger than the preview store will serve
- **THEN** the capture is refused as too large, stating the maximum, before any
  network call is made

#### Scenario: The data plane's cap is reached

- **GIVEN** an account already at the data plane's maximum number of reports
- **WHEN** one more is captured
- **THEN** the capture is refused with the stated limit, existing reports untouched

### Requirement: Saving the same document twice is a choice, not an error

The system MUST recognize a repeat save — by the same source identity or by an equal
content hash — and return the report that already holds those bytes, offering to open
it or save a copy.

The content-hash arm carries the weight: regenerating a run produces a new message id
for identical bytes, and keying on identity alone would call that a new report.

#### Scenario: The same document saved twice

- **GIVEN** a report already saved from a message
- **WHEN** the same document is saved again
- **THEN** the result names the existing report, offering to open it or save a copy

#### Scenario: Regenerated, byte-identical

- **GIVEN** a report saved from one run
- **WHEN** the run is regenerated and the identical document is saved from the new
  message
- **THEN** it is recognized as the same report, not stored twice

### Requirement: A report outlives the conversation it came from

The system MUST keep a report readable after its source conversation or message is
deleted. Source identity SHALL be recorded without a foreign key and SHALL be
presentable as absent.

#### Scenario: The source conversation is deleted

- **GIVEN** a saved report whose source conversation is deleted
- **THEN** the report still lists and opens, its origin reading as unavailable

### Requirement: An unknown report id and someone else's are the same answer

The system MUST fold three cases into one refusal: a report that does not exist, one
belonging to another account, and an id that is not well-formed. The refusal MUST be
identical across every id-scoped operation — open, rename, pin, delete.

#### Scenario: An id that was never issued

- **WHEN** a report is opened by an id no report carries
- **THEN** it is refused as not found

#### Scenario: Another account's report

- **GIVEN** a report saved under a different account
- **WHEN** it is opened while that account is not active
- **THEN** it is refused as not found, indistinguishable from absent

### Requirement: Reports is a destination that names itself

The system MUST present reports as a top-level workspace destination, reachable from
the sidebar and the command palette, marked current while open. Every destination's
titlebar name, region name, and subtitle MUST be resolved from a mapping total over the
set of destinations — no destination may fall back to another's name.

An unwired destination MUST NOT appear in the command palette.

#### Scenario: Opening reports from the sidebar

- **WHEN** the reader selects Reports
- **THEN** the main region is the reports surface, named Reports, marked current

#### Scenario: Chat is not left mounted underneath

- **WHEN** reports is opened from chat
- **THEN** the chat surface is unmounted rather than hidden

### Requirement: A report opens in the main region, not the artifact canvas

The system MUST render an opened report in the main region as part of the reports
destination, leaving the artifact canvas closed. The canvas coexists with whichever
destination is open; a destination replaces the main region.

#### Scenario: A report opens in place

- **WHEN** the reader opens a report from the list
- **THEN** it renders in the main region and the canvas stays closed

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

### Requirement: A report must carry a name a reader can recognize it by

The system MUST require a non-empty, length-bounded name on capture and rename, trimmed
of surrounding whitespace before it is judged, refusing an unusable one as an invalid
input rather than storing it.

An invalid name is an operation failure, not a capture outcome.

#### Scenario: A blank name

- **WHEN** a report is saved with a name that is empty or only whitespace
- **THEN** the save is refused as an invalid name

#### Scenario: A name with surrounding space

- **WHEN** a report is saved with a name padded by spaces
- **THEN** the stored name is the trimmed one

### Requirement: A report is identified by its id, never by its name

Every report-scoped control MUST carry an accessible name that distinguishes which
report it acts on — the title together with its last-changed time. Two reports MAY
carry the same name; nothing keys a report by its title.

A naming dialog MAY note that another report already carries the typed name only where
that surface holds the whole list, and MUST NOT block the operation.

#### Scenario: Two reports with one name

- **GIVEN** two saved reports carrying exactly the same name
- **WHEN** the reader deletes one
- **THEN** only that row is removed and the other is still readable

#### Scenario: Renaming onto a name in use

- **GIVEN** a rename onto a name another report carries
- **THEN** the surface notes it and the rename still proceeds

### Requirement: Reports read locally first and reconcile against the data plane

The system MUST serve a report list from the local cache immediately and
reconcile it against the data plane in the same call, following the pattern
`chat-conversation-open` already established: a successful reconcile replaces
the cache and reports the list as current; a failed reconcile against a cache
that has synced before serves the cached rows and reports the list as an
offline copy; a failed reconcile with no prior successful sync has nothing to
fall back to and fails outright rather than reporting an empty list as
current. An unauthorized answer from the data plane MUST always be reported
as a failure, never folded into the offline-copy case.

#### Scenario: A normal list

- **WHEN** the reader opens Reports with a working connection
- **THEN** the list reflects the data plane and is not marked as an offline copy

#### Scenario: Listing while offline, previously synced

- **GIVEN** a device that has successfully listed reports before
- **WHEN** the reader opens Reports without a connection
- **THEN** the previously cached list is shown, marked as an offline copy

#### Scenario: Listing while offline, never synced

- **GIVEN** a device that has never successfully listed reports
- **WHEN** the reader opens Reports without a connection
- **THEN** the list fails to load rather than showing as empty and current

#### Scenario: The session is rejected

- **WHEN** the data plane answers a list request as unauthorized
- **THEN** the list fails to load rather than falling back to a cached copy

### Requirement: Writing a report requires an active session

Capturing, renaming, pinning, unpinning, and deleting a report all MUST
require a usable data-plane session, refused clearly when one is not
available — the same precedent chat's own message-send already set: reads
work offline, writes do not.

Where a list is served as an offline copy, that is already known before the
reader acts, so the surface MUST present those writes as unavailable rather
than offer them and fail. The reason MUST be stated once where the list is
read, in a form a screen reader receives. A write that was already begun when
the list became an offline copy MUST be refused with what was kept intact,
not retried into the same failure.

#### Scenario: Acting on an offline copy

- **GIVEN** a list served as an offline copy
- **WHEN** the reader looks at a report's actions
- **THEN** renaming, pinning and deleting are unavailable, and the list states
  why in a form a screen reader receives

#### Scenario: A write already open when the list goes stale

- **GIVEN** a rename or delete begun while the list was current
- **WHEN** the list becomes an offline copy before the write is sent
- **THEN** the write is refused, stating what was kept, and nothing is sent

#### Scenario: Capturing while signed out

- **GIVEN** no active account is signed in
- **WHEN** the reader tries to save a report
- **THEN** the save is refused with a message naming that a sign-in is
  needed, not a generic storage error

#### Scenario: Renaming with an expired session

- **GIVEN** a session that has expired
- **WHEN** the reader renames a report
- **THEN** the rename is refused, stating the session expired

### Requirement: Reports saved before this capability existed migrate to the data plane once

The system MUST, on the first list after upgrading an account that has
reports saved under the earlier local-only storage, submit each one it can
still read to the data plane, and MUST NOT repeat this migration on a later
list.

The earlier store holds the only copy of every row in it, so the system MUST
NOT remove it except when every row was confirmed accepted by the data plane.
A row that failed for a reason a retry could resolve MUST leave the migration
unrecorded, so the next list attempts it again; a resubmission MUST NOT create
a second copy of a report that already crossed. A row that no retry could ever
resolve MUST complete the migration without removing the store — its bytes
must not be destroyed in order to record that nothing could be done with them.

#### Scenario: An account with pre-existing local reports

- **GIVEN** an account with reports saved before this capability existed
- **WHEN** the reader opens Reports for the first time after upgrading
- **THEN** each readable report is submitted to the data plane, and it is
  visible there afterward
- **AND** the earlier store is removed only once every row has crossed

#### Scenario: The data plane is unreachable mid-migration

- **GIVEN** an account whose migration cannot reach the data plane
- **WHEN** the reader opens Reports
- **THEN** the earlier store is kept and the migration stays unrecorded
- **AND** the next list retries it without duplicating anything already sent

#### Scenario: A row nothing can resolve

- **GIVEN** a stored row that names no conversation the data plane knows
- **WHEN** the migration runs
- **THEN** the migration completes and does not run again
- **AND** the earlier store is kept, because it holds the only copy

#### Scenario: Migration runs exactly once

- **GIVEN** an account whose migration has already completed
- **WHEN** Reports is opened again
- **THEN** nothing already migrated is submitted a second time

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

### Requirement: A project conversation saves reports into its project folder

The system MUST save a dashboard from a conversation bound to a project as an HTML file
in that project's folder, under `reports/`, and MUST NOT send it to the DA reports
store. A conversation with no project binding MUST keep the server flow unchanged.

The capture MUST take the same identity as the server flow — the conversation, the
message, and the document's ordinal — and a name. It MUST NOT take markup, a path, or a
project id. The backend SHALL re-read the document from the stored message and SHALL
take the project from the conversation's own binding.

A conversation with no binding MUST be refused as an outcome, never saved to the server
in its place.

#### Scenario: A dashboard in a project chat

- **GIVEN** a conversation bound to a trusted project and a finished answer holding a
  fenced HTML document
- **WHEN** the reader names it "Q3 revenue" and saves it
- **THEN** `reports/Q3 revenue.html` exists in the project folder, holding the
  document's bytes as persisted, and no DA report is created

#### Scenario: An ordinary chat

- **GIVEN** a conversation with no project binding
- **WHEN** the reader saves a dashboard
- **THEN** it is saved to the DA reports store, exactly as before

#### Scenario: A project chat reaches the server save

- **GIVEN** a conversation bound to a project, whose card offered the server save
  because the shell had not listed the project yet
- **WHEN** the reader saves a dashboard
- **THEN** the server save is refused as in a project, and no DA report is created

#### Scenario: The binding is gone

- **GIVEN** a project save naming a conversation that is not bound to a project
- **THEN** the save is refused as not in a project, and nothing is written anywhere

#### Scenario: The renderer cannot choose the folder

- **GIVEN** the project save command's inputs
- **THEN** none of them carries markup, a path, or a project id

### Requirement: A project report is written the way a viewer save is written

The system MUST write a project report through the one path a person's own file-viewer
save uses: a plan proposed against the current trusted root, journaled, audited,
containment re-checked before anything is touched, and written atomically.

The operation MUST be a create. A file that exists at the chosen path MUST NOT be
overwritten, including one that appeared after the name was chosen.

A folder that cannot be reached MUST fail the save with a reason, and MUST NOT fall
back to the server.

#### Scenario: The reports folder does not exist yet

- **GIVEN** a project with no `reports` folder
- **WHEN** the first report is saved
- **THEN** the folder is created under containment and the file is written inside it

#### Scenario: The project folder is unavailable

- **GIVEN** a bound project whose folder cannot be reopened
- **WHEN** a dashboard is saved
- **THEN** the save fails as project unavailable and nothing is written

### Requirement: A project report's file name is derived from its name

The system MUST derive the file name from the report name in the backend. Characters
that no supported platform accepts in a file name MUST be replaced, and the result MUST
pass the project path policy. A name that cannot become a valid file name MUST fall
back to `Report`.

A file name already used in the `reports` folder, compared without regard to case, MUST
get a numbered suffix — `Name (2).html`, then `Name (3).html` — rather than replace the
existing file.

#### Scenario: A name with separators

- **WHEN** a report is saved as `Revenue: EMEA/APAC`
- **THEN** the file is `reports/Revenue- EMEA-APAC.html`

#### Scenario: A name already in use

- **GIVEN** `reports/Pipeline.html` holding a different document
- **WHEN** another document is saved as `pipeline`
- **THEN** it is written as `reports/pipeline (2).html` and the first file is untouched

### Requirement: Saving the same document into a project twice is a choice

The system MUST recognise a `.html` file in the project's `reports` folder whose bytes
equal the document's, and MUST return it as a duplicate naming that file, so the reader
can open it or save a copy. Saving a copy MUST write a new file.

Only regular files that are not classified as sensitive, and whose size equals the
document's, SHALL be read for the comparison.

#### Scenario: The same dashboard saved twice

- **GIVEN** a dashboard already saved into the project
- **WHEN** the same document is saved again
- **THEN** the result names the existing file, and nothing is written

#### Scenario: Saving a copy on purpose

- **GIVEN** that duplicate result
- **WHEN** the reader chooses to save a copy
- **THEN** a second file with a numbered name is written

### Requirement: A report saved into a project opens in the project viewer

The system MUST refuse to save into a project a document larger than the project write
path accepts in one file (64 MiB), stating the ceiling. Within that bound a saved report
opens in the project file view (`add-large-local-project-io`), which previews HTML up to
32 MiB and shows a larger document as its source.

#### Scenario: A document over the project ceiling

- **GIVEN** a document larger than the project write path accepts
- **WHEN** it is saved into a project
- **THEN** the save is refused as too large, stating the maximum, and nothing is written

## Provenance

From the delta of `chat-reports-tab`, whose `reports` requirements were already
written at code grade, then amended by `migrate-reports-to-data-plane`,
`update-reports-workspace`, `add-project-report-destination`, and
`migrate-reports-to-report-kits`.
