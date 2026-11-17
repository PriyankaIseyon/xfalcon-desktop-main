# reports — Spec Delta

## ADDED Requirements

### Requirement: A report is captured from the stored message, never from the renderer

The system MUST capture a report from a source identity — the conversation,
the message, and the ordinal of the HTML document within that message —
together with a name the user chose. The renderer MUST NOT send the
document's markup, and the backend MUST NOT accept it if sent.

The backend SHALL load the named message from the encrypted local
projection, re-scan its text for fenced HTML documents, and take the
document at the given ordinal. Re-reading from what was persisted is what
makes the capture path unusable for turning renderer-supplied markup into a
stored report.

Three conditions MUST refuse the capture, and MUST be reported as outcomes
of a successful call rather than as operation failures:

- the message does not belong to the named conversation,
- the message is not an assistant turn,
- the message holds no document at that ordinal.

#### Scenario: A dashboard is saved

- **GIVEN** an assistant message in the open conversation holding one
  fenced HTML document
- **WHEN** the user names it and saves it
- **THEN** a report is stored whose bytes are the document's bytes as they
  were persisted in that message

#### Scenario: A message from another conversation

- **GIVEN** a capture naming a message that belongs to a different
  conversation
- **THEN** the capture is refused as not found, telling the caller nothing
  about whether that message exists

#### Scenario: A message with no such document

- **GIVEN** a capture naming an ordinal past the last HTML document in the
  message
- **THEN** the capture is refused with that reason, and no row is written

#### Scenario: The renderer cannot supply the bytes

- **GIVEN** the capture command's inputs
- **THEN** none of them carries document markup, so there is nothing for a
  caller to substitute

### Requirement: Both fence scanners MUST agree on which document was saved

The system MUST count HTML documents inside a message identically in the
renderer, which supplies the ordinal, and in the backend, which resolves
it. Display-time detection stays in the frontend and resolution stays in
Rust, so two implementations exist and MUST be pinned to each other by a
shared fixture set exercised from both sides.

Divergence here is the worst failure this capability can have: the user
saves a document other than the one they clicked, and nothing in the
result says so.

#### Scenario: A message with several documents

- **GIVEN** a message holding three fenced HTML documents separated by prose
- **THEN** both scanners report the same three, in the same order

#### Scenario: The fixtures are shared, not duplicated

- **GIVEN** the fixture set of HTML-in-prose cases
- **THEN** the frontend test and the Rust test read the same file, so a case
  added for one is enforced on the other

### Requirement: Report HTML never crosses the IPC boundary

The system MUST NOT return a report's markup to the renderer. Opening a
report SHALL decrypt the snapshot in the backend, register it with the
bounded preview-document store, and return only the content-addressed id.
The viewer SHALL load that document from the `xf-preview` origin.

The viewer MUST use `sandbox="allow-scripts"` and MUST NOT set
`allow-same-origin`. It MUST NOT use `srcdoc`, `data:`, or `blob:` — under
WebView2 those inherit the app CSP, which refuses the document's own
styling, and a saved report rendered that way is permanently wrong rather
than momentarily wrong.

There SHALL be one preview path in the app. The viewer reuses the contract
the chat preview card already honours rather than introducing a second one
that could drift from it.

#### Scenario: Opening a report

- **WHEN** the user opens a saved report
- **THEN** the call returns an opaque document id and no markup, and the
  document renders from its own non-app origin

#### Scenario: The document has no reach into the app

- **GIVEN** a report whose HTML attempts to read app storage or call the
  app's IPC
- **THEN** it cannot, because the frame is sandboxed without same-origin
  access even on its own origin

### Requirement: The viewer re-opens a report rather than caching its document id

The system MUST resolve a report to a fresh preview document id on every
mount of the viewer. The preview store is in-memory and bounded, with
least-recently-served eviction, so an id is not durable: it can be evicted
by other documents and is gone after a restart.

A frame that fails to load MUST re-open once before reporting an error to
the user, because the overwhelmingly likely cause is an evicted id and not
a lost report.

#### Scenario: A report opened after a restart

- **GIVEN** a report that was open before the app restarted
- **WHEN** the user opens it again
- **THEN** it renders, because the viewer asked for a new document id
  instead of reusing a remembered one

#### Scenario: An evicted id

