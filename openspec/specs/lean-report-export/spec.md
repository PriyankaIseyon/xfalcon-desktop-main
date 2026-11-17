# lean-report-export Specification

## Purpose

How a document leaves this app: the commands that can write outside app-local data,
and the two destinations a conversation can be exported to.

The governing rule is that a caller may ask for a write but may never name where it
lands. The renderer sends bytes (or, for a PDF, only a suggested name); the native
dialog opens on the Rust side and the path comes back from the OS.

## Scope

| File                                           | What it owns here                                                    |
| ---------------------------------------------- | -------------------------------------------------------------------- |
| `src-tauri/src/export/mod.rs`                  | the save command, the dialog, the extension re-assertion             |
| `src-tauri/src/export/mod.rs:37`               | `MAX_DOCUMENT_BYTES`                                                 |
| `src-tauri/src/export/mod.rs:42`               | `MAX_BASE_NAME_CHARS`                                                |
| `src-tauri/src/export/mod.rs:48`               | `RESERVED_BASE_NAMES`                                                |
| `src-tauri/src/export/mod.rs:194`              | `safe_file_name`                                                     |
| `src-tauri/src/export/mod.rs:162`              | `with_expected_extension`                                            |
| `crates/contracts/src/export.rs:32`            | `DocumentExportFormat` and its extensions                            |
| `crates/contracts/src/export.rs:69`            | `DocumentExportResult` — a cancelled save is a success               |
| `src/features/export/exportDocument.ts`        | composing the document, and the one selection both destinations read |
| `src/features/export/PrintDocument.tsx`        | the print sheet, mounted beside the app root                         |
| `src/features/export/ExportDocumentDialog.tsx` | the two formats and the selection UI                                 |
| `src-tauri/src/export/pdf.rs`                  | Windows WebView2 `PrintToPdf` (no print preview)                     |

## Boundaries

- **The transcript this document is composed from** belongs to `chat-thread-layout` and
  `conversation-projection`. This capability reads turns and composes; it does not
  render the conversation.
- **A grid's CSV and a chart's vector file** are composed by
  `dynamic-canvas-artifacts` and written through this capability's one command.
- **A saved report's own store** belongs to `reports`; a report is not a file.
- **Forbidden filesystem commands** belong to `webview-capability-lockdown`, which is
  what makes this command the only writer.
- **Locale-aware timestamp formatting** is out of scope; the export timestamp is
  deliberately fixed-format and non-locale, and `add-locale-aware-formatting` leaves it
  alone.

## Requirements

### Requirement: One command writes every file, and never chooses where

The system MUST expose no command that accepts a destination path. Bytes the
renderer already holds go through `document_export_save`. A PDF of the mounted
print sheet goes through `document_export_pdf`, which takes only a suggested name
and is written by the webview print engine. A renderer that has been turned
against the reader can therefore ask for a write, but cannot choose where the
write lands: a person picks the file first, every time. There is no silent write
and no way to accumulate one from repeated calls.

Caller-supplied files — an exported Markdown conversation, a grid's rows, a
chart's image, a model-authored document — MUST stay on `document_export_save`.
A per-surface writer of those bytes would be a per-surface chance for this rule
to be weaker.

A caller MUST NOT write a file through an anchor with a download attribute. That path has
no destination step at all, which is exactly what this command replaces.

A dismissed dialog MUST come back as a successful call whose answer is that nothing was
written, never as an error. A caller that reported an error there would accuse the reader
of a fault they did not commit.

A document over the size ceiling MUST be refused rather than written. Every message is
already capped on the way into the projection and the renderer composes from that, so a
body over the ceiling is a bug or an attempt to use the export path as a general
file-writing primitive.

An empty body MUST be refused.

#### Scenario: The renderer cannot name a path

- **WHEN** either export command's arguments are inspected
- **THEN** they carry a suggested name and no path, and the bytes command also
  carries bytes and a format

#### Scenario: A dismissed dialog is not a failure

- **WHEN** the reader cancels the save dialog
- **THEN** the call succeeds, reports that nothing was written, and names no file

#### Scenario: An oversized body is refused

