## ADDED Requirements

### Requirement: Large files use a datasets page, not the composer

The desktop SHALL provide a dedicated datasets surface for files that become
queryable tenant data. That surface MUST be reachable from the workspace, not
from the chat composer attach control.

Composer attach SHALL remain chat-attachment only. A file attached in chat
MUST NOT appear in the datasets catalog, and a dataset upload MUST NOT bind to
a chat message.

The composer's pick command SHALL NOT take an upload purpose. Datasets have
their own pick command, so a purpose parameter would carry exactly one legal
value.

#### Scenario: Composer and datasets stay separate

- **WHEN** the reader attaches a file in chat
- **THEN** it is a chat attachment on that conversation
- **AND** it does not appear as a dataset

#### Scenario: A dataset does not belong to a conversation

- **GIVEN** a dataset uploaded from the datasets page
- **WHEN** the reader opens any conversation
- **THEN** no message references that dataset as an attachment

### Requirement: Dataset uploads use the persistent-dataset pipeline

The datasets page SHALL pick a local file through the native picker in Rust
and upload it with `UploadPurpose::PersistentDataset` (presign, multipart when
DA requires it, finalize, abort on cancel).

The page SHALL accept files up to the persistent-dataset size ceiling already
enforced by the upload runtime (2 GiB). Files above that ceiling SHALL be
refused before any DA call.

Presign and finalize SHALL both carry the chosen worksheet index and header
row index when the reader chose one, and SHALL omit them otherwise so DA
applies its own defaults.

When a cleanup recipe was applied, both SHALL be omitted: the recipe already
selected the worksheet and consumed the header row, and what is uploaded is a
CSV whose first row is the header. Sending them then would make the server skip
a row of real data.

#### Scenario: A large spreadsheet uploads as a dataset

- **GIVEN** a signed-in session and a CSV under 2 GiB
- **WHEN** the reader uploads it from the datasets page
- **THEN** the transfer uses the persistent-dataset endpoints
- **AND** progress, cancel, and retry are visible on that page

#### Scenario: The chosen sheet reaches both calls

- **GIVEN** a workbook whose second worksheet the reader selected
- **WHEN** the upload runs
- **THEN** presign and finalize both carry that worksheet index
- **AND** they carry the same value

### Requirement: Extra-large transfers stay in Rust

The webview SHALL never receive a path, a presigned URL, or a storage key.
Bytes SHALL stream from disk in bounded parts. Cancelling a multipart upload
SHALL abort it server-side so abandoned parts are not billed.

#### Scenario: Cancel mid-transfer

- **GIVEN** a multipart dataset upload in flight
- **WHEN** the reader cancels it
- **THEN** the local transfer stops
- **AND** the server-side multipart upload is aborted

### Requirement: The workspace has a datasets destination

`Datasets` SHALL be a workspace destination with its own sidebar entry, its
own accessible region name, and its own titlebar name. The command palette
SHALL be able to reach it.

The destination's name MUST be a total map over the destination union, so a
new destination cannot compile without being named.

#### Scenario: Reaching datasets from the sidebar

- **WHEN** the reader activates the Datasets sidebar entry
- **THEN** the datasets destination is shown
- **AND** the entry is marked as the current page
- **AND** the titlebar names it

#### Scenario: Reaching datasets from the palette

- **WHEN** the reader opens the command palette and chooses the datasets command
- **THEN** the datasets destination is shown

### Requirement: A file is validated before any DA call

A picked or dropped file SHALL be refused locally, with a reason a person can
act on, when its extension is outside the accepted set, when its declared
extension disagrees with its content, or when it is encrypted.

A `.xlsx` MUST be verified to be an Office workbook — a zip whose entries
include the workbook part — and not merely a file beginning with a zip
signature. There MUST be no file size above which that check is skipped: an
archive's entry names are read by seeking rather than by loading it, so a
ceiling would only let the largest polyglot through. The number of entry names
read SHALL be bounded, because an archive can declare millions of them.