- **GIVEN** a document id the preview store has evicted
- **WHEN** the frame fails to load it
- **THEN** the report is re-opened once and renders, and only a second
  failure is shown to the user

### Requirement: A saved report is sealed under the account and cleared with it

The system MUST store reports at
`<app_local_data>/accounts/<account_id>/reports/reports.sqlite3`. Only what
ordering and deduplication need SHALL be plaintext — timestamps, byte size,
content hash, source identity, and the sealing metadata. The title,
description, and document SHALL live in one AEAD ciphertext under its own
authenticated-data family, following the encrypted chat projection.

Reports SHALL NOT be stored under `local-projections/`. That name means a
rebuildable cache of data the data plane still holds; a report is the only
copy of its bytes, and filing the only copy under a name that reads as
"disposable" invites the first disk-reclaim sweep to delete it.

Clearing an account's local data MUST remove every report. The
`ArtifactCache` resource SHALL be registered with a handler covering the
`reports/` directory. Leaving it unregistered fails twice over: the reports
survive a clear that claimed to remove them, and the post-handler prune
then finds the account root non-empty and fails, so every account that ever
saved a report reports a partial clear.

Clearing MUST succeed while the reports store is live. The store therefore
holds no connection between commands — an open handle blocks directory
removal on Windows, and a clear that fails because the app is still holding
its own file is not a failure the user can act on.

#### Scenario: Clearing local data

- **GIVEN** an account with saved reports
- **WHEN** the user clears local data
- **THEN** the reports directory is gone, the clear reports the artifact
  resource among those cleared, and the result is not partial

#### Scenario: Clearing while the store is in use

- **GIVEN** a reports store that has served a list and an open in this
  session
- **WHEN** local data is cleared
- **THEN** the removal still succeeds

#### Scenario: Nothing readable is left on disk

- **GIVEN** a stored report
- **THEN** its title and document are absent from the plaintext columns, so
  the file discloses only that a report of some size exists

### Requirement: A row that cannot be decrypted is skipped and recorded

The system MUST skip a report row it cannot decrypt when listing, rather
than failing the list — one unreadable row must not hide the other
twenty-nine. It MUST record that this happened.

This is not the chat projection's cache miss. There the row can be
re-fetched; here it was the only copy, so a silent skip would make a report
disappear with nothing anywhere saying why.

#### Scenario: One unreadable row

- **GIVEN** three stored reports, one of which cannot be decrypted
- **THEN** the list returns two and the third's loss is recorded

### Requirement: Saving a report grants no filesystem write authority

The system MUST NOT gain any ability to write a file as part of this
capability. No command SHALL accept a path from the renderer, and
`write_file` and `write_text_file` SHALL remain forbidden.

No download, export, or save-to-disk affordance SHALL be offered for a
report or for the chat document it came from. Save as report is the
replacement for that action: the bytes move between two stores the app
already owns, inside the account's own directory, and no path crosses the
boundary in either direction.

#### Scenario: The forbidden set is unchanged

- **GIVEN** this capability's six commands
- **THEN** the commands the renderer may never reach are exactly what they
  were before it, and none of the six takes a path

#### Scenario: No download button appears

- **GIVEN** an open report and an HTML preview card in chat
- **THEN** neither offers to write the document to disk

### Requirement: A report that could not be opened is never saved

The system MUST refuse to store a document larger than the preview store
will serve. The per-document ceiling SHALL equal the preview ceiling, so a
report that stores is a report that opens; a larger ceiling would accept
documents into a permanently unopenable row.

The system MUST cap the number of reports per account and the total bytes
they occupy. Both SHALL be checked in the same transaction as the insert,
so two saves racing cannot both pass a stale count.

Reaching a cap is an outcome, not an operation failure, and the refusal
MUST state the limit it hit so the surface can say what the user is up
against rather than that something went wrong.

#### Scenario: A document over the ceiling

- **GIVEN** a document larger than the preview store will serve
- **THEN** the capture is refused as too large, stating the maximum, and no
  row is written

#### Scenario: One past the count cap

- **GIVEN** an account already holding the maximum number of reports
- **WHEN** one more is captured
- **THEN** the capture is refused with the limit stated, and the existing
  reports are untouched

#### Scenario: The cap cannot be raced

- **GIVEN** two captures arriving at the cap boundary together
- **THEN** exactly one is stored