- **GIVEN** a composed document larger than the ceiling
- **THEN** the save is refused and nothing is written

#### Scenario: One command, several surfaces

- **WHEN** a conversation export, a grid's rows, and a chart are saved
- **THEN** all three go through the same command

### Requirement: The suggested name is untrusted, and the extension is the app's to assert

The suggested file name MUST be reduced to a plain base name before it reaches the OS
dialog. It seeds a control the OS renders, and it arrives from a conversation title the
model may have written.

Reduction MUST remove path separators and traversal, control characters, and wildcard
characters, and MUST fall back to a fixed name when nothing usable remains.

A base name that a platform reserves MUST fall back, whatever its case, because such a
name cannot be used at any extension.

The name MUST be capped in characters without splitting a character, and the cap MUST
count any separator the reduction itself inserts.

Whatever the person then types is the OS's business — except the extension, which the app
MUST re-assert, so a document promised as one format cannot be written under another's
suffix. Each format has exactly one extension.

The name reported back MUST be the name that was actually written, not the one that was
requested.

#### Scenario: A title becomes a plain base name

- **GIVEN** a conversation title with punctuation and spaces
- **THEN** the dialog opens on a plain base name the reader recognizes

#### Scenario: Separators and traversal cannot survive a title

- **GIVEN** a title containing path separators and parent-directory segments
- **THEN** none of them reach the suggested name

#### Scenario: A reserved name falls back

- **GIVEN** a title that reduces to a platform-reserved device name, in any case
- **THEN** the fallback name is used

#### Scenario: Each format asserts its own extension

- **GIVEN** a save in each supported format
- **THEN** each file is written under that format's extension, whatever was typed

#### Scenario: The reported name is the one written

- **GIVEN** a destination the OS adjusted
- **THEN** the result names the file as written

### Requirement: One selection feeds every destination

The turns, and the reader's choices about them, MUST be resolved by ONE pure module that
both destinations read.

If each surface filtered for itself, a turn the reader deselected would have two chances
to survive into one of them.

A system turn MUST be dropped before the reader can choose, and MUST NOT be selectable
back in. An export omits raw system prompts, and a checkbox that could re-admit one would
make that requirement a preference.

An empty turn MUST be dropped too: a heading over nothing is not a section, and a
streaming turn cancelled before its first token leaves exactly that.

Selected turns MUST appear in transcript order regardless of the order the caller holds
the selection in.

#### Scenario: A deselected turn reaches neither destination

- **GIVEN** a reader who deselected one turn
- **WHEN** the conversation is exported to either destination
- **THEN** that turn is absent from both

#### Scenario: A system turn cannot be selected

- **GIVEN** a conversation carrying a system turn
- **THEN** it is not offered for selection and not present in the output

#### Scenario: An empty turn is dropped

- **GIVEN** a turn whose text is blank
- **THEN** it is not offered and not exported

### Requirement: Internal payloads and reasoning leave only when asked for

Tool-call payloads MUST be excluded by default and MUST NOT be implied by any other
choice. Those payloads are the part of a transcript most likely to carry a connection
string, a row of somebody's data, or a file path.

When included, a call MUST be named with its status in plain words rather than the
projection's own vocabulary. The wire status describes bookkeeping; on a page read by
someone who was not watching the run, an "unanswered" call invites the reading that the
tool refused, when it means the transcript was captured before the result landed.

A payload MUST be carried verbatim, and a truncation marker MUST be carried with it. A
capped payload must not read as a complete one. Re-serializing to prove a payload is
valid JSON would rewrite something the reader asked to see verbatim, so a fence tag is
treated as a highlighter hint, not an assertion.

Reasoning MUST be excluded by default, and the dialog MUST say when there is none to
include rather than offering a switch that silently does nothing. Reasoning is never
persisted, so a reloaded conversation carries none.

#### Scenario: Tool payloads are off unless ticked

- **GIVEN** a conversation whose turns carry tool calls
- **WHEN** it is exported with the default choices
- **THEN** no call name, argument, or result appears

#### Scenario: A capped payload says so