A password-protected workbook, which is a compound-document file rather than a
zip, MUST be refused with that stated as the reason.

No refusal at this stage may reach DA. The point of the check is that a
mistaken pick costs a message, not a transfer.

#### Scenario: A renamed file is refused

- **GIVEN** a PDF renamed to `report.xlsx`
- **WHEN** the reader picks it
- **THEN** it is refused before any network call
- **AND** the reason says the extension and the content disagree

#### Scenario: A password-protected workbook is refused

- **GIVEN** an encrypted `.xlsx`
- **WHEN** the reader picks it
- **THEN** it is refused
- **AND** the reason says the file is password-protected

### Requirement: A picked file is staged, and the webview holds only an opaque id

Picking a file SHALL stage it: the path stays in Rust, and the webview
receives an opaque staged id, a display name, a size, a format, and the
worksheet names when the file is a workbook.

Discarding a staged file SHALL release it. Staged files SHALL be bounded in
number, and SHALL not survive app exit.

Starting an upload SHALL name a staged id. There MUST be no command that
starts an upload from a path.

#### Scenario: Staging returns no path

- **WHEN** the reader picks a file on the datasets page
- **THEN** the webview receives an id, a name, a size, and a format
- **AND** it receives no path

#### Scenario: A discarded stage cannot be uploaded

- **GIVEN** a staged file the reader discarded
- **WHEN** an upload naming that staged id is requested
- **THEN** it is refused as unknown

### Requirement: The person sees the file before it is sent

Before any byte moves, the datasets surface SHALL show the staged file's first
rows, its column names, and the row and column counts sampled.

The preview MUST state when it truncated — rows, columns, or a cell — rather
than presenting a truncated sample as the whole file.

A file too large to parse for preview SHALL say so and remain uploadable
without one.

Preview cells SHALL carry the file's own lexical form, not a reformatted one.
Each column SHALL carry an inferred type for display.

The preview's columns SHALL be the columns the upload carries. A worksheet's
leading empty columns are outside its used range, so they are not columns of the
uploaded table either: showing them would offer a cleanup step naming a column
the upload does not have, which is refused only after the reader confirms.

#### Scenario: A CSV previews before upload

- **GIVEN** a staged CSV of 400,000 rows
- **WHEN** the confirm step opens
- **THEN** the first 50 rows are shown with their column names
- **AND** the surface states that the sample is truncated

#### Scenario: A wide sheet truncates its columns

- **GIVEN** a staged file with 4,000 columns
- **WHEN** the preview renders
- **THEN** at most the column cap is shown
- **AND** the surface states that columns were truncated

#### Scenario: A worksheet whose data starts past column A

- **GIVEN** a staged workbook whose block of data begins in column C
- **WHEN** the preview renders
- **THEN** its first column is the one in column C
- **AND** the cleanup editor offers no column the uploaded file lacks

#### Scenario: An unparseable file still uploads

- **GIVEN** a staged workbook above the preview parse ceiling
- **WHEN** the confirm step opens
- **THEN** it says no preview is available and why
- **AND** the upload control remains usable

### Requirement: The person chooses the worksheet and the header row

When a staged workbook has more than one worksheet, the surface SHALL let the
reader choose which one to ingest, and SHALL re-render the preview for that
choice.

The surface SHALL let the reader choose which sampled row holds the column
headers, and SHALL suggest one. Rows above the chosen header row SHALL be
excluded from the preview and from what is uploaded.

#### Scenario: A title row above the headers

- **GIVEN** a staged CSV whose first row is a report title and whose second row holds the column names
- **WHEN** the preview opens
- **THEN** the second row is suggested as the header row
- **AND** choosing it names the columns from that row

#### Scenario: Switching worksheets

- **GIVEN** a staged workbook with four worksheets
- **WHEN** the reader selects the third
- **THEN** the preview shows that worksheet's rows
- **AND** the upload carries that worksheet's index