### Requirement: Saving the same document twice is a choice, not an error

The system MUST recognize a repeat save and return the report that already
holds those bytes, so the surface can offer to open the existing report or
save a copy without asking again.

A repeat SHALL be recognized either by the same source identity or by an
equal content hash. The content-hash arm carries the weight here:
regenerating a run produces a new message id for identical bytes, and
keying on identity alone would call that a new report.

#### Scenario: The same document saved twice

- **GIVEN** a report already saved from a message
- **WHEN** the same document is saved again
- **THEN** the result names the existing report, and the surface offers to
  open it or to save a copy

#### Scenario: Regenerated, byte-identical

- **GIVEN** a report saved from one run
- **WHEN** the run is regenerated and the identical document is saved from
  the new message
- **THEN** it is recognized as the same report, not stored twice

### Requirement: A report outlives the conversation it came from

The system MUST keep a report readable after its source conversation or
message is deleted. Source identity SHALL be recorded without a foreign key
and SHALL be presentable as absent, so deleting a conversation removes the
route back to the report's origin and nothing else.

A user who saved a dashboard has said it should outlast the chat. Losing it
when the chat is tidied up would make saving it pointless.

#### Scenario: The source conversation is deleted

- **GIVEN** a saved report whose source conversation is deleted
- **THEN** the report still lists and still opens, and its origin reads as
  unavailable rather than as an error

### Requirement: An unknown report id and someone else's are the same answer

The system MUST fold three cases into one refusal: a report that does not
exist, a report belonging to another account, and an id that is not
well-formed. Distinguishing them would let a caller learn which ids are
real by asking.

The refusal MUST be identical across every id-scoped operation — open,
rename, pin, delete — because a rename that answers differently from an
open discloses exactly what the fold exists to hide.

#### Scenario: An id that was never issued

- **WHEN** a report is opened by an id no report carries
- **THEN** it is refused as not found

#### Scenario: A malformed id

- **WHEN** a report is opened by an id that is not well-formed
- **THEN** it is refused as not found, not as a malformed input

#### Scenario: Another account's report

- **GIVEN** a report saved under a different account
- **WHEN** it is opened while that account is not active
- **THEN** it is refused as not found, indistinguishable from absent

### Requirement: Reports is a destination that names itself

The system MUST present reports as a top-level workspace destination,
reachable from the sidebar and from the command palette, and MUST mark it
as the current destination while it is open. A saved report is somewhere
the user returns to, not a configuration surface, so it belongs beside Chat
and Files rather than inside Settings.

Every workspace destination's titlebar name, accessible region name, and
subtitle MUST be resolved from mappings that are total over the set of
destinations. Today the titlebar name is a ternary chain whose final
else-branch is the files case, so a new destination compiles clean and
titles itself with the active project's name. That mapping SHALL be made
total before reports is added, so the shell cannot ship a destination that
answers to the wrong name.

An unwired destination MUST NOT appear in the command palette. A listed
command the build cannot reach is worse than an absent one.

#### Scenario: Opening reports from the sidebar

- **WHEN** the user selects Reports
- **THEN** the main region is the reports surface, is named Reports, and is
  marked as the current destination

#### Scenario: Every destination is named

- **GIVEN** the set of workspace destinations
- **THEN** each has its own titlebar name, and none falls back to another
  destination's

#### Scenario: Chat is not left mounted underneath

- **WHEN** reports is opened from chat
- **THEN** the chat surface is unmounted rather than hidden, so its layout
  cannot win over the hidden state

### Requirement: A report opens in the main region, not the artifact canvas

The system MUST render an opened report in the main region, as part of the
reports destination, and MUST leave the artifact canvas closed. The canvas
is a companion pane on a different axis: it coexists with whichever
destination is open, while a destination replaces the main region.

Hosting reports in the canvas would produce a destination with no
accessible region name, no subtitle, and no current-destination marking,
because all three are resolved from the open destination and the canvas has
no state that could name what it is showing.

#### Scenario: A report opens in place

- **WHEN** the user opens a report from the list
- **THEN** it renders in the main region and the canvas stays closed

#### Scenario: Returning to the list

- **GIVEN** an open report
- **WHEN** the user goes back
- **THEN** the list returns in the main region, still the reports
  destination throughout

### Requirement: A report is named, pinned, renamed, and deleted from its list