- **GIVEN** an included payload the projection truncated
- **THEN** its heading states that it was truncated

#### Scenario: A reloaded conversation offers no reasoning

- **GIVEN** a conversation opened from storage
- **THEN** the dialog states that there is no reasoning to include

### Requirement: A Markdown export carries the model's own Markdown through

An exported document MUST be GitHub-flavoured Markdown with a title, a metadata block,
and one heading per turn.

Assistant prose MUST pass through untouched. It is already Markdown, so the tables,
fences, and headings the model wrote are exactly what the file should carry, and
reformatting them here could only degrade them.

The metadata block MUST state when the export happened and how many turns it holds, on
their own lines within one paragraph, and MUST be followed by a rule with a genuinely
blank line above it — a whitespace-only line would read as a thematic break in one parser
and as a setext underline in a stricter one, silently turning the last fact into a
heading.

An empty selection MUST produce a document that says so rather than an empty file.

The file MUST end with exactly one trailing newline.

A reasoning segment MUST be quoted with every line prefixed, because a marker on the
first line alone stops quoting at the first blank line inside the segment.

#### Scenario: A model's table survives the export

- **GIVEN** an answer containing a Markdown table
- **THEN** the exported file carries that table unchanged

#### Scenario: The metadata rule is a rule

- **WHEN** the exported document is parsed
- **THEN** the metadata's last fact is a fact, not a heading

#### Scenario: An empty selection says so

- **GIVEN** no turns selected
- **THEN** the document states that nothing was selected

### Requirement: PDF is written through the native save dialog

A PDF export MUST be written by the webview's print engine to a path the native save
dialog returns. The renderer sends only a suggested name. It MUST NOT call
`window.print()` on Windows: that call blocks the renderer on a print preview that can
fail to appear, which leaves Close and Cancel as no-ops.

The printable document MUST be a separate tree mounted beside the app root, never the
transcript with parts hidden. The transcript is virtualized, so the turns a reader
selected are mostly absent from the DOM and printing the live surface would quietly
produce whatever happened to be scrolled into view.

That tree MUST be inert on screen and MUST NOT be walked by assistive technology: it
costs no layout while the app is in use, and a hidden second copy of the transcript is
not something a screen reader should read.

The printed page MUST carry no interactive affordances. A printed page cannot be
clicked.

The export timestamp MUST be frozen when the print starts, so the sheet and its header
agree on one time.

A platform that cannot write a PDF MUST be detected and reported, and the reader MUST
be offered the Markdown route with their selection unchanged.

The export dialog MUST stay dismissible while a PDF write is in flight. Unmounting the
sheet aborts the in-app wait; a native save dialog the person already has open is
theirs to cancel.

#### Scenario: Printing exports what was selected, not what was scrolled into view

- **GIVEN** a long conversation with turns selected from the top
- **WHEN** it is exported as PDF
- **THEN** the printed document holds the selected turns

#### Scenario: A platform that cannot write a PDF offers Markdown

- **GIVEN** a platform that cannot write a PDF
- **WHEN** the reader asks for a PDF
- **THEN** they are told, the format switches to Markdown, and the selection is
  unchanged

#### Scenario: Close dismisses the dialog while a PDF write is in flight

- **WHEN** a PDF export has been asked for and has not yet finished
- **THEN** Close and Cancel still dismiss the dialog

#### Scenario: The printed page is not interactive

- **WHEN** the print sheet renders
- **THEN** it carries no buttons, links, or controls

## Provenance

Consolidated from the delta of `add-lean-report-export`, reconciled against the shipped
code. That delta was two requirements of coarse prose; almost everything above is new to
the baseline and describes shipped behaviour.

Two of the delta's statements needed correcting. It said an export "MUST omit raw system
prompts or internal tool JSON payloads unless explicitly checked by the user" — as
shipped, a system turn cannot be checked back in at all, and only tool payloads and
reasoning are optional. It also implied a PDF is rendered and saved directly; as shipped
now, Windows writes the PDF through WebView2 `PrintToPdf` and the native save dialog.
`window.print()` is not used on Windows because its print preview can block the
renderer indefinitely.