### Requirement: The person can shape the data before it is sent

The surface SHALL accept an ordered recipe over the staged file supporting:
filter, sort, rename, cast, trim, upper, lower, replace, split, deduplicate,
remove nulls, and fill nulls. The recipe SHALL be bounded in length.

The recipe SHALL be applied by one implementation, used both to render the
preview and to produce what is uploaded. The preview MUST therefore show what
will be uploaded.

Both paths SHALL read the file the same way: the same delimiter, and every row
squared to the header's width before the first step runs. A step that addressed
a column present in one path and absent in the other would act on rows the
person never reviewed.

A step naming an unknown column, or carrying an unparseable parameter, SHALL
be refused with the step's position and the reason. It MUST NOT be silently
skipped: a dropped step means the uploaded data differs from the reviewed
preview with nothing on screen to say so.

A file above the recipe parse ceiling SHALL have the recipe controls
unavailable, with the ceiling stated. It remains uploadable unshaped.

When a recipe is applied, the transformed file SHALL be materialised locally,
uploaded, and then deleted — on success, on failure, on cancel, and when the
queue refuses the file outright.

Each materialised file SHALL occupy a location no other materialisation can
claim. Two files with one name would otherwise write one path, and the second
write would truncate a file the first transfer is still reading.

The staging location SHALL be emptied at launch. A transfer cannot outlive the
process that started it, so anything left there is a file a previous run did not
get to delete — and it holds the person's own data.

#### Scenario: The preview reflects the recipe

- **GIVEN** a staged CSV and a recipe that drops null rows and casts a column to number
- **WHEN** the preview renders
- **THEN** it shows the rows as transformed
- **AND** the uploaded file matches what was shown

#### Scenario: A step naming a missing column is refused

- **GIVEN** a recipe whose third step renames a column the file does not have
- **WHEN** the reader applies it
- **THEN** the surface names the third step and the missing column
- **AND** no upload starts

#### Scenario: A transformed file leaves no residue

- **GIVEN** a recipe upload that failed mid-transfer
- **WHEN** the failure is reported
- **THEN** the locally materialised file is gone

#### Scenario: Two files with one name

- **GIVEN** two staged files both named `sales.csv`, each with a recipe
- **WHEN** both are uploaded
- **THEN** each transfer reads its own materialised file

#### Scenario: A previous run's residue

- **GIVEN** a materialised file left behind by a run that was killed
- **WHEN** the app next launches
- **THEN** the staging location is empty

#### Scenario: A semicolon-separated export

- **GIVEN** a staged CSV whose values are separated by semicolons, and a recipe
- **WHEN** it is uploaded
- **THEN** the uploaded file holds the columns the preview showed

### Requirement: The catalog lists what is ready

The datasets surface SHALL list the account's datasets, walking DA's
pagination to exhaustion, and SHALL show only rows whose status is ready —
an interrupted upload leaves a row that is not a dataset a person can use.

The list SHALL be searchable by name and sortable by name and by upload time.

When the page cap is reached the list SHALL state that it is truncated rather
than presenting a partial list as complete.

Timestamps SHALL cross the boundary as epoch milliseconds, not as strings for
the renderer to parse.

#### Scenario: An interrupted upload is not listed

- **GIVEN** a dataset row DA reports as uploading
- **WHEN** the catalog loads
- **THEN** that row is not shown

#### Scenario: A long catalog says it is truncated

- **GIVEN** more datasets than the page cap can list
- **WHEN** the catalog loads
- **THEN** the surface states the list is truncated

### Requirement: A dataset's schema is inspectable

Opening a dataset SHALL show, for each of its files, the table name, the row
count, the column count, the size, the original filename, and the inferred
column types in the order the server returned them.

A count the server has not populated SHALL render as unknown, distinctly from
zero.

#### Scenario: Reading a dataset's columns

- **WHEN** the reader opens a dataset with one file of fourteen columns
- **THEN** each column's name and inferred type is shown
- **AND** the row and column counts are shown