The system MUST let the user name a report when saving it, and afterwards
rename it, edit its description, pin and unpin it, and delete it. These
four are the whole action set.

The list SHALL show pinned reports first and then the most recently
updated, and SHALL be searchable over the names and descriptions it is
showing. Pinning SHALL be expressed as an intent — pinned or not — and
resolved to a timestamp in the backend, so two surfaces cannot disagree
about when something was pinned.

That order SHALL be presented in three disjoint zones, following
xfalcon-web: pinned reports as cards, the most recently updated few as
cards, and the remainder in a table. Disjoint matters — a report drawn in
two zones would carry two sets of controls for one row. The zones SHALL NOT
re-sort; each is a filter over the one ordering above, so the frontend keeps
a single ordering rule to hold in agreement with SQL. A report SHALL be
opened by activating its name, in either zone.

Deleting a report MUST require an explicit confirmation. It removes the only
copy of the document, and the conversation it came from may be long gone.

An action the user takes SHALL be reflected immediately and reverted if it
fails, so the list never shows a change that did not happen.

#### Scenario: Renaming

- **GIVEN** a saved report
- **WHEN** the user renames it
- **THEN** the list shows the new name, and a failure restores the old one

#### Scenario: Pinning

- **GIVEN** several reports
- **WHEN** one is pinned
- **THEN** it sorts ahead of the unpinned ones, and the pin time comes from
  the backend

#### Scenario: Deleting

- **WHEN** the user deletes a report
- **THEN** a confirmation is required first, and only then is the report
  gone

#### Scenario: An empty list says what to do

- **GIVEN** an account that has saved no reports
- **THEN** the surface says how one is made, rather than showing an empty
  container

### Requirement: A report must carry a name a user can recognize it by

The system MUST require a non-empty name on capture and on rename, MUST
bound its length, and MUST refuse an unusable one as an invalid input rather
than storing it. A name is the only handle the list offers: the document is
behind a frame, the description is optional, and the source conversation may
be gone — a blank row is a report the user cannot tell from any other.

Leading and trailing whitespace SHALL be trimmed before the name is judged,
so a name of spaces is refused rather than stored as one that renders empty.

An invalid name is an operation failure, not a capture outcome. The outcomes
describe what the app found when it looked for the document; this describes
input the caller can correct and resend.

#### Scenario: A blank name

- **WHEN** a report is saved with a name that is empty or only whitespace
- **THEN** the save is refused as an invalid name, and no row is written

#### Scenario: A name past the bound

- **WHEN** a report is renamed to a name longer than the bound
- **THEN** the rename is refused and the previous name stands

#### Scenario: A name with surrounding space

- **WHEN** a report is saved with a name padded by spaces
- **THEN** the stored name is the trimmed one, so the list and a later
  duplicate check see the same text the user typed

### Requirement: A report is identified by its id, never by its name

A report MUST be identified by its id, and two reports MAY carry the same
name. Nothing in the system keys a report by its title: the store's primary
key is `(account_id, report_id)`, every operation crosses the IPC boundary
carrying an id and nothing else, duplicate detection compares the document's
content hash and its source identity, and the title lives inside the
ciphertext where no index could constrain it. Saving a copy of an edited
dashboard is the ordinary way two same-named reports come to exist, so
refusing the second would refuse a legitimate act.

Because two rows may therefore read identically, every report-scoped
CONTROL MUST carry an accessible name that distinguishes which report it
acts on — the report's title together with the time that report last
changed. Duplicate visible text is merely a duplicate title; duplicate
control labels leave a screen-reader user choosing blind. A control whose
visible text is the report's name MUST contain that text verbatim in its
accessible name.

A naming dialog MAY tell the user that another report already carries the
name they typed, and SHALL do so only where that surface holds the whole
list and the answer is authoritative — a notice that appears only sometimes
misleads more than none. Such a notice MUST NOT block the operation.

#### Scenario: Two reports with one name

- **GIVEN** two saved reports carrying exactly the same name
- **WHEN** the user deletes one of them
- **THEN** that report's id crosses the boundary and only that row is
  removed, and the other is still readable

#### Scenario: Renaming onto a name in use

- **GIVEN** a report being renamed to a name another report already carries,
  differing only in case or surrounding space
- **THEN** the surface says the name is already used and the rename still
  proceeds, because titles carry no uniqueness constraint