#### Scenario: A legacy row with no counts

- **GIVEN** a dataset whose row count the server returns as null
- **WHEN** the reader opens it
- **THEN** the row count reads as unknown, not as zero

### Requirement: A dataset can be deleted, with its name typed to confirm

Deleting a dataset SHALL require the reader to type its name. Deletion SHALL
be refused until the typed name matches.

A successful delete SHALL remove the row from the catalog without a full
reload.

#### Scenario: Type-to-confirm

- **GIVEN** the delete dialog for a dataset
- **WHEN** the typed name does not match
- **THEN** the destructive control is unavailable

#### Scenario: A deleted dataset leaves the list

- **GIVEN** a confirmed delete
- **WHEN** it succeeds
- **THEN** the dataset is gone from the catalog

### Requirement: A dataset file downloads without the presigned URL entering the webview

Downloading a dataset file SHALL be requested by dataset id and file id. The
backend SHALL obtain the presigned URL, fetch it, and write the body to a
destination the reader chose in the native save dialog.

The webview MUST NOT receive the presigned URL, and the download command MUST
NOT take a destination path.

#### Scenario: Saving a dataset file

- **WHEN** the reader downloads a dataset file and chooses a location
- **THEN** the file is written there
- **AND** the webview never received the URL

#### Scenario: No renderer-chosen destination

- **WHEN** the download command is invoked
- **THEN** it accepts no path parameter

### Requirement: Uploads can be disabled by an administrator

The surface SHALL honour the server's uploads-enabled flag, treating its
absence as enabled so an older server is not read as disabled.

When uploads are disabled, the upload control SHALL be unavailable with the
reason shown, while existing datasets remain listed, inspectable, and
downloadable.

#### Scenario: A disabled source

- **GIVEN** a server reporting uploads as disabled
- **WHEN** the catalog loads
- **THEN** the upload control is unavailable with the reason shown
- **AND** the existing datasets are still listed

#### Scenario: An older server omits the flag

- **GIVEN** a server that does not return the flag
- **WHEN** the catalog loads
- **THEN** uploads are treated as enabled

### Requirement: Failures are named by code, never by DA's prose

Every catalog, staging, preview, and download failure SHALL be reported to the
webview as a closed set of codes, and the surface SHALL write the sentence for
each one. A transfer's own failure SHALL instead be reported through the upload
queue's existing safe-error field, written in the backend, because the queue
row is where a person reads it.

Both paths SHALL derive from the server's stable error code where one is
present and from the transport status otherwise, through one mapping — so
there is a single place that knows the server's vocabulary.

DA's own message text MUST NOT be forwarded verbatim on either path: it can
carry a URL, a storage key, or a bucket name.

An unrecognised server code SHALL be reported as a refusal rather than as an
unlabelled code, and every code the boundary can send MUST have copy.

#### Scenario: A too-large file server-side

- **GIVEN** DA refusing an upload with its file-too-large code, in prose that names a bucket
- **WHEN** the failure surfaces
- **THEN** the reader is told the file is too large for the workspace
- **AND** neither the bucket nor any other part of DA's message text appears

#### Scenario: An unknown code

- **GIVEN** DA returning a code this build does not know
- **WHEN** the failure surfaces
- **THEN** the reader is told the workspace refused it
- **AND** no raw code is shown as the message

#### Scenario: Every code has words

- **WHEN** the surface is asked to describe any code the boundary can send
- **THEN** it produces a sentence
- **AND** no code renders as nothing

### Requirement: Several datasets upload at once, and each can be cancelled or retried

The datasets surface SHALL show every persistent-dataset transfer in the
existing upload queue with its state and progress, and SHALL offer cancel
while active and retry after failure.

The surface MUST show only persistent-dataset transfers. Composer attachments
belong to their conversation.

The confirmation dialog SHALL stay open on the transfer it started and show
that transfer's state, progress, completion, and failure, and the surface's own
list SHALL omit that one transfer for as long as the dialog is showing it: the
person who pressed Upload watches it land where they pressed it, and one
transfer is never two bars.

Closing the dialog while its transfer is running MUST NOT end the transfer. The
bar returns to the surface's list. Only cancel ends a transfer.

A finished upload SHALL say so in the dialog and SHALL close it without being
asked, after long enough to read the result.

#### Scenario: Three transfers at once

- **GIVEN** three staged datasets started together
- **WHEN** the surface renders
- **THEN** each shows its own state and progress

#### Scenario: A chat attachment does not appear here

- **GIVEN** an attachment uploading in a conversation
- **WHEN** the datasets surface renders
- **THEN** that upload is not listed

#### Scenario: The dialog holds the transfer it started

- **GIVEN** a confirmed upload that is transferring
- **WHEN** the surface renders
- **THEN** the dialog shows that transfer's state and progress
- **AND** the surface's list does not show it a second time

#### Scenario: Closing the dialog does not end the upload

- **GIVEN** a transfer running under the dialog
- **WHEN** the dialog is closed
- **THEN** the transfer keeps running
- **AND** the surface's list shows it again

#### Scenario: A finished upload says so, then leaves

- **GIVEN** a transfer under the dialog that has finished
- **WHEN** the dialog renders
- **THEN** it says the dataset is ready to query
- **AND** it closes itself once the result has had time to be read

### Requirement: A dropped file is enqueued in Rust

Dropping files onto the window SHALL stage the files through the same
validation as the picker, with the drop handled in the backend.

The webview MUST NOT receive the dropped paths, and the backend MUST NOT ask
the webview anything about the drop — not even which destination is showing. It
stages or refuses a dropped path on its own and reports the outcome.

A staged drop SHALL reach the person: the shell navigates to the datasets
destination, which opens the confirmation step on it. A staged file the
renderer never showed would hold a staging slot with nothing able to claim it,
and would read as a drop that did nothing.

A refused drop SHALL be reported only while the datasets destination is
already showing. The window accepts drops for other reasons — a skill import
uses the same gesture — so a file that is not a dataset must not raise a
dataset error over an unrelated surface.

The number of files taken from one drop SHALL be bounded, and a drop carrying
more than the bound SHALL say so. Files taken silently would read as files
uploading.

Every staged file from one drop SHALL reach the confirmation step, one at a
time. One gesture can drop several files and the confirmation step holds one, so
the rest wait their turn rather than being replaced: a staged file the
confirmation step never reaches is one nobody can confirm or discard, and it
holds a staging slot until the app exits.

A refusal SHALL NOT wait behind files still being confirmed, and only the
newest SHALL be shown. A refusal is a sentence rather than a step, and one
shown after three confirmations names a gesture the reader has forgotten.

#### Scenario: Dropping a CSV

- **WHEN** the reader drops a CSV while the datasets destination is showing
- **THEN** it is staged exactly as a picked file is
- **AND** the confirmation step opens on it
- **AND** the webview never received its path

#### Scenario: Dropping a CSV while reading a conversation

- **WHEN** the reader drops a CSV while the chat destination is showing
- **THEN** the datasets destination opens
- **AND** the confirmation step opens on the staged file

#### Scenario: A dropped file that fails validation

- **WHEN** the reader drops a file whose extension is not accepted
- **THEN** nothing is staged
- **AND** the refusal is shown if the datasets destination is showing, and
  discarded if it is not

#### Scenario: Dropping several CSVs at once

- **WHEN** the reader drops three CSVs in one gesture
- **THEN** all three are staged
- **AND** the confirmation step opens on the first
- **AND** the second opens when the first is confirmed or cancelled, and the
  third when the second is
- **AND** no staged file is left with nothing able to claim it

#### Scenario: Dropping more files than one drop accepts

- **WHEN** the reader drops more files than the per-drop bound
- **THEN** the files up to the bound are staged
- **AND** one refusal reports the bound, rather than one per file past it
